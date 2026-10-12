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
/// show the whole entry, and what it showed last.
#[derive(Debug)]
struct Typed {
    elapsed: Duration,
    shown: Option<Duration>,
    text: Option<String>,
}

impl Typed {
    fn whole(&self) -> bool {
        self.text.as_deref() == Some(ENTRY)
    }
}

/// Types the entry into the focused, empty field `cadence` apart, then reads
/// the field until it has shown the whole entry for [`HOLD`], or for
/// [`READ_BACK`] after the last key.
fn type_entry(driver: &SimulatorDriver, udid: &str, identifier: &str, cadence: Cadence) -> Typed {
    let entry = driver
        .type_text_at(udid, ENTRY, TextMethod::Keyboard, cadence)
        .expect("the entry is typed");
    assert_eq!(entry.method, TextMethod::Keyboard);
    let typed = Instant::now();
    let mut whole_since: Option<Instant> = None;
    let mut shown = None;
    loop {
        let text = value(driver, udid, identifier);
        if text.as_deref() == Some(ENTRY) {
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
            let typed = type_entry(&driver, &udid, USERNAME, cadences[index]);
            if !typed.whole() {
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
        let whole = typed.iter().filter(|t| t.whole()).count();
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
