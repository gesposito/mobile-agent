//! Client for the adb server's smart-socket protocol (design D3).
//!
//! The driver talks to the local adb server on port 5037 directly instead of
//! spawning the `adb` binary, which costs 16 to 22 ms more per operation
//! (spike S6). Each request opens its own server connection, so no state is
//! shared between calls and a failed request never affects the next one.
//!
//! - [`AdbServer::devices`] keeps every connection state, including
//!   unauthorized, offline and `no permissions`, and [`AdbServer::targets`]
//!   adds what the driver reports about each target.
//! - [`Device::shell`] uses shell protocol v2, which keeps the exit status and a
//!   separate stderr.
//! - [`Device::push`] sends sync `SEND` in 64 KiB chunks, each header written
//!   together with its data.
//! - [`Device::forward`] and [`Device::reverse`] read through the second status,
//!   so a refusal by the server or by adbd surfaces as [`Error::Fail`].
//! - [`Device::open_service`] opens a device service such as
//!   `localabstract:<name>` without installing a rule; a missing listener fails
//!   the open with `closed`. This is how the driver reaches the agent.
//!
//! The client is blocking. Every connection has a connect deadline and read and
//! write deadlines (10 s by default), and each call takes a few milliseconds, so
//! async callers run it on a blocking thread.

mod devices;
mod error;
mod wire;

use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wire::{SYNC_MAX_PATH, Status, shell_id};

pub use devices::{DeviceEntry, PROP_QUERY, State, Target, parse_devices};
pub use error::Error;
pub use wire::{Rule, SYNC_MAX_CHUNK, ShellOutput, ShellPacket};

/// Result of an adb request.
pub type Result<T> = std::result::Result<T, Error>;

/// Port of the adb server when [`SERVER_PORT_VAR`] is unset.
pub const DEFAULT_SERVER_PORT: u16 = 5037;

/// Environment variable naming the adb server's port, as the `adb` binary reads it.
pub const SERVER_PORT_VAR: &str = "ANDROID_ADB_SERVER_PORT";

/// Deadline for connecting to the server.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Default deadline for each read and write on a server connection.
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// The port in a [`SERVER_PORT_VAR`] value; the default port when the value is
/// absent, empty or not a port number.
pub fn server_port(value: Option<&str>) -> u16 {
    value
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_SERVER_PORT)
}

/// Wraps an I/O error met while serving `request`.
fn io_error(request: &str) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        request: request.to_owned(),
        source,
    }
}

/// The encoded form of `service`, refused when it is too long to send.
fn encoded(service: &str) -> Result<Vec<u8>> {
    wire::encode_request(service).ok_or_else(|| Error::Invalid {
        request: service.chars().take(80).collect(),
        detail: format!(
            "{} bytes is longer than the {}-byte limit of a request",
            service.len(),
            wire::MAX_REQUEST_LEN
        ),
    })
}

/// Sends one encoded request and reads its first status.
fn send_request(s: &mut TcpStream, service: &str, bytes: &[u8]) -> Result<()> {
    s.write_all(bytes).map_err(io_error(service))?;
    expect_okay(s, service)
}

/// Reads a status and turns `FAIL` into [`Error::Fail`].
fn expect_okay(s: &mut TcpStream, request: &str) -> Result<()> {
    match wire::read_status(s).map_err(io_error(request))? {
        Status::Okay => Ok(()),
        Status::Fail(message) => Err(Error::Fail {
            request: request.to_owned(),
            message,
        }),
    }
}

/// Everything the peer sends until it closes the connection.
fn read_rest(s: &mut TcpStream, request: &str) -> Result<Vec<u8>> {
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).map_err(io_error(request))?;
    Ok(rest)
}

/// The port in the hex block a server sends after binding a TCP listener, or
/// `None` when nothing followed the statuses.
fn bound_port(rest: &[u8], request: &str) -> Result<Option<u16>> {
    if rest.is_empty() {
        return Ok(None);
    }
    let protocol = |detail: String| Error::Protocol {
        request: request.to_owned(),
        detail,
    };
    let body = wire::read_hex_block(&mut &rest[..])
        .map_err(|e| protocol(format!("unreadable port after the status: {e}")))?;
    let text = String::from_utf8_lossy(&body);
    text.trim()
        .parse()
        .map(Some)
        .map_err(|_| protocol(format!("bound port {text:?} is not a port number")))
}

/// Fills `buf` from `r` unless the input ends first; returns the bytes read.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Where a push reads its bytes from.
enum Source<'a> {
    Bytes(&'a [u8]),
    File(&'a Path, File),
}

/// How adbd connects a shell command's standard streams.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Stdio {
    /// Pipes, which keep binary output intact and stderr separate. C programs
    /// such as `getevent` then buffer their output in blocks.
    #[default]
    Raw,
    /// A pseudo-terminal: programs flush every line, stderr is merged into
    /// stdout and line ends arrive as `\r\n`.
    Pty,
}

/// An adb server, reached over TCP on the loopback interface.
#[derive(Debug, Clone)]
pub struct AdbServer {
    addr: SocketAddr,
    io_timeout: Duration,
}

impl AdbServer {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            io_timeout: DEFAULT_IO_TIMEOUT,
        }
    }

    /// The server on 127.0.0.1 at the port [`SERVER_PORT_VAR`] names, else 5037.
    pub fn from_env() -> Self {
        let port = server_port(std::env::var(SERVER_PORT_VAR).ok().as_deref());
        Self::new(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
    }

    /// Sets the deadline for each read and write on a server connection.
    pub fn with_io_timeout(mut self, timeout: Duration) -> Self {
        self.io_timeout = timeout;
        self
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Every target the server knows, in every state (`host:devices-l`).
    pub fn devices(&self) -> Result<Vec<DeviceEntry>> {
        let request = "host:devices-l";
        let mut s = self.request(request)?;
        let listing = wire::read_hex_block(&mut s).map_err(io_error(request))?;
        Ok(parse_devices(&String::from_utf8_lossy(&listing)))
    }

    /// [`Self::devices`] with each online target's API level, model and emulator
    /// flag read through one shell command. A target whose properties cannot be
    /// read is still listed, without them.
    pub fn targets(&self) -> Result<Vec<Target>> {
        Ok(self
            .devices()?
            .into_iter()
            .map(|entry| {
                let props = (entry.state == State::Device)
                    .then(|| self.device(&entry.serial).shell(PROP_QUERY).ok())
                    .flatten()
                    .filter(ShellOutput::success)
                    .map(|out| out.stdout_text());
                Target::new(entry, props.as_deref())
            })
            .collect())
    }

    /// A new connection with this server's deadlines.
    fn connect(&self) -> Result<TcpStream> {
        let s = TcpStream::connect_timeout(&self.addr, CONNECT_TIMEOUT).map_err(|source| {
            Error::ServerUnavailable {
                addr: self.addr,
                source,
            }
        })?;
        let request = "connect";
        s.set_nodelay(true).map_err(io_error(request))?;
        s.set_read_timeout(Some(self.io_timeout))
            .map_err(io_error(request))?;
        s.set_write_timeout(Some(self.io_timeout))
            .map_err(io_error(request))?;
        Ok(s)
    }

    /// A new connection that has sent `service` and read an `OKAY` for it.
    fn request(&self, service: &str) -> Result<TcpStream> {
        let bytes = encoded(service)?;
        let mut s = self.connect()?;
        send_request(&mut s, service, &bytes)?;
        Ok(s)
    }

    /// The device with this serial; nothing is sent until a request is made.
    pub fn device(&self, serial: &str) -> Device {
        Device {
            server: self.clone(),
            serial: serial.to_owned(),
        }
    }
}

/// One device on an [`AdbServer`], named by its serial.
#[derive(Debug, Clone)]
pub struct Device {
    server: AdbServer,
    serial: String,
}

impl Device {
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Runs `command` with shell protocol v2 and collects its output. Standard
    /// input is closed at once, so a command that reads it sees end of file.
    pub fn shell(&self, command: &str) -> Result<ShellOutput> {
        let mut stream = self.shell_stream(command)?;
        // A command that already exited may have closed the stream; its output
        // is still readable, so a failed close is not an error.
        let _ = stream.close_stdin();
        stream.finish()
    }

    /// Starts `command` with shell protocol v2 and hands back the stream, for
    /// output that must be read as it arrives. Closing the stream makes adbd
    /// send SIGHUP to the command.
    pub fn shell_stream(&self, command: &str) -> Result<ShellStream> {
        self.shell_stream_with(command, Stdio::Raw)
    }

    /// [`Self::shell_stream`] with the command's streams connected as `stdio`
    /// says.
    pub fn shell_stream_with(&self, command: &str, stdio: Stdio) -> Result<ShellStream> {
        let streams = match stdio {
            Stdio::Raw => "raw",
            Stdio::Pty => "pty",
        };
        let request = format!("shell,v2,{streams}:{command}");
        let stream = self.service(&request)?;
        Ok(ShellStream { stream, request })
    }

    /// Copies the local file at `local` to `remote` with permission bits `mode`.
    ///
    /// adbd copies the owner's bits to group and other, so 0o644 arrives as
    /// 0666 and 0o755 as 0777; restrict the file afterwards if that matters.
    pub fn push(&self, local: &Path, remote: &str, mode: u32) -> Result<()> {
        let file = File::open(local).map_err(|source| Error::Local {
            path: local.to_owned(),
            source,
        })?;
        self.send(Source::File(local, file), remote, mode)
    }

    /// Writes `data` to `remote` with permission bits `mode`, which adbd widens
    /// as [`Self::push`] describes.
    pub fn push_bytes(&self, data: &[u8], remote: &str, mode: u32) -> Result<()> {
        self.send(Source::Bytes(data), remote, mode)
    }

    /// Sync `SEND`: the header goes out with the first chunk, each later
    /// `DATA` header with its chunk, then `DONE` and adbd's status.
    fn send(&self, source: Source<'_>, remote: &str, mode: u32) -> Result<()> {
        let request = format!("push {remote}");
        if remote.len() > SYNC_MAX_PATH {
            return Err(Error::Invalid {
                request,
                detail: format!("remote paths are at most {SYNC_MAX_PATH} bytes"),
            });
        }
        let mut s = self.service("sync:")?;
        let mut buf = wire::sync_request(b"SEND", wire::send_arg(remote, mode).as_bytes());
        let mut send_chunk = |buf: &mut Vec<u8>, chunk: &[u8]| {
            wire::push_data_chunk(buf, chunk);
            let sent = write_or_sync_fail(&mut s, buf, &request);
            buf.clear();
            sent
        };
        match source {
            Source::Bytes(data) => {
                for chunk in data.chunks(wire::SYNC_MAX_CHUNK) {
                    send_chunk(&mut buf, chunk)?;
                }
            }
            Source::File(path, mut file) => {
                let mut chunk = vec![0; wire::SYNC_MAX_CHUNK];
                loop {
                    let n = read_full(&mut file, &mut chunk).map_err(|source| Error::Local {
                        path: path.to_owned(),
                        source,
                    })?;
                    if n == 0 {
                        break;
                    }
                    send_chunk(&mut buf, &chunk[..n])?;
                }
            }
        }
        let mtime = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        wire::push_done(&mut buf, mtime);
        write_or_sync_fail(&mut s, &buf, &request)?;
        match wire::read_sync_status(&mut s).map_err(io_error(&request))? {
            Ok(()) => {
                // The transfer is complete; a failed goodbye changes nothing.
                let _ = s.write_all(b"QUIT\0\0\0\0");
                Ok(())
            }
            Err(message) => Err(Error::Fail { request, message }),
        }
    }

    /// Installs `adb forward <host> <device>`. Returns the host port when the
    /// host side is a TCP socket; `tcp:0` lets the server pick a free one.
    pub fn forward(&self, host: &str, device: &str) -> Result<Option<u16>> {
        let request = self.host_request(&format!("forward:{host};{device}"));
        let mut s = self.server.request(&request)?;
        expect_okay(&mut s, &request)?;
        // After binding a TCP listener the server sends its port.
        let rest = read_rest(&mut s, &request)?;
        bound_port(&rest, &request)
    }

    /// Forwards a free host TCP port, chosen by the server, to `device`.
    pub fn forward_tcp(&self, device: &str) -> Result<u16> {
        let request = self.host_request(&format!("forward:tcp:0;{device}"));
        self.forward("tcp:0", device)?
            .ok_or_else(|| Error::Protocol {
                request,
                detail: "the server bound no TCP port".into(),
            })
    }

    /// Removes the forward rule whose host side is `host`.
    pub fn remove_forward(&self, host: &str) -> Result<()> {
        let request = self.host_request(&format!("killforward:{host}"));
        let mut s = self.server.request(&request)?;
        expect_okay(&mut s, &request)
    }

    /// This device's forward rules.
    pub fn forwards(&self) -> Result<Vec<Rule>> {
        let request = self.host_request("list-forward");
        let mut s = self.server.request(&request)?;
        let listing = wire::read_hex_block(&mut s).map_err(io_error(&request))?;
        Ok(wire::parse_forward_listing(
            &String::from_utf8_lossy(&listing),
            &self.serial,
        ))
    }

    /// Installs `adb reverse <device> <host>`: adbd listens on `device` and
    /// relays each connection to `host`.
    ///
    /// Remove a reverse rule only after the connections through it have closed
    /// and nothing on the device will dial it again: when a device-side
    /// connection arrives for a rule the server has already dropped, the adb
    /// server aborts (spike S6).
    pub fn reverse(&self, device: &str, host: &str) -> Result<()> {
        self.adbd_request(&format!("reverse:forward:{device};{host}"))
    }

    /// Removes the reverse rule whose device side is `device`.
    pub fn remove_reverse(&self, device: &str) -> Result<()> {
        self.adbd_request(&format!("reverse:killforward:{device}"))
    }

    /// adbd's reverse rules on this device.
    pub fn reverses(&self) -> Result<Vec<Rule>> {
        let request = "reverse:list-forward";
        // adbd answers with the listing alone, without a status of its own.
        let mut s = self.service(request)?;
        let listing = wire::read_hex_block(&mut s).map_err(io_error(request))?;
        Ok(wire::parse_reverse_listing(&String::from_utf8_lossy(
            &listing,
        )))
    }

    /// Opens a device service such as `localabstract:<name>` or `tcp:<port>` as
    /// a byte stream, with no forward rule. A service nothing listens on fails
    /// with [`Error::Fail`] and the message `closed`. The stream is returned
    /// without read or write deadlines; the caller sets its own.
    pub fn open_service(&self, service: &str) -> Result<TcpStream> {
        let s = self.service(service)?;
        s.set_read_timeout(None).map_err(io_error(service))?;
        s.set_write_timeout(None).map_err(io_error(service))?;
        Ok(s)
    }

    /// [`Self::open_service`] for the abstract Unix socket `name`.
    pub fn open_abstract(&self, name: &str) -> Result<TcpStream> {
        self.open_service(&format!("localabstract:{name}"))
    }

    /// A host request scoped to this serial, such as `forward:...`.
    fn host_request(&self, request: &str) -> String {
        format!("host-serial:{}:{request}", self.serial)
    }

    /// A connection switched to this device's transport that has opened
    /// `service`, keeping the server's deadlines. Both requests are encoded
    /// before connecting, so an oversized one never reaches the server.
    fn service(&self, service: &str) -> Result<TcpStream> {
        let transport = format!("host:transport:{}", self.serial);
        let switch = encoded(&transport)?;
        let open = encoded(service)?;
        let mut s = self.server.connect()?;
        send_request(&mut s, &transport, &switch)?;
        send_request(&mut s, service, &open)?;
        Ok(s)
    }

    /// A request adbd answers with a status of its own after the server's
    /// `OKAY`, such as `reverse:forward:...`.
    fn adbd_request(&self, request: &str) -> Result<()> {
        let mut s = self.service(request)?;
        expect_okay(&mut s, request)
    }
}

/// Writes `bytes` of a sync transfer. adbd reports a file it cannot create with
/// `FAIL` and closes the connection, so a failed write returns that message
/// when it can be read.
fn write_or_sync_fail(s: &mut TcpStream, bytes: &[u8], request: &str) -> Result<()> {
    let Err(write_error) = s.write_all(bytes) else {
        return Ok(());
    };
    match wire::read_sync_status(s) {
        Ok(Err(message)) => Err(Error::Fail {
            request: request.to_owned(),
            message,
        }),
        _ => Err(io_error(request)(write_error)),
    }
}

/// A running shell v2 command whose packets are read as they arrive.
#[derive(Debug)]
pub struct ShellStream {
    stream: TcpStream,
    request: String,
}

impl ShellStream {
    /// The next packet, or `None` once the command has ended and the stream closed.
    pub fn read_packet(&mut self) -> Result<Option<ShellPacket>> {
        wire::read_shell_packet(&mut self.stream).map_err(io_error(&self.request))
    }

    /// Writes `data` to the command's standard input.
    pub fn write_stdin(&mut self, data: &[u8]) -> Result<()> {
        self.stream
            .write_all(&wire::shell_packet(shell_id::STDIN, data))
            .map_err(io_error(&self.request))
    }

    /// Closes the command's standard input.
    pub fn close_stdin(&mut self) -> Result<()> {
        self.stream
            .write_all(&wire::shell_packet(shell_id::CLOSE_STDIN, b""))
            .map_err(io_error(&self.request))
    }

    /// Replaces the deadline for each read; `None` waits indefinitely.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.stream
            .set_read_timeout(timeout)
            .map_err(io_error(&self.request))
    }

    /// Reads the remaining packets and returns everything the command wrote
    /// from now on, with its exit status.
    pub fn finish(mut self) -> Result<ShellOutput> {
        wire::read_shell_output(&mut self.stream).map_err(io_error(&self.request))
    }
}

#[cfg(test)]
mod tests {
    use super::wire::{shell_id, shell_packet};
    use super::*;
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};
    use std::time::Instant;

    type Script = Box<dyn FnOnce(&mut TcpStream) + Send>;

    /// Stands in for the adb server: serves one scripted exchange per accepted
    /// connection, in order. A failed expectation inside a script fails the test
    /// at [`FakeServer::finish`].
    struct FakeServer {
        addr: SocketAddr,
        handle: Option<JoinHandle<()>>,
    }

    impl FakeServer {
        fn start(scripts: Vec<Script>) -> Self {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = thread::spawn(move || {
                for (i, script) in scripts.into_iter().enumerate() {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    let mut s = loop {
                        match listener.accept() {
                            Ok((s, _)) => break s,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(Instant::now() < deadline, "no connection for script {i}");
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(e) => panic!("accept: {e}"),
                        }
                    };
                    s.set_nonblocking(false).unwrap();
                    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    script(&mut s);
                }
            });
            Self {
                addr,
                handle: Some(handle),
            }
        }

        fn server(&self) -> AdbServer {
            AdbServer::new(self.addr).with_io_timeout(Duration::from_secs(2))
        }

        /// Waits until every script has run.
        fn finish(mut self) {
            if let Err(panic) = self.handle.take().unwrap().join() {
                std::panic::resume_unwind(panic);
            }
        }
    }

    /// Reads one smart-socket request and checks its service name.
    fn expect_request(s: &mut TcpStream, want: &str) {
        let body = wire::read_hex_block(s).unwrap();
        assert_eq!(String::from_utf8_lossy(&body), want);
    }

    fn expect_bytes(s: &mut TcpStream, want: &[u8]) {
        let mut got = vec![0; want.len()];
        s.read_exact(&mut got).unwrap();
        assert_eq!(got, want);
    }

    fn reply(s: &mut TcpStream, bytes: &[u8]) {
        s.write_all(bytes).unwrap();
    }

    fn fail(message: &str) -> Vec<u8> {
        format!("FAIL{:04x}{message}", message.len()).into_bytes()
    }

    fn block(body: &str) -> Vec<u8> {
        format!("{:04x}{body}", body.len()).into_bytes()
    }

    /// Answers `host:transport:<serial>` with OKAY.
    fn switch_to(s: &mut TcpStream, serial: &str) {
        expect_request(s, &format!("host:transport:{serial}"));
        reply(s, b"OKAY");
    }

    const LISTING: &str = "emulator-5554 device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
        R58M123ABC unauthorized usb:1-1 transport_id:2\n\
        emulator-5556 offline transport_id:3\n\
        0123456789ABCDEF no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:1-2 transport_id:4\n";

    fn serve_listing(s: &mut TcpStream) {
        expect_request(s, "host:devices-l");
        reply(s, b"OKAY");
        reply(s, &block(LISTING));
    }

    #[test]
    fn the_server_port_comes_from_the_environment_value() {
        assert_eq!(server_port(None), DEFAULT_SERVER_PORT);
        assert_eq!(server_port(Some("")), DEFAULT_SERVER_PORT);
        assert_eq!(server_port(Some("5038")), 5038);
        assert_eq!(server_port(Some(" 5039 ")), 5039);
        assert_eq!(server_port(Some("not a port")), DEFAULT_SERVER_PORT);
        assert_eq!(server_port(Some("70000")), DEFAULT_SERVER_PORT);
    }

    #[test]
    fn devices_keep_every_state() {
        let fake = FakeServer::start(vec![Box::new(serve_listing)]);
        let entries = fake.server().devices().unwrap();
        fake.finish();
        let states: Vec<_> = entries.iter().map(|e| e.state.clone()).collect();
        assert_eq!(
            states,
            [
                State::Device,
                State::Unauthorized,
                State::Offline,
                State::NoPermissions(
                    "no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html]".into()
                ),
            ]
        );
    }

    /// Spec scenario "Unauthorized device" (android-driver, Target discovery).
    #[test]
    fn an_unauthorized_device_is_listed_with_instructions() {
        let fake = FakeServer::start(vec![
            Box::new(serve_listing),
            // Only the online target is asked for its properties.
            Box::new(|s| {
                switch_to(s, "emulator-5554");
                expect_request(s, &format!("shell,v2,raw:{PROP_QUERY}"));
                reply(s, b"OKAY");
                expect_bytes(s, &shell_packet(shell_id::CLOSE_STDIN, b""));
                reply(
                    s,
                    &shell_packet(shell_id::STDOUT, b"36\n1\n\nsdk_gphone64_arm64\n"),
                );
                reply(s, &shell_packet(shell_id::EXIT, &[0]));
            }),
        ]);
        let targets = fake.server().targets().unwrap();
        fake.finish();

        let serials: Vec<_> = targets.iter().map(|t| t.serial.as_str()).collect();
        assert_eq!(
            serials,
            [
                "emulator-5554",
                "R58M123ABC",
                "emulator-5556",
                "0123456789ABCDEF"
            ]
        );
        let online = &targets[0];
        assert_eq!(online.api_level, Some(36));
        assert!(online.emulator);
        assert_eq!(online.advice(), None);

        let unauthorized = &targets[1];
        assert_eq!(unauthorized.state, State::Unauthorized);
        assert_eq!(unauthorized.api_level, None);
        assert!(!unauthorized.emulator);
        let advice = unauthorized
            .advice()
            .expect("advice for an unauthorized target");
        assert!(advice.contains("Allow USB debugging"), "{advice}");
        assert!(advice.contains("on the device"), "{advice}");

        assert_eq!(targets[2].state, State::Offline);
        assert!(targets[2].advice().is_some());
        assert!(targets[3].advice().unwrap().contains("udev"));
    }

    #[test]
    fn a_target_whose_properties_cannot_be_read_stays_listed() {
        let fake = FakeServer::start(vec![
            Box::new(|s| {
                expect_request(s, "host:devices-l");
                reply(s, b"OKAY");
                reply(s, &block("emulator-5554 device transport_id:1\n"));
            }),
            Box::new(|s| {
                expect_request(s, "host:transport:emulator-5554");
                reply(s, &fail("device offline"));
            }),
        ]);
        let targets = fake.server().targets().unwrap();
        fake.finish();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].state, State::Device);
        assert_eq!(targets[0].api_level, None);
        assert!(targets[0].emulator);
    }

    #[test]
    fn shell_closes_stdin_and_splits_stdout_stderr_and_exit() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "shell,v2,raw:echo out; echo err >&2; exit 3");
            reply(s, b"OKAY");
            expect_bytes(s, &shell_packet(shell_id::CLOSE_STDIN, b""));
            reply(s, &shell_packet(shell_id::STDOUT, b"out\n"));
            reply(s, &shell_packet(shell_id::STDERR, b"err\n"));
            reply(s, &shell_packet(shell_id::EXIT, &[3]));
        })]);
        let out = fake
            .server()
            .device("emulator-5554")
            .shell("echo out; echo err >&2; exit 3")
            .unwrap();
        fake.finish();
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
        assert_eq!(out.exit, Some(3));
    }

    #[test]
    fn a_shell_stream_hands_over_packets_as_they_arrive() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "shell,v2,raw:getevent -l");
            reply(s, b"OKAY");
            reply(s, &shell_packet(shell_id::STDOUT, b"add device 1\n"));
            expect_bytes(s, &shell_packet(shell_id::STDIN, b"x"));
            reply(s, &shell_packet(shell_id::STDERR, b"late\n"));
            reply(s, &shell_packet(shell_id::EXIT, &[0]));
        })]);
        let mut stream = fake
            .server()
            .device("emulator-5554")
            .shell_stream("getevent -l")
            .unwrap();
        assert_eq!(
            stream.read_packet().unwrap(),
            Some(ShellPacket::Stdout(b"add device 1\n".to_vec()))
        );
        stream.write_stdin(b"x").unwrap();
        let rest = stream.finish().unwrap();
        fake.finish();
        assert_eq!(rest.stderr, b"late\n");
        assert_eq!(rest.exit, Some(0));
    }

    #[test]
    fn a_pty_stream_asks_adbd_for_a_terminal() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "shell,v2,pty:getevent -l");
            reply(s, b"OKAY");
            reply(s, &shell_packet(shell_id::EXIT, &[0]));
        })]);
        let stream = fake
            .server()
            .device("emulator-5554")
            .shell_stream_with("getevent -l", Stdio::Pty)
            .unwrap();
        assert_eq!(stream.finish().unwrap().exit, Some(0));
        fake.finish();
    }

    #[test]
    fn push_sends_send_data_done_and_quit() {
        let data: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let expected = data.clone();
        let fake = FakeServer::start(vec![Box::new(move |s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "sync:");
            reply(s, b"OKAY");
            let arg = b"/data/local/tmp/agent.jar,33188";
            expect_bytes(s, &wire::sync_request(b"SEND", arg));
            let mut received = Vec::new();
            for len in [SYNC_MAX_CHUNK, expected.len() - SYNC_MAX_CHUNK] {
                let mut header = [0u8; 8];
                s.read_exact(&mut header).unwrap();
                assert_eq!(&header[..4], b"DATA");
                assert_eq!(
                    u32::from_le_bytes(header[4..].try_into().unwrap()) as usize,
                    len
                );
                let mut chunk = vec![0; len];
                s.read_exact(&mut chunk).unwrap();
                received.extend(chunk);
            }
            assert_eq!(received, expected);
            let mut done = [0u8; 8];
            s.read_exact(&mut done).unwrap();
            assert_eq!(&done[..4], b"DONE");
            reply(s, b"OKAY\0\0\0\0");
            expect_bytes(s, b"QUIT\0\0\0\0");
        })]);
        fake.server()
            .device("emulator-5554")
            .push_bytes(&data, "/data/local/tmp/agent.jar", 0o644)
            .unwrap();
        fake.finish();
    }

    #[test]
    fn a_push_adbd_refuses_reports_its_message() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "sync:");
            reply(s, b"OKAY");
            expect_bytes(s, &wire::sync_request(b"SEND", b"/system/x,33188"));
            let mut rest = [0u8; 8 + 1 + 8];
            s.read_exact(&mut rest).unwrap();
            let message = "couldn't create file: Read-only file system";
            reply(
                s,
                &[
                    b"FAIL".as_slice(),
                    &(message.len() as u32).to_le_bytes(),
                    message.as_bytes(),
                ]
                .concat(),
            );
        })]);
        let err = fake
            .server()
            .device("emulator-5554")
            .push_bytes(b"x", "/system/x", 0o644)
            .unwrap_err();
        fake.finish();
        assert_eq!(
            err.fail_message(),
            Some("couldn't create file: Read-only file system"),
            "{err}"
        );
    }

    #[test]
    fn forward_reads_both_statuses_and_the_bound_port() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            expect_request(
                s,
                "host-serial:emulator-5554:forward:tcp:0;localabstract:mobile-agent",
            );
            reply(s, b"OKAYOKAY");
            reply(s, &block("40123"));
        })]);
        let port = fake
            .server()
            .device("emulator-5554")
            .forward_tcp("localabstract:mobile-agent")
            .unwrap();
        fake.finish();
        assert_eq!(port, 40123);
    }

    #[test]
    fn a_forward_without_a_tcp_host_side_has_no_port() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            expect_request(
                s,
                "host-serial:emulator-5554:forward:localabstract:host-side;localabstract:agent",
            );
            reply(s, b"OKAYOKAY");
        })]);
        let port = fake
            .server()
            .device("emulator-5554")
            .forward("localabstract:host-side", "localabstract:agent")
            .unwrap();
        fake.finish();
        assert_eq!(port, None);
    }

    #[test]
    fn a_forward_the_server_refuses_reports_its_message() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            expect_request(s, "host-serial:emulator-5554:forward:tcp:1;tcp:2");
            reply(s, &fail("cannot bind listener: Permission denied"));
        })]);
        let err = fake
            .server()
            .device("emulator-5554")
            .forward("tcp:1", "tcp:2")
            .unwrap_err();
        fake.finish();
        assert_eq!(
            err.fail_message(),
            Some("cannot bind listener: Permission denied")
        );
    }

    #[test]
    fn reverse_reads_adbds_own_status() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "reverse:forward:tcp:70000;tcp:1");
            reply(s, b"OKAY");
            reply(s, &fail("cannot bind listener: bad port number '70000'"));
        })]);
        let err = fake
            .server()
            .device("emulator-5554")
            .reverse("tcp:70000", "tcp:1")
            .unwrap_err();
        fake.finish();
        assert_eq!(
            err.fail_message(),
            Some("cannot bind listener: bad port number '70000'")
        );
    }

    #[test]
    fn rules_are_listed_and_removed_with_serial_scoped_requests() {
        let fake = FakeServer::start(vec![
            Box::new(|s| {
                expect_request(s, "host-serial:emulator-5554:list-forward");
                reply(s, b"OKAY");
                reply(
                    s,
                    &block("emulator-5554 tcp:40123 localabstract:a\nemulator-5556 tcp:1 tcp:2\n"),
                );
            }),
            Box::new(|s| {
                expect_request(s, "host-serial:emulator-5554:killforward:tcp:40123");
                reply(s, b"OKAYOKAY");
            }),
            Box::new(|s| {
                switch_to(s, "emulator-5554");
                expect_request(s, "reverse:list-forward");
                reply(s, b"OKAY");
                reply(s, &block("host-16 localabstract:b tcp:5000\n"));
            }),
            Box::new(|s| {
                switch_to(s, "emulator-5554");
                expect_request(s, "reverse:killforward:localabstract:b");
                reply(s, b"OKAYOKAY");
            }),
        ]);
        let device = fake.server().device("emulator-5554");
        assert_eq!(
            device.forwards().unwrap(),
            [Rule {
                host: "tcp:40123".into(),
                device: "localabstract:a".into()
            }]
        );
        device.remove_forward("tcp:40123").unwrap();
        assert_eq!(
            device.reverses().unwrap(),
            [Rule {
                host: "tcp:5000".into(),
                device: "localabstract:b".into()
            }]
        );
        device.remove_reverse("localabstract:b").unwrap();
        fake.finish();
    }

    #[test]
    fn a_service_stream_opens_through_the_transport() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "localabstract:mobile-agent");
            reply(s, b"OKAY");
            reply(s, b"hello");
        })]);
        let mut stream = fake
            .server()
            .device("emulator-5554")
            .open_abstract("mobile-agent")
            .unwrap();
        let mut hello = [0u8; 5];
        stream.read_exact(&mut hello).unwrap();
        fake.finish();
        assert_eq!(&hello, b"hello");
        assert_eq!(stream.read_timeout().unwrap(), None);
    }

    #[test]
    fn a_service_stream_to_a_missing_socket_fails_with_closed() {
        let fake = FakeServer::start(vec![Box::new(|s| {
            switch_to(s, "emulator-5554");
            expect_request(s, "localabstract:nobody");
            reply(s, &fail("closed"));
        })]);
        let err = fake
            .server()
            .device("emulator-5554")
            .open_abstract("nobody")
            .unwrap_err();
        fake.finish();
        assert_eq!(err.fail_message(), Some("closed"), "{err}");
    }

    #[test]
    fn no_server_is_an_actionable_error() {
        let addr = {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            listener.local_addr().unwrap()
        };
        let err = AdbServer::new(addr).devices().unwrap_err();
        assert!(matches!(err, Error::ServerUnavailable { .. }), "{err:?}");
        assert!(err.to_string().contains("adb start-server"), "{err}");
    }

    #[test]
    fn a_silent_server_fails_at_the_io_deadline() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (done, wait) = std::sync::mpsc::channel::<()>();
        let holder = thread::spawn(move || {
            // Accepts, then holds the connection open without answering.
            let (s, _) = listener.accept().unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(5));
            drop(s);
        });
        let started = Instant::now();
        let err = AdbServer::new(addr)
            .with_io_timeout(Duration::from_millis(200))
            .devices()
            .unwrap_err();
        let elapsed = started.elapsed();
        done.send(()).unwrap();
        holder.join().unwrap();
        assert!(err.is_timeout(), "{err:?}");
        assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");
    }

    #[test]
    fn an_oversized_request_is_refused_before_connecting() {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 9));
        let command = "x".repeat(wire::MAX_REQUEST_LEN);
        let err = AdbServer::new(addr)
            .device("emulator-5554")
            .shell(&command)
            .unwrap_err();
        assert!(matches!(err, Error::Invalid { .. }), "{err:?}");
    }
}
