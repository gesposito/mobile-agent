//! The developer disk image (design D18 and D20): the developer services,
//! the test manager and `dtuhidd` among them, live on it, and a device loses
//! it at every restart. When the image mounter lists none, the driver mounts
//! the selected Xcode's personalized image through `idevice`, and through
//! `xcrun devicectl device info ddiServices` when that fails or hangs, as it
//! hung once in spike S10. Either asks Apple's signing server for the
//! device's ticket, so a Mac that is offline cannot mount it.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use idevice::IdeviceService;
use idevice::lockdown::LockdownClient;
use idevice::mobile_image_mounter::ImageMounter;
use idevice::provider::{IdeviceProvider, UsbmuxdProvider};
use plist::{Dictionary, Value};
use tokio::time::timeout;

use crate::discovery::disk_image;
use crate::model::{DiskImage, MountAttempt, MountPath, MountReport};

use super::host::{describe, mounted_images};

/// Where Xcode 27 installs its developer disk images.
pub(crate) const DDI_DIRECTORY: &str = "/Library/Developer/DeveloperDiskImages/iOS_DDI/Restore";

/// Mounts the developer disk image on one device: the seam the tests
/// replace.
pub(crate) trait Mounter {
    /// The image mounter's `CopyDevices` entries.
    async fn images(&self) -> std::result::Result<Vec<Value>, String>;

    /// The personalized mount through `idevice`.
    async fn mount_with_idevice(&self) -> std::result::Result<(), String>;

    /// The mount through `devicectl`.
    async fn mount_with_devicectl(&self) -> std::result::Result<(), String>;
}

/// How long each way of mounting gets. A mount uploads a 16 MB image and
/// asks Apple's signing server; `idevice`'s hung for 5.5 minutes in S10.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MountLimits {
    pub idevice: Duration,
    pub devicectl: Duration,
}

impl Default for MountLimits {
    fn default() -> MountLimits {
        MountLimits {
            idevice: Duration::from_secs(60),
            devicectl: Duration::from_secs(120),
        }
    }
}

/// Why the image is still not mounted: every attempt, and whether a failure
/// said this Mac could not reach the signing server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MountFailure {
    pub attempts: Vec<MountAttempt>,
    pub offline: bool,
}

/// Mounts the developer disk image unless the image mounter lists it:
/// `idevice`'s personalized mount first, `devicectl`'s after it, each timed
/// and written to standard error as `label`'s; the image counts as mounted
/// once the image mounter lists it.
pub(crate) async fn ensure_mounted<M: Mounter>(
    mounter: &M,
    limits: MountLimits,
    label: &str,
) -> std::result::Result<MountReport, MountFailure> {
    let _ = (
        mounter,
        limits,
        label,
        Instant::now,
        timeout::<std::future::Ready<()>>,
        disk_image,
    );
    todo!()
}

/// Whether a mount's failure says this Mac could not reach Apple's signing
/// server.
pub(crate) fn looks_offline(failure: &str) -> bool {
    let _ = failure;
    todo!()
}

/// The image and trust cache, relative to the image's directory, that the
/// build manifest names for a device of `product_type`; `None` when the
/// manifest has no personalized identity for it, as for a device too new
/// for the image, which takes the Cryptex1 form.
pub(crate) fn ddi_files(manifest: &Dictionary, product_type: &str) -> Option<(String, String)> {
    let _ = (manifest, product_type);
    todo!()
}

/// Writes one line about the mount to standard error, the daemon's log.
fn log(label: &str, line: &str) {
    let _ = writeln!(std::io::stderr(), "mobile-agent ios {label}: {line}");
}

/// The real mounter: the image mounter through lockdown, the selected
/// Xcode's image, and `devicectl`.
pub(crate) struct UsbMounter {
    pub(crate) provider: UsbmuxdProvider,
    pub(crate) udid: String,
    /// Lockdown's `ProductType`, which picks the manifest's identity.
    pub(crate) product_type: Option<String>,
    pub(crate) directory: PathBuf,
}

impl Mounter for UsbMounter {
    async fn images(&self) -> std::result::Result<Vec<Value>, String> {
        mounted_images(&self.provider).await
    }

    async fn mount_with_idevice(&self) -> std::result::Result<(), String> {
        let _ = (
            &self.directory,
            &self.product_type,
            LockdownClient::connect,
            ImageMounter::connect,
            describe,
            log,
        );
        todo!()
    }

    async fn mount_with_devicectl(&self) -> std::result::Result<(), String> {
        let _ = &self.udid;
        todo!()
    }
}

/// The device's `UniqueChipID`, which the signing server's ticket names,
/// read in a lockdown session.
async fn unique_chip_id(provider: &dyn IdeviceProvider) -> std::result::Result<u64, String> {
    let _ = provider;
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macos::fake::FakeMounter;

    const LIMITS: MountLimits = MountLimits {
        idevice: Duration::from_millis(200),
        devicectl: Duration::from_millis(200),
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    }

    #[test]
    fn a_mounted_image_is_left_alone() {
        let mounter = FakeMounter::new(true, Err("not called".into()), Err("not called".into()));
        let report = runtime()
            .block_on(ensure_mounted(&mounter, LIMITS, "placeholder-udid-phone"))
            .expect("mounted");
        assert!(report.attempts.is_empty());
        assert!(matches!(report.image, DiskImage::Mounted { .. }));
        assert_eq!(mounter.calls(), vec!["images"]);
    }

    #[test]
    fn a_missing_image_is_mounted_through_idevice_and_checked() {
        let mounter = FakeMounter::new(false, Ok(()), Err("not called".into()));
        let report = runtime()
            .block_on(ensure_mounted(&mounter, LIMITS, "placeholder-udid-phone"))
            .expect("mounted");
        assert_eq!(report.attempts.len(), 1);
        assert_eq!(report.attempts[0].path, MountPath::Idevice);
        assert_eq!(report.attempts[0].failure, None);
        assert!(matches!(report.image, DiskImage::Mounted { .. }));
        assert_eq!(mounter.calls(), vec!["images", "idevice", "images"]);
    }

    #[test]
    fn idevice_hanging_or_failing_falls_back_to_devicectl() {
        let mut hanging = FakeMounter::new(false, Ok(()), Ok(()));
        hanging.idevice_hangs = true;
        let report = runtime()
            .block_on(ensure_mounted(&hanging, LIMITS, "placeholder-udid-phone"))
            .expect("mounted by devicectl");
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(report.attempts[0].path, MountPath::Idevice);
        let failure = report.attempts[0].failure.as_deref().expect("timed out");
        assert!(failure.contains("did not finish within"), "{failure}");
        assert!(report.attempts[0].elapsed >= LIMITS.idevice);
        assert_eq!(report.attempts[1].path, MountPath::Devicectl);
        assert_eq!(report.attempts[1].failure, None);

        let failing = FakeMounter::new(false, Err("ImageMounter: InternalError".into()), Ok(()));
        let report = runtime()
            .block_on(ensure_mounted(&failing, LIMITS, "placeholder-udid-phone"))
            .expect("mounted by devicectl");
        assert_eq!(
            report
                .attempts
                .iter()
                .map(|attempt| attempt.path)
                .collect::<Vec<_>>(),
            vec![MountPath::Idevice, MountPath::Devicectl]
        );
        assert_eq!(
            report.attempts[0].failure.as_deref(),
            Some("ImageMounter: InternalError")
        );
    }

    #[test]
    fn a_success_the_image_mounter_does_not_confirm_falls_back_too() {
        let mut mounter = FakeMounter::new(false, Ok(()), Ok(()));
        mounter.idevice_mounts = false;
        let report = runtime()
            .block_on(ensure_mounted(&mounter, LIMITS, "placeholder-udid-phone"))
            .expect("mounted by devicectl");
        assert_eq!(
            mounter.calls(),
            vec!["images", "idevice", "images", "devicectl", "images"]
        );
        assert_eq!(report.attempts.len(), 2);
        let hollow = report.attempts[0]
            .failure
            .as_deref()
            .expect("not confirmed");
        assert!(hollow.contains("lists no developer disk image"), "{hollow}");
    }

    #[test]
    fn both_failing_offline_say_the_signing_server_is_needed() {
        let mounter = FakeMounter::new(
            false,
            Err("HTTP request failed: dns error: failed to lookup address information".into()),
            Err("devicectl exited with 1: The Internet connection appears to be offline.".into()),
        );
        let failure = runtime()
            .block_on(ensure_mounted(&mounter, LIMITS, "placeholder-udid-phone"))
            .expect_err("nothing mounted");
        assert_eq!(failure.attempts.len(), 2);
        assert!(failure.offline);
        let error = crate::error::Error::DiskImage {
            target: "placeholder-udid-phone".into(),
            attempts: failure.attempts,
            offline: failure.offline,
        };
        assert_eq!(error.code(), "disk_image_unmounted");
        let message = error.to_string();
        for part in [
            "signing server",
            "offline",
            "idevice's personalized mount",
            "devicectl",
        ] {
            assert!(
                message.contains(part),
                "{part:?} is missing from: {message}"
            );
        }
    }

    #[test]
    fn failures_that_name_the_network_are_offline_and_others_are_not() {
        for failure in [
            "HTTP request failed: dns error",
            "error sending request for url (https://gs.apple.com/TSS/controller?action=2)",
            "The Internet connection appears to be offline.",
            "Could not connect to the server.",
            "A server with the specified hostname could not be found.",
        ] {
            assert!(looks_offline(failure), "{failure}");
        }
        for failure in [
            "ImageMounter: InternalError",
            "did not finish within 60s",
            "devicectl exited with 1: The device is locked.",
        ] {
            assert!(!looks_offline(failure), "{failure}");
        }
    }

    #[test]
    fn the_manifest_names_the_image_and_trust_cache_of_the_devices_identity() {
        let manifest: Dictionary = plist::from_bytes(include_bytes!(
            "../../tests/fixtures/ddi-build-manifest.plist"
        ))
        .expect("the manifest parses");
        assert_eq!(
            ddi_files(&manifest, "iPhone14,6"),
            Some((
                "022-20190-411.dmg".to_owned(),
                "Firmware/022-20190-411.dmg.trustcache".to_owned()
            ))
        );
        assert_eq!(
            ddi_files(&manifest, "iPhone18,9"),
            None,
            "a device the manifest does not name takes the Cryptex1 form"
        );
    }
}
