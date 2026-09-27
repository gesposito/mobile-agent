//! Byte formats of the adb server protocol: smart-socket requests and statuses,
//! shell protocol v2 packets, sync messages and forward and reverse listings.

use std::io::{self, ErrorKind, Read};

/// Longest service name a request can carry, since its length travels as four
/// hex digits.
pub const MAX_REQUEST_LEN: usize = 0xffff;

/// Largest `DATA` chunk the sync protocol allows.
pub const SYNC_MAX_CHUNK: usize = 64 * 1024;

/// Longest remote path adbd's sync service accepts.
pub const SYNC_MAX_PATH: usize = 1024;

/// Shell protocol v2 packet ids, from adb's `shell_protocol.h`.
pub mod shell_id {
    pub const STDIN: u8 = 0;
    pub const STDOUT: u8 = 1;
    pub const STDERR: u8 = 2;
    pub const EXIT: u8 = 3;
    pub const CLOSE_STDIN: u8 = 4;
}

/// Regular-file type bits of `st_mode`, which sync `SEND` expects in its mode.
const S_IFREG: u32 = 0o100000;

/// Reply to a smart-socket request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Okay,
    Fail(String),
}

fn invalid(message: String) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

/// A request: the service name's length as four lowercase hex digits, then the
/// name. `None` when the name is longer than [`MAX_REQUEST_LEN`].
pub fn encode_request(service: &str) -> Option<Vec<u8>> {
    (service.len() <= MAX_REQUEST_LEN)
        .then(|| format!("{:04x}{service}", service.len()).into_bytes())
}

/// Reads `OKAY`, or `FAIL` and its hex-length message. Anything else is
/// `InvalidData`.
pub fn read_status<R: Read>(r: &mut R) -> io::Result<Status> {
    let mut id = [0u8; 4];
    r.read_exact(&mut id)?;
    match &id {
        b"OKAY" => Ok(Status::Okay),
        b"FAIL" => {
            let message = read_hex_block(r)?;
            Ok(Status::Fail(String::from_utf8_lossy(&message).into_owned()))
        }
        other => Err(invalid(format!(
            "expected OKAY or FAIL, got {:?}",
            String::from_utf8_lossy(other)
        ))),
    }
}

/// Reads four hex digits and that many bytes.
pub fn read_hex_block<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut hex = [0u8; 4];
    r.read_exact(&mut hex)?;
    let len = std::str::from_utf8(&hex)
        .ok()
        .filter(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|s| usize::from_str_radix(s, 16).ok())
        .ok_or_else(|| {
            invalid(format!(
                "bad hex length {:?}",
                String::from_utf8_lossy(&hex)
            ))
        })?;
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    Ok(body)
}

/// One shell protocol v2 packet from the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellPacket {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// The command's exit status; the last packet of a command.
    Exit(u8),
}

/// Output of one command run through shell protocol v2.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// `None` when the stream ended without an exit packet.
    pub exit: Option<u8>,
}

impl ShellOutput {
    /// Whether the command exited with status 0.
    pub fn success(&self) -> bool {
        self.exit == Some(0)
    }

    /// Standard output as text, with invalid UTF-8 replaced.
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Standard error as text, with invalid UTF-8 replaced.
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A host-to-device shell v2 packet: the id byte, a u32 little-endian length
/// and the payload.
pub fn shell_packet(id: u8, data: &[u8]) -> Vec<u8> {
    let len = u32::try_from(data.len()).expect("a shell packet fits in u32");
    let mut out = Vec::with_capacity(5 + data.len());
    out.push(id);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Reads the next packet. `Ok(None)` is a clean end of stream between packets;
/// a stream that ends inside a packet is `UnexpectedEof`. Packet ids the host
/// never receives from adbd are skipped.
pub fn read_shell_packet<R: Read>(r: &mut R) -> io::Result<Option<ShellPacket>> {
    loop {
        let mut header = [0u8; 5];
        let mut got = 0;
        while got < header.len() {
            match r.read(&mut header[got..]) {
                Ok(0) if got == 0 => return Ok(None),
                Ok(0) => {
                    return Err(io::Error::new(
                        ErrorKind::UnexpectedEof,
                        "stream ended inside a shell v2 header",
                    ));
                }
                Ok(n) => got += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let len = u32::from_le_bytes([header[1], header[2], header[3], header[4]]) as usize;
        let mut data = vec![0; len];
        r.read_exact(&mut data)?;
        match header[0] {
            shell_id::STDOUT => return Ok(Some(ShellPacket::Stdout(data))),
            shell_id::STDERR => return Ok(Some(ShellPacket::Stderr(data))),
            shell_id::EXIT => {
                let status = data
                    .first()
                    .copied()
                    .ok_or_else(|| invalid("empty shell v2 exit packet".into()))?;
                return Ok(Some(ShellPacket::Exit(status)));
            }
            _ => {}
        }
    }
}

/// Collects packets until the exit packet or the end of the stream.
pub fn read_shell_output<R: Read>(r: &mut R) -> io::Result<ShellOutput> {
    let mut out = ShellOutput::default();
    while let Some(packet) = read_shell_packet(r)? {
        match packet {
            ShellPacket::Stdout(data) => out.stdout.extend_from_slice(&data),
            ShellPacket::Stderr(data) => out.stderr.extend_from_slice(&data),
            ShellPacket::Exit(status) => {
                out.exit = Some(status);
                break;
            }
        }
    }
    Ok(out)
}

/// A sync request: the four-byte id, a u32 little-endian length, the argument.
pub fn sync_request(id: &[u8; 4], arg: &[u8]) -> Vec<u8> {
    let len = u32::try_from(arg.len()).expect("a sync argument fits in u32");
    let mut out = Vec::with_capacity(8 + arg.len());
    out.extend_from_slice(id);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(arg);
    out
}

/// The argument of `SEND`: the remote path, a comma and the mode in decimal,
/// with the regular-file type bits added to the permission bits in `mode`.
pub fn send_arg(remote: &str, mode: u32) -> String {
    format!("{remote},{}", S_IFREG | (mode & 0o7777))
}

/// Appends one `DATA` message to `buf`.
///
/// # Panics
///
/// When `data` is longer than [`SYNC_MAX_CHUNK`].
pub fn push_data_chunk(buf: &mut Vec<u8>, data: &[u8]) {
    assert!(
        data.len() <= SYNC_MAX_CHUNK,
        "sync DATA chunks are at most 64 KiB, got {} bytes",
        data.len()
    );
    buf.extend_from_slice(b"DATA");
    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
    buf.extend_from_slice(data);
}

/// Appends the `DONE` message that ends a `SEND`, carrying the file's mtime.
pub fn push_done(buf: &mut Vec<u8>, mtime: u32) {
    buf.extend_from_slice(b"DONE");
    buf.extend_from_slice(&mtime.to_le_bytes());
}

/// Reads the status that answers `DONE`: `Ok(Ok(()))` for `OKAY`, `Ok(Err)`
/// with adbd's message for `FAIL`, and `InvalidData` for anything else.
pub fn read_sync_status<R: Read>(r: &mut R) -> io::Result<Result<(), String>> {
    let mut msg = [0u8; 8];
    r.read_exact(&mut msg)?;
    let len = u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]) as usize;
    match &msg[..4] {
        b"OKAY" => Ok(Ok(())),
        b"FAIL" => {
            let mut text = vec![0; len];
            r.read_exact(&mut text)?;
            Ok(Err(String::from_utf8_lossy(&text).into_owned()))
        }
        other => Err(invalid(format!(
            "expected a sync OKAY or FAIL, got {:?}",
            String::from_utf8_lossy(other)
        ))),
    }
}

/// A forward or reverse rule, named by its two sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The host-side socket spec, such as `tcp:40123`.
    pub host: String,
    /// The device-side socket spec, such as `localabstract:mobile-agent`.
    pub device: String,
}

/// The three fields of a listing line, or `None` for any other line.
fn three_fields(line: &str) -> Option<[&str; 3]> {
    let mut fields = line.split_whitespace();
    let parsed = [fields.next()?, fields.next()?, fields.next()?];
    fields.next().is_none().then_some(parsed)
}

/// Rules of `serial` in a `list-forward` answer, whose lines read
/// `<serial> <host side> <device side>`.
pub fn parse_forward_listing(listing: &str, serial: &str) -> Vec<Rule> {
    listing
        .lines()
        .filter_map(three_fields)
        .filter(|[s, _, _]| *s == serial)
        .map(|[_, host, device]| Rule {
            host: host.to_owned(),
            device: device.to_owned(),
        })
        .collect()
}

/// Rules in adbd's `reverse:list-forward` answer, whose lines read
/// `<transport> <device side> <host side>`.
pub fn parse_reverse_listing(listing: &str) -> Vec<Rule> {
    listing
        .lines()
        .filter_map(three_fields)
        .map(|[_, device, host]| Rule {
            host: host.to_owned(),
            device: device.to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, ErrorKind};

    #[test]
    fn requests_carry_a_four_digit_lowercase_hex_length() {
        assert_eq!(
            encode_request("host:devices-l").unwrap(),
            b"000ehost:devices-l"
        );
        assert_eq!(
            encode_request("host:transport:emulator-5554").unwrap(),
            b"001chost:transport:emulator-5554"
        );
        assert_eq!(encode_request("").unwrap(), b"0000");
    }

    #[test]
    fn a_request_longer_than_four_hex_digits_allow_is_refused() {
        let longest = "x".repeat(MAX_REQUEST_LEN);
        assert_eq!(encode_request(&longest).unwrap()[..4], *b"ffff");
        assert_eq!(encode_request(&"x".repeat(MAX_REQUEST_LEN + 1)), None);
    }

    #[test]
    fn okay_fail_and_garbage_statuses() {
        assert_eq!(
            read_status(&mut Cursor::new(b"OKAY".to_vec())).unwrap(),
            Status::Okay
        );
        assert_eq!(
            read_status(&mut Cursor::new(b"FAIL000edevice offline".to_vec())).unwrap(),
            Status::Fail("device offline".into())
        );
        assert_eq!(
            read_status(&mut Cursor::new(b"FAIL0006closed".to_vec())).unwrap(),
            Status::Fail("closed".into())
        );
        let garbage = read_status(&mut Cursor::new(b"WHAT".to_vec())).unwrap_err();
        assert_eq!(garbage.kind(), ErrorKind::InvalidData);
        let short = read_status(&mut Cursor::new(b"OK".to_vec())).unwrap_err();
        assert_eq!(short.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn hex_length_blocks() {
        assert_eq!(
            read_hex_block(&mut Cursor::new(b"0005hello".to_vec())).unwrap(),
            b"hello"
        );
        assert_eq!(
            read_hex_block(&mut Cursor::new(b"0000".to_vec())).unwrap(),
            b""
        );
        assert_eq!(
            read_hex_block(&mut Cursor::new(b"000Ahelloworld".to_vec())).unwrap(),
            b"helloworld"
        );
        let bad = read_hex_block(&mut Cursor::new(b"zzzz".to_vec())).unwrap_err();
        assert_eq!(bad.kind(), ErrorKind::InvalidData);
        let short = read_hex_block(&mut Cursor::new(b"0005hel".to_vec())).unwrap_err();
        assert_eq!(short.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn shell_packets_carry_the_id_and_a_little_endian_length() {
        assert_eq!(
            shell_packet(shell_id::STDIN, b"abc"),
            [0, 3, 0, 0, 0, b'a', b'b', b'c']
        );
        assert_eq!(shell_packet(shell_id::CLOSE_STDIN, b""), [4, 0, 0, 0, 0]);
    }

    #[test]
    fn shell_v2_separates_streams_and_keeps_the_exit_status() {
        let bytes = [
            shell_packet(shell_id::STDOUT, b"ok\n"),
            shell_packet(shell_id::STDERR, b"warn"),
            shell_packet(shell_id::STDOUT, b"more"),
            shell_packet(shell_id::EXIT, &[7]),
        ]
        .concat();
        let out = read_shell_output(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(out.stdout, b"ok\nmore");
        assert_eq!(out.stderr, b"warn");
        assert_eq!(out.exit, Some(7));
        assert!(!out.success());
    }

    #[test]
    fn packets_read_one_at_a_time_end_cleanly() {
        let bytes = [
            shell_packet(shell_id::STDOUT, b"a"),
            shell_packet(shell_id::EXIT, &[0]),
        ]
        .concat();
        let mut r = Cursor::new(bytes);
        assert_eq!(
            read_shell_packet(&mut r).unwrap(),
            Some(ShellPacket::Stdout(b"a".to_vec()))
        );
        assert_eq!(
            read_shell_packet(&mut r).unwrap(),
            Some(ShellPacket::Exit(0))
        );
        assert_eq!(read_shell_packet(&mut r).unwrap(), None);
    }

    #[test]
    fn shell_v2_without_an_exit_packet_has_no_status() {
        let out =
            read_shell_output(&mut Cursor::new(shell_packet(shell_id::STDOUT, b"ok"))).unwrap();
        assert_eq!(out.stdout, b"ok");
        assert_eq!(out.exit, None);
        assert!(!out.success());
    }

    #[test]
    fn a_truncated_shell_v2_packet_is_unexpected_eof() {
        let header_only = read_shell_output(&mut Cursor::new(vec![1, 10, 0])).unwrap_err();
        assert_eq!(header_only.kind(), ErrorKind::UnexpectedEof);
        let short_body =
            read_shell_output(&mut Cursor::new(vec![1, 10, 0, 0, 0, b'o'])).unwrap_err();
        assert_eq!(short_body.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn unknown_shell_packet_ids_are_skipped() {
        let bytes = [
            shell_packet(5, b"24x80"),
            shell_packet(shell_id::STDOUT, b"ok"),
            shell_packet(shell_id::EXIT, &[0]),
        ]
        .concat();
        let out = read_shell_output(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(out.stdout, b"ok");
        assert!(out.success());
    }

    #[test]
    fn sync_messages_use_little_endian_lengths() {
        assert_eq!(
            sync_request(b"SEND", b"/data/local/tmp/x,33188"),
            [
                b"SEND".as_slice(),
                &23u32.to_le_bytes(),
                b"/data/local/tmp/x,33188"
            ]
            .concat()
        );
        let mut buf = b"prefix".to_vec();
        push_data_chunk(&mut buf, b"abc");
        push_done(&mut buf, 0x0102_0304);
        assert_eq!(
            buf,
            [
                b"prefix".as_slice(),
                b"DATA",
                &3u32.to_le_bytes(),
                b"abc",
                b"DONE",
                &[4, 3, 2, 1]
            ]
            .concat()
        );
    }

    #[test]
    fn a_full_chunk_is_allowed() {
        let mut buf = Vec::new();
        push_data_chunk(&mut buf, &vec![0; SYNC_MAX_CHUNK]);
        assert_eq!(buf.len(), 8 + SYNC_MAX_CHUNK);
        assert_eq!(buf[4..8], (SYNC_MAX_CHUNK as u32).to_le_bytes());
    }

    #[test]
    #[should_panic(expected = "64 KiB")]
    fn a_chunk_over_64_kib_is_refused() {
        push_data_chunk(&mut Vec::new(), &vec![0; SYNC_MAX_CHUNK + 1]);
    }

    #[test]
    fn the_send_mode_marks_a_regular_file() {
        assert_eq!(
            send_arg("/data/local/tmp/a.jar", 0o644),
            "/data/local/tmp/a.jar,33188"
        );
        assert_eq!(
            send_arg("/data/local/tmp/run", 0o755),
            "/data/local/tmp/run,33261"
        );
        // Only permission bits are taken from the caller.
        assert_eq!(send_arg("/x", 0o170644), "/x,33188");
    }

    #[test]
    fn sync_status_okay_and_fail() {
        let okay = [b"OKAY".as_slice(), &[0; 4]].concat();
        assert_eq!(read_sync_status(&mut Cursor::new(okay)).unwrap(), Ok(()));
        let fail = [b"FAIL".as_slice(), &6u32.to_le_bytes(), b"denied"].concat();
        assert_eq!(
            read_sync_status(&mut Cursor::new(fail)).unwrap(),
            Err("denied".to_owned())
        );
        let other = [b"DATA".as_slice(), &[0; 4]].concat();
        let err = read_sync_status(&mut Cursor::new(other)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn forward_listing_keeps_this_serials_rules() {
        let listing = "emulator-5554 tcp:40123 localabstract:mobile-agent\n\
                       emulator-5556 tcp:40124 localabstract:other\n\
                       emulator-5554 tcp:8081 tcp:8081\n";
        assert_eq!(
            parse_forward_listing(listing, "emulator-5554"),
            [
                Rule {
                    host: "tcp:40123".into(),
                    device: "localabstract:mobile-agent".into()
                },
                Rule {
                    host: "tcp:8081".into(),
                    device: "tcp:8081".into()
                },
            ]
        );
        assert!(parse_forward_listing(listing, "R58M123ABC").is_empty());
        assert!(parse_forward_listing("", "emulator-5554").is_empty());
    }

    #[test]
    fn reverse_listing_reads_the_device_side_first() {
        let listing =
            "host-16 localabstract:s6r_1 tcp:5000\nhost-16 tcp:8081 tcp:8081\nmalformed\n";
        assert_eq!(
            parse_reverse_listing(listing),
            [
                Rule {
                    host: "tcp:5000".into(),
                    device: "localabstract:s6r_1".into()
                },
                Rule {
                    host: "tcp:8081".into(),
                    device: "tcp:8081".into()
                },
            ]
        );
        assert!(parse_reverse_listing("").is_empty());
    }
}
