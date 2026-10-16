//! A cancel flag (design D7 of the `mcp-server` change): the daemon gives a
//! mirror a flag that its waits look at before they block and at least
//! every `CANCEL_SLICE` while they do; once it is raised, `run_until`, `wait`
//! and `settle` fail with `cancelled`, and a flag that stays down changes
//! nothing they do. A wait that has nothing to wait for, as a settle on a
//! screen already quiet, ends as it would without the flag.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mirror::backend::{EventStream, StreamItem, Timed};
use mirror::testing::FakeTarget;
use mirror::{
    CANCEL_SLICE, Condition, Mirror, MirrorConfig, MirrorError, Selector, SettleOptions, code,
};
use support::Screen;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The login screen's mirror, read once, with `flag` as its cancel flag.
fn login(flag: &Arc<AtomicBool>) -> (FakeTarget, Mirror) {
    let target = FakeTarget::new(Screen::load("login").read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    mirror.cancel_on(Arc::clone(flag));
    (target, mirror)
}

#[test]
fn a_raised_flag_ends_run_until_wait_and_settle_with_cancelled_before_they_block() {
    let flag = Arc::new(AtomicBool::new(true));
    let (target, mut mirror) = login(&flag);
    let started = target.elapsed();
    let deadline = target.instant(started + secs(5));
    let run = mirror.run_until(deadline).unwrap_err();
    let wait = mirror
        .wait(&Selector::text("Welcome"), Condition::Appear, secs(5))
        .unwrap_err();
    // A window change noted now gives the settle a quiet window to wait.
    mirror.note_window_change();
    let settle = mirror
        .settle(&SettleOptions {
            max: secs(5),
            ..SettleOptions::default()
        })
        .unwrap_err();
    let errors = [run, wait, settle];
    for error in errors {
        assert_eq!(error.code(), code::CANCELLED, "{error:?}");
        assert_eq!(error, MirrorError::Cancelled);
    }
    assert_eq!(target.elapsed(), started, "nothing waited");
}

/// The fake target's event stream, raising `flag` once its clock passes
/// `at`, as a cancel from another thread does while a wait blocks.
struct RaisedAt {
    events: Box<dyn EventStream>,
    target: FakeTarget,
    flag: Arc<AtomicBool>,
    at: Instant,
}

impl EventStream for RaisedAt {
    fn next(&mut self, deadline: Instant) -> Option<Timed<StreamItem>> {
        let item = self.events.next(deadline);
        if self.target.instant(self.target.elapsed()) >= self.at {
            self.flag.store(true, Ordering::SeqCst);
        }
        item
    }
}

#[test]
fn a_flag_raised_while_a_wait_blocks_ends_it_within_a_slice() {
    for which in ["run_until", "wait"] {
        let target = FakeTarget::new(Screen::load("login").read());
        let flag = Arc::new(AtomicBool::new(false));
        let mut backend = target.backend();
        backend.events = Box::new(RaisedAt {
            events: backend.events,
            target: target.clone(),
            flag: Arc::clone(&flag),
            at: target.instant(secs(1)),
        });
        let mut mirror = Mirror::new(backend, MirrorConfig::default());
        mirror.pump().unwrap();
        mirror.cancel_on(Arc::clone(&flag));
        let error = match which {
            "run_until" => mirror.run_until(target.instant(secs(5))).unwrap_err(),
            _ => mirror
                .wait(&Selector::text("Welcome"), Condition::Appear, secs(5))
                .unwrap_err(),
        };
        assert_eq!(error.code(), code::CANCELLED, "{which}: {error:?}");
        let ended = target.elapsed();
        assert!(
            ended >= secs(1) && ended <= secs(1) + CANCEL_SLICE,
            "{which}: ended at {ended:?}"
        );
    }
}

/// With the flag down, a wait times out where it would without one, with
/// the same reads.
#[test]
fn a_flag_that_stays_down_changes_nothing_a_wait_does() {
    let mut outcomes = Vec::new();
    for flagged in [false, true] {
        let target = FakeTarget::new(Screen::load("login").read());
        let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
        if flagged {
            mirror.cancel_on(Arc::new(AtomicBool::new(false)));
        }
        let error = mirror
            .wait(&Selector::text("Welcome"), Condition::Appear, secs(2))
            .unwrap_err();
        assert_eq!(error.code(), code::WAIT_TIMEOUT, "{error:?}");
        let settled = mirror.settle(&SettleOptions::default()).unwrap();
        assert!(settled.settled, "{settled:?}");
        let reads: Vec<_> = target
            .reads()
            .iter()
            .map(|read| (read.at, read.request.reason))
            .collect();
        outcomes.push((target.elapsed(), settled.elapsed, reads));
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(outcomes[0].0, secs(2), "the wait timed out at its bound");
}
