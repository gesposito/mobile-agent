//! Other UI automation holders, for the ownership tests.

use std::collections::HashSet;
use std::process::{Child, Stdio};
use std::time::Duration;

use crate::adb::Device;
use crate::poll;

/// Pids of processes named `uiautomator`.
pub fn uiautomator_pids(device: &Device) -> HashSet<u32> {
    device
        .shell("ps -A -o PID,NAME")
        .lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            (words.next()? == "uiautomator").then_some(pid)
        })
        .collect()
}

/// Waits until no process holds UI automation, for up to `timeout`.
pub fn wait_role_free(device: &Device, timeout: Duration) {
    let free = poll(timeout, Duration::from_millis(250), || {
        (!device.ui_automation_registered()).then_some(())
    });
    assert!(
        free.is_some(),
        "another process kept UI automation for {timeout:?}: {:?}",
        device.processes_matching("uiautomator")
    );
}

/// `adb shell uiautomator events`: the platform tool holding UI automation
/// for as long as it runs, killed by pid when dropped.
pub struct UiautomatorEvents {
    device: Device,
    child: Child,
    pub pid: u32,
}

impl UiautomatorEvents {
    /// Starts it and waits until it holds the role.
    pub fn start(device: &Device) -> UiautomatorEvents {
        wait_role_free(device, Duration::from_secs(60));
        let before = uiautomator_pids(device);
        let child = device
            .adb()
            .args(["shell", "uiautomator events"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn adb shell uiautomator events");
        let mut holder = UiautomatorEvents {
            device: device.clone(),
            child,
            pid: 0,
        };
        holder.pid = poll(Duration::from_secs(15), Duration::from_millis(100), || {
            uiautomator_pids(device).difference(&before).next().copied()
        })
        .expect("uiautomator events did not start");
        poll(Duration::from_secs(15), Duration::from_millis(100), || {
            device.ui_automation_registered().then_some(())
        })
        .expect("uiautomator events did not take UI automation");
        holder
    }

    /// Whether the process still runs.
    pub fn alive(&self) -> bool {
        uiautomator_pids(&self.device).contains(&self.pid)
    }
}

impl Drop for UiautomatorEvents {
    fn drop(&mut self) {
        if self.alive() {
            let _ = self.device.shell_output(&format!("kill -9 {}", self.pid));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
