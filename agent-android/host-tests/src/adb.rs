//! The device through adb: the `adb` binary for setup commands, and adb
//! server service streams for the agent's socket (design D3), so no forward
//! rule exists to leak.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

/// One device, by serial.
#[derive(Clone, Debug)]
pub struct Device {
    pub serial: String,
    adb: PathBuf,
}

/// The `adb` binary: `$ADB`, else the SDK's under `$ANDROID_HOME` or the
/// default macOS SDK location, else `adb` on PATH.
pub fn adb_binary() -> PathBuf {
    if let Some(adb) = std::env::var_os("ADB") {
        return PathBuf::from(adb);
    }
    let home = std::env::var_os("ANDROID_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")));
    if let Some(home) = home {
        let adb = home.join("platform-tools/adb");
        if adb.is_file() {
            return adb;
        }
    }
    PathBuf::from("adb")
}

impl Device {
    pub fn new(serial: String) -> Device {
        Device {
            serial,
            adb: adb_binary(),
        }
    }

    /// An `adb -s <serial>` command.
    pub fn adb(&self) -> Command {
        let mut cmd = Command::new(&self.adb);
        cmd.arg("-s").arg(&self.serial);
        cmd
    }

    /// Fails unless the device is online.
    pub fn check_ready(&self) -> Result<(), String> {
        let out = self
            .adb()
            .arg("get-state")
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.adb.display()))?;
        let state = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if state == "device" {
            Ok(())
        } else {
            Err(format!(
                "device {} is not usable (state '{state}', {})",
                self.serial,
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    /// Runs `cmd` in a device shell and returns its output, whatever the status.
    pub fn shell_output(&self, cmd: &str) -> Output {
        self.adb()
            .arg("shell")
            .arg(cmd)
            .stdin(Stdio::null())
            .output()
            .expect("run adb shell")
    }

    /// Runs `cmd` in a device shell and returns its stdout; panics when it fails.
    pub fn shell(&self, cmd: &str) -> String {
        let out = self.shell_output(cmd);
        assert!(
            out.status.success(),
            "adb shell {cmd:?} failed with {}: {}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).replace('\r', "")
    }

    /// Copies a host file to the device.
    pub fn push(&self, local: &std::path::Path, remote: &str) {
        let out = self
            .adb()
            .arg("push")
            .arg(local)
            .arg(remote)
            .output()
            .expect("run adb push");
        assert!(
            out.status.success(),
            "adb push {} {remote} failed: {}",
            local.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Whether a process with this pid exists on the device.
    pub fn pid_alive(&self, pid: u32) -> bool {
        self.shell_output(&format!("kill -0 {pid}"))
            .status
            .success()
    }

    /// `ps -A -o PID,USER,NAME,ARGS` rows whose text contains `needle`.
    pub fn processes_matching(&self, needle: &str) -> Vec<String> {
        self.shell("ps -A -o PID,USER,NAME,ARGS")
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_owned)
            .collect()
    }

    /// Whether the accessibility manager lists a UI automation registration.
    pub fn ui_automation_registered(&self) -> bool {
        self.shell("dumpsys accessibility")
            .contains("Ui Automation")
    }

    /// Opens a device service stream through the adb server: the connection
    /// `localabstract:<name>` returns reaches the agent's socket directly.
    pub fn service_stream(&self, service: &str) -> io::Result<TcpStream> {
        let port = std::env::var("ANDROID_ADB_SERVER_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(5037);
        let mut stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_nodelay(true)?;
        request(&mut stream, &format!("host:transport:{}", self.serial))?;
        request(&mut stream, service)?;
        stream.set_read_timeout(None)?;
        Ok(stream)
    }
}

/// Sends one adb server request and reads its status.
fn request(stream: &mut TcpStream, payload: &str) -> io::Result<()> {
    write!(stream, "{:04x}{payload}", payload.len())?;
    let mut status = [0u8; 4];
    stream.read_exact(&mut status)?;
    match &status {
        b"OKAY" => Ok(()),
        b"FAIL" => {
            let mut len = [0u8; 4];
            stream.read_exact(&mut len)?;
            let len =
                usize::from_str_radix(std::str::from_utf8(&len).unwrap_or("0"), 16).unwrap_or(0);
            let mut message = vec![0u8; len];
            stream.read_exact(&mut message)?;
            Err(io::Error::other(format!(
                "adb server refused {payload:?}: {}",
                String::from_utf8_lossy(&message)
            )))
        }
        other => Err(io::Error::other(format!(
            "adb server answered {payload:?} with {:?}",
            String::from_utf8_lossy(other)
        ))),
    }
}
