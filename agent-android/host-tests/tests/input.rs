//! Task 4.6: taps, long presses, swipes with duration, multi-step gestures
//! and keys through InputManagerGlobal with WAIT_FOR_RESULT, each
//! acknowledged with the platform's verdict (spec android-driver, Input
//! injection; design D2), and the fresh read of Compose windows after an
//! injected action (design D5).

use std::time::{Duration, Instant};

use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, center, find};
use agent_android_host_tests::{Agent, Launch, device, fixture, ms, percentile};
use protocol::message::{Call, InputAck, KeyParams, Reply, SnapshotParams, SwipeParams, TapParams};
use protocol::snapshot::Snapshot;

fn snapshot(client: &mut Client) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams::default())) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

/// Sends an input call and returns whether it was accepted and the time to
/// the acknowledgement.
fn inject(client: &mut Client, call: Call) -> (bool, Duration) {
    let t0 = Instant::now();
    let reply = client.call(call);
    let took = t0.elapsed();
    match reply {
        Ok(Reply::Input(InputAck { accepted })) => (accepted, took),
        other => panic!("input failed: {other:?}"),
    }
}

fn tap(x: i32, y: i32) -> Call {
    Call::Tap(TapParams {
        x,
        y,
        duration_ms: None,
    })
}

#[test]
fn a_tap_is_acknowledged_as_accepted_within_50_ms() {
    let Some(dev) = device("a_tap_is_acknowledged_as_accepted_within_50_ms") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    // The screen title: a tap there changes nothing, so every tap meets the same screen.
    let title = tree::nodes(&s)
        .into_iter()
        .find(|(f, _)| f.node.text.as_deref() == Some("Login") && f.depth > 3)
        .map(|(f, _)| center(f.node.bounds))
        .expect("the Login title");
    let mut acks = Vec::new();
    for _ in 0..60 {
        let (accepted, took) = inject(&mut client, tap(title.0, title.1));
        assert!(accepted, "a tap was not accepted");
        acks.push(ms(took));
    }
    let acks = &acks[10..];
    let over = acks.iter().filter(|&&a| a > 50.0).count();
    println!(
        "tap acknowledgement over {} taps: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms, {over} over 50 ms",
        acks.len(),
        percentile(acks, 50.0),
        percentile(acks, 95.0),
        percentile(acks, 100.0)
    );
    assert!(percentile(acks, 95.0) < 50.0, "p95 over 50 ms");
    assert!(
        over <= acks.len() / 20,
        "{over} of {} taps over 50 ms",
        acks.len()
    );
}

#[test]
fn a_tap_on_a_compose_tab_is_in_the_next_snapshot_without_a_poll() {
    let Some(dev) = device("a_tap_on_a_compose_tab_is_in_the_next_snapshot_without_a_poll") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default().arg("--compose-poll-ms").arg("0"));
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    for (tab, present, absent) in [
        ("tab-list", "row-1", "login-username"),
        ("tab-anim", "anim-toggle", "row-1"),
        ("tab-login", "login-username", "anim-toggle"),
    ] {
        let current = snapshot(&mut client);
        let (node, _) = find(&current, tab).unwrap_or_else(|| panic!("{tab}"));
        let (x, y) = center(node.node.bounds);
        let (accepted, _) = inject(&mut client, tap(x, y));
        assert!(accepted);
        // The idle wait counts the tap as a change, and the Compose window is
        // re-read with its cache evicted.
        let after = snapshot(&mut client);
        assert!(
            find(&after, present).is_some(),
            "{present} missing after tapping {tab}"
        );
        assert!(
            find(&after, absent).is_none(),
            "{absent} still present after tapping {tab}"
        );
    }
    assert!(find(&before, "login-username").is_some());
}

#[test]
fn a_long_press_holds_for_its_duration() {
    let Some(dev) = device("a_long_press_holds_for_its_duration") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let (node, _) = find(&s, "login-status").expect("login-status");
    let (x, y) = center(node.node.bounds);
    for hold in [400u32, 800] {
        let (accepted, took) = inject(
            &mut client,
            Call::Tap(TapParams {
                x,
                y,
                duration_ms: Some(hold),
            }),
        );
        assert!(accepted);
        println!(
            "long press of {hold} ms acknowledged after {:.1} ms",
            ms(took)
        );
        assert!(
            took >= Duration::from_millis(u64::from(hold)),
            "released early"
        );
        assert!(
            took < Duration::from_millis(u64::from(hold) + 50),
            "acknowledged {:.1} ms after the hold",
            ms(took) - f64::from(hold)
        );
    }
}

/// The number and top of the first row on screen.
fn first_row(s: &Snapshot) -> (u32, i32) {
    tree::find_all(s, "row-")
        .into_iter()
        .filter_map(|f| {
            let id = tree::resource_id(s, f.node)?;
            Some((
                id.strip_prefix("row-")?.parse::<u32>().ok()?,
                f.node.bounds.top,
            ))
        })
        .min_by_key(|(n, _)| *n)
        .expect("a row")
}

#[test]
fn a_swipe_with_duration_scrolls_the_list() {
    let Some(dev) = device("a_swipe_with_duration_scrolls_the_list") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "list");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    assert_eq!(first_row(&before).0, 1);
    let (accepted, took) = inject(
        &mut client,
        Call::Swipe(SwipeParams {
            x1: 540,
            y1: 1800,
            x2: 540,
            y2: 600,
            duration_ms: 300,
            rest_ms: None,
        }),
    );
    assert!(accepted);
    println!("swipe of 300 ms acknowledged after {:.1} ms", ms(took));
    assert!(took >= Duration::from_millis(300) && took < Duration::from_millis(350));
    std::thread::sleep(Duration::from_millis(800));
    let after = snapshot(&mut client);
    assert!(
        first_row(&after).0 > 5,
        "the list did not scroll: {:?}",
        first_row(&after)
    );
}

#[test]
fn a_slow_multi_step_drag_moves_the_list_along_its_path() {
    let Some(dev) = device("a_slow_multi_step_drag_moves_the_list_along_its_path") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "list");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let before = snapshot(&mut client);
    // A row low enough on screen to stay visible after moving up 600 px.
    let (row, top) = tree::find_all(&before, "row-")
        .into_iter()
        .filter(|f| (900..1600).contains(&f.node.bounds.top))
        .filter_map(|f| Some((tree::resource_id(&before, f.node)?, f.node.bounds.top)))
        .next()
        .expect("a row between 900 and 1600 px");
    // A drag slow enough not to fling: the content follows the pointer, so
    // it moves as far as the pointer did, which only the steps between DOWN
    // and UP can make it do.
    let distance = 600;
    let (accepted, took) = inject(
        &mut client,
        Call::Swipe(SwipeParams {
            x1: 540,
            y1: 1700,
            x2: 540,
            y2: 1700 - distance,
            duration_ms: 1500,
            rest_ms: None,
        }),
    );
    assert!(accepted);
    assert!(took < Duration::from_millis(1550), "{took:?}");
    std::thread::sleep(Duration::from_millis(600));
    let after = snapshot(&mut client);
    let (moved_row, _) = find(&after, &row).unwrap_or_else(|| panic!("{row} left the screen"));
    let moved = top - moved_row.node.bounds.top;
    println!("a {distance} px drag over 1500 ms moved the list {moved} px");
    assert!(
        (moved - distance).abs() <= 80,
        "the list moved {moved} px for a {distance} px drag"
    );
}

/// Task 10.16: a swipe that rests at its end before the pointer lifts
/// leaves the list where the pointer left it, however fast the pointer
/// went, where a swipe that lifts moving flings it on (2,187 px for this
/// 1,200 px swipe on the Expo list, API 36 emulator, with an agent that
/// did not rest). A View-based list (the Expo fixture's React Native scroll
/// view) takes its velocity when the pointer lifts, from moves only, so the
/// rest ends with a move; Compose drops the velocity itself when the lift
/// comes more than 40 ms after the last move.
#[test]
fn a_swipe_that_rests_before_the_lift_moves_the_list_as_far_as_the_pointer() {
    let Some(dev) =
        device("a_swipe_that_rests_before_the_lift_moves_the_list_as_far_as_the_pointer")
    else {
        return;
    };
    for app in [fixture::EXPO, fixture::COMPOSE] {
        fixture::launch(&dev, app, "list");
        let agent = Agent::launch(&dev, Launch::default());
        let mut client = agent.connect();
        // The list starts at its top: how far it has scrolled is where a
        // row on screen is against where it started, rows a pitch apart.
        let before = snapshot(&mut client);
        let top_of = |s: &Snapshot, n: u32| {
            find(s, &format!("row-{n}")).map(|(found, _)| found.node.bounds.top)
        };
        let first = top_of(&before, 1).unwrap_or_else(|| panic!("{app}: row-1 on screen"));
        let pitch = top_of(&before, 2).unwrap_or_else(|| panic!("{app}: row-2 on screen")) - first;
        let distance = 1200;
        let (accepted, took) = inject(
            &mut client,
            Call::Swipe(SwipeParams {
                x1: 540,
                y1: 2000,
                x2: 540,
                y2: 2000 - distance,
                duration_ms: 300,
                rest_ms: Some(60),
            }),
        );
        assert!(accepted);
        std::thread::sleep(Duration::from_millis(1500));
        let after = snapshot(&mut client);
        // The second row on screen: the list may cut the first one off, and
        // Android reports what shows of it.
        let (first_shown, _) = first_row(&after);
        let n = first_shown + 1;
        let top = top_of(&after, n).unwrap_or_else(|| panic!("{app}: row-{n} on screen"));
        let n = i32::try_from(n).expect("a row number");
        let moved = (n - 1) * pitch + first - top;
        println!(
            "{app}: a {distance} px swipe over 300 ms that rests 60 ms moved the list {moved} px, \
             acknowledged after {:.1} ms",
            ms(took)
        );
        assert!(
            (moved - distance).abs() <= 80,
            "{app}: the list moved {moved} px for a {distance} px swipe"
        );
        assert!(
            took >= Duration::from_millis(360) && took < Duration::from_millis(410),
            "{app}: {took:?}"
        );
        drop(client);
        drop(agent);
        fixture::force_stop(&dev, app);
    }
}

#[test]
fn key_presses_are_acknowledged_and_typed_into_the_focused_field() {
    let Some(dev) = device("key_presses_are_acknowledged_and_typed_into_the_focused_field") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let (field, _) = find(&s, "login-username").expect("login-username");
    let (x, y) = tree::tap_point(&s, &field);
    assert!(inject(&mut client, tap(x, y)).0);
    client.idle(Duration::from_millis(800));
    let mut acks = Vec::new();
    // KEYCODE_H, I, SPACE, 1, 2, typed at a person's pace: the dispatcher
    // holds a key until the app has handled the previous one, so keys sent
    // back to back would time React Native's re-render, not the injection.
    for code in [36u32, 37, 62, 8, 9] {
        client.idle(Duration::from_millis(150));
        let (accepted, took) = inject(&mut client, Call::Key(KeyParams { code, meta: None }));
        assert!(accepted);
        acks.push(ms(took));
    }
    // Shift+A
    let (accepted, _) = inject(
        &mut client,
        Call::Key(KeyParams {
            code: 29,
            meta: Some(1),
        }),
    );
    assert!(accepted);
    println!("key acknowledgements: {acks:.2?} ms");
    assert!(acks.iter().all(|&a| a < 50.0), "{acks:?}");
    let after = snapshot(&mut client);
    let (typed, _) = find(&after, "login-username").expect("login-username");
    fixture::force_stop(&dev, fixture::EXPO);
    assert_eq!(typed.node.text.as_deref(), Some("hi 12A"));
}

#[test]
fn a_tap_the_platform_refuses_is_acknowledged_as_not_accepted() {
    let Some(dev) = device("a_tap_the_platform_refuses_is_acknowledged_as_not_accepted") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    // Far outside the display no window takes the touch, and dispatch fails it.
    let (accepted, took) = inject(&mut client, tap(99_999, 99_999));
    assert!(!accepted, "a tap outside the display was accepted");
    assert!(took < Duration::from_millis(50), "{took:?}");
    let (accepted, _) = inject(&mut client, tap(540, 1500));
    assert!(accepted);
}
