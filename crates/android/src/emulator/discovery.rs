//! Emulator discovery files.
//!
//! A running emulator advertises itself with one `pid_<pid>.ini` file in its
//! discovery directory. The file holds `key=value` lines: the console port
//! (`port.serial`, which names the adb serial `emulator-<port>`), the gRPC
//! controller port (`grpc.port`, only with `-grpc`) and the bearer token
//! (`grpc.token`, only with `-grpc-use-token`). Tokens are base64 and may end
//! in `=`, so a line splits at its first `=` only.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Parsed contents of one discovery file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// Emulator process id, from the `pid_<pid>.ini` file name.
    pub pid: Option<u32>,
    /// Console port (`port.serial`); the adb serial is `emulator-<port>`.
    pub console_port: Option<u16>,
    /// adb port of the emulated device (`port.adb`).
    pub adb_port: Option<u16>,
    /// AVD name (`avd.name`).
    pub avd_name: Option<String>,
    /// Emulator version (`emulator.version`).
    pub emulator_version: Option<String>,
    /// gRPC controller port (`grpc.port`); absent without `-grpc`.
    pub grpc_port: Option<u16>,
    /// Bearer token (`grpc.token`); absent without `-grpc-use-token`.
    pub grpc_token: Option<String>,
    /// Every key and value as written, including keys not modelled above.
    pub entries: BTreeMap<String, String>,
}

/// A discovery file that could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// A port field held something other than a port number.
    BadPort { key: String, value: String },
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiscoveryError::BadPort { key, value } => {
                write!(f, "{key} is not a port number: {value:?}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

impl Discovery {
    /// Parses the text of a discovery file; `pid` comes from the file name.
    pub fn parse(contents: &str, pid: Option<u32>) -> Result<Self, DiscoveryError> {
        let _ = (contents, pid);
        todo!()
    }

    /// The adb serial of this emulator, `emulator-<console port>`.
    pub fn serial(&self) -> Option<String> {
        todo!()
    }

    /// The controller endpoint on loopback, when the file names a gRPC port.
    pub fn grpc_endpoint(&self) -> Option<String> {
        todo!()
    }

    /// The AVD's directory (`avd.dir`), which holds its `config.ini`.
    pub fn avd_dir(&self) -> Option<PathBuf> {
        todo!()
    }
}

/// The pid in a discovery file name: `pid_59226.ini` gives 59226.
pub fn pid_from_file_name(name: &str) -> Option<u32> {
    let _ = name;
    todo!()
}

/// The console port in an emulator serial: `emulator-5554` gives 5554.
pub fn console_port_from_serial(serial: &str) -> Option<u16> {
    let _ = serial;
    todo!()
}

/// The discovery whose console port matches `serial`. When several match, as
/// when a crashed emulator left a stale file next to the live one, the last one
/// in `found` wins; [`scan_dir`] orders files oldest first.
pub fn find_for_serial<'a>(found: &'a [Discovery], serial: &str) -> Option<&'a Discovery> {
    let _ = (found, serial);
    todo!()
}

/// Directories the emulator writes discovery files to, in search order:
/// `$XDG_RUNTIME_DIR/avd/running` when set, then the per-OS default
/// (`~/Library/Caches/TemporaryItems/avd/running` on macOS,
/// `/run/user/<uid>/avd/running` on Linux, `%LOCALAPPDATA%\Temp\avd\running` on
/// Windows), then `avd/running` under the temporary directory. For one
/// emulator the authoritative answer is its console command `avd discoverypath`.
pub fn candidate_dirs() -> Vec<PathBuf> {
    todo!()
}

/// One discovery file found on disk.
#[derive(Debug)]
pub struct Found {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub parsed: Result<Discovery, DiscoveryError>,
}

/// Reads every `pid_*.ini` in `dir`, oldest modification first.
pub fn scan_dir(dir: &Path) -> std::io::Result<Vec<Found>> {
    let _ = dir;
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    /// The layout emulator 37.1.11 writes, with a placeholder token that has
    /// base64 padding like the real one.
    const REAL_LAYOUT: &str = "emulator.build=15917651\n\
avd.id=pixel_api36\n\
emulator.version=37.1.11.0\n\
port.serial=5554\n\
port.adb=5555\n\
avd.name=pixel_api36\n\
avd.dir=/placeholder/.android/avd/pixel_api36.avd\n\
cmdline=\"/placeholder/emulator/qemu/darwin-aarch64/qemu-system-aarch64-headless\" \"-avd\" \"pixel_api36\" \"-grpc\" \"8554\" \"-grpc-use-token\"\n\
grpc.token=placeholder+token/value==\n\
launcher.dir=/placeholder/emulator\n\
grpc.allowlist=/placeholder/emulator/lib/emulator_access.json\n\
grpc.port=8554\n";

    /// What an emulator started without `-grpc` writes, when it writes a file.
    const NO_GRPC: &str = "emulator.version=37.1.11.0\n\
port.serial=5556\n\
port.adb=5557\n\
avd.name=pixel_api36\n";

    /// An empty directory of its own under the system temporary directory.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mobile-agent-discovery-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_the_real_layout() {
        let d = Discovery::parse(REAL_LAYOUT, Some(59226)).unwrap();
        assert_eq!(d.pid, Some(59226));
        assert_eq!(d.console_port, Some(5554));
        assert_eq!(d.adb_port, Some(5555));
        assert_eq!(d.avd_name.as_deref(), Some("pixel_api36"));
        assert_eq!(d.emulator_version.as_deref(), Some("37.1.11.0"));
        assert_eq!(d.grpc_port, Some(8554));
        assert_eq!(d.entries.len(), 12);
        assert_eq!(
            d.avd_dir(),
            Some(PathBuf::from("/placeholder/.android/avd/pixel_api36.avd"))
        );
    }

    #[test]
    fn the_token_keeps_its_equals_signs() {
        let d = Discovery::parse(REAL_LAYOUT, None).unwrap();
        assert_eq!(d.grpc_token.as_deref(), Some("placeholder+token/value=="));
    }

    #[test]
    fn quoted_values_are_kept_verbatim() {
        let d = Discovery::parse(REAL_LAYOUT, None).unwrap();
        assert!(d.entries["cmdline"].starts_with("\"/placeholder/emulator/qemu/"));
        assert!(d.entries["cmdline"].ends_with("\"-grpc-use-token\""));
    }

    #[test]
    fn a_missing_grpc_port_means_no_endpoint() {
        let d = Discovery::parse(NO_GRPC, Some(1)).unwrap();
        assert_eq!(d.grpc_port, None);
        assert_eq!(d.grpc_token, None);
        assert_eq!(d.grpc_endpoint(), None);
        assert_eq!(d.serial().as_deref(), Some("emulator-5556"));
        assert_eq!(d.avd_dir(), None);
    }

    #[test]
    fn a_grpc_port_without_a_token_is_an_open_endpoint() {
        let d = Discovery::parse("port.serial=5554\ngrpc.port=8554\ngrpc.token=\n", None).unwrap();
        assert_eq!(d.grpc_endpoint().as_deref(), Some("http://127.0.0.1:8554"));
        assert_eq!(d.grpc_token, None);
    }

    #[test]
    fn crlf_blank_and_comment_lines_are_tolerated() {
        let text = "; written on Windows\r\n\r\nport.serial=5554\r\n# note\r\ngrpc.port=8554\r\ngrpc.token=abc=\r\n";
        let d = Discovery::parse(text, None).unwrap();
        assert_eq!(d.console_port, Some(5554));
        assert_eq!(d.grpc_port, Some(8554));
        assert_eq!(d.grpc_token.as_deref(), Some("abc="));
        assert_eq!(d.entries.len(), 3);
    }

    #[test]
    fn a_bad_port_is_an_error() {
        let err = Discovery::parse("port.serial=5554\ngrpc.port=85x4\n", None).unwrap_err();
        assert_eq!(
            err,
            DiscoveryError::BadPort {
                key: "grpc.port".into(),
                value: "85x4".into()
            }
        );
    }

    #[test]
    fn the_pid_comes_from_the_file_name() {
        assert_eq!(pid_from_file_name("pid_59226.ini"), Some(59226));
        assert_eq!(pid_from_file_name("pid_.ini"), None);
        assert_eq!(pid_from_file_name("pid_12.txt"), None);
        assert_eq!(pid_from_file_name("pid_12_info.ini"), None);
        assert_eq!(pid_from_file_name("emulator.ini"), None);
    }

    #[test]
    fn a_serial_maps_to_its_console_port() {
        assert_eq!(console_port_from_serial("emulator-5554"), Some(5554));
        assert_eq!(console_port_from_serial("emulator-"), None);
        assert_eq!(console_port_from_serial("R58M123ABC"), None);
        let d = Discovery::parse(REAL_LAYOUT, None).unwrap();
        assert_eq!(d.serial().as_deref(), Some("emulator-5554"));
        assert_eq!(d.grpc_endpoint().as_deref(), Some("http://127.0.0.1:8554"));
    }

    #[test]
    fn find_for_serial_matches_the_console_port_and_prefers_the_newest() {
        let a = Discovery::parse(REAL_LAYOUT, Some(1)).unwrap();
        let b = Discovery::parse(NO_GRPC, Some(2)).unwrap();
        let stale = Discovery::parse("port.serial=5554\ngrpc.port=9999\n", Some(3)).unwrap();
        let found = vec![stale, b.clone(), a.clone()];
        assert_eq!(find_for_serial(&found, "emulator-5556"), Some(&b));
        assert_eq!(find_for_serial(&found, "emulator-5554"), Some(&a));
        assert_eq!(find_for_serial(&found, "emulator-5558"), None);
        assert_eq!(find_for_serial(&found, "R58M123ABC"), None);
    }

    /// Writes `contents` to `dir/name`, modified `age` ago.
    fn write_aged(dir: &Path, name: &str, contents: &str, age: Duration) {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn scan_dir_reads_pid_files_oldest_first() {
        let dir = temp_dir("scan");
        let secs = Duration::from_secs;
        write_aged(
            &dir,
            "pid_1.ini",
            "port.serial=5554\ngrpc.port=9999\n",
            secs(30),
        );
        write_aged(&dir, "notes.txt", "port.serial=5554\n", secs(25));
        write_aged(
            &dir,
            "pid_3.ini",
            "port.serial=5554\ngrpc.port=bad\n",
            secs(20),
        );
        write_aged(&dir, "pid_2.ini", REAL_LAYOUT, secs(10));
        let found = scan_dir(&dir).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        let names: Vec<_> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["pid_1.ini", "pid_3.ini", "pid_2.ini"]);
        let parsed: Vec<Discovery> = found.into_iter().filter_map(|f| f.parsed.ok()).collect();
        assert_eq!(
            parsed.len(),
            2,
            "the file with a bad port is reported, not parsed"
        );
        let newest = find_for_serial(&parsed, "emulator-5554").unwrap();
        assert_eq!((newest.pid, newest.grpc_port), (Some(2), Some(8554)));
    }

    #[test]
    fn the_candidate_dirs_end_with_the_temporary_directory() {
        let dirs = candidate_dirs();
        assert!(dirs.len() >= 2, "{dirs:?}");
        assert_eq!(dirs.last(), Some(&std::env::temp_dir().join("avd/running")));
        if cfg!(target_os = "macos") {
            assert!(
                dirs.iter()
                    .any(|d| d.ends_with("Library/Caches/TemporaryItems/avd/running")),
                "{dirs:?}"
            );
        }
    }
}
