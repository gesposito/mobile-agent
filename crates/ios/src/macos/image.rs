//! The developer disk image (design D18 and D20): the developer services,
//! the test manager and `dtuhidd` among them, live on it, and a device loses
//! it at every restart. When the image mounter lists none, the driver mounts
//! the selected Xcode's personalized image through `idevice`, and through
//! `xcrun devicectl device info ddiServices` when that fails or hangs, as it
//! hung once in spike S10. Either asks Apple's signing server for the
//! device's ticket, so a Mac that is offline cannot mount it.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
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

/// How long `devicectl` gets, which it is told as its own timeout too.
const DEVICECTL_SECONDS: u64 = 120;

impl Default for MountLimits {
    fn default() -> MountLimits {
        MountLimits {
            idevice: Duration::from_secs(60),
            devicectl: Duration::from_secs(DEVICECTL_SECONDS),
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
    let listed = |entries: std::result::Result<Vec<Value>, String>| match entries {
        Ok(entries) => disk_image(&entries),
        Err(reason) => DiskImage::Unreadable(reason),
    };
    let image = listed(mounter.images().await);
    if matches!(image, DiskImage::Mounted { .. }) {
        return Ok(MountReport {
            image,
            attempts: Vec::new(),
        });
    }
    log(
        label,
        &format!("the developer disk image is not mounted ({image:?}); mounting it"),
    );
    let mut attempts = Vec::new();
    for path in [MountPath::Idevice, MountPath::Devicectl] {
        let started = Instant::now();
        let (outcome, limit) = match path {
            MountPath::Idevice => (
                timeout(limits.idevice, mounter.mount_with_idevice()).await,
                limits.idevice,
            ),
            MountPath::Devicectl => (
                timeout(limits.devicectl, mounter.mount_with_devicectl()).await,
                limits.devicectl,
            ),
        };
        let elapsed = started.elapsed();
        let mut failure = match outcome {
            Ok(Ok(())) => None,
            Ok(Err(reason)) => Some(reason),
            Err(_) => Some(format!("did not finish within {limit:?}")),
        };
        // Whatever the path said, the image mounter tells: in S10 a mount
        // that never returned had mounted the image.
        let image = listed(mounter.images().await);
        let mounted = matches!(image, DiskImage::Mounted { .. });
        if failure.is_none() && !mounted {
            failure = Some(
                "it reported success, but the image mounter lists no developer disk image".into(),
            );
        }
        log(
            label,
            &format!(
                "{path} after {:.1} s: {}{}",
                elapsed.as_secs_f64(),
                failure.as_deref().unwrap_or("done"),
                if mounted {
                    "; the image is mounted"
                } else {
                    ""
                }
            ),
        );
        attempts.push(MountAttempt {
            path,
            elapsed,
            failure,
        });
        if mounted {
            return Ok(MountReport { image, attempts });
        }
    }
    let offline = attempts
        .iter()
        .filter_map(|attempt| attempt.failure.as_deref())
        .any(looks_offline);
    Err(MountFailure { attempts, offline })
}

/// Whether a mount's failure says this Mac could not reach Apple's signing
/// server.
pub(crate) fn looks_offline(failure: &str) -> bool {
    let failure = failure.to_ascii_lowercase();
    [
        "http request failed",
        "error sending request",
        "dns",
        "gs.apple.com",
        "offline",
        "could not connect to the server",
        "hostname could not be found",
        "network is unreachable",
    ]
    .iter()
    .any(|sign| failure.contains(sign))
}

/// The image and trust cache, relative to the image's directory, that the
/// build manifest names for a device of `product_type`; `None` when the
/// manifest has no personalized identity for it, as for a device too new
/// for the image, which takes the Cryptex1 form.
pub(crate) fn ddi_files(manifest: &Dictionary, product_type: &str) -> Option<(String, String)> {
    let identities = manifest.get("BuildIdentities")?.as_array()?;
    identities
        .iter()
        .filter_map(Value::as_dictionary)
        .filter(|identity| {
            identity.get("Ap,ProductType").and_then(Value::as_string) == Some(product_type)
        })
        .find_map(|identity| {
            let path = |component: &str| {
                identity
                    .get("Manifest")?
                    .as_dictionary()?
                    .get(component)?
                    .as_dictionary()?
                    .get("Info")?
                    .as_dictionary()?
                    .get("Path")?
                    .as_string()
                    .map(str::to_owned)
            };
            Some((path("PersonalizedDMG")?, path("LoadableTrustCache")?))
        })
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
        let directory = &self.directory;
        let read = |name: &str| {
            std::fs::read(directory.join(name))
                .map_err(|error| format!("reading {}: {error}", directory.join(name).display()))
        };
        let manifest_bytes = read("BuildManifest.plist")?;
        let manifest: Dictionary = plist::from_bytes(&manifest_bytes)
            .map_err(|error| format!("the developer disk image's build manifest: {error}"))?;
        let product_type = self
            .product_type
            .as_deref()
            .ok_or("lockdown gave no ProductType to pick the image's identity with")?;
        let (image, trust_cache) = ddi_files(&manifest, product_type).ok_or_else(|| {
            format!(
                "{product_type} is not in the build manifest of {}; such a device takes the image \
                 as a Cryptex1, which devicectl mounts",
                directory.display()
            )
        })?;
        let image = read(&image)?;
        let trust_cache = read(&trust_cache)?;
        let chip = unique_chip_id(&self.provider).await?;
        let mut mounter = ImageMounter::connect(&self.provider)
            .await
            .map_err(|error| format!("the image mounter did not start: {}", describe(&error)))?;
        mounter
            .mount_personalized(
                &self.provider,
                image,
                trust_cache,
                &manifest_bytes,
                None,
                chip,
            )
            .await
            .map_err(|error| format!("the personalized mount: {}", describe(&error)))
    }

    async fn mount_with_devicectl(&self) -> std::result::Result<(), String> {
        let timeout = DEVICECTL_SECONDS.to_string();
        let output = tokio::process::Command::new("xcrun")
            .args(["devicectl", "device", "info", "ddiServices", "--device"])
            .arg(&self.udid)
            .args(["--auto-mount-ddis", "--quiet", "--timeout", &timeout])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| format!("xcrun devicectl did not start: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
        Err(format!(
            "devicectl exited with {}: {}",
            output.status,
            last.map_or("no message", str::trim)
        ))
    }
}

/// The device's `UniqueChipID`, which the signing server's ticket names,
/// read in a lockdown session.
async fn unique_chip_id(provider: &dyn IdeviceProvider) -> std::result::Result<u64, String> {
    let lockdown_error = |error: idevice::IdeviceError| format!("lockdown: {}", describe(&error));
    let mut lockdown = LockdownClient::connect(provider)
        .await
        .map_err(lockdown_error)?;
    let pairing = provider.get_pairing_file().await.map_err(lockdown_error)?;
    lockdown
        .start_session(&pairing)
        .await
        .map_err(lockdown_error)?;
    lockdown
        .get_value(Some("UniqueChipID"), None)
        .await
        .map_err(lockdown_error)?
        .as_unsigned_integer()
        .ok_or_else(|| "lockdown's UniqueChipID is not an integer".to_owned())
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
