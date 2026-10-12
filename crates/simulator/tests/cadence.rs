//! Task 3.5 of `rn-app-fixes` (design D42): the cadence of typed keys
//! (simulator-driver spec, "Text entry"). The entry is read back from the
//! tree, as the field's value.

#![cfg(target_os = "macos")]

mod common;

use std::thread;
use std::time::{Duration, Instant};

use simulator::{Cadence, Input, Key, Modifiers, SimulatorDriver, TextMethod, Tree, TreeOptions};

use common::{ExpoOnLogin, expo};

/// The benchmark harness's text entry (`TEXT_ENTRY` in
/// `crates/bench/src/primitive.rs`): 36 keys, none with Shift.
const ENTRY: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// The fixtures' username field (`fixtures/CONTRACT.md`).
const USERNAME: &str = "login-username";

/// How long the field must show the whole entry before it counts as read
/// back, as `fill` holds typed text.
const HOLD: Duration = Duration::from_millis(200);

/// How long after the last key the field may take to show the entry.
const READ_BACK: Duration = Duration::from_secs(2);

/// The value of the field with this identifier in a fresh read; `None` for
/// an empty field, whose value the reader drops.
fn value(driver: &SimulatorDriver, udid: &str, identifier: &str) -> Option<String> {
    let tree = driver
        .snapshot(udid, &TreeOptions::default())
        .expect("the tree is read");
    tree.find(identifier).and_then(|node| node.text.clone())
}

fn shortcut(driver: &SimulatorDriver, udid: &str, usage: u32, command: bool) {
    let modifiers = Modifiers {
        command,
        ..Modifiers::default()
    };
    driver
        .key(udid, Key { usage, modifiers })
        .expect("a key press");
}

/// Selects what the focused field holds and deletes it, then waits until a
/// read shows the field empty.
fn empty(driver: &SimulatorDriver, udid: &str, identifier: &str) {
    shortcut(driver, udid, 0x04, true);
    shortcut(driver, udid, 0x2A, false);
    assert!(
        common::wait_for(Duration::from_secs(3), || value(driver, udid, identifier)
            .is_none()),
        "the field did not empty"
    );
}

/// One typed entry: how long typing took, how long the field then took to
/// show the whole text, and what it showed last.
#[derive(Debug)]
struct Typed {
    elapsed: Duration,
    shown: Option<Duration>,
    text: Option<String>,
}

impl Typed {
    fn whole(&self, text: &str) -> bool {
        self.text.as_deref() == Some(text)
    }
}

/// Types `text` into the focused, empty field `cadence` apart, then reads
/// the field until it has shown the whole text for [`HOLD`], or for
/// [`READ_BACK`] after the last key.
fn type_entry(
    driver: &SimulatorDriver,
    udid: &str,
    identifier: &str,
    text: &str,
    cadence: Cadence,
) -> Typed {
    let entry = driver
        .type_text_at(udid, text, TextMethod::Keyboard, cadence)
        .expect("the text is typed");
    assert_eq!(entry.method, TextMethod::Keyboard);
    let expected = text;
    let typed = Instant::now();
    let mut whole_since: Option<Instant> = None;
    let mut shown = None;
    loop {
        let text = value(driver, udid, identifier);
        if text.as_deref() == Some(expected) {
            let since = *whole_since.get_or_insert_with(Instant::now);
            shown.get_or_insert_with(|| since - typed);
            if since.elapsed() >= HOLD {
                return Typed {
                    elapsed: entry.elapsed,
                    shown,
                    text,
                };
            }
        } else {
            whole_since = None;
            shown = None;
        }
        if typed.elapsed() > READ_BACK {
            return Typed {
                elapsed: entry.elapsed,
                shown: None,
                text,
            };
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Relaunches the Expo fixture on Login and focuses its username field,
/// leaving the keyboard time to slide in (S3: about 1.2 s).
fn focus_expo_username(driver: &SimulatorDriver, udid: &str) {
    common::show_expo(driver, udid, "login");
    driver.tap(udid, expo::USERNAME).expect("a tap");
    thread::sleep(Duration::from_millis(1500));
}

/// Relaunches the SwiftUI fixture on Login and focuses its username field
/// where a read puts it, leaving the keyboard time to slide in.
fn focus_swiftui_username(driver: &SimulatorDriver, udid: &str) {
    common::show_swiftui(driver, udid, "login");
    let tree = common::snapshot_until(driver, udid, Duration::from_secs(10), |tree| {
        tree.find(USERNAME).is_some()
    });
    let field = tree.find(USERNAME).expect("the username field");
    driver.tap(udid, tree.center(field)).expect("a tap");
    thread::sleep(Duration::from_millis(1500));
}

/// The cadence a fresh read of the screen calls for.
fn cadence_of_screen(driver: &SimulatorDriver, udid: &str) -> Cadence {
    let tree = driver
        .snapshot(udid, &TreeOptions::default())
        .expect("the tree is read");
    Cadence::for_read(&tree.snapshot)
}

/// Task 3.5: a React Native field's keys go at React Native's cadence and a
/// native field's at the native cadence, each told from a read of its
/// screen, and both fields take the text whole.
#[test]
fn a_react_native_field_is_typed_at_its_cadence_and_a_native_one_at_the_native_cadence() {
    let Some(udid) = common::gated(
        "a_react_native_field_is_typed_at_its_cadence_and_a_native_one_at_the_native_cadence",
    ) else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    let text = "cadence0test";
    let gaps = text.chars().count() as u32 - 1;

    focus_expo_username(&driver, &udid);
    let cadence = cadence_of_screen(&driver, &udid);
    assert_eq!(cadence, Cadence::REACT_NATIVE, "the Expo fixture's screen");
    let typed = type_entry(&driver, &udid, USERNAME, text, cadence);
    assert!(
        typed.elapsed >= Cadence::REACT_NATIVE.gap * gaps,
        "{gaps} gaps of {:?} took {:?}",
        Cadence::REACT_NATIVE.gap,
        typed.elapsed
    );
    assert!(typed.whole(text), "the field shows {:?}", typed.text);

    focus_swiftui_username(&driver, &udid);
    let cadence = cadence_of_screen(&driver, &udid);
    assert_eq!(cadence, Cadence::NATIVE, "the SwiftUI fixture's screen");
    let typed = type_entry(&driver, &udid, USERNAME, text, cadence);
    assert!(
        typed.elapsed < Cadence::REACT_NATIVE.gap * gaps,
        "{gaps} native gaps took {:?}, as long as React Native's would",
        typed.elapsed
    );
    assert!(typed.whole(text), "the field shows {:?}", typed.text);
}

/// Spec scenario "React Native field typed whole" (simulator-driver, "Text
/// entry"): the 36-character entry, typed twenty times into the Expo
/// fixture's username field at the cadence a read of its screen calls for,
/// reads back whole every time.
#[test]
fn a_react_native_field_takes_the_36_character_entry_whole_twenty_times() {
    let Some(udid) =
        common::gated("a_react_native_field_takes_the_36_character_entry_whole_twenty_times")
    else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    focus_expo_username(&driver, &udid);
    let cadence = cadence_of_screen(&driver, &udid);
    assert_eq!(cadence, Cadence::REACT_NATIVE, "the Expo fixture's screen");
    empty(&driver, &udid, USERNAME);
    let mut lost = Vec::new();
    for run in 0..20 {
        let typed = type_entry(&driver, &udid, USERNAME, ENTRY, cadence);
        if !typed.whole(ENTRY) {
            lost.push((run, typed.text));
        }
        empty(&driver, &udid, USERNAME);
    }
    assert!(
        lost.is_empty(),
        "runs the field did not show whole: {lost:?}"
    );
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The measurement behind task 3.5's decision: 20 entries at each of three
/// cadences into the Expo fixture's username field, the cadences taken in
/// turn within each trial so the host's load falls on all three alike.
/// Prints each cadence's drops and times, and the host's load over the
/// series.
#[test]
#[ignore = "a measurement of about four minutes; run it by name on a quiet host"]
fn the_36_character_entry_at_three_cadences() {
    let Some(udid) = common::gated("the_36_character_entry_at_three_cadences") else {
        return;
    };
    let _serial = common::exclusive();
    let driver = common::driver();
    let _restore = ExpoOnLogin {
        driver: &driver,
        udid: udid.clone(),
    };
    let cadences = [15u64, 30, 60].map(|gap| Cadence {
        gap: Duration::from_millis(gap),
    });
    let trials = 20;
    focus_expo_username(&driver, &udid);
    empty(&driver, &udid, USERNAME);
    let mut loads = vec![common::load_average()];
    let mut results: Vec<Vec<Typed>> = cadences.iter().map(|_| Vec::new()).collect();
    for trial in 0..trials {
        for turn in 0..cadences.len() {
            let index = (trial + turn) % cadences.len();
            let typed = type_entry(&driver, &udid, USERNAME, ENTRY, cadences[index]);
            if !typed.whole(ENTRY) {
                eprintln!(
                    "trial {trial}, {:?} apart: the field shows {:?}",
                    cadences[index].gap, typed.text
                );
            }
            results[index].push(typed);
            empty(&driver, &udid, USERNAME);
        }
        loads.push(common::load_average());
    }
    let mut sorted_loads = loads.clone();
    sorted_loads.sort_by(f64::total_cmp);
    eprintln!(
        "1-minute load: before {:.2}, after {:.2}; min / median / max {:.2} / {:.2} / {:.2}",
        loads[0],
        loads[loads.len() - 1],
        sorted_loads[0],
        sorted_loads[sorted_loads.len() / 2],
        sorted_loads[sorted_loads.len() - 1]
    );
    eprintln!(
        "| gap | whole | typing p50 / p95 / max (ms) | shown after the last key p50 / p95 (ms) |"
    );
    for (cadence, typed) in cadences.iter().zip(&results) {
        let whole = typed.iter().filter(|t| t.whole(ENTRY)).count();
        let mut elapsed: Vec<Duration> = typed.iter().map(|t| t.elapsed).collect();
        elapsed.sort();
        let mut shown: Vec<Duration> = typed.iter().filter_map(|t| t.shown).collect();
        shown.sort();
        let shown = if shown.is_empty() {
            "none".to_owned()
        } else {
            format!(
                "{:.1} / {:.1}",
                millis(percentile(&shown, 50.0)),
                millis(percentile(&shown, 95.0))
            )
        };
        eprintln!(
            "| {:?} | {whole} of {} | {:.1} / {:.1} / {:.1} | {shown} |",
            cadence.gap,
            typed.len(),
            millis(percentile(&elapsed, 50.0)),
            millis(percentile(&elapsed, 95.0)),
            millis(elapsed[elapsed.len() - 1]),
        );
    }
}
