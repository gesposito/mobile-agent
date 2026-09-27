//! Task 4.7: direct text setting on the focused editable node, the key-event
//! fallback, and the method reported (spec android-driver, Direct text
//! setting with fallback).

use std::time::{Duration, Instant};

use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, find};
use agent_android_host_tests::{Agent, Launch, device, fixture, ms};
use protocol::code;
use protocol::event::{EventPayload, TreeChangeKind};
use protocol::message::{
    Call, KeyParams, Reply, SnapshotParams, TapParams, TextAck, TextMethod, TextParams,
};
use protocol::snapshot::{NodeFlags, Snapshot};

fn snapshot(client: &mut Client) -> Snapshot {
    match client.call(Call::Snapshot(SnapshotParams::default())) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("snapshot failed: {other:?}"),
    }
}

fn set_text(
    client: &mut Client,
    text: &str,
    node: Option<u32>,
    mode: Option<TextMethod>,
) -> (TextMethod, Duration) {
    let t0 = Instant::now();
    let reply = client.call(Call::TextSet(TextParams {
        text: text.to_owned(),
        node,
        mode,
    }));
    match reply {
        Ok(Reply::Text(TextAck { method })) => (method, t0.elapsed()),
        other => panic!("text_set failed: {other:?}"),
    }
}

/// Taps a field through the agent and lets focus and the keyboard settle.
fn focus(client: &mut Client, s: &Snapshot, id: &str) {
    let (field, _) = find(s, id).unwrap_or_else(|| panic!("{id}"));
    let (x, y) = tree::tap_point(s, &field);
    let reply = client.call(Call::Tap(TapParams {
        x,
        y,
        duration_ms: None,
    }));
    assert!(matches!(reply, Ok(Reply::Input(_))), "{reply:?}");
    client.idle(Duration::from_millis(800));
}

#[test]
fn a_focused_react_native_input_is_filled_directly_in_one_operation() {
    let Some(dev) = device("a_focused_react_native_input_is_filled_directly_in_one_operation")
    else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    focus(&mut client, &s, "login-username");
    let text = "forty characters of text set in one go!!";
    assert_eq!(text.chars().count(), 40);
    let seen = client.events.len();
    let (method, took) = set_text(&mut client, text, None, None);
    client.idle(Duration::from_millis(500));
    let after = snapshot(&mut client);
    let (field, path) = find(&after, "login-username").expect("login-username");
    let text_events = client.events[seen..]
        .iter()
        .filter(|(_, e)| {
            matches!(&e.payload, EventPayload::Tree(t)
                if t.change == TreeChangeKind::TEXT && t.path == path)
        })
        .count();
    fixture::force_stop(&dev, fixture::EXPO);
    println!(
        "40 characters set {method} in {:.1} ms, {text_events} text change events",
        ms(took)
    );
    assert_eq!(method, TextMethod::DIRECT);
    assert_eq!(field.node.text.as_deref(), Some(text));
    // One operation: one text change, where typing sends one per character.
    assert!(text_events <= 2, "{text_events} text change events");
}

#[test]
fn a_focused_element_without_direct_setting_gets_key_events_and_the_fallback_is_named() {
    let Some(dev) = device(
        "a_focused_element_without_direct_setting_gets_key_events_and_the_fallback_is_named",
    ) else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    focus(&mut client, &s, "login-username");
    assert_eq!(
        set_text(&mut client, "someone", None, None).0,
        TextMethod::DIRECT
    );
    // Tab twice: from the username field past the password field to the
    // Sign in button, which takes input focus but not text.
    for _ in 0..2 {
        let reply = client.call(Call::Key(KeyParams {
            code: 61,
            meta: None,
        }));
        assert!(matches!(reply, Ok(Reply::Input(_))), "{reply:?}");
        client.idle(Duration::from_millis(300));
    }
    let focused = snapshot(&mut client);
    let (button, _) = find(&focused, "login-submit").expect("login-submit");
    assert!(
        button.node.flags.contains(NodeFlags::FOCUSED),
        "Sign in did not take focus"
    );
    // Enter, typed as a key event, presses the focused button.
    let (method, _) = set_text(&mut client, "\n", None, None);
    assert_eq!(method, TextMethod::KEYS);
    client.idle(Duration::from_millis(300));
    let after = snapshot(&mut client);
    let (status, _) = find(&after, "login-status").expect("login-status");
    fixture::force_stop(&dev, fixture::COMPOSE);
    assert_eq!(status.node.text.as_deref(), Some("Invalid credentials"));
}

#[test]
fn keys_on_request_replace_the_field_text_and_name_the_key_method() {
    let Some(dev) = device("keys_on_request_replace_the_field_text_and_name_the_key_method") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    focus(&mut client, &s, "login-username");
    assert_eq!(
        set_text(&mut client, "old text", None, None).0,
        TextMethod::DIRECT
    );
    let (method, took) = set_text(&mut client, "Typed 42!", None, Some(TextMethod::KEYS));
    client.idle(Duration::from_millis(500));
    let after = snapshot(&mut client);
    let (field, _) = find(&after, "login-username").expect("login-username");
    fixture::force_stop(&dev, fixture::EXPO);
    println!("9 characters typed in {:.1} ms", ms(took));
    assert_eq!(method, TextMethod::KEYS);
    assert_eq!(field.node.text.as_deref(), Some("Typed 42!"));
}

#[test]
fn a_node_named_from_the_last_snapshot_is_set_directly() {
    let Some(dev) = device("a_node_named_from_the_last_snapshot_is_set_directly") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let (password, _) = find(&s, "login-password").expect("login-password");
    let (method, _) = set_text(&mut client, "hunter2", Some(password.node.id), None);
    assert_eq!(method, TextMethod::DIRECT);
    client.idle(Duration::from_millis(400));
    let after = snapshot(&mut client);
    let (field, _) = find(&after, "login-password").expect("login-password");
    fixture::force_stop(&dev, fixture::EXPO);
    assert!(field.node.flags.contains(NodeFlags::PASSWORD));
    assert_eq!(
        field
            .node
            .text
            .as_deref()
            .map(str::chars)
            .map(Iterator::count),
        Some(7)
    );
}

#[test]
fn direct_text_for_a_node_without_it_or_an_unknown_node_is_refused() {
    let Some(dev) = device("direct_text_for_a_node_without_it_or_an_unknown_node_is_refused")
    else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = snapshot(&mut client);
    let (status, _) = find(&s, "login-status").expect("login-status");
    for (node, mode) in [(status.node.id, Some(TextMethod::DIRECT)), (99_999, None)] {
        let error = client
            .call(Call::TextSet(TextParams {
                text: "x".to_owned(),
                node: Some(node),
                mode,
            }))
            .expect_err("text_set succeeded with nothing to set");
        assert_eq!(error.code, code::INVALID_PARAMS, "{error}");
        println!("{}", error.message);
    }
}
