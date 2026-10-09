//! `IosDriver`: the driver traits over `idevice`, on a tokio runtime the
//! driver owns, as the Android driver's emulator controller owns one
//! (`crates/android/src/session/controller.rs`). The traits are synchronous;
//! each call blocks on the runtime, from a thread outside it.
//!
//! The logic lives in [`Core`], generic over the host so that the tests run
//! it on recorded listings and values instead of a device.

use plist::Dictionary;

use crate::driver::Devices;
use crate::error::{Error, Result};
use crate::model::{DeviceInfo, Probe};
use crate::readiness::Need;

use super::host::{Host, UsbHost};

/// Drives physical iOS devices attached over USB.
pub struct IosDriver {
    core: Core<UsbHost>,
}

impl IosDriver {
    /// Starts the driver's runtime; nothing is asked of usbmuxd or a device
    /// until a method needs it.
    pub fn open() -> Result<IosDriver> {
        Ok(IosDriver {
            core: Core::new(UsbHost)?,
        })
    }
}

impl Devices for IosDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        self.core.devices()
    }

    fn require(&self, target: &str, need: Need) -> Result<DeviceInfo> {
        self.core.require(target, need)
    }
}

/// The driver's state and logic over a host.
pub(crate) struct Core<H: Host> {
    runtime: tokio::runtime::Runtime,
    host: H,
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
        Ok(Core { runtime, host })
    }

    pub(crate) fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let _ = (
            &self.runtime,
            &self.host,
            Dictionary::new(),
            Probe::untrusted,
        );
        todo!()
    }

    pub(crate) fn require(&self, target: &str, need: Need) -> Result<DeviceInfo> {
        let _ = (target, need);
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use plist::Value;

    use super::*;
    use crate::discovery::tests::{identity, recorded_images, recorded_values};
    use crate::discovery::{DEVELOPER_MODE_KEY, LOCK_KEY, probe as probe_from};
    use crate::identity::IdentityCheck;
    use crate::model::{Connection, MuxEntry, Trust};
    use crate::readiness::Readiness;

    const PHONE: &str = "placeholder-udid-phone";

    /// A host that answers with recorded values instead of usbmuxd and
    /// lockdown.
    pub(crate) struct FakeHost {
        listing: std::result::Result<Vec<MuxEntry>, String>,
        devices: HashMap<String, (Dictionary, Probe)>,
        identity: IdentityCheck,
        probed: Mutex<Vec<MuxEntry>>,
    }

    impl FakeHost {
        /// The recorded phone, ready, listed on the network and on USB as
        /// Wi-Fi sync lists it.
        pub(crate) fn ready() -> FakeHost {
            let values = recorded_values();
            let probe = probe_from(&values, Trust::Trusted, Some(Ok(recorded_images())));
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
                devices: HashMap::from([(PHONE.to_owned(), (values, probe))]),
                identity: identity(),
                probed: Mutex::new(Vec::new()),
            }
        }

        /// The recorded phone with `values` changed and the probe that
        /// follows from them with `trust` and `images`.
        fn with(
            changes: &[(&str, Value)],
            trust: Trust,
            images: Option<std::result::Result<Vec<Value>, String>>,
        ) -> FakeHost {
            let mut values = recorded_values();
            for (key, value) in changes {
                values.insert((*key).to_owned(), value.clone());
            }
            let probe = probe_from(&values, trust, images);
            FakeHost {
                devices: HashMap::from([(PHONE.to_owned(), (values, probe))]),
                ..FakeHost::ready()
            }
        }
    }

    impl Host for FakeHost {
        async fn list(&self) -> Result<Vec<MuxEntry>> {
            self.listing
                .clone()
                .map_err(|reason| Error::Usbmuxd { reason })
        }

        async fn probe(&self, entry: &MuxEntry) -> (Dictionary, Probe) {
            self.probed.lock().expect("probed").push(entry.clone());
            self.devices.get(&entry.udid).cloned().unwrap_or_else(|| {
                (
                    Dictionary::new(),
                    Probe::untrusted(Trust::Unreachable("unknown".into())),
                )
            })
        }

        fn identity(&self) -> IdentityCheck {
            self.identity.clone()
        }
    }

    fn core(host: FakeHost) -> Core<FakeHost> {
        Core::new(host).expect("the runtime starts")
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
}
