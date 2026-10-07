//! The device pasteboard: which applications may paste from it, and the
//! text put on it. A `SET` sent to the guest's `dtpasteboardd` puts it
//! there, as `devicectl device pasteboard copy` does from a process of its
//! own, which is the fallback; `simctl pbcopy` goes to `CoreSimulatorBridge`,
//! which ignores it once `dtpasteboardd` runs (research note, task 10.31).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use block2::RcBlock;

use crate::error::{Error, Result};
use crate::macos::coresim::{Device, developer_dir};
use crate::macos::objc;
use crate::macos::xpc::{self, Sim4Symbols, Value};

/// The guest service `dtpasteboardd` vends, of the CoreDevice feature
/// services that dtuhidd's digitizer belongs to.
pub const SERVICE: &str = "com.apple.coredevice.feature.pasteboard";

/// The types `devicectl` writes text under, each with the same bytes.
const TEXT_TYPES: [&str; 3] = ["public.utf8-plain-text", "public.plain-text", "public.text"];

/// How long `dtpasteboardd` gets to answer a `SET`. With its lookup and
/// connection, a `SET` took 8 to 35 ms on a loaded host (task 10.31); the
/// first one of a boot launches the daemon.
const REPLY: Duration = Duration::from_secs(5);

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

/// Puts `text` on the device's general pasteboard as plain text: a `SET`
/// to the guest's `dtpasteboardd`, or `devicectl` when that fails.
pub fn copy(device: &Device, text: &str) -> Result<()> {
    copy_with(|| set(device, text), || devicectl(&device.udid()?, text))
}

/// `direct`, then `fallback` when `direct` fails, as for a guest that does
/// not vend the service or a `dtpasteboardd` that no longer reads this
/// `SET`. When both fail, the error says why each did.
fn copy_with(
    direct: impl FnOnce() -> std::result::Result<(), String>,
    fallback: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let Err(why) = direct() else {
        return Ok(());
    };
    fallback().map_err(|error| Error::Io {
        what: "setting the device pasteboard".into(),
        reason: format!("a SET to dtpasteboardd failed ({why}), then {error}"),
    })
}

/// Sends the `SET` for `text` to the guest's `dtpasteboardd` over a
/// connection of its own, reached as dtuhidd is (spike S3), and waits for
/// its reply, which carries the pasteboard as the `SET` left it.
fn set(device: &Device, text: &str) -> std::result::Result<(), String> {
    let symbols = Sim4Symbols::resolve()?;
    let port = device
        .lookup(SERVICE)
        .map_err(|error| format!("the guest does not vend {SERVICE}: {error}"))?;
    let connection = symbols
        .connect(port)
        .ok_or("xpc_connection_create_from_endpoint returned nothing")?;
    let queue = objc::completion_queue();
    connection.set_target_queue(queue);
    // Errors reach the reply handler as well; libxpc wants a handler before
    // the connection resumes.
    let events = RcBlock::new(|_event: xpc::Object| {});
    connection.set_event_handler(&events);
    connection.resume();
    let (tx, rx) = mpsc::sync_channel(1);
    connection.send_with_reply(&set_message(text), queue, move |reply| {
        // SAFETY: XPC hands the handler a live object.
        let _ = tx.try_send(unsafe { Value::retaining(reply) });
    });
    let reply = rx
        .recv_timeout(REPLY)
        .map_err(|_| format!("no reply within {REPLY:?}"))?;
    outcome(reply.as_ref())
}

/// The `SET` that puts `text` on the general pasteboard, as one item under
/// each plain-text type: `{command, pasteboardName, items: [{types, data:
/// {<type>: {data}}}]}`, the shape `dtpasteboardd` of CoreDevice 642.16
/// decodes (task 10.31).
fn set_message(text: &str) -> Value {
    let types = Value::array();
    let data = Value::dictionary();
    for kind in TEXT_TYPES {
        types.append(&Value::text(kind));
        let representation = Value::dictionary();
        representation.set_value("data", &Value::data(text.as_bytes()));
        data.set_value(kind, &representation);
    }
    let item = Value::dictionary();
    item.set_value("types", &types);
    item.set_value("data", &data);
    let items = Value::array();
    items.append(&item);
    let message = Value::dictionary();
    message.set_string("command", "SET");
    message.set_string("pasteboardName", "general");
    message.set_value("items", &items);
    message
}

/// What `dtpasteboardd` answered a `SET` with: `SET_REPLY` once the text is
/// on the pasteboard, otherwise why not, such as the key it could not
/// decode or the connection's error.
fn outcome(reply: Option<&Value>) -> std::result::Result<(), String> {
    let reply = reply.ok_or("a nil reply")?;
    if reply.is_error() {
        return Err(reply.error_reason());
    }
    if reply.string("command").as_deref() == Some("SET_REPLY") {
        return Ok(());
    }
    Err(reply
        .value("error")
        .and_then(|error| error.value("userInfo"))
        .and_then(|info| info.string("NSDebugDescription"))
        .unwrap_or_else(|| reply.description()))
}

/// `devicectl device pasteboard copy`, a process of its own.
fn devicectl(udid: &str, text: &str) -> Result<()> {
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

    #[test]
    fn a_paste_goes_to_dtpasteboardd_and_leaves_devicectl_alone() {
        let ran_devicectl = Cell::new(false);
        copy_with(
            || Ok(()),
            || {
                ran_devicectl.set(true);
                Ok(())
            },
        )
        .expect("the text is on the pasteboard");
        assert!(!ran_devicectl.get(), "devicectl ran after the SET");
    }

    #[test]
    fn a_guest_that_does_not_take_the_set_gets_devicectl() {
        let ran_devicectl = Cell::new(0);
        copy_with(
            || Err("the guest does not vend com.apple.coredevice.feature.pasteboard".into()),
            || {
                ran_devicectl.set(ran_devicectl.get() + 1);
                Ok(())
            },
        )
        .expect("devicectl stands in");
        assert_eq!(ran_devicectl.get(), 1);
    }

    #[test]
    fn when_devicectl_fails_too_the_error_names_both() {
        let error = copy_with(
            || Err("Expected to find key items.".into()),
            || {
                Err(Error::Io {
                    what: "devicectl device pasteboard copy".into(),
                    reason: "exit status: 1".into(),
                })
            },
        )
        .expect_err("neither route set the pasteboard");
        assert_eq!(error.code(), "io");
        let message = error.to_string();
        for part in ["Expected to find key items.", "devicectl", "exit status: 1"] {
            assert!(
                message.contains(part),
                "{part:?} is missing from: {message}"
            );
        }
    }

    #[test]
    fn the_set_carries_the_text_under_each_plain_text_type() {
        let text = "abcdefghijklmnopqrstuvwxyz0123456789";
        let message = set_message(text);
        assert_eq!(message.string("command").as_deref(), Some("SET"));
        assert_eq!(message.string("pasteboardName").as_deref(), Some("general"));
        let items = message.value("items").expect("items").items();
        assert_eq!(items.len(), 1, "one item");
        let types: Vec<String> = items[0]
            .value("types")
            .expect("types")
            .items()
            .iter()
            .filter_map(Value::as_text)
            .collect();
        assert_eq!(
            types,
            ["public.utf8-plain-text", "public.plain-text", "public.text"]
        );
        let data = items[0].value("data").expect("data");
        for kind in &types {
            let bytes = data
                .value(kind)
                .and_then(|representation| representation.value("data"))
                .and_then(|data| data.as_bytes());
            assert_eq!(bytes.as_deref(), Some(text.as_bytes()), "{kind}");
        }
    }

    #[test]
    fn a_set_reply_succeeds_and_any_other_answer_says_why_not() {
        let reply = Value::dictionary();
        reply.set_string("command", "SET_REPLY");
        assert_eq!(outcome(Some(&reply)), Ok(()));

        // A decoding error, as dtpasteboardd answers a SET it cannot read.
        let user_info = Value::dictionary();
        user_info.set_string("NSDebugDescription", "Expected to find key items.");
        let error = Value::dictionary();
        error.set_string("domain", "NSCocoaErrorDomain");
        error.set_value("userInfo", &user_info);
        let refused = Value::dictionary();
        refused.set_value("error", &error);
        assert_eq!(
            outcome(Some(&refused)),
            Err("Expected to find key items.".to_string())
        );

        let other = Value::dictionary();
        other.set_string("command", "PULL_REPLY");
        let why = outcome(Some(&other)).expect_err("not a SET's reply");
        assert!(why.contains("PULL_REPLY"), "{why}");
        assert!(outcome(None).is_err());
    }

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
