//! The device pasteboard: which applications may paste from it, and the
//! text put on it, written through `devicectl device pasteboard copy`: under
//! Xcode 27, `simctl pbcopy` exits 0 and writes nothing on iOS 26.5 guests
//! (research note, from baguette).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use crate::error::{Error, Result};
use crate::macos::coresim::developer_dir;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether one device's user applications may paste from other
/// applications, as of the last change to its installed applications.
///
/// Listing the applications took 11 to 22 ms (task 10.31), so the listing
/// and the grants are kept until the device's application directory changes:
/// every install, upgrade and uninstall gives the application a new bundle
/// directory or removes one, whether the driver or another tool made it.
#[derive(Default)]
pub struct PasteAccess {
    /// The application directory's modification time when every user
    /// application was last granted.
    granted_at: Mutex<Option<SystemTime>>,
}

impl PasteAccess {
    /// Lets every user application in `list` paste, granting each with
    /// `grant`, unless they were granted while the application directory
    /// was last modified at `applications_changed`. An uninstall drops its
    /// application's grant, so after a change every application is granted
    /// again. An unknown modification time lists on every paste.
    pub fn allow(
        &self,
        applications_changed: Option<SystemTime>,
        list: impl FnOnce() -> Result<Vec<(String, bool)>>,
        mut grant: impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        let mut granted_at = lock(&self.granted_at);
        if applications_changed.is_some() && *granted_at == applications_changed {
            return Ok(());
        }
        for (bundle_id, user) in list()? {
            if user {
                grant(&bundle_id)?;
            }
        }
        *granted_at = applications_changed;
        Ok(())
    }
}

/// When the device's installed applications last changed: the modification
/// time of the directory that holds their bundles, one per application,
/// under the device's data directory. `None` when it cannot be read.
pub fn applications_changed(data: &Path) -> Option<SystemTime> {
    std::fs::metadata(data.join("Containers/Bundle/Application"))
        .and_then(|metadata| metadata.modified())
        .ok()
}

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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    use super::*;

    /// A device's listing: the fixtures as user applications and one of the
    /// runtime's own.
    fn listing() -> Vec<(String, bool)> {
        vec![
            ("dev.mobileagent.fixture.swiftui".to_string(), true),
            ("com.apple.mobilesafari".to_string(), false),
            ("dev.mobileagent.fixture.expo".to_string(), true),
        ]
    }

    /// A fake device: counts its listings and records its grants.
    #[derive(Default)]
    struct FakeDevice {
        listings: Cell<u32>,
        grants: RefCell<Vec<String>>,
    }

    impl FakeDevice {
        fn paste(&self, access: &PasteAccess, changed: Option<SystemTime>) -> Result<()> {
            access.allow(
                changed,
                || {
                    self.listings.set(self.listings.get() + 1);
                    Ok(listing())
                },
                |bundle_id| {
                    self.grants.borrow_mut().push(bundle_id.to_string());
                    Ok(())
                },
            )
        }
    }

    fn at(seconds: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
    }

    #[test]
    fn the_second_paste_lists_nothing() {
        let (device, access) = (FakeDevice::default(), PasteAccess::default());
        device.paste(&access, at(100)).expect("the first paste");
        device.paste(&access, at(100)).expect("the second paste");
        assert_eq!(device.listings.get(), 1, "the second paste listed again");
        assert_eq!(
            *device.grants.borrow(),
            [
                "dev.mobileagent.fixture.swiftui",
                "dev.mobileagent.fixture.expo"
            ],
            "each user application once, the runtime's own left alone"
        );
    }

    #[test]
    fn a_changed_application_directory_lists_and_grants_again() {
        let (device, access) = (FakeDevice::default(), PasteAccess::default());
        device.paste(&access, at(100)).expect("the first paste");
        // An install or an uninstall, by the driver or another tool; an
        // uninstall drops the application's grant, so every one is renewed.
        device
            .paste(&access, at(160))
            .expect("a paste after an install");
        device
            .paste(&access, at(160))
            .expect("the paste after that");
        assert_eq!(device.listings.get(), 2);
        assert_eq!(device.grants.borrow().len(), 4);
    }

    #[test]
    fn an_unreadable_application_directory_lists_on_every_paste() {
        let (device, access) = (FakeDevice::default(), PasteAccess::default());
        device.paste(&access, None).expect("the first paste");
        device.paste(&access, None).expect("the second paste");
        assert_eq!(device.listings.get(), 2);
    }

    #[test]
    fn the_application_directory_changes_with_an_install_and_is_none_when_missing() {
        let data = std::env::temp_dir().join(format!(
            "mobile-agent-pasteboard-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let applications = data.join("Containers/Bundle/Application");
        assert_eq!(applications_changed(&data), None, "no directory yet");
        std::fs::create_dir_all(&applications).expect("the directory");
        let before = applications_changed(&data).expect("a modification time");
        std::thread::sleep(Duration::from_millis(5));
        // An install adds a bundle directory, as installd does for every
        // install and upgrade.
        std::fs::create_dir(applications.join("C30C67E1-23FD-4036-8A03-99D7238C4E01"))
            .expect("a bundle directory");
        let after = applications_changed(&data).expect("a modification time");
        std::fs::remove_dir_all(&data).expect("cleaned up");
        assert_ne!(before, after);
    }

    #[test]
    fn a_failed_grant_is_tried_again_on_the_next_paste() {
        let (device, access) = (FakeDevice::default(), PasteAccess::default());
        let refused = access.allow(
            at(100),
            || Ok(listing()),
            |_| {
                Err(Error::Io {
                    what: "the grant".into(),
                    reason: "refused".into(),
                })
            },
        );
        assert!(refused.is_err());
        device.paste(&access, at(100)).expect("the next paste");
        assert_eq!(device.listings.get(), 1, "the next paste listed again");
        assert_eq!(device.grants.borrow().len(), 2);
    }
}
