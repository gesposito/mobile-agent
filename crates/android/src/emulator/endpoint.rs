//! Resolving an emulator's adb serial to its controller endpoint, and the
//! warning the driver emits when there is none.

use std::fmt;
use std::path::PathBuf;

use super::discovery;

/// Where an emulator's controller listens and how to authenticate to it.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The adb serial, `emulator-<console port>`.
    pub serial: String,
    /// The controller URI, `http://127.0.0.1:<grpc.port>`.
    pub uri: String,
    /// Bearer token from the discovery file; present with `-grpc-use-token`.
    pub token: Option<String>,
    /// The emulator console's port.
    pub console_port: Option<u16>,
    /// The discovery file the endpoint was read from.
    pub discovery_file: Option<PathBuf>,
}

impl fmt::Debug for Endpoint {
    /// Shows whether a token is present, never the token.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("serial", &self.serial)
            .field("uri", &self.uri)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("console_port", &self.console_port)
            .field("discovery_file", &self.discovery_file)
            .finish()
    }
}

/// The emulator's control endpoint cannot be used, so taps and frames fall back
/// to the agent. [`Unavailable::warning`] is the text for the target status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    pub serial: String,
    /// Why the endpoint cannot be used.
    pub reason: String,
    /// The emulator console's port, when known.
    pub console_port: Option<u16>,
}

impl Unavailable {
    pub fn new(serial: &str, console_port: Option<u16>, reason: impl Into<String>) -> Self {
        Self {
            serial: serial.to_owned(),
            reason: reason.into(),
            console_port,
        }
    }

    /// The gRPC port to suggest: the console port plus 3000, the convention of
    /// Google's emulator scripts (8554 for emulator-5554), else 8554.
    pub fn suggested_port(&self) -> u16 {
        todo!()
    }

    /// What falls back and how to enable the fast path: the launch flag, or the
    /// console command that enables it without a restart.
    pub fn warning(&self) -> String {
        todo!()
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.warning())
    }
}

impl std::error::Error for Unavailable {}

/// Resolves `serial` through the discovery files in
/// [`discovery::candidate_dirs`].
pub fn resolve(serial: &str) -> Result<Endpoint, Unavailable> {
    resolve_in(serial, &discovery::candidate_dirs())
}

/// Resolves `serial` through the discovery files in `dirs`; when several files
/// name the serial, the newest wins.
pub fn resolve_in(serial: &str, dirs: &[PathBuf]) -> Result<Endpoint, Unavailable> {
    let _ = (serial, dirs);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mobile-agent-endpoint-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_warning_names_the_launch_flag_and_the_console_command() {
        let warning = Unavailable::new("emulator-5554", Some(5554), "connection refused").warning();
        assert!(warning.contains("emulator-5554"), "{warning}");
        assert!(warning.contains("connection refused"), "{warning}");
        assert!(warning.contains("fall back to the agent"), "{warning}");
        assert!(warning.contains("-grpc 8554 -grpc-use-token"), "{warning}");
        assert!(warning.contains("`grpc 8554`"), "{warning}");
        assert!(
            warning.contains("adb -s emulator-5554 emu grpc 8554"),
            "{warning}"
        );
        assert!(warning.contains("without a restart"), "{warning}");
    }

    #[test]
    fn the_suggested_port_follows_the_console_port() {
        assert_eq!(
            Unavailable::new("emulator-5556", Some(5556), "").suggested_port(),
            8556
        );
        assert_eq!(
            Unavailable::new("emulator-5554", None, "").suggested_port(),
            8554
        );
        let warning = Unavailable::new("emulator-5556", Some(5556), "no grpc.port").warning();
        assert!(warning.contains("-grpc 8556 -grpc-use-token"), "{warning}");
    }

    #[test]
    fn a_discovery_file_with_a_grpc_port_resolves_to_an_endpoint() {
        let dir = temp_dir("resolve");
        let file = dir.join("pid_42.ini");
        fs::write(
            &file,
            "port.serial=5554\ngrpc.port=8554\ngrpc.token=placeholder+token==\n",
        )
        .unwrap();
        let endpoint = resolve_in("emulator-5554", std::slice::from_ref(&dir));
        fs::remove_dir_all(&dir).unwrap();
        let endpoint = endpoint.unwrap();
        assert_eq!(endpoint.uri, "http://127.0.0.1:8554");
        assert_eq!(endpoint.token.as_deref(), Some("placeholder+token=="));
        assert_eq!(endpoint.console_port, Some(5554));
        assert_eq!(endpoint.discovery_file, Some(file));
        assert!(!format!("{endpoint:?}").contains("placeholder+token"));
    }

    #[test]
    fn no_discovery_file_is_unavailable() {
        let dir = temp_dir("none");
        let err = resolve_in("emulator-5556", std::slice::from_ref(&dir)).unwrap_err();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(err.console_port, Some(5556));
        assert!(err.reason.contains("no discovery file"), "{}", err.reason);
        assert!(err.warning().contains("-grpc 8556 -grpc-use-token"));
    }

    #[test]
    fn a_discovery_file_without_a_grpc_port_is_unavailable() {
        let dir = temp_dir("nogrpc");
        fs::write(dir.join("pid_7.ini"), "port.serial=5556\nport.adb=5557\n").unwrap();
        let err = resolve_in("emulator-5556", std::slice::from_ref(&dir)).unwrap_err();
        fs::remove_dir_all(&dir).unwrap();
        assert!(err.reason.contains("grpc.port"), "{}", err.reason);
    }

    #[test]
    fn a_device_serial_has_no_controller() {
        let err = resolve_in("R58M123ABC", &[]).unwrap_err();
        assert_eq!(err.console_port, None);
        assert!(err.reason.contains("not an emulator"), "{}", err.reason);
    }
}
