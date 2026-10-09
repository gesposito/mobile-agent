//! The application bundle an install takes (spec `ios-device-driver`, "App
//! lifecycle"): a `.app` directory built for a device and signed, which the
//! driver checks on the host before uploading it, so that a Simulator build
//! or an unsigned one is refused with a reason instead of the installation
//! proxy's verification error.

use std::path::Path;

/// What the driver reads from a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    /// `CFBundleIdentifier`.
    pub bundle_id: String,
    /// `CFBundleExecutable`.
    pub executable: String,
}

/// Reads the bundle at `app` and checks it can be installed on a device:
/// a directory with an `Info.plist` naming its identifier and executable,
/// built for iPhoneOS, signed (`_CodeSignature/CodeResources`) and carrying
/// its provisioning profile (`embedded.mobileprovision`). Fails with the
/// reason when it cannot.
pub fn inspect(app: &Path) -> Result<Bundle, String> {
    let _ = app;
    todo!()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use plist::{Dictionary, Value};

    use super::*;

    /// A bundle laid out as Xcode lays out a device build, in a directory
    /// of its own that is removed when the value is dropped.
    struct Fixture {
        root: PathBuf,
        app: PathBuf,
    }

    impl Fixture {
        fn new(test: &str) -> Fixture {
            let root =
                std::env::temp_dir().join(format!("ios-bundle-{test}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let app = root.join("FixtureSwiftUI.app");
            fs::create_dir_all(app.join("_CodeSignature")).expect("the bundle directory");
            let mut info = Dictionary::new();
            info.insert(
                "CFBundleIdentifier".into(),
                Value::String("dev.mobileagent.fixture.swiftui".into()),
            );
            info.insert(
                "CFBundleExecutable".into(),
                Value::String("FixtureSwiftUI".into()),
            );
            info.insert(
                "CFBundleSupportedPlatforms".into(),
                Value::Array(vec![Value::String("iPhoneOS".into())]),
            );
            plist::to_file_binary(app.join("Info.plist"), &info).expect("Info.plist");
            fs::write(app.join("FixtureSwiftUI"), b"\xca\xfe\xba\xbe").expect("the executable");
            fs::write(app.join("_CodeSignature/CodeResources"), b"<plist/>")
                .expect("the signature");
            fs::write(app.join("embedded.mobileprovision"), b"profile").expect("the profile");
            Fixture { root, app }
        }

        fn set(&self, key: &str, value: Value) {
            let mut info: Dictionary =
                plist::from_file(self.app.join("Info.plist")).expect("Info.plist");
            info.insert(key.into(), value);
            plist::to_file_binary(self.app.join("Info.plist"), &info).expect("Info.plist");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_signed_device_build_is_read_for_its_identifier_and_executable() {
        let fixture = Fixture::new("signed");
        assert_eq!(
            inspect(&fixture.app),
            Ok(Bundle {
                bundle_id: "dev.mobileagent.fixture.swiftui".into(),
                executable: "FixtureSwiftUI".into(),
            })
        );
    }

    #[test]
    fn a_simulator_build_is_refused_naming_the_device_destination() {
        let fixture = Fixture::new("simulator");
        fixture.set(
            "CFBundleSupportedPlatforms",
            Value::Array(vec![Value::String("iPhoneSimulator".into())]),
        );
        let reason = inspect(&fixture.app).expect_err("a Simulator build");
        assert!(reason.contains("Simulator"), "{reason}");
        assert!(reason.contains("generic/platform=iOS"), "{reason}");
    }

    #[test]
    fn an_unsigned_or_unprovisioned_bundle_is_refused_with_the_missing_part() {
        let unsigned = Fixture::new("unsigned");
        fs::remove_dir_all(unsigned.app.join("_CodeSignature")).expect("unsigned");
        let reason = inspect(&unsigned.app).expect_err("unsigned");
        assert!(reason.contains("not signed"), "{reason}");

        let unprovisioned = Fixture::new("unprovisioned");
        fs::remove_file(unprovisioned.app.join("embedded.mobileprovision")).expect("no profile");
        let reason = inspect(&unprovisioned.app).expect_err("no profile");
        assert!(reason.contains("provisioning profile"), "{reason}");
    }

    #[test]
    fn something_that_is_not_a_bundle_is_refused() {
        let fixture = Fixture::new("not-a-bundle");
        let missing = fixture.root.join("Missing.app");
        assert!(
            inspect(&missing)
                .expect_err("missing")
                .contains("no such bundle")
        );
        let file = fixture.root.join("App.ipa");
        fs::write(&file, b"PK").expect("a file");
        assert!(
            inspect(&file).expect_err("a file").contains(".app"),
            "an archive is not a bundle"
        );
        fs::remove_file(fixture.app.join("Info.plist")).expect("no Info.plist");
        assert!(
            inspect(&fixture.app)
                .expect_err("no Info.plist")
                .contains("Info.plist")
        );
    }

    #[test]
    fn a_bundle_without_its_executable_or_identifier_is_refused() {
        let fixture = Fixture::new("no-executable");
        fs::remove_file(fixture.app.join("FixtureSwiftUI")).expect("no executable");
        assert!(
            inspect(&fixture.app)
                .expect_err("no executable")
                .contains("FixtureSwiftUI")
        );
        let nameless = Fixture::new("no-identifier");
        let mut info: Dictionary =
            plist::from_file(nameless.app.join("Info.plist")).expect("Info.plist");
        info.remove("CFBundleIdentifier");
        plist::to_file_binary(nameless.app.join("Info.plist"), &info).expect("Info.plist");
        assert!(
            inspect(&nameless.app)
                .expect_err("no identifier")
                .contains("CFBundleIdentifier")
        );
    }
}
