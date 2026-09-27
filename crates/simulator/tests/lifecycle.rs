//! Task 6.1: discovery and lifecycle through CoreSimulator (simulator-driver
//! spec, "Simulator discovery and lifecycle").

#![cfg(target_os = "macos")]

mod common;

use std::time::Duration;

use simulator::{DeviceState, Error, LaunchOptions, Lifecycle, PreferencesOutcome};

use common::{SWIFTUI_BUNDLE, SimctlDevice};

const PREFERENCES: [&str; 3] = [
    "AccessibilityEnabled",
    "ApplicationAccessibilityEnabled",
    "AutomationEnabled",
];

#[test]
fn simctl_lines_parse() {
    let line =
        "    iPad Pro (11-inch) (4th generation) (7B292E1E-7A29-41A5-9C69-2ABA1CEB3683) (Booted) ";
    assert_eq!(
        common::parse_simctl_line(line),
        Some(SimctlDevice {
            udid: "7B292E1E-7A29-41A5-9C69-2ABA1CEB3683".into(),
            name: "iPad Pro (11-inch) (4th generation)".into(),
            state: "Booted".into(),
        })
    );
    assert_eq!(common::parse_simctl_line("-- iOS 26.5 --"), None);
}

#[test]
fn every_simulator_is_listed_with_identifier_name_runtime_and_state() {
    let Some(udid) = common::gated("every_simulator_is_listed") else {
        return;
    };
    let driver = common::driver();
    // Other processes may create or delete devices meanwhile, so only devices
    // present in the same state before and after the call are compared.
    let before = common::simctl_devices();
    let listed = driver.devices().expect("devices are listed");
    let after = common::simctl_devices();
    let stable: Vec<&SimctlDevice> = before.iter().filter(|d| after.contains(d)).collect();
    assert!(!stable.is_empty(), "simctl lists no device to compare with");
    for device in stable {
        let row = listed
            .iter()
            .find(|row| row.udid == device.udid)
            .unwrap_or_else(|| panic!("{} ({}) is not listed", device.name, device.udid));
        assert_eq!(row.name, device.name, "name of {}", device.udid);
        assert_eq!(
            row.state.to_string(),
            device.state,
            "state of {}",
            device.udid
        );
    }

    let gated = listed
        .iter()
        .find(|device| device.udid.eq_ignore_ascii_case(&udid))
        .expect("the gated Simulator is listed");
    assert_eq!(gated.state, DeviceState::Booted);
    let runtime = gated.runtime.as_ref().expect("its runtime is installed");
    assert!(!runtime.version.is_empty(), "{runtime:?}");
    assert!(runtime.name.ends_with(&runtime.version), "{runtime:?}");
    assert!(!runtime.build.is_empty(), "{runtime:?}");
    let device_type = gated.device_type.as_ref().expect("its device type");
    assert!(!device_type.name.is_empty(), "{device_type:?}");
}

#[test]
fn launch_with_environment() {
    let Some(_) = common::gated("launch_with_environment") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let app = common::swiftui_app();
    let device = common::TemporaryDevice::create(&driver, "lifecycle");
    let udid = device.udid.clone();
    let state = || {
        driver
            .devices()
            .expect("devices")
            .into_iter()
            .find(|d| d.udid == udid)
            .map(|d| d.state)
    };
    assert_eq!(state(), Some(DeviceState::Shutdown));

    // S4: the accessibility preferences must be in the data path before the
    // device's first boot.
    let preferences = common::accessibility_preferences(&udid);
    for key in PREFERENCES {
        assert_eq!(
            common::plist_value(&preferences, key).as_deref(),
            Some("true"),
            "{key} in {} before the first boot",
            preferences.display()
        );
    }

    let boot = driver.boot(&udid).expect("the device boots");
    assert!(!boot.already_booted);
    assert_eq!(boot.accessibility, PreferencesOutcome::AlreadySet);
    assert_eq!(state(), Some(DeviceState::Booted));
    eprintln!("first boot to ready: {:?}", boot.elapsed);

    driver.install(&udid, &app).expect("the fixture installs");
    let marker = common::unique("marker");
    let launched = driver
        .launch(
            &udid,
            SWIFTUI_BUNDLE,
            &LaunchOptions::new()
                .env("MOBILE_AGENT_TEST_MARKER", &marker)
                .env("FIXTURE_INITIAL_TAB", "list")
                .argument("--marker-argument"),
        )
        .expect("the fixture launches");
    assert!(launched.pid > 0);
    eprintln!("launch: {:?}", launched.elapsed);

    let seen = common::process_environment(launched.pid).expect("the process is running");
    assert!(
        seen.contains(&format!("MOBILE_AGENT_TEST_MARKER={marker}")),
        "the marker is in the process environment: {seen}"
    );
    assert!(seen.contains("FIXTURE_INITIAL_TAB=list"), "{seen}");
    assert!(seen.contains("--marker-argument"), "{seen}");

    driver
        .terminate(&udid, SWIFTUI_BUNDLE)
        .expect("the fixture terminates");
    assert!(
        common::wait_for(Duration::from_secs(10), || !common::process_alive(
            launched.pid
        )),
        "process {} is gone after terminate",
        launched.pid
    );
    driver
        .terminate(&udid, SWIFTUI_BUNDLE)
        .expect("terminating an application that is not running changes nothing");

    driver.shutdown(&udid).expect("the device shuts down");
    assert_eq!(state(), Some(DeviceState::Shutdown));
    driver
        .shutdown(&udid)
        .expect("shutting down a device that is shut down changes nothing");
    device.delete();
    assert_eq!(state(), None, "the deleted device is no longer listed");
}

#[test]
fn a_booted_device_is_left_alone() {
    let Some(udid) = common::gated("a_booted_device_is_left_alone") else {
        return;
    };
    let driver = common::driver();
    let boot = driver.boot(&udid).expect("boot of a booted device");
    assert!(boot.already_booted);
    assert_eq!(
        boot.accessibility,
        PreferencesOutcome::Skipped,
        "a booted device's preferences belong to the guest"
    );
    assert!(matches!(
        driver.prepare_accessibility(&udid),
        Err(Error::InvalidState { .. })
    ));
}

#[test]
fn an_unknown_device_is_not_found() {
    let Some(_) = common::gated("an_unknown_device_is_not_found") else {
        return;
    };
    let driver = common::driver();
    let target = "00000000-0000-0000-0000-000000000000";
    assert_eq!(
        driver.terminate(target, SWIFTUI_BUNDLE),
        Err(Error::NotFound {
            target: target.into()
        })
    );
}
