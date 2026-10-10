//! Discovery (design D18): usbmuxd's listing reduced to one USB entry per
//! device, and what the driver read from each device turned into its
//! description, its probe and its readiness. The reading itself, over
//! usbmuxd and lockdown, is `macos/host.rs`; everything here works on the
//! values it returns, recorded ones in the tests.

use plist::{Dictionary, Value};

use crate::identity::IdentityCheck;
use crate::model::{Battery, Connection, DeviceInfo, DiskImage, MuxEntry, Probe, Trust};
use crate::readiness;

/// The lockdown values that describe a device, read without a session.
pub const DESCRIPTION_KEYS: [&str; 4] = [
    "DeviceName",
    "ProductType",
    "ProductVersion",
    "BuildVersion",
];

/// The key the driver stores lockdown's `DeveloperModeStatus` of the
/// `com.apple.security.mac.amfi` domain under.
pub const DEVELOPER_MODE_KEY: &str = "DeveloperModeStatus";
/// The domain `DeveloperModeStatus` is read from.
pub const DEVELOPER_MODE_DOMAIN: &str = "com.apple.security.mac.amfi";
/// Lockdown's `PasswordProtected`, which follows the lock (spike S9).
pub const LOCK_KEY: &str = "PasswordProtected";
/// The lockdown domain that gives the screen's pixels and scale.
pub const SCREEN_DOMAIN: &str = "com.apple.mobile.iTunes";
/// The lockdown domain that gives the battery, read inside a session; the
/// probe stores its dictionary under the domain's name.
pub const BATTERY_DOMAIN: &str = "com.apple.mobile.battery";

/// The screen `values`, lockdown's `com.apple.mobile.iTunes` domain, give:
/// `ScreenWidth` and `ScreenHeight` in pixels and `ScreenScaleFactor`, 750,
/// 1334 and 2.0 on the iPhone SE (3rd generation).
pub fn screen(values: &Value) -> Option<crate::model::Screen> {
    let values = values.as_dictionary()?;
    let pixels = |key: &str| {
        values
            .get(key)?
            .as_unsigned_integer()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let scale = match values.get("ScreenScaleFactor")? {
        Value::Real(scale) => *scale,
        Value::Integer(scale) => scale.as_unsigned()? as f64,
        _ => return None,
    };
    (scale > 0.0).then_some(crate::model::Screen {
        width: pixels("ScreenWidth")?,
        height: pixels("ScreenHeight")?,
        scale,
    })
}

/// One entry per device attached over USB, in usbmuxd's order. A phone with
/// Wi-Fi sync on is listed twice, on USB and on the network; its USB entry is
/// kept. A device listed only on the network is dropped, since this driver
/// drives devices over USB only.
pub fn usb_entries(entries: Vec<MuxEntry>) -> Vec<MuxEntry> {
    let mut kept: Vec<MuxEntry> = Vec::new();
    for entry in entries {
        if entry.connection == Connection::Usb && !kept.iter().any(|seen| seen.udid == entry.udid) {
            kept.push(entry);
        }
    }
    kept
}

/// What the probe read, from the values it stored (the description, then
/// `DeveloperModeStatus`, `PasswordProtected` and the battery domain when the
/// session started),
/// whether the session started, and the image mounter's `CopyDevices`
/// entries when it was asked.
pub fn probe(
    values: &Dictionary,
    trust: Trust,
    images: Option<Result<Vec<Value>, String>>,
) -> Probe {
    if trust != Trust::Trusted {
        return Probe::untrusted(trust);
    }
    Probe {
        trust,
        developer_mode: values.get(DEVELOPER_MODE_KEY).and_then(Value::as_boolean),
        disk_image: images.map(|images| match images {
            Ok(entries) => disk_image(&entries),
            Err(reason) => DiskImage::Unreadable(reason),
        }),
        password_protected: values.get(LOCK_KEY).and_then(Value::as_boolean),
    }
}

/// The developer disk image from the image mounter's `CopyDevices` entries:
/// mounted when an entry is a developer image (`Personalized` with
/// `PersonalizedImageType` `DeveloperDiskImage`, or `Developer` before iOS 17)
/// that says it is mounted or names its mount path.
pub fn disk_image(entries: &[Value]) -> DiskImage {
    for entry in entries {
        let Some(entry) = entry.as_dictionary() else {
            continue;
        };
        let text = |key: &str| entry.get(key).and_then(Value::as_string);
        let developer = match text("DiskImageType") {
            Some("Personalized") => text("PersonalizedImageType")
                .is_none_or(|personalized| personalized == "DeveloperDiskImage"),
            Some("Developer") => true,
            _ => false,
        };
        if !developer {
            continue;
        }
        let mount_path = text("MountPath").map(str::to_owned);
        let mounted = entry
            .get("IsMounted")
            .and_then(Value::as_boolean)
            .unwrap_or(mount_path.is_some());
        if mounted {
            return DiskImage::Mounted {
                image_type: text("DiskImageType").unwrap_or_default().to_owned(),
                mount_path,
            };
        }
    }
    DiskImage::NotMounted
}

/// The device as `devices` lists it: its identifier, its description from
/// lockdown's values, what the probe read and the readiness that follows,
/// with the host's signing identity.
pub fn device_info(
    udid: &str,
    values: &Dictionary,
    probe: Probe,
    identity: &IdentityCheck,
) -> DeviceInfo {
    let text = |key: &str| {
        values
            .get(key)
            .and_then(Value::as_string)
            .map(str::to_owned)
    };
    let ios_version = text("ProductVersion");
    let readiness = readiness::readiness(ios_version.as_deref(), &probe, identity);
    DeviceInfo {
        udid: udid.to_owned(),
        name: text("DeviceName"),
        model: text("ProductType"),
        ios_version,
        build: text("BuildVersion"),
        battery: values.get(BATTERY_DOMAIN).and_then(battery),
        probe,
        readiness,
    }
}

/// The battery lockdown's `com.apple.mobile.battery` domain gives in
/// `values`: `BatteryCurrentCapacity`, `BatteryIsCharging` and
/// `ExternalConnected`; `None` when it gives none of the three.
pub fn battery(values: &Value) -> Option<Battery> {
    let values = values.as_dictionary()?;
    let flag = |key: &str| values.get(key).and_then(Value::as_boolean);
    let battery = Battery {
        level: values
            .get("BatteryCurrentCapacity")
            .and_then(Value::as_unsigned_integer)
            .and_then(|level| u8::try_from(level).ok()),
        charging: flag("BatteryIsCharging"),
        external_power: flag("ExternalConnected"),
    };
    (battery.level.is_some() || battery.charging.is_some() || battery.external_power.is_some())
        .then_some(battery)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::readiness::Readiness;

    /// The lockdown values recorded from the iPhone SE (3rd generation)
    /// with iOS 26.6.2, the name a placeholder.
    pub(crate) fn recorded_values() -> Dictionary {
        plist::from_bytes(include_bytes!("../tests/fixtures/lockdown-values.plist"))
            .expect("the recorded lockdown values parse")
    }

    /// The image mounter's `CopyDevices` entries recorded with the developer
    /// disk image mounted.
    pub(crate) fn recorded_images() -> Vec<Value> {
        plist::from_bytes(include_bytes!(
            "../tests/fixtures/image-mounter-copy-devices.plist"
        ))
        .expect("the recorded CopyDevices entries parse")
    }

    pub(crate) fn identity() -> IdentityCheck {
        IdentityCheck::Found {
            name: "Apple Development: placeholder-developer (PLACEHOLD0)".into(),
            team: "TEAMPLACE1".into(),
        }
    }

    fn entry(udid: &str, device_id: u32, connection: Connection) -> MuxEntry {
        MuxEntry {
            udid: udid.into(),
            device_id,
            connection,
        }
    }

    #[test]
    fn a_phone_listed_on_usb_and_the_network_is_kept_once_by_its_usb_entry() {
        let listing = vec![
            entry("placeholder-udid-phone", 2, Connection::Network),
            entry("placeholder-udid-phone", 1, Connection::Usb),
            entry("placeholder-udid-other", 3, Connection::Usb),
        ];
        assert_eq!(
            usb_entries(listing),
            vec![
                entry("placeholder-udid-phone", 1, Connection::Usb),
                entry("placeholder-udid-other", 3, Connection::Usb),
            ]
        );
    }

    #[test]
    fn a_device_listed_only_on_the_network_or_twice_on_usb_is_not_duplicated_or_kept() {
        let listing = vec![
            entry("placeholder-udid-wifi", 4, Connection::Network),
            entry("placeholder-udid-phone", 1, Connection::Usb),
            entry("placeholder-udid-phone", 5, Connection::Usb),
            entry(
                "placeholder-udid-odd",
                6,
                Connection::Other("Bluetooth".into()),
            ),
        ];
        assert_eq!(
            usb_entries(listing),
            vec![entry("placeholder-udid-phone", 1, Connection::Usb)]
        );
        assert!(usb_entries(Vec::new()).is_empty());
    }

    /// Lockdown's battery domain as the phone gives it inside a session, on
    /// its cable and charging.
    fn battery_values() -> Value {
        let mut battery = Dictionary::new();
        battery.insert("BatteryCurrentCapacity".into(), Value::Integer(76.into()));
        battery.insert("BatteryIsCharging".into(), Value::Boolean(true));
        battery.insert("ExternalChargeCapable".into(), Value::Boolean(true));
        battery.insert("ExternalConnected".into(), Value::Boolean(true));
        battery.insert("FullyCharged".into(), Value::Boolean(false));
        battery.insert("GasGaugeCapability".into(), Value::Boolean(true));
        battery.insert("HasBattery".into(), Value::Boolean(true));
        Value::Dictionary(battery)
    }

    #[test]
    fn the_battery_domain_gives_the_charge_and_whether_it_charges_on_external_power() {
        let charging = Battery {
            level: Some(76),
            charging: Some(true),
            external_power: Some(true),
        };
        assert_eq!(battery(&battery_values()), Some(charging));
        let mut values = recorded_values();
        values.insert(BATTERY_DOMAIN.into(), battery_values());
        let read = probe(&values, Trust::Trusted, Some(Ok(recorded_images())));
        let info = device_info("placeholder-udid-phone", &values, read, &identity());
        assert_eq!(info.battery, Some(charging));
        // A probe that read no battery domain gives no battery.
        let values = recorded_values();
        let read = probe(&values, Trust::Trusted, Some(Ok(recorded_images())));
        assert_eq!(
            device_info("placeholder-udid-phone", &values, read, &identity()).battery,
            None
        );
        let mut partial = Dictionary::new();
        partial.insert("BatteryCurrentCapacity".into(), Value::Integer(100.into()));
        assert_eq!(
            battery(&Value::Dictionary(partial)),
            Some(Battery {
                level: Some(100),
                charging: None,
                external_power: None,
            })
        );
        assert_eq!(battery(&Value::Dictionary(Dictionary::new())), None);
        assert_eq!(battery(&Value::Boolean(true)), None);
    }

    #[test]
    fn the_recorded_values_describe_the_phone_and_make_it_ready() {
        let values = recorded_values();
        let probe = probe(&values, Trust::Trusted, Some(Ok(recorded_images())));
        assert_eq!(
            probe,
            Probe {
                trust: Trust::Trusted,
                developer_mode: Some(true),
                disk_image: Some(DiskImage::Mounted {
                    image_type: "Personalized".into(),
                    mount_path: Some("/System/Developer".into()),
                }),
                password_protected: Some(false),
            }
        );
        let info = device_info("placeholder-udid-phone", &values, probe, &identity());
        assert_eq!(info.udid, "placeholder-udid-phone");
        assert_eq!(info.name.as_deref(), Some("placeholder-iphone"));
        assert_eq!(info.model.as_deref(), Some("iPhone14,6"));
        assert_eq!(info.ios_version.as_deref(), Some("26.6.2"));
        assert_eq!(info.build.as_deref(), Some("23G90"));
        assert_eq!(info.readiness, Readiness::Ready);
    }

    #[test]
    fn each_recorded_value_that_changes_gives_its_state() {
        let mut off = recorded_values();
        off.insert(DEVELOPER_MODE_KEY.into(), Value::Boolean(false));
        let info = device_info(
            "placeholder-udid-phone",
            &off,
            probe(&off, Trust::Trusted, None),
            &identity(),
        );
        assert_eq!(info.readiness, Readiness::DeveloperModeOff);
        assert_eq!(info.probe.disk_image, None, "the image is not asked for");

        let values = recorded_values();
        let unmounted = probe(&values, Trust::Trusted, Some(Ok(Vec::new())));
        assert_eq!(unmounted.disk_image, Some(DiskImage::NotMounted));
        assert_eq!(
            device_info("placeholder-udid-phone", &values, unmounted, &identity()).readiness,
            Readiness::DiskImageUnmounted
        );

        let failed = probe(
            &values,
            Trust::Trusted,
            Some(Err("the image mounter did not start".into())),
        );
        assert_eq!(
            failed.disk_image,
            Some(DiskImage::Unreadable(
                "the image mounter did not start".into()
            ))
        );

        let mut locked = recorded_values();
        locked.insert(LOCK_KEY.into(), Value::Boolean(true));
        let info = device_info(
            "placeholder-udid-phone",
            &locked,
            probe(&locked, Trust::Trusted, Some(Ok(recorded_images()))),
            &identity(),
        );
        assert_eq!(info.readiness, Readiness::Locked);

        let mut old = recorded_values();
        old.insert("ProductVersion".into(), Value::String("16.7.10".into()));
        let info = device_info(
            "placeholder-udid-phone",
            &old,
            probe(&old, Trust::Trusted, Some(Ok(recorded_images()))),
            &identity(),
        );
        assert_eq!(
            info.readiness,
            Readiness::UnsupportedIos {
                version: "16.7.10".into()
            }
        );
    }

    #[test]
    fn an_untrusted_phone_is_described_from_what_lockdown_gives_without_a_session() {
        let mut values = recorded_values();
        values.remove(DEVELOPER_MODE_KEY);
        values.remove(LOCK_KEY);
        let probe = probe(&values, Trust::NotPaired, None);
        assert_eq!(probe, Probe::untrusted(Trust::NotPaired));
        let info = device_info("placeholder-udid-phone", &values, probe, &identity());
        assert_eq!(info.name.as_deref(), Some("placeholder-iphone"));
        assert_eq!(info.ios_version.as_deref(), Some("26.6.2"));
        assert!(matches!(info.readiness, Readiness::NotTrusted { .. }));
        // Nothing read at all still lists the device by its identifier.
        let bare = device_info(
            "placeholder-udid-phone",
            &Dictionary::new(),
            Probe::untrusted(Trust::Unreachable("lockdown did not answer".into())),
            &identity(),
        );
        assert_eq!(bare.name, None);
        assert_eq!(bare.model, None);
        match bare.readiness {
            Readiness::NotTrusted { reason } => {
                assert!(reason.contains("lockdown did not answer"), "{reason}")
            }
            other => panic!("expected not trusted, got {other:?}"),
        }
    }

    #[test]
    fn a_developer_image_listed_without_is_mounted_counts_by_its_mount_path() {
        let entry = |pairs: &[(&str, Value)]| {
            Value::Dictionary(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
            )
        };
        let legacy = [entry(&[
            ("DiskImageType", Value::String("Developer".into())),
            ("MountPath", Value::String("/Developer".into())),
        ])];
        assert_eq!(
            disk_image(&legacy),
            DiskImage::Mounted {
                image_type: "Developer".into(),
                mount_path: Some("/Developer".into()),
            }
        );
        let other_image = [entry(&[
            ("DiskImageType", Value::String("Personalized".into())),
            (
                "PersonalizedImageType",
                Value::String("SomethingElse".into()),
            ),
            ("IsMounted", Value::Boolean(true)),
        ])];
        assert_eq!(disk_image(&other_image), DiskImage::NotMounted);
        let unmounted = [entry(&[
            ("DiskImageType", Value::String("Personalized".into())),
            (
                "PersonalizedImageType",
                Value::String("DeveloperDiskImage".into()),
            ),
            ("IsMounted", Value::Boolean(false)),
        ])];
        assert_eq!(disk_image(&unmounted), DiskImage::NotMounted);
    }
}
