//! The listing's reply: the data of `snapshot` without `delta`, written
//! from the mirror's snapshot of the screen straight into JSON text: the
//! listing's text, its refs map, the refs that left the screen, the
//! snapshot's origin, revision and verdict, and what the worker's answer
//! cost (`README.md`, Protocol).
//!
//! The text is the one `serde_json` writes for those fields as values, byte
//! for byte, as the other commands' replies are written: compact, the keys
//! in its map's sorted order, the strings escaped as `mirror::json` escapes
//! the listing's, the numbers formatted as `serde_json` formats them. A ref
//! costs no allocation of its own, where its key and entry as values would
//! take six (design D3 of `profiling`, `daemon/wire/reply-snapshot-big`).

use std::fmt::Write as _;

use mirror::json::push_str;
use mirror::{Element, Ref, Snapshot};
use serde_json::Number;

use crate::handlers::{origin, verdict_json, verdict_warning};
use crate::wire::Success;

/// What the listing's reply says of the worker's answer beside the
/// snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Answer<'a> {
    /// The target's worker's time to answer, device reads included, in
    /// milliseconds.
    pub read_ms: f64,
    /// Whether the worker answered without reading the device.
    pub served_from_replica: bool,
    /// The target's id.
    pub target: &'a str,
}

/// The reply to `snapshot` without `delta` for `snapshot`, which the
/// target's worker answered as `answer` says, with the warning a degraded
/// snapshot carries: its data written as JSON text, the keys in their
/// sorted order.
pub fn reply(snapshot: &Snapshot, answer: &Answer<'_>) -> Success {
    let mut out = String::with_capacity(capacity(snapshot));
    out.push_str(r#"{"origin":"#);
    push_str(&mut out, &origin(snapshot));
    out.push_str(r#","readMs":"#);
    push_f64(&mut out, answer.read_ms);
    out.push_str(r#","refs":"#);
    push_refs(&mut out, &snapshot.elements);
    out.push_str(r#","removedRefs":["#);
    for (i, reference) in snapshot.removed_refs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, r#""@{reference}""#);
    }
    // The mirror's replica revision.
    let _ = write!(out, r#"],"replicaRevision":{}"#, snapshot.revision);
    let _ = write!(
        out,
        r#","servedFromReplica":{}"#,
        answer.served_from_replica
    );
    out.push_str(r#","snapshot":"#);
    push_str(&mut out, &snapshot.text);
    out.push_str(r#","target":"#);
    push_str(&mut out, answer.target);
    let _ = write!(out, r#","verdict":{}}}"#, verdict_json(&snapshot.verdict));
    Success::written(out).warn(verdict_warning(&snapshot.verdict))
}

/// About the length of the reply's text, so that it is seldom grown while
/// it is written: the listing's text with an eighth more for its escapes,
/// the backslashes before each line's quotes and newline (three in a line
/// of about 45 characters on a 500-cell screen), each ref's entry and the
/// rest.
fn capacity(snapshot: &Snapshot) -> usize {
    let refs: usize = snapshot
        .elements
        .iter()
        .filter(|element| element.reference.is_some())
        .map(|element| element.name.len() + element.role.as_str().len() + 40)
        .sum();
    snapshot.text.len() * 9 / 8 + refs + snapshot.removed_refs.len() * 16 + 512
}

/// Appends `value` as `serde_json` writes an `f64`: null when it is not
/// finite, which no number in JSON stands for.
fn push_f64(out: &mut String, value: f64) {
    match Number::from_f64(value) {
        Some(number) => {
            let _ = write!(out, "{number}");
        }
        None => out.push_str("null"),
    }
}

/// Appends the refs map: each listed ref's name and role, by the ref, in the
/// order `serde_json`'s map keeps its keys, by their text (`e10` before
/// `e2`); of elements listed under one ref, the last.
fn push_refs(out: &mut String, elements: &[Element]) {
    let mut listed: Vec<(u64, usize, Ref, &Element)> = Vec::with_capacity(elements.len());
    listed.extend(elements.iter().enumerate().filter_map(|(i, element)| {
        let reference = element.reference?;
        Some((text_order(reference), i, reference, element))
    }));
    listed.sort_unstable_by_key(|&(order, i, _, _)| (order, i));
    out.push('{');
    let mut first = true;
    for (at, &(order, _, reference, element)) in listed.iter().enumerate() {
        if listed.get(at + 1).is_some_and(|next| next.0 == order) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(r#""e"#);
        push_digits(out, reference.0);
        out.push_str(r#"":{"name":"#);
        push_str(out, &element.name);
        out.push_str(r#","role":"#);
        push_str(out, element.role.as_str());
        out.push('}');
    }
    out.push('}');
}

/// A key that orders refs as their text sorts, `e1`, `e10`, `e100`, `e2`:
/// the ref's number with zeros after it to ten digits, the most a `u32`
/// has, which orders by the digits from the left, then its own number of
/// digits, which puts a number before the longer ones it starts.
fn text_order(reference: Ref) -> u64 {
    let n = u64::from(reference.0);
    let digits = n.checked_ilog10().unwrap_or(0) + 1;
    n * 10u64.pow(10 - digits) * 16 + u64::from(digits)
}

/// Appends the decimal digits of `n`.
fn push_digits(out: &mut String, mut n: u32) {
    let mut digits = [0u8; 10];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.push_str(std::str::from_utf8(&digits[at..]).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mirror::backend::{BackendInfo, Owner};
    use mirror::{Degradation, ElementId, Mirror, Rect, Role, SnapshotOptions, Verdict};
    use serde_json::{Value, json};

    use super::*;
    use crate::driver::{AttachOptions, Driver, Launch};
    use crate::fake::FakeDriver;
    use crate::wire::reply_line;

    /// The fake emulator's id and the warm interactive listing of the
    /// fixture's Big tab, its 500 cells and five tabs.
    fn big_tab() -> (String, Snapshot) {
        let driver = FakeDriver::parse("android").unwrap();
        let info = driver.discover().unwrap().remove(0);
        let mut attachment = driver.attach(&info, &AttachOptions::default()).unwrap();
        let big = Launch {
            relaunch: false,
            env: vec![("initial_tab".into(), "big".into())],
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        attachment
            .apps
            .launch("dev.mobileagent.fixture.expo", &big, deadline)
            .unwrap();
        let mut mirror = Mirror::new(attachment.backend, attachment.config);
        let options = SnapshotOptions::interactive();
        while mirror.snapshot(&options).unwrap().refs().len() < 505 {
            assert!(Instant::now() < deadline, "no Big tab");
            mirror
                .run_until(Instant::now() + Duration::from_millis(10))
                .unwrap();
        }
        let snapshot = mirror.snapshot(&options).unwrap();
        (info.id, snapshot)
    }

    /// The reply line of `snapshot`'s reply, answered as `answer` says.
    fn line(snapshot: &Snapshot, answer: &Answer<'_>) -> String {
        reply_line(&json!("r123456"), &Ok(reply(snapshot, answer)))
    }

    /// The data and warning of reply `line`, checked to be the line
    /// `serde_json` writes for them read back as values, byte for byte:
    /// with the values below, the line the reply built as values gave.
    fn read_back(line: &str) -> (Value, Value) {
        let reply: Value = serde_json::from_str(line).unwrap();
        let warning = reply["warning"].as_str().map(str::to_owned);
        let values = Success::new(reply["data"].clone()).warn(warning);
        assert_eq!(line, reply_line(&reply["id"], &Ok(values)));
        (reply["data"].clone(), reply["warning"].clone())
    }

    #[test]
    fn the_big_tab_s_reply_is_the_line_serde_json_writes_for_its_values() {
        let (target, snapshot) = big_tab();
        let answer = Answer {
            read_ms: 0.731,
            served_from_replica: true,
            target: &target,
        };
        let (data, warning) = read_back(&line(&snapshot, &answer));
        let refs = data["refs"].as_object().unwrap();
        assert_eq!(refs.len(), 505, "{data}");
        for element in &snapshot.elements {
            let key = element.reference.unwrap().to_string();
            let role = element.role.as_str();
            assert_eq!(refs[&key], json!({"name": element.name, "role": role}));
        }
        assert_eq!(
            data,
            json!({
                "origin": origin(&snapshot),
                "readMs": 0.731,
                "refs": refs,
                "removedRefs": [],
                "replicaRevision": snapshot.revision,
                "servedFromReplica": true,
                "snapshot": snapshot.text,
                "target": target,
                "verdict": verdict_json(&snapshot.verdict),
            })
        );
        assert_eq!(warning, Value::Null);
    }

    /// An element of `role` named `name`, with `reference` when given.
    fn element(id: u64, reference: Option<u32>, role: Role, name: &str) -> Element {
        Element {
            id: ElementId(id),
            reference: reference.map(Ref),
            role,
            name: name.to_owned(),
            identifier: None,
            value: None,
            bounds: Rect {
                left: 0.0,
                top: 0.0,
                right: 10.0,
                bottom: 10.0,
            },
            enabled: true,
            focused: false,
            selected: false,
            checked: None,
            visible: None,
            actionable: true,
            window: 1,
            depth: 0,
            class: "View".to_owned(),
        }
    }

    #[test]
    fn strings_numbers_and_refs_go_in_as_serde_json_writes_them() {
        let names = [
            "plain",
            "a \"b\" \\ c",
            "l1\nl2\r\t",
            "\u{1}\u{1f}\u{8}\u{c}",
            "/ é “q” \u{7f} 😀",
            "",
        ];
        let mut elements: Vec<Element> = names
            .iter()
            .zip([1, 2, 10, 100, 11, 1000])
            .enumerate()
            .map(|(i, (name, n))| element(i as u64, Some(n), Role::Button, name))
            .collect();
        elements.push(element(20, None, Role::Text, "no ref"));
        elements.push(element(21, Some(9), Role::Switch, "Wi-Fi"));
        elements.push(element(22, Some(u32::MAX), Role::Cell, "last"));
        // Two elements under one ref: the map keeps the later one's.
        elements.push(element(23, Some(10), Role::Link, "again"));
        let mut snapshot = Snapshot {
            revision: u64::MAX,
            taken_at: Instant::now(),
            options: SnapshotOptions::interactive(),
            scale: 3.0,
            foreground: Owner::System("com.example \"alert\"".to_owned()),
            verdict: Verdict {
                backend: BackendInfo {
                    name: "simulator-translator".to_owned(),
                    fallback: true,
                },
                degraded: true,
                reasons: vec![
                    Degradation::FallbackBackend {
                        name: "simulator-translator".to_owned(),
                    },
                    Degradation::AgentWarning("lost \"automation\"\n".to_owned()),
                ],
            },
            windows: Vec::new(),
            elements,
            hints: Vec::new(),
            removed_refs: vec![Ref(3), Ref(12), Ref(1)],
            text: names.join("\n"),
        };
        let answer = Answer {
            read_ms: 0.731,
            served_from_replica: false,
            target: "FAKE \"1\"",
        };
        let (data, warning) = read_back(&line(&snapshot, &answer));
        let button = |name: &str| json!({"name": name, "role": "button"});
        assert_eq!(
            data,
            json!({
                "origin": "com.example \"alert\"",
                "readMs": 0.731,
                "refs": {
                    "e1": button(names[0]),
                    "e2": button(names[1]),
                    "e10": {"name": "again", "role": "link"},
                    "e100": button(names[3]),
                    "e11": button(names[4]),
                    "e1000": button(names[5]),
                    "e9": {"name": "Wi-Fi", "role": "switch"},
                    "e4294967295": {"name": "last", "role": "cell"},
                },
                "removedRefs": ["@e3", "@e12", "@e1"],
                "replicaRevision": u64::MAX,
                "servedFromReplica": false,
                "snapshot": names.join("\n"),
                "target": "FAKE \"1\"",
                "verdict": verdict_json(&snapshot.verdict),
            })
        );
        assert_eq!(warning, json!(verdict_warning(&snapshot.verdict)));
        for read_ms in [0.0, 1.0, 12_345.678, 1e-7, 1e21, 0.1 + 0.2, f64::NAN] {
            let answer = Answer { read_ms, ..answer };
            let (data, _) = read_back(&line(&snapshot, &answer));
            assert_eq!(data["readMs"], json!(read_ms), "{read_ms}");
        }
        snapshot.removed_refs.clear();
        snapshot.foreground = Owner::Target;
        snapshot.verdict.degraded = false;
        snapshot.verdict.reasons.clear();
        snapshot.elements.clear();
        let (data, warning) = read_back(&line(&snapshot, &answer));
        assert_eq!(data["refs"], json!({}));
        assert_eq!(data["removedRefs"], json!([]));
        assert_eq!(data["origin"], "");
        assert_eq!(warning, Value::Null);
    }

    #[test]
    fn refs_order_as_their_text_sorts() {
        let refs = [
            1,
            2,
            9,
            10,
            11,
            19,
            20,
            99,
            100,
            101,
            109,
            110,
            1000,
            u32::MAX,
        ];
        let mut by_number: Vec<Ref> = refs.iter().copied().map(Ref).collect();
        by_number.sort_by_key(|r| text_order(*r));
        let mut as_text: Vec<String> = refs.iter().map(|n| format!("e{n}")).collect();
        as_text.sort();
        let sorted: Vec<String> = by_number.iter().map(Ref::to_string).collect();
        assert_eq!(sorted, as_text);
        for n in refs {
            let mut digits = String::new();
            push_digits(&mut digits, n);
            assert_eq!(digits, n.to_string());
        }
    }
}
