//! The device pasteboard, written through `devicectl device pasteboard
//! copy`: under Xcode 27, `simctl pbcopy` exits 0 and writes nothing on
//! iOS 26.5 guests (research note, from baguette).

use std::io::Write as _;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::macos::coresim::developer_dir;

/// Puts `text` on the device's general pasteboard as plain text.
pub fn copy(udid: &str, text: &str) -> Result<()> {
    let devicectl = developer_dir().join("usr/bin/devicectl");
    let mut command = if devicectl.exists() {
        Command::new(devicectl)
    } else {
        let mut xcrun = Command::new("xcrun");
        xcrun.arg("devicectl");
        xcrun
    };
    let failed = |reason: String| Error::Io {
        what: "devicectl device pasteboard copy".into(),
        reason,
    };
    let mut child = command
        .args([
            "device",
            "pasteboard",
            "copy",
            "--timeout",
            "30",
            "--device",
            udid,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| failed(error.to_string()))?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(text.as_bytes())
        .map_err(|error| failed(error.to_string()))?;
    let output = child
        .wait_with_output()
        .map_err(|error| failed(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failed(format!(
            "{}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
