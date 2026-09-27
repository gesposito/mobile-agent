//! Task 6.7: text entry through keyboard events and the pasteboard
//! (simulator-driver spec, "Text entry"). The text is read back from the
//! field by copying it to the device pasteboard.

#![cfg(target_os = "macos")]

mod common;

use std::time::Duration;

use simulator::{Input, SimulatorDriver, TextMethod};

use common::{ExpoOnLogin, expo};

/// Focuses the Expo fixture's username field on a fresh Login tab.
fn focus_username(driver: &SimulatorDriver, udid: &str) {
    common::show_expo(driver, udid, "login");
    driver.tap(udid, expo::USERNAME).expect("a tap");
    // The software keyboard slides in (S3: about 1.2 s).
    std::thread::sleep(Duration::from_millis(1500));
}

#[test]
fn long_string() {
    let Some(udid) = common::gated("long_string") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    focus_username(&driver, &udid);
    let text: String = (0..500)
        .map(|i| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[i * 7 % 36]))
        .collect();
    let entry = driver
        .type_text(&udid, &text, TextMethod::Auto)
        .expect("the text is entered");
    eprintln!(
        "500 characters by {:?} in {:?}",
        entry.method, entry.elapsed
    );
    assert_eq!(entry.method, TextMethod::Paste, "a long string is pasted");
    assert_eq!(entry.characters, 500);
    assert_eq!(common::field_text(&driver, &udid), text);
}

#[test]
fn a_short_string_is_typed_through_keyboard_events() {
    let Some(udid) = common::gated("a_short_string_is_typed_through_keyboard_events") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    focus_username(&driver, &udid);
    let text = "Hunter2, (the) test_user!";
    let entry = driver
        .type_text(&udid, text, TextMethod::Auto)
        .expect("the text is typed");
    eprintln!(
        "{} characters by {:?} in {:?}",
        entry.characters, entry.method, entry.elapsed
    );
    assert_eq!(entry.method, TextMethod::Keyboard);
    assert_eq!(common::field_text(&driver, &udid), text);
}

#[test]
fn text_no_key_types_is_pasted() {
    let Some(udid) = common::gated("text_no_key_types_is_pasted") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = SimulatorDriver::open().expect("the driver opens");
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    focus_username(&driver, &udid);
    let text = "café ✓ 東京";
    let entry = driver
        .type_text(&udid, text, TextMethod::Auto)
        .expect("the text is entered");
    assert_eq!(entry.method, TextMethod::Paste);
    assert_eq!(common::field_text(&driver, &udid), text);
    assert!(matches!(
        driver.type_text(&udid, text, TextMethod::Keyboard),
        Err(simulator::Error::InvalidArgument(_))
    ));
}
