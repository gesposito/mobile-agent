//! The catch-up before a tree read on a screen that animates (design D3 of
//! the `profiling` change, spike S15's first hot spot): on a fake
//! Simulator, whose windows send no change events, the fixture's Anim tab
//! keeps the frames changing, so every listing finds a frame newer than the
//! latest read. The listing then reads the tree in about a read's time
//! rather than wait for the re-read the mirror schedules no sooner than its
//! minimum interval after the read before. Its replies are checked as the
//! daemon's animating benchmark checks the ones it times
//! (`support::check_anim`), so that benchmark's input cannot drift silently.

mod support;

use std::time::Duration;

use mirror::DEFAULT_MIN_REREAD_INTERVAL;
use serde_json::json;
use support::Daemon;

/// How long after the reply before it each listing is asked for: half the
/// minimum re-read interval, about what a listing that comes at a random
/// time between the re-reads a Simulator's frames call for has left of it
/// (S15: 106 ms of a 119 ms listing waited for it).
const PAUSE: Duration = DEFAULT_MIN_REREAD_INTERVAL.checked_div(2).unwrap();

/// How long a listing of the Anim tab may take at most: a fake target's
/// read, under a millisecond, and a margin for a busy host, well under the
/// half interval a wait for the re-read would take.
const READ_AND_MARGIN_MS: f64 = 40.0;

/// Ten listings of the Anim tab, each half the minimum re-read interval
/// after the reply before it, read the tree the spinner changed, and the
/// worker takes about a read's time for them, not the rest of the interval.
#[test]
fn a_listing_of_an_animating_screen_reads_the_tree_in_about_a_read_s_time() {
    let daemon = Daemon::start("ios");
    support::open_anim(&daemon);
    let mut taken = Vec::new();
    for _ in 0..10 {
        std::thread::sleep(PAUSE);
        let reply = daemon.request(json!({"action": "snapshot", "interactive": true}));
        support::check_anim(&reply);
        taken.push(
            reply["data"]["readMs"]
                .as_f64()
                .expect("readMs is a number"),
        );
    }
    let mut sorted = taken.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    assert!(
        median < READ_AND_MARGIN_MS,
        "the worker's time for each listing, ms: {taken:?}"
    );
}
