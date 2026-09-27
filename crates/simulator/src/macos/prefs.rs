//! The accessibility preferences a device needs before its first boot.
//!
//! Without `AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and
//! `AutomationEnabled` in the device's `com.apple.Accessibility` domain, both
//! tree read paths fail for several seconds after a boot (spike S4, "Accessibility
//! enablement"). They are written into the file under the device's data path
//! while the device is shut down, so no process is spawned in the guest and
//! `CoreSimulatorBridge` needs no restart.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::model::PreferencesOutcome;

/// The preferences file, relative to `-[SimDevice dataPath]`.
pub const FILE: &str = "Library/Preferences/com.apple.Accessibility.plist";

/// The keys set to true.
pub const KEYS: [&str; 3] = [
    "AccessibilityEnabled",
    "ApplicationAccessibilityEnabled",
    "AutomationEnabled",
];

/// The preferences file of the device whose data path is `data_path`.
pub fn path(data_path: &Path) -> PathBuf {
    data_path.join(FILE)
}

/// Sets the three keys to true in the file under `data_path`, creating it
/// when missing and keeping every other key. Reports whether it wrote.
pub fn write(data_path: &Path) -> Result<PreferencesOutcome> {
    let _ = data_path;
    unimplemented!("task 6.1: accessibility preferences")
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    /// A directory under the system temporary directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "simulator-prefs-{label}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("temporary directory");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn plutil(args: &[&str], file: &Path) -> (bool, String) {
        let output = Command::new("plutil")
            .args(args)
            .arg(file)
            .output()
            .expect("plutil runs");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )
    }

    /// The value at `key` as `plutil -extract ... raw` prints it.
    fn value(file: &Path, key: &str) -> Option<String> {
        let (ok, value) = plutil(&["-extract", key, "raw", "-o", "-"], file);
        ok.then_some(value)
    }

    #[test]
    fn a_fresh_data_path_gets_a_file_with_the_three_keys() {
        let data = TempDir::new("fresh");
        assert_eq!(write(&data.0), Ok(PreferencesOutcome::Written));
        let file = path(&data.0);
        for key in KEYS {
            assert_eq!(value(&file, key).as_deref(), Some("true"), "{key}");
        }
        let (ok, _) = plutil(&["-lint"], &file);
        assert!(ok, "{} is a valid property list", file.display());
    }

    #[test]
    fn existing_keys_are_kept_and_a_second_write_changes_nothing() {
        let data = TempDir::new("existing");
        let file = path(&data.0);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("preferences directory");
        assert!(plutil(&["-create", "binary1"], &file).0);
        assert!(plutil(&["-insert", "ReduceMotionEnabled", "-integer", "0"], &file).0);
        assert!(plutil(&["-insert", "AutomationEnabled", "-bool", "false"], &file).0);

        assert_eq!(write(&data.0), Ok(PreferencesOutcome::Written));
        assert_eq!(value(&file, "ReduceMotionEnabled").as_deref(), Some("0"));
        for key in KEYS {
            assert_eq!(value(&file, key).as_deref(), Some("true"), "{key}");
        }
        let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok();

        assert_eq!(write(&data.0), Ok(PreferencesOutcome::AlreadySet));
        assert_eq!(
            std::fs::metadata(&file).and_then(|m| m.modified()).ok(),
            modified,
            "an unchanged file is not rewritten"
        );
    }

    #[test]
    fn a_file_that_is_not_a_dictionary_is_refused_and_left_alone() {
        let data = TempDir::new("array");
        let file = path(&data.0);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("preferences directory");
        std::fs::write(
            &file,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><array/></plist>\n",
        )
        .expect("an array plist");
        let before = std::fs::read(&file).expect("readable");
        assert!(write(&data.0).is_err());
        assert_eq!(std::fs::read(&file).expect("readable"), before);
    }
}
