//! Integration tests for the adb client against a live device (task 5.1).
//!
//! They run only when `MOBILE_AGENT_ANDROID_SERIAL` names a connected device and
//! print a skip line otherwise. Pushed files are deleted and every rule is
//! removed; no process is started on the device.
//!
//! A live abstract socket comes from adbd itself: `adb reverse
//! localabstract:<name> tcp:<port>` makes adbd listen on `@<name>` and relay
//! each connection to a host listener, so forwards and service streams reach a
//! real listener with nothing running on the device.

mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use android::adb::{AdbServer, Device, Error, Rule, ShellPacket, State, Stdio};
use common::{pattern, percentile, serial_or_skip, unique_name};

const IO: Duration = Duration::from_secs(5);

fn device(serial: &str) -> Device {
    AdbServer::from_env().device(serial)
}

/// A host listener behind an adbd-owned abstract socket. Dropping it removes
/// the forward rules the test added, then the reverse rule. By then every
/// connection through the socket must be closed: a device-side connection
/// that reaches the server after it dropped the reverse rule aborts the server.
struct AbstractLoop {
    device: Device,
    name: String,
    listener: TcpListener,
    forwards: Vec<String>,
    removed: bool,
}

impl AbstractLoop {
    fn new(device: &Device, tag: &str) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind host listener");
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let name = unique_name(tag);
        device
            .reverse(&format!("localabstract:{name}"), &format!("tcp:{port}"))
            .expect("reverse to the host listener");
        Self {
            device: device.clone(),
            name,
            listener,
            forwards: Vec::new(),
            removed: false,
        }
    }

    fn device_spec(&self) -> String {
        format!("localabstract:{}", self.name)
    }

    fn host_spec(&self) -> String {
        format!("tcp:{}", self.listener.local_addr().unwrap().port())
    }

    /// Forwards a server-chosen host port to the abstract socket.
    fn forward(&mut self) -> u16 {
        let port = self
            .device
            .forward_tcp(&self.device_spec())
            .expect("forward to the abstract socket");
        self.forwards.push(format!("tcp:{port}"));
        port
    }

    /// The host end of the next connection adbd relays, within [`IO`].
    fn accept(&self) -> TcpStream {
        let deadline = Instant::now() + IO;
        loop {
            match self.listener.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).unwrap();
                    s.set_read_timeout(Some(IO)).unwrap();
                    return s;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "adbd relayed no connection");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("accept: {e}"),
            }
        }
    }

    /// Removes the rules, forwards first; the caller has closed its connections.
    fn remove(&mut self) {
        if self.removed {
            return;
        }
        self.removed = true;
        for host in self.forwards.drain(..) {
            let _ = self.device.remove_forward(&host);
        }
        let _ = self.device.remove_reverse(&self.device_spec());
    }
}

impl Drop for AbstractLoop {
    fn drop(&mut self) {
        self.remove();
    }
}

/// A device file removed when the test ends, pass or fail. Removal is retried
/// for a few seconds because the emulator's adb transport occasionally drops
/// and reconnects (spike S6 saw it four times in one session).
struct RemoteFile {
    device: Device,
    path: String,
}

impl RemoteFile {
    fn new(device: &Device, tag: &str) -> Self {
        Self {
            device: device.clone(),
            path: format!("/data/local/tmp/{}.bin", unique_name(tag)),
        }
    }
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        for _ in 0..5 {
            let removed = self.device.shell(&format!("rm -f {}", self.path));
            if removed.is_ok_and(|out| out.success()) {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
        eprintln!("could not remove {} from the device", self.path);
    }
}

/// A host file removed when the test ends, pass or fail.
struct LocalFile(std::path::PathBuf);

impl Drop for LocalFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Exchanges a greeting in each direction between the two ends of a relayed
/// connection.
fn exchange(client: &mut TcpStream, relayed: &mut TcpStream) {
    client.set_read_timeout(Some(IO)).unwrap();
    relayed.write_all(b"hello from the host listener").unwrap();
    let mut hello = [0u8; 28];
    client.read_exact(&mut hello).unwrap();
    assert_eq!(&hello, b"hello from the host listener");
    client.write_all(b"ping").unwrap();
    let mut ping = [0u8; 4];
    relayed.read_exact(&mut ping).unwrap();
    assert_eq!(&ping, b"ping");
}

#[test]
fn listing_shows_the_device() {
    let Some(serial) = serial_or_skip("listing_shows_the_device") else {
        return;
    };
    let entries = AdbServer::from_env().devices().expect("host:devices-l");
    let entry = entries
        .iter()
        .find(|e| e.serial == serial)
        .unwrap_or_else(|| panic!("{serial} is not listed: {entries:?}"));
    assert_eq!(entry.state, State::Device, "{entry:?}");
    assert!(entry.transport_id().is_some(), "{entry:?}");
    assert!(entry.model().is_some(), "{entry:?}");
}

#[test]
fn targets_report_the_api_level_and_the_emulator_flag() {
    let Some(serial) = serial_or_skip("targets_report_the_api_level_and_the_emulator_flag") else {
        return;
    };
    let targets = AdbServer::from_env().targets().expect("targets");
    let target = targets
        .iter()
        .find(|t| t.serial == serial)
        .unwrap_or_else(|| panic!("{serial} is not listed: {targets:?}"));
    let sdk = device(&serial)
        .shell("getprop ro.build.version.sdk")
        .unwrap();
    assert_eq!(target.api_level, sdk.stdout_text().trim().parse().ok());
    let qemu = device(&serial)
        .shell("getprop ro.boot.qemu; getprop ro.kernel.qemu")
        .unwrap();
    assert_eq!(
        target.emulator,
        qemu.stdout_text().lines().any(|l| l.trim() == "1")
    );
    assert_eq!(target.advice(), None);
}

#[test]
fn shell_v2_keeps_the_exit_status_and_stderr_apart() {
    let Some(serial) = serial_or_skip("shell_v2_keeps_the_exit_status_and_stderr_apart") else {
        return;
    };
    let out = device(&serial)
        .shell("echo out; echo err >&2; exit 3")
        .unwrap();
    assert_eq!(out.stdout, b"out\n");
    assert_eq!(out.stderr, b"err\n");
    assert_eq!(out.exit, Some(3));
}

#[test]
fn a_command_reading_stdin_sees_end_of_file() {
    let Some(serial) = serial_or_skip("a_command_reading_stdin_sees_end_of_file") else {
        return;
    };
    let started = Instant::now();
    let out = device(&serial).shell("cat; echo done").unwrap();
    assert_eq!(out.stdout, b"done\n");
    assert!(out.success());
    assert!(started.elapsed() < IO);
}

#[test]
fn closing_a_shell_stream_ends_the_command() {
    let Some(serial) = serial_or_skip("closing_a_shell_stream_ends_the_command") else {
        return;
    };
    let device = device(&serial);
    let mut stream = device.shell_stream("echo $$; exec sleep 60").unwrap();
    let pid = match stream.read_packet().unwrap() {
        Some(ShellPacket::Stdout(line)) => String::from_utf8(line).unwrap().trim().to_owned(),
        other => panic!("expected the pid, got {other:?}"),
    };
    assert!(device.shell(&format!("kill -0 {pid}")).unwrap().success());
    drop(stream);
    let deadline = Instant::now() + IO;
    while device.shell(&format!("kill -0 {pid}")).unwrap().success() {
        if Instant::now() > deadline {
            device.shell(&format!("kill {pid}")).unwrap();
            panic!("pid {pid} outlived its closed shell stream");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_pty_stream_runs_the_command_on_a_terminal() {
    let Some(serial) = serial_or_skip("a_pty_stream_runs_the_command_on_a_terminal") else {
        return;
    };
    let device = device(&serial);
    let piped = device.shell("tty").unwrap();
    assert_eq!(piped.stdout_text().trim(), "not a tty");
    let stream = device.shell_stream_with("tty", Stdio::Pty).unwrap();
    let out = stream.finish().unwrap();
    assert!(
        out.stdout_text().starts_with("/dev/pts/"),
        "{:?}",
        out.stdout_text()
    );
    assert!(out.stdout.ends_with(b"\r\n"), "{:?}", out.stdout_text());
}

#[test]
fn shell_round_trip() {
    let Some(serial) = serial_or_skip("shell_round_trip") else {
        return;
    };
    let device = device(&serial);
    for _ in 0..5 {
        device.shell("echo ok").unwrap();
    }
    let mut samples = Vec::new();
    for _ in 0..100 {
        let started = Instant::now();
        let out = device.shell("echo ok").unwrap();
        samples.push(started.elapsed().as_secs_f64() * 1e3);
        assert_eq!(out.stdout, b"ok\n");
    }
    let p50 = percentile(&mut samples, 50.0);
    let p95 = percentile(&mut samples, 95.0);
    let max = percentile(&mut samples, 100.0);
    eprintln!(
        "shell `echo ok` round trip, {} samples: p50 {p50:.2} ms, p95 {p95:.2} ms, max {max:.2} ms",
        samples.len()
    );
    common::under(
        "the shell round trip at p50",
        Duration::from_secs_f64(p50 / 1e3),
        Duration::from_millis(50),
    );
}

/// adbd copies the owner's permission bits to group and other, so 0o750
/// arrives as 0777.
#[test]
fn push_writes_the_bytes_with_the_mode() {
    let Some(serial) = serial_or_skip("push_writes_the_bytes_with_the_mode") else {
        return;
    };
    let device = device(&serial);
    // Three full chunks and a partial one.
    let data = pattern(3 * android::adb::SYNC_MAX_CHUNK + 123, 7);
    let remote = RemoteFile::new(&device, "push");
    let path = &remote.path;
    device.push_bytes(&data, path, 0o750).unwrap();
    let back = device
        .shell(&format!("cat {path}; stat -c %a {path}"))
        .unwrap();
    assert!(back.success(), "{}", back.stderr_text());
    let (content, mode) = back.stdout.split_at(data.len());
    assert!(content == data.as_slice(), "the pushed bytes differ");
    assert_eq!(mode, b"777\n");
}

#[test]
fn push_copies_a_local_file() {
    let Some(serial) = serial_or_skip("push_copies_a_local_file") else {
        return;
    };
    let device = device(&serial);
    let data = pattern(100_000, 11);
    let local = LocalFile(std::env::temp_dir().join(unique_name("push-file")));
    std::fs::write(&local.0, &data).unwrap();
    let remote = RemoteFile::new(&device, "push-file");
    device.push(&local.0, &remote.path, 0o644).unwrap();
    let back = device.shell(&format!("cat {}", remote.path)).unwrap();
    assert!(back.stdout == data, "the pushed bytes differ");
}

#[test]
fn a_push_adbd_refuses_reports_its_message() {
    let Some(serial) = serial_or_skip("a_push_adbd_refuses_reports_its_message") else {
        return;
    };
    let err = device(&serial)
        .push_bytes(b"x", "/system/mobile-agent-it.bin", 0o644)
        .unwrap_err();
    let message = err.fail_message().unwrap_or_else(|| panic!("{err:?}"));
    assert!(message.contains("couldn't create file"), "{message}");
}

#[test]
fn a_forwarded_port_connects() {
    let Some(serial) = serial_or_skip("a_forwarded_port_connects") else {
        return;
    };
    let device = device(&serial);
    let mut lp = AbstractLoop::new(&device, "fwd");
    let port = lp.forward();
    let rule = Rule {
        host: format!("tcp:{port}"),
        device: lp.device_spec(),
    };
    assert!(device.forwards().unwrap().contains(&rule));
    let reverse = Rule {
        host: lp.host_spec(),
        device: lp.device_spec(),
    };
    assert!(device.reverses().unwrap().contains(&reverse));
    {
        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let mut relayed = lp.accept();
        exchange(&mut client, &mut relayed);
    }
    lp.remove();
    assert!(!device.forwards().unwrap().contains(&rule));
    assert!(!device.reverses().unwrap().contains(&reverse));
}

#[test]
fn a_service_stream_reaches_a_live_abstract_socket() {
    let Some(serial) = serial_or_skip("a_service_stream_reaches_a_live_abstract_socket") else {
        return;
    };
    let device = device(&serial);
    let lp = AbstractLoop::new(&device, "svc");
    let mut client = device.open_abstract(&lp.name).expect("service stream");
    let mut relayed = lp.accept();
    exchange(&mut client, &mut relayed);
}

#[test]
fn a_service_stream_to_a_missing_socket_fails_with_closed() {
    let Some(serial) = serial_or_skip("a_service_stream_to_a_missing_socket_fails_with_closed")
    else {
        return;
    };
    let err = device(&serial)
        .open_abstract(&unique_name("missing"))
        .unwrap_err();
    assert_eq!(err.fail_message(), Some("closed"), "{err:?}");
}

#[test]
fn a_reverse_adbd_refuses_reports_its_error() {
    let Some(serial) = serial_or_skip("a_reverse_adbd_refuses_reports_its_error") else {
        return;
    };
    let err = device(&serial).reverse("tcp:70000", "tcp:1").unwrap_err();
    let message = err.fail_message().unwrap_or_else(|| panic!("{err:?}"));
    assert!(message.contains("cannot bind listener"), "{message}");
}

#[test]
fn a_missing_serial_fails_with_the_servers_message() {
    let Some(_) = serial_or_skip("a_missing_serial_fails_with_the_servers_message") else {
        return;
    };
    let err = device("emulator-1").shell("true").unwrap_err();
    assert!(matches!(err, Error::Fail { .. }), "{err:?}");
    assert!(err.to_string().contains("not found"), "{err}");
}
