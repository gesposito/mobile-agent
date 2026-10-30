//! Task 7.6: the structural diff and agent-browser's delta snapshots. A diff
//! matches elements by identity and lists what was added, removed and
//! changed; a delta snapshot numbers each listing and returns the whole
//! listing, `unchanged`, or the ref changes plus a line splice, falling back
//! to the whole listing when the delta is 70% of its size or more.

mod support;

use std::time::Duration;

use mirror::testing::{FakeTarget, Step};
use mirror::{
    Action, DeltaSnapshot, Diff, Field, Mirror, MirrorConfig, Ref, RefChange, RefField,
    SettleOptions, SnapshotOptions, Target, TreeSplice,
};
use protocol::event::{EventPayload, TreeChange, TreeChangeKind};
use serde_json::{Value, json};
use support::{Screen, mirror_on, ms};

fn content_changed(window: u32) -> Step {
    Step::Event(EventPayload::Tree(TreeChange {
        change: TreeChangeKind::CONTENT,
        window,
        path: vec![0],
    }))
}

fn identifiers(elements: &[mirror::Element]) -> Vec<&str> {
    elements
        .iter()
        .map(|e| e.identifier.as_deref().unwrap_or(""))
        .collect()
}

#[test]
fn a_form_submit_diff_lists_removed_fields_and_added_confirmation() {
    let screen = Screen::load("login");
    let submitted = screen.patched(json!([
        {"remove": "login-username"},
        {"remove": "login-password"},
        {"remove": "login-submit"},
        {"insert_before": "login-status", "node":
            {"class": "android.widget.TextView", "id": "login-welcome", "text": "Welcome, alice",
             "bounds": [16, 96, 392, 144]}}
    ]));
    let target = FakeTarget::new(screen.read());
    target.on_tap(
        "login-submit",
        vec![
            (ms(30), Step::Show(submitted.read())),
            (ms(30), content_changed(1)),
        ],
    );
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let baseline = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    let submit = baseline
        .by_identifier("login-submit")
        .unwrap()
        .reference
        .unwrap();
    mirror
        .act(&Action::Tap {
            target: Target::Ref(submit),
            hold: None,
        })
        .unwrap();
    assert!(mirror.settle(&SettleOptions::default()).unwrap().settled);

    let diff = mirror.diff(&baseline).unwrap();
    assert_eq!(
        identifiers(&diff.removed),
        ["login-username", "login-password", "login-submit"]
    );
    assert_eq!(identifiers(&diff.added), ["login-welcome"]);
    assert!(diff.changed.is_empty(), "{:?}", diff.changed);
    assert_eq!(diff.baseline_revision, baseline.revision);
    assert!(diff.revision > baseline.revision);
    assert_eq!(
        diff.text(),
        [
            "added:",
            "  - text \"Welcome, alice\" [id=login-welcome]",
            "removed:",
            "  - textbox \"Username\" [id=login-username, ref=e1]",
            "  - textbox \"Password\" [id=login-password, ref=e2]",
            "  - button \"Sign in\" [id=login-submit, ref=e3]",
        ]
        .join("\n")
    );
}

#[test]
fn a_diff_against_an_unchanged_screen_is_empty() {
    let (_, mut mirror) = mirror_on("login");
    let baseline = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    let diff = mirror.diff(&baseline).unwrap();
    assert!(diff.is_empty());
    assert_eq!(diff.text(), "(no changes)");
}

#[test]
fn changed_elements_list_the_fields_that_changed() {
    let screen = Screen::load("login");
    let later = screen.patched(json!([
        {"find": "login-status", "text": "Invalid credentials"},
        {"find": "login-submit", "off": ["enabled"]},
        {"find": "tab-login", "off": ["selected"]},
        {"find": "tab-list", "flags": ["selected"]}
    ]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(later.read()));
    target.at(ms(100), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let baseline = mirror.snapshot(&SnapshotOptions::full()).unwrap();
    mirror.run_until(target.instant(ms(200))).unwrap();

    let diff = Diff::between(
        &baseline,
        &mirror.snapshot(&SnapshotOptions::full()).unwrap(),
    );
    let fields: Vec<(&str, Vec<Field>)> = diff
        .changed
        .iter()
        .map(|c| {
            (
                c.after.identifier.as_deref().unwrap_or(""),
                c.fields.clone(),
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            ("login-submit", vec![Field::Enabled]),
            ("login-status", vec![Field::Name]),
            ("tab-login", vec![Field::Selected]),
            ("tab-list", vec![Field::Selected]),
        ]
    );
    let text = diff.text();
    assert!(
        text.contains(
            "  - text \"Invalid credentials\" [id=login-status]\n    name: \"Signed out\" -> \"Invalid credentials\""
        ),
        "{text}"
    );
    assert!(text.contains("    enabled: true -> false"), "{text}");
}

#[test]
fn delta_snapshots_follow_agent_browser_kinds_and_revisions() {
    let screen = Screen::load("grid");
    let done = screen.patched(json!([{"find": "cell-250", "desc": "Cell 250 (done)"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(500), Step::Show(done.read()));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let options = SnapshotOptions::interactive();

    let first = mirror.delta_snapshot(&options, false).unwrap();
    let DeltaSnapshot::Full {
        revision: 1,
        tree,
        refs,
    } = &first
    else {
        panic!("{first:?}")
    };
    assert_eq!(refs.len(), 505);
    assert_eq!(refs[0].reference, Ref(1));
    assert_eq!(refs[0].name, "Cell 1");

    let second = mirror.delta_snapshot(&options, false).unwrap();
    assert_eq!(
        second,
        DeltaSnapshot::Unchanged {
            revision: 2,
            base_revision: 1
        }
    );
    assert_eq!(
        second.to_json(),
        r#"{"baseRevision":1,"kind":"unchanged","revision":2}"#
    );

    target.advance(Duration::from_secs(1));
    let third = mirror.delta_snapshot(&options, false).unwrap();
    assert_eq!(
        third,
        DeltaSnapshot::Delta {
            revision: 3,
            base_revision: 2,
            changes: vec![RefChange::Replace {
                reference: Ref(250),
                field: RefField::Name,
                value: "Cell 250 (done)".to_owned(),
            }],
            tree_change: TreeSplice {
                start_line: 249,
                delete_count: 1,
                lines: vec!["- button \"Cell 250 (done)\" [id=cell-250, ref=e250]".to_owned()],
            },
        }
    );
    assert_eq!(
        third.to_json(),
        r#"{"baseRevision":2,"changes":[{"field":"name","op":"replace","ref":"@e250","value":"Cell 250 (done)"}],"kind":"delta","revision":3,"treeChange":{"deleteCount":1,"lines":["- button \"Cell 250 (done)\" [id=cell-250, ref=e250]"],"startLine":249}}"#
    );

    let mut rebuilt: Vec<&str> = tree.split('\n').collect();
    let DeltaSnapshot::Delta { tree_change, .. } = &third else {
        unreachable!()
    };
    rebuilt.splice(
        tree_change.start_line..tree_change.start_line + tree_change.delete_count,
        tree_change.lines.iter().map(String::as_str),
    );
    assert_eq!(
        rebuilt.join("\n"),
        mirror.snapshot(&options).unwrap().text,
        "the splice rebuilds the new listing from the old one"
    );

    let forced = mirror.delta_snapshot(&options, true).unwrap();
    assert_eq!(forced.kind(), "full");
    assert_eq!(forced.revision(), 4);
}

/// Profiling, task 3.1: a caller that needs the listing beside its delta,
/// as the daemon's snapshot handler does, gets both from one listing, the
/// delta the one `delta_snapshot` gives, both calls numbering one sequence.
#[test]
fn a_listing_and_its_delta_come_from_one_listing() {
    let screen = Screen::load("grid");
    let done = screen.patched(json!([{"find": "cell-250", "desc": "Cell 250 (done)"}]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(500), Step::Show(done.read()));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let options = SnapshotOptions::interactive();

    let (listed, first) = mirror.snapshot_with_delta(&options, false).unwrap();
    let DeltaSnapshot::Full {
        revision: 1,
        tree,
        refs,
    } = &first
    else {
        panic!("{first:?}")
    };
    assert_eq!(tree, &listed.text, "the delta is the listing's");
    let named: Vec<(Ref, &str)> = refs
        .iter()
        .map(|entry| (entry.reference, entry.name.as_str()))
        .collect();
    let elements: Vec<(Ref, &str)> = listed
        .elements
        .iter()
        .filter_map(|e| Some((e.reference?, e.name.as_str())))
        .collect();
    assert_eq!(named, elements, "the delta's refs are the listing's");

    let (again, second) = mirror.snapshot_with_delta(&options, false).unwrap();
    assert_eq!(again.text, listed.text);
    assert!(again.removed_refs.is_empty());
    assert_eq!(
        second,
        DeltaSnapshot::Unchanged {
            revision: 2,
            base_revision: 1
        }
    );

    target.advance(Duration::from_secs(1));
    let (changed, third) = mirror.snapshot_with_delta(&options, false).unwrap();
    let DeltaSnapshot::Delta {
        revision: 3,
        base_revision: 2,
        tree_change,
        ..
    } = &third
    else {
        panic!("{third:?}")
    };
    let mut rebuilt: Vec<&str> = tree.split('\n').collect();
    rebuilt.splice(
        tree_change.start_line..tree_change.start_line + tree_change.delete_count,
        tree_change.lines.iter().map(String::as_str),
    );
    assert_eq!(
        rebuilt.join("\n"),
        changed.text,
        "the splice rebuilds the listing given beside it"
    );

    assert_eq!(
        mirror.delta_snapshot(&options, false).unwrap(),
        DeltaSnapshot::Unchanged {
            revision: 4,
            base_revision: 3
        },
        "delta_snapshot goes on from the listing given beside its delta"
    );
}

#[test]
fn a_delta_of_70_percent_or_more_of_the_listing_falls_back_to_full() {
    let screen = Screen::load("login");
    let renamed = screen.patched(json!([
        {"find": "tab-login", "desc": "Sign-in tab"},
        {"find": "tab-list", "desc": "Rows tab"},
        {"find": "tab-big", "desc": "Grid tab"},
        {"find": "tab-anim", "desc": "Motion tab"},
        {"find": "tab-system", "desc": "Prompts tab"}
    ]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(100), Step::Show(renamed.read()));
    target.at(ms(100), content_changed(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let options = SnapshotOptions::interactive();
    assert_eq!(
        mirror.delta_snapshot(&options, false).unwrap().kind(),
        "full"
    );
    mirror.run_until(target.instant(ms(200))).unwrap();
    let next = mirror.delta_snapshot(&options, false).unwrap();
    let DeltaSnapshot::Full { revision, tree, .. } = &next else {
        panic!("{next:?}")
    };
    assert_eq!(*revision, 2);
    assert!(tree.contains("\"Prompts tab\""));
    let json: Value = serde_json::from_str(&next.to_json()).unwrap();
    assert_eq!(
        serde_json::to_string(&json).unwrap(),
        next.to_json(),
        "the bytes agent-browser's serde_json writes, sorted keys included"
    );
    assert_eq!(json["kind"], "full");
    assert_eq!(
        json["refs"]["e8"],
        json!({"role": "button", "name": "Prompts tab"})
    );
    assert_eq!(json["tree"].as_str(), Some(tree.as_str()));
}

#[test]
fn delta_ref_changes_follow_agent_browser_ops() {
    let screen = Screen::load("grid");
    // The new cell takes cell 3's place; listed after cell 4, at cell 4's
    // bounds it would be drawn over it.
    let swapped = screen.patched(json!([
        {"remove": "cell-3"},
        {"insert_before": "cell-5", "node":
            {"class": "XCUIElementTypeButton", "id": "cell-new", "desc": "New cell", "bounds": [41, 90, 61, 118]}}
    ]));
    let target = FakeTarget::new(screen.read());
    target.at(ms(500), Step::Show(swapped.read()));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    let options = SnapshotOptions::interactive();
    mirror.delta_snapshot(&options, false).unwrap();
    target.advance(Duration::from_secs(1));
    let delta = mirror.delta_snapshot(&options, false).unwrap();
    let DeltaSnapshot::Delta { changes, .. } = &delta else {
        panic!("{delta:?}")
    };
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0], RefChange::Remove(Ref(3)));
    let RefChange::Add(added) = &changes[1] else {
        panic!("{changes:?}")
    };
    assert_eq!(added.reference, Ref(506));
    assert_eq!(added.name, "New cell");
    let json: Value = serde_json::from_str(&delta.to_json()).unwrap();
    assert_eq!(serde_json::to_string(&json).unwrap(), delta.to_json());
    assert_eq!(json["changes"][0], json!({"op": "remove", "ref": "@e3"}));
    assert_eq!(
        json["changes"][1],
        json!({"op": "add", "ref": "@e506", "node": {"role": "button", "name": "New cell"}})
    );
}

#[test]
fn changing_the_listing_options_starts_again_with_the_whole_listing() {
    let (_, mut mirror) = mirror_on("login");
    let first = mirror
        .delta_snapshot(&SnapshotOptions::interactive(), false)
        .unwrap();
    assert_eq!(first.kind(), "full");
    let other = mirror
        .delta_snapshot(&SnapshotOptions::full(), false)
        .unwrap();
    assert_eq!(other.kind(), "full");
    assert_eq!(other.revision(), 2);
}
