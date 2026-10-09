//! Task 3.1 on the device: discovery over usbmuxd and lockdown, and the
//! readiness the phone is listed with.

#![cfg(target_os = "macos")]

mod common;

use ios::{Devices, IosDriver, Need, Readiness, TEAM_VARIABLE};

#[test]
fn the_phone_is_listed_once_with_its_identifier_name_model_and_ios_version() {
    let Some(udid) = common::gated("the_phone_is_listed_once") else {
        return;
    };
    let driver = IosDriver::open().expect("the driver opens");
    let devices = driver.devices().expect("usbmuxd lists the devices");
    let listed: Vec<_> = devices
        .iter()
        .filter(|device| device.udid == udid)
        .collect();
    assert_eq!(
        listed.len(),
        1,
        "the phone is listed once, by its USB entry, out of {} devices",
        devices.len()
    );
    let phone = listed[0];
    assert!(phone.name.as_deref().is_some_and(|name| !name.is_empty()));
    assert!(
        phone
            .model
            .as_deref()
            .is_some_and(|model| model.starts_with("iPhone")),
        "{:?}",
        phone.model
    );
    let version = phone.ios_version.as_deref().expect("its iOS version");
    assert!(ios::supported_ios(version), "{version}");
    assert!(phone.build.is_some());
    // The phone is trusted, unlocked, with Developer Mode on and the disk
    // image mounted; the identity is the host's, so without the team the
    // phone is listed as missing it.
    if std::env::var(TEAM_VARIABLE).is_ok_and(|team| !team.trim().is_empty()) {
        assert_eq!(phone.readiness, Readiness::Ready, "{}", phone.readiness);
    } else {
        assert_eq!(phone.readiness.code(), "no_signing_identity");
    }
}

#[test]
fn a_command_reads_the_phone_again_and_an_unknown_identifier_is_not_found() {
    let Some(udid) = common::gated("a_command_reads_the_phone_again") else {
        return;
    };
    let driver = IosDriver::open().expect("the driver opens");
    let phone = driver
        .require(&udid, Need::DiskImage)
        .expect("the phone is ready for the developer services");
    assert_eq!(phone.udid, udid);
    let error = driver
        .require("placeholder-udid-missing", Need::Trust)
        .expect_err("no such device");
    assert_eq!(error.code(), "not_found");
}
