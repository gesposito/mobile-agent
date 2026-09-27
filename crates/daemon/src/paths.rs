//! Where the daemon keeps its socket and sidecar files, and the version it
//! records in them.
//!
//! The directory is `MOBILE_AGENT_SOCKET_DIR` when set, else
//! `$XDG_RUNTIME_DIR/mobile-agent`, else `~/.mobile-agent`, else
//! `mobile-agent` under the temporary directory, the order agent-browser
//! resolves its own in. It holds the socket, the pid and version files, a
//! lock that serializes daemon startups, and the log of a daemon the CLI
//! started.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const SOCKET_FILE: &str = "daemon.sock";
pub const PID_FILE: &str = "daemon.pid";
pub const VERSION_FILE: &str = "daemon.version";
pub const LOCK_FILE: &str = "daemon.lock";
pub const LOG_FILE: &str = "daemon.log";

/// The longest socket path: a Unix socket address holds 104 bytes on
/// macOS, the terminating NUL included.
pub const MAX_SOCKET_PATH: usize = 103;

/// The socket directory the environment names, as described above.
pub fn default_dir() -> PathBuf {
    let var = |key: &str| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("MOBILE_AGENT_SOCKET_DIR") {
        return dir;
    }
    if let Some(runtime) = var("XDG_RUNTIME_DIR") {
        return runtime.join("mobile-agent");
    }
    if let Some(home) = var("HOME") {
        return home.join(".mobile-agent");
    }
    std::env::temp_dir().join("mobile-agent")
}

/// The files of one socket directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    dir: PathBuf,
}

impl Paths {
    pub fn new(dir: impl Into<PathBuf>) -> Paths {
        Paths { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join(SOCKET_FILE)
    }

    pub fn pid(&self) -> PathBuf {
        self.dir.join(PID_FILE)
    }

    pub fn version(&self) -> PathBuf {
        self.dir.join(VERSION_FILE)
    }

    pub fn lock(&self) -> PathBuf {
        self.dir.join(LOCK_FILE)
    }

    pub fn log(&self) -> PathBuf {
        self.dir.join(LOG_FILE)
    }

    /// Fails when the socket path does not fit in a Unix socket address.
    pub fn check_socket_len(&self) -> Result<(), String> {
        let socket = self.socket();
        let len = socket.as_os_str().len();
        if len > MAX_SOCKET_PATH {
            return Err(format!(
                "the socket path {} is {len} bytes, more than the {MAX_SOCKET_PATH} a Unix socket address holds; set MOBILE_AGENT_SOCKET_DIR to a shorter directory",
                socket.display()
            ));
        }
        Ok(())
    }
}

/// A build's version as the version file records it: the crate version,
/// `+`, then the executable's size and modification time in nanoseconds,
/// both in hex. A rebuilt binary reads as another version, so the CLI
/// replaces a daemon started from an older build of the same release.
pub fn build_version(exe: &Path) -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), fingerprint(exe))
}

fn fingerprint(exe: &Path) -> String {
    let Ok(meta) = std::fs::metadata(exe) else {
        return "unknown".to_owned();
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    format!("{:x}.{:x}", meta.len(), mtime)
}

/// The version of the running executable.
pub fn own_version() -> String {
    match std::env::current_exe() {
        Ok(exe) => build_version(&exe),
        Err(_) => format!("{}+unknown", env!("CARGO_PKG_VERSION")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_path_over_103_bytes_is_refused_naming_the_override() {
        let fits = Paths::new(format!(
            "/tmp/{}",
            "d".repeat(103 - 5 - 1 - SOCKET_FILE.len())
        ));
        assert_eq!(fits.socket().as_os_str().len(), 103);
        assert!(fits.check_socket_len().is_ok());
        let long = Paths::new(format!("/tmp/{}", "d".repeat(103 - 5 - SOCKET_FILE.len())));
        let error = long.check_socket_len().expect_err("104 bytes do not fit");
        assert!(error.contains("MOBILE_AGENT_SOCKET_DIR"), "{error}");
    }

    #[test]
    fn the_version_names_the_release_and_the_build() {
        let version = build_version(Path::new("/definitely/not/here"));
        assert_eq!(version, format!("{}+unknown", env!("CARGO_PKG_VERSION")));
        let exe = std::env::current_exe().expect("the test binary has a path");
        let version = build_version(&exe);
        let (release, build) = version.split_once('+').expect("a build part");
        assert_eq!(release, env!("CARGO_PKG_VERSION"));
        let (size, mtime) = build.split_once('.').expect("size and time");
        assert!(u64::from_str_radix(size, 16).is_ok() && u128::from_str_radix(mtime, 16).is_ok());
    }
}
