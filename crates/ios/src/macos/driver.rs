//! `IosDriver`: the driver traits over `idevice`, on a tokio runtime the
//! driver owns, as the Android driver's emulator controller owns one
//! (`crates/android/src/session/controller.rs`). The traits are synchronous;
//! each call blocks on the runtime, from a thread outside it.
//!
//! The logic lives in [`Core`], generic over the host so that the tests run
//! it on recorded listings and values and a fake endpoint instead of a
//! device. What the driver opens for a device, its tunnel and the service
//! connections over it, is kept per target until the target is reset.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::discovery::{self, usb_entries};
use crate::driver::Devices;
use crate::error::{Error, Result};
use crate::model::{Connected, DeviceInfo, MountReport, MuxEntry, TargetStatus};
use crate::readiness::{self, Need};

use super::host::{Host, UsbHost};
use super::image::{self, MountLimits};
use super::tunnel::{Limits, Opener, Tunnel};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Drives physical iOS devices attached over USB.
pub struct IosDriver {
    core: Core<UsbHost>,
}

impl IosDriver {
    /// Starts the driver's runtime; nothing is asked of usbmuxd or a device
    /// until a method needs it.
    pub fn open() -> Result<IosDriver> {
        Ok(IosDriver {
            core: Core::new(UsbHost::new()?)?,
        })
    }

    /// Closes what the driver opened for `target`, its tunnel and the
    /// service connections over it; the next command opens them anew.
    pub fn reset_target(&self, target: &str) -> Result<()> {
        self.core.reset_target(target)
    }
}

impl Devices for IosDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        self.core.devices()
    }

    fn require(&self, target: &str, need: Need) -> Result<DeviceInfo> {
        self.core.require(target, need)
    }

    fn connect(&self, target: &str) -> Result<Connected> {
        self.core.connect(target)
    }

    fn status(&self, target: &str) -> Result<TargetStatus> {
        self.core.status(target)
    }
}

/// What the driver keeps for one device.
pub(crate) struct Target<O: Opener> {
    tunnel: Tunnel<O>,
    disk_image: Option<MountReport>,
}

/// A device's state, behind the lock its commands take, and what `status`
/// reports, which it reads without that lock while a command runs.
pub(crate) struct Slot<O: Opener> {
    state: tokio::sync::Mutex<Target<O>>,
    snapshot: Mutex<TargetStatus>,
}

/// The driver's state and logic over a host.
pub(crate) struct Core<H: Host> {
    runtime: tokio::runtime::Runtime,
    host: H,
    targets: Mutex<HashMap<String, Arc<Slot<H::Opener>>>>,
    limits: Limits,
    mount_limits: MountLimits,
}

impl<H: Host> Core<H> {
    pub(crate) fn new(host: H) -> Result<Core<H>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ios-driver")
            .enable_all()
            .build()
            .map_err(|error| Error::Io {
                what: "starting the iOS driver's runtime".into(),
                reason: error.to_string(),
            })?;
        Ok(Core {
            runtime,
            host,
            targets: Mutex::new(HashMap::new()),
            limits: Limits::default(),
            mount_limits: MountLimits::default(),
        })
    }

    /// Shorter waits, for the tests.
    #[cfg(test)]
    pub(crate) fn with_limits(mut self, limits: Limits, mount_limits: MountLimits) -> Core<H> {
        self.limits = limits;
        self.mount_limits = mount_limits;
        self
    }

    pub(crate) fn devices(&self) -> Result<Vec<DeviceInfo>> {
        self.runtime.block_on(async {
            let entries = usb_entries(self.host.list().await?);
            let identity = self.host.identity();
            let mut devices = Vec::with_capacity(entries.len());
            for entry in &entries {
                let (values, probe) = self.host.probe(entry).await;
                devices.push(discovery::device_info(
                    &entry.udid,
                    &values,
                    probe,
                    &identity,
                ));
            }
            Ok(devices)
        })
    }

    pub(crate) fn require(&self, target: &str, need: Need) -> Result<DeviceInfo> {
        self.runtime.block_on(async {
            let entry = self.entry(target).await?;
            let (values, probe) = self.host.probe(&entry).await;
            let identity = self.host.identity();
            let info = discovery::device_info(target, &values, probe, &identity);
            readiness::assess(info.ios_version.as_deref(), &info.probe, &identity, need).map_err(
                |readiness| Error::NotReady {
                    target: target.to_owned(),
                    readiness,
                },
            )?;
            Ok(info)
        })
    }

    /// Gets the device ready for its developer services: fails on what
    /// stands in the way, mounts the developer disk image when it is
    /// missing, then opens the tunnel unless it is open.
    pub(crate) fn connect(&self, target: &str) -> Result<Connected> {
        let _ = (target, image::ensure_mounted::<H::Mounter>, lock::<()>);
        todo!()
    }

    /// What the driver holds for `target`; nothing for a device it has not
    /// connected to.
    pub(crate) fn status(&self, target: &str) -> Result<TargetStatus> {
        let _ = target;
        todo!()
    }

    /// Closes what the driver opened for `target`.
    pub(crate) fn reset_target(&self, target: &str) -> Result<()> {
        let _ = target;
        todo!()
    }

    /// The USB entry of the device whose identifier is `target`.
    async fn entry(&self, target: &str) -> Result<MuxEntry> {
        usb_entries(self.host.list().await?)
            .into_iter()
            .find(|entry| entry.udid == target)
            .ok_or_else(|| Error::NotFound {
                target: target.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use plist::{Dictionary, Value};

    use super::*;
    use crate::discovery::tests::{identity, recorded_images, recorded_values};
    use crate::discovery::{DEVELOPER_MODE_KEY, LOCK_KEY, probe as probe_from};
    use crate::identity::IdentityCheck;
    use crate::macos::fake::{FakeDevice, FakeMounter, FakeOpener};
    use crate::model::{Connection, DiskImage, MountPath, Probe, Trust};
    use crate::readiness::Readiness;
    use crate::services;

    pub(crate) const PHONE: &str = "placeholder-udid-phone";

    /// What the fake host's image mounter lists.
    #[derive(Clone)]
    pub(crate) enum Images {
        /// What the probe's image mounter answered, or `None` when it was
        /// not asked.
        Fixed(Option<std::result::Result<Vec<Value>, String>>),
        /// What the fake mounter has mounted so far.
        Mounter,
    }

    /// A host that answers with recorded values instead of usbmuxd and
    /// lockdown, and whose phone is a fake device behind a fake tunnel.
    pub(crate) struct FakeHost {
        pub(crate) listing: std::result::Result<Vec<MuxEntry>, String>,
        pub(crate) values: Dictionary,
        pub(crate) trust: Trust,
        pub(crate) images: Images,
        pub(crate) identity: IdentityCheck,
        pub(crate) device: Arc<FakeDevice>,
        pub(crate) mounter: FakeMounter,
        pub(crate) probed: Mutex<Vec<MuxEntry>>,
    }

    impl FakeHost {
        /// The recorded phone, ready, listed on the network and on USB as
        /// Wi-Fi sync lists it.
        pub(crate) fn ready() -> FakeHost {
            FakeHost {
                listing: Ok(vec![
                    MuxEntry {
                        udid: PHONE.into(),
                        device_id: 2,
                        connection: Connection::Network,
                    },
                    MuxEntry {
                        udid: PHONE.into(),
                        device_id: 1,
                        connection: Connection::Usb,
                    },
                ]),
                values: recorded_values(),
                trust: Trust::Trusted,
                images: Images::Fixed(Some(Ok(recorded_images()))),
                identity: identity(),
                device: FakeDevice::recorded(),
                mounter: FakeMounter::new(true, Err("not called".into()), Err("not called".into())),
                probed: Mutex::new(Vec::new()),
            }
        }

        /// The recorded phone with `values` changed, `trust` and `images`.
        pub(crate) fn with(
            changes: &[(&str, Value)],
            trust: Trust,
            images: Option<std::result::Result<Vec<Value>, String>>,
        ) -> FakeHost {
            let mut values = recorded_values();
            for (key, value) in changes {
                values.insert((*key).to_owned(), value.clone());
            }
            FakeHost {
                values,
                trust,
                images: Images::Fixed(images),
                ..FakeHost::ready()
            }
        }

        /// The recorded phone without the disk image, which `mounter`
        /// mounts as it says.
        pub(crate) fn unmounted(mounter: FakeMounter) -> FakeHost {
            FakeHost {
                images: Images::Mounter,
                mounter,
                ..FakeHost::ready()
            }
        }
    }

    impl Host for FakeHost {
        type Opener = FakeOpener;
        type Mounter = FakeMounter;

        async fn list(&self) -> Result<Vec<MuxEntry>> {
            self.listing
                .clone()
                .map_err(|reason| Error::Usbmuxd { reason })
        }

        async fn probe(&self, entry: &MuxEntry) -> (Dictionary, Probe) {
            self.probed.lock().expect("probed").push(entry.clone());
            if entry.udid != PHONE {
                return (
                    Dictionary::new(),
                    Probe::untrusted(Trust::Unreachable("unknown".into())),
                );
            }
            let images = match &self.images {
                Images::Fixed(images) => images.clone(),
                Images::Mounter => Some(Ok(self.mounter.listing())),
            };
            let probe = probe_from(&self.values, self.trust.clone(), images);
            (self.values.clone(), probe)
        }

        fn identity(&self) -> IdentityCheck {
            self.identity.clone()
        }

        fn opener(&self, _entry: &MuxEntry) -> FakeOpener {
            FakeOpener::new(&self.device)
        }

        fn mounter(&self, _entry: &MuxEntry, _device: &DeviceInfo) -> FakeMounter {
            self.mounter.clone()
        }
    }

    pub(crate) const SHORT: Limits = Limits {
        open: Duration::from_millis(500),
        connect: Duration::from_millis(500),
    };

    pub(crate) const SHORT_MOUNTS: MountLimits = MountLimits {
        idevice: Duration::from_millis(200),
        devicectl: Duration::from_millis(200),
    };

    pub(crate) fn core(host: FakeHost) -> Core<FakeHost> {
        Core::new(host)
            .expect("the runtime starts")
            .with_limits(SHORT, SHORT_MOUNTS)
    }

    #[test]
    fn a_ready_phone_is_listed_once_with_its_identifier_name_model_and_version() {
        let core = core(FakeHost::ready());
        let devices = core.devices().expect("devices");
        assert_eq!(devices.len(), 1, "{devices:?}");
        let phone = &devices[0];
        assert_eq!(phone.udid, PHONE);
        assert_eq!(phone.name.as_deref(), Some("placeholder-iphone"));
        assert_eq!(phone.model.as_deref(), Some("iPhone14,6"));
        assert_eq!(phone.ios_version.as_deref(), Some("26.6.2"));
        assert_eq!(phone.readiness, Readiness::Ready);
        let probed = core.host.probed.lock().expect("probed").clone();
        assert_eq!(probed.len(), 1, "probed once: {probed:?}");
        assert_eq!(
            probed[0].connection,
            Connection::Usb,
            "through its USB entry"
        );
        assert_eq!(probed[0].device_id, 1);
    }

    /// Each unready phone, with the code a command on it fails with.
    fn unready() -> Vec<(FakeHost, &'static str)> {
        vec![
            (FakeHost::with(&[], Trust::NotPaired, None), "not_trusted"),
            (FakeHost::with(&[], Trust::Refused, None), "not_trusted"),
            (
                FakeHost::with(
                    &[(DEVELOPER_MODE_KEY, Value::Boolean(false))],
                    Trust::Trusted,
                    None,
                ),
                "developer_mode_off",
            ),
            (
                FakeHost::with(&[], Trust::Trusted, Some(Ok(Vec::new()))),
                "disk_image_unmounted",
            ),
            (
                FakeHost::with(
                    &[(LOCK_KEY, Value::Boolean(true))],
                    Trust::Trusted,
                    Some(Ok(recorded_images())),
                ),
                "device_locked",
            ),
            (
                FakeHost {
                    identity: IdentityCheck::NoTeam,
                    ..FakeHost::ready()
                },
                "no_signing_identity",
            ),
            (
                FakeHost::with(
                    &[("ProductVersion", Value::String("16.7.10".into()))],
                    Trust::Trusted,
                    Some(Ok(recorded_images())),
                ),
                "unsupported_ios",
            ),
        ]
    }

    #[test]
    fn an_unready_phone_is_listed_in_its_state_with_the_step_to_take() {
        for (host, code) in unready() {
            let devices = core(host).devices().expect("devices");
            assert_eq!(devices.len(), 1, "{code}");
            let state = &devices[0].readiness;
            assert_eq!(state.code(), code);
            assert!(!state.message().is_empty(), "{code}");
        }
    }

    #[test]
    fn a_command_on_an_unready_phone_fails_with_a_code_naming_the_state() {
        for (host, code) in unready() {
            let error = core(host).require(PHONE, Need::Session).expect_err(code);
            assert_eq!(error.code(), code, "{error}");
            assert!(error.to_string().contains(PHONE), "{error}");
        }
        let ready = core(FakeHost::ready())
            .require(PHONE, Need::Session)
            .expect("a ready phone");
        assert_eq!(ready.readiness, Readiness::Ready);
    }

    #[test]
    fn a_command_needs_only_what_its_operation_needs() {
        let locked = FakeHost::with(
            &[(LOCK_KEY, Value::Boolean(true))],
            Trust::Trusted,
            Some(Ok(recorded_images())),
        );
        let core = core(locked);
        let info = core
            .require(PHONE, Need::DiskImage)
            .expect("the tunnel's services need no unlock");
        assert_eq!(info.readiness, Readiness::Locked, "still listed as locked");
        assert_eq!(
            core.require(PHONE, Need::Unlocked)
                .expect_err("a launch needs it unlocked")
                .code(),
            "device_locked"
        );
    }

    #[test]
    fn an_identifier_no_usb_device_has_is_not_found_and_names_are_not_identifiers() {
        let core = core(FakeHost::ready());
        for target in ["placeholder-udid-other", "placeholder-iphone", ""] {
            let error = core.require(target, Need::Trust).expect_err(target);
            assert_eq!(error.code(), "not_found", "{target}: {error}");
        }
    }

    #[test]
    fn usbmuxd_that_cannot_be_asked_is_reported() {
        let host = FakeHost {
            listing: Err("connecting to /var/run/usbmuxd: no such file".into()),
            ..FakeHost::ready()
        };
        let error = core(host).devices().expect_err("no usbmuxd");
        assert_eq!(error.code(), "usbmuxd_unavailable");
        assert!(error.to_string().contains("no such file"), "{error}");
    }

    #[test]
    fn connecting_a_ready_phone_opens_the_tunnel_and_lists_its_services() {
        let core = core(FakeHost::ready());
        assert_eq!(core.status(PHONE).expect("status"), TargetStatus::default());
        let connected = core.connect(PHONE).expect("connects");
        assert_eq!(connected.device.udid, PHONE);
        assert!(connected.disk_image.attempts.is_empty(), "already mounted");
        assert!(matches!(
            connected.disk_image.image,
            DiskImage::Mounted { .. }
        ));
        assert_eq!(connected.tunnel.opens, 1);
        assert_eq!(connected.tunnel.services.len(), 82);
        assert!(connected.tunnel.missing.is_empty());
        let status = core.status(PHONE).expect("status");
        assert_eq!(status.tunnel.as_ref(), Some(&connected.tunnel));
        assert_eq!(status.disk_image.as_ref(), Some(&connected.disk_image));
        let again = core.connect(PHONE).expect("connects again");
        assert_eq!(again.tunnel.opens, 1, "the tunnel is kept");
        assert_eq!(core.host.device.opens(), 1);
    }

    #[test]
    fn connecting_a_locked_phone_opens_the_tunnel_since_its_services_need_no_unlock() {
        let core = core(FakeHost::with(
            &[(LOCK_KEY, Value::Boolean(true))],
            Trust::Trusted,
            Some(Ok(recorded_images())),
        ));
        let connected = core.connect(PHONE).expect("connects");
        assert_eq!(connected.device.readiness, Readiness::Locked);
        assert_eq!(connected.tunnel.opens, 1);
    }

    #[test]
    fn connecting_a_phone_that_is_not_ready_opens_nothing_and_names_its_state() {
        for (host, code) in [
            (FakeHost::with(&[], Trust::NotPaired, None), "not_trusted"),
            (
                FakeHost::with(
                    &[(DEVELOPER_MODE_KEY, Value::Boolean(false))],
                    Trust::Trusted,
                    None,
                ),
                "developer_mode_off",
            ),
        ] {
            let core = core(host);
            let error = core.connect(PHONE).expect_err(code);
            assert_eq!(error.code(), code, "{error}");
            assert_eq!(core.host.device.opens(), 0, "{code}: no tunnel");
            assert_eq!(
                core.host.mounter.calls(),
                Vec::<&str>::new(),
                "{code}: no mount"
            );
            assert_eq!(core.status(PHONE).expect("status"), TargetStatus::default());
        }
    }

    #[test]
    fn a_phone_without_the_disk_image_has_it_mounted_before_the_tunnel_opens() {
        let core = core(FakeHost::unmounted(FakeMounter::new(
            false,
            Ok(()),
            Err("not called".into()),
        )));
        let connected = core.connect(PHONE).expect("mounted and connected");
        assert_eq!(connected.disk_image.attempts.len(), 1);
        assert_eq!(connected.disk_image.attempts[0].path, MountPath::Idevice);
        assert!(matches!(
            connected.disk_image.image,
            DiskImage::Mounted { .. }
        ));
        assert_eq!(connected.device.readiness, Readiness::Ready);
        assert_eq!(connected.tunnel.opens, 1);
        assert_eq!(
            core.status(PHONE).expect("status").disk_image,
            Some(connected.disk_image)
        );
    }

    #[test]
    fn a_mount_that_fails_offline_fails_naming_the_signing_server_and_opens_nothing() {
        let core = core(FakeHost::unmounted(FakeMounter::new(
            false,
            Err("HTTP request failed: dns error".into()),
            Err("devicectl exited with 1: The Internet connection appears to be offline.".into()),
        )));
        let error = core.connect(PHONE).expect_err("not mounted");
        assert_eq!(error.code(), "disk_image_unmounted");
        assert!(error.to_string().contains("signing server"), "{error}");
        assert_eq!(core.host.device.opens(), 0);
    }

    #[test]
    fn missing_services_are_reported_with_the_tunnel() {
        let host = FakeHost {
            device: FakeDevice::without(&[services::HID_INDIGO, services::HID_UNIVERSAL]),
            ..FakeHost::ready()
        };
        let connected = core(host).connect(PHONE).expect("connects");
        assert_eq!(
            connected.tunnel.missing,
            vec![services::HID_INDIGO, services::HID_UNIVERSAL]
        );
    }

    #[test]
    fn reset_closes_the_tunnel_and_the_next_connect_opens_a_new_one() {
        let core = core(FakeHost::ready());
        core.connect(PHONE).expect("connects");
        core.reset_target(PHONE).expect("resets");
        assert_eq!(core.status(PHONE).expect("status"), TargetStatus::default());
        assert_eq!(core.host.device.closed(), 1);
        let connected = core.connect(PHONE).expect("connects again");
        assert_eq!(connected.tunnel.opens, 1, "a new target's first tunnel");
        assert_eq!(core.host.device.opens(), 2);
        core.reset_target("placeholder-udid-other")
            .expect("resetting what was never opened does nothing");
    }

    #[test]
    fn connecting_an_unknown_identifier_is_not_found() {
        let error = core(FakeHost::ready())
            .connect("placeholder-udid-other")
            .expect_err("no such device");
        assert_eq!(error.code(), "not_found");
    }
}
