//! Task 9.4: the adapter for `mobile-agent`, the tool under test. It drives the
//! daemon in `--json` mode through the documented commands and maps each spec
//! primitive onto the CLI's timings, the tap spans onto the fields a click's
//! reply carries; a span the target cannot produce is not measured, with the
//! reason. The tests run it against the daemon's fake targets, and against a
//! scripted stand-in for replies the fakes do not give.

mod common;

use std::fs;

use bench::adapter::{Adapter, Clock, Detection, Role, Scenario};
use bench::cli::{CliAdapter, CliConfig, SESSION};
use bench::fixture::{Fixture, Screen};
use bench::flow;
use bench::gates::{Gate, GateSet, GateStatus};
use bench::harness::{Harness, RunConfig, Series, Subject};
use bench::plan;
use bench::primitive::{Primitive, TEXT_ENTRY};
use bench::report::{Measurement, Report};
use bench::target::{Platform, Target, TargetClass};
use common::{FakeDevices, stand_in};
use serde_json::json;

const ITERATIONS: u32 = 3;
const WARMUP: u32 = 1;

fn emulator() -> Target {
    Target::new("emulator-5554", TargetClass::AndroidEmulator)
}

fn simulator() -> Target {
    Target::new("FAKE-IOS-1", TargetClass::Simulator)
}

/// The plan's series of `primitives` for Expo on `target`.
fn planned(target: &Target, primitives: &[Primitive]) -> Vec<Series> {
    plan::series(target, &[Fixture::Expo])
        .into_iter()
        .filter(|s| matches!(s.subject, Subject::Primitive(p) if primitives.contains(&p)))
        .collect()
}

fn run(adapter: CliAdapter, target: Target, series: Vec<Series>, config: RunConfig) -> Report {
    Harness::new(config, vec![target], series)
        .flows(plan::flows(&[Fixture::Expo]))
        .tool(Box::new(adapter))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap()
}

fn config() -> RunConfig {
    RunConfig {
        iterations: ITERATIONS,
        warmup: WARMUP,
        retries: 0,
    }
}

fn find(report: &Report, primitive: Primitive) -> &Measurement {
    report
        .measurements
        .iter()
        .find(|m| m.subject == Subject::Primitive(primitive))
        .unwrap_or_else(|| panic!("no {primitive} measurement in {:?}", report.measurements))
}

/// `args` for the adapter's session on `target`.
fn in_session<'a>(target: &'a Target, args: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec!["--session", SESSION, "--device", target.id.as_str()];
    all.extend_from_slice(args);
    all
}

/// How late the fake targets show a change in the tests that time tap spans.
const LATENCY_US: u64 = 20_000;

#[test]
fn every_listed_primitive_appears_through_the_cli_adapter_on_a_fake_emulator() {
    let devices = FakeDevices::slow("android", (LATENCY_US / 1000) as u32);
    let work = common::scratch("cli-every-primitive");
    let adapter = CliAdapter::new(devices.config(&work));
    let report = run(
        adapter,
        emulator(),
        planned(&emulator(), &Primitive::ALL),
        config(),
    );
    let tool = &report.provenance.tools[0];
    assert_eq!(
        (tool.name.as_str(), tool.role, tool.version.as_deref()),
        (
            "mobile-agent",
            Role::UnderTest,
            Some(env!("CARGO_PKG_VERSION"))
        )
    );
    let measured = [
        (Primitive::CliStart, Clock::Wall, None),
        (Primitive::ColdAttach, Clock::Wall, None),
        (Primitive::FirstSnapshot, Clock::Wall, Some(505)),
        (Primitive::WarmSnapshot, Clock::Wall, Some(505)),
        (Primitive::TapDispatch, Clock::Tool, None),
        (Primitive::TapToFrameChange, Clock::Tool, None),
        (Primitive::TapToTreeDelta, Clock::Tool, None),
        (Primitive::TextEntry, Clock::Wall, None),
        (Primitive::Screenshot, Clock::Wall, None),
        (Primitive::Settle, Clock::Tool, None),
    ];
    for primitive in Primitive::LISTED {
        assert!(measured.iter().any(|(p, _, _)| *p == primitive));
    }
    for (primitive, clock, nodes) in measured {
        let m = find(&report, primitive);
        assert_eq!(m.error, None, "{primitive}");
        assert_eq!(m.not_measured, None, "{primitive}");
        assert_eq!(
            m.stats.map(|s| s.n),
            Some(ITERATIONS),
            "{primitive}: {:?}",
            m.samples
        );
        assert_eq!(m.reliability.failures, 0, "{primitive}: {:?}", m.samples);
        assert_eq!(m.clock, Some(clock), "{primitive}");
        assert_eq!(
            m.nodes.map(|n| (n.min, n.max)),
            nodes.map(|n| (n, n)),
            "{primitive}"
        );
        let scenario = plan::scenario(primitive, Fixture::Expo);
        assert_eq!((m.fixture, m.screen), (scenario.fixture, scenario.screen));
    }
    // The fake shows the detail a tap opens 20 ms late: the first changed
    // frame and the first tree delta come after that, the dispatch before.
    let p50 = |primitive| find(&report, primitive).stats.unwrap().p50_us;
    assert!(p50(Primitive::TapDispatch) < LATENCY_US);
    assert!(p50(Primitive::TapToFrameChange) >= LATENCY_US);
    assert!(p50(Primitive::TapToTreeDelta) >= LATENCY_US);
    let details = |primitive| -> Vec<Option<String>> {
        find(&report, primitive)
            .samples
            .iter()
            .map(|s| s.detail.clone())
            .collect()
    };
    for detail in details(Primitive::Settle) {
        assert!(
            detail
                .as_deref()
                .is_some_and(|d| d.starts_with("ended by ")),
            "{detail:?}"
        );
    }
    // The fill read the field back; on Android the fill that empties the
    // field is not read back, since an empty field reads as its hint.
    for detail in details(Primitive::TextEntry) {
        let detail = detail.unwrap_or_default();
        assert!(
            detail.starts_with("method direct, read back as text in ")
                && detail.ends_with(" ms; cleared: method direct, not read back"),
            "{detail}"
        );
    }
}

#[test]
fn a_tap_on_an_android_device_has_no_frame_change_and_is_listed_as_not_measured() {
    let devices = FakeDevices::slow("android:R58M123", (LATENCY_US / 1000) as u32);
    let device = Target::new("R58M123", TargetClass::AndroidDevice);
    let work = common::scratch("cli-no-frames");
    let report = run(
        CliAdapter::new(devices.config(&work)),
        device.clone(),
        planned(
            &device,
            &[Primitive::TapToFrameChange, Primitive::TapToTreeDelta],
        ),
        config(),
    );
    let frame = find(&report, Primitive::TapToFrameChange);
    let reason = frame
        .not_measured
        .as_deref()
        .unwrap_or_else(|| panic!("tap-to-frame-change was measured: {frame:?}"));
    assert!(
        reason.contains("D7") && reason.contains("frame"),
        "{reason}"
    );
    assert_eq!((frame.error.as_deref(), frame.stats), (None, None));
    assert!(frame.samples.is_empty());
    assert_eq!(frame.reliability.failures, 0);
    let tree = find(&report, Primitive::TapToTreeDelta);
    assert_eq!(tree.not_measured, None);
    assert!(
        tree.stats
            .is_some_and(|s| s.n == ITERATIONS && s.p50_us >= LATENCY_US)
    );

    let md = report.to_markdown();
    let comparison = common::table(&md, "Comparison");
    let row = comparison
        .rows
        .iter()
        .find(|r| comparison.cell(r, "Subject") == "tap-to-frame-change")
        .unwrap_or_else(|| panic!("no tap-to-frame-change row in {md}"));
    assert_eq!(comparison.cell(row, "mobile-agent"), "not measured");
    assert!(
        md.contains("Series not measured:")
            && md.contains(&format!(
                "- mobile-agent on R58M123, tap-to-frame-change: {reason}"
            )),
        "{md}"
    );
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["measurements"][0]["not_measured"], reason);
    common::assert_every_markdown_number_is_in_the_json(&md, &json);

    // A gate on the span lists it as not measured and does not fail the run.
    let gates = GateSet {
        gates: vec![Gate::new(
            TargetClass::AndroidDevice,
            Primitive::TapToFrameChange,
            50.0,
        )],
    };
    let verdict = gates.evaluate(&report);
    assert_eq!(verdict.rows.len(), 1);
    assert_eq!(verdict.rows[0].status, GateStatus::NotMeasured);
    assert!(verdict.passed, "{}", verdict.table());
}

/// A click reply carrying the frame and tree spans task 8.9 adds.
const CLICK_WITH_SPANS: &str = r#"{"success":true,"data":{"accepted":true,"clicked":"id=row-1","dispatchMs":0.25,"frameChangeMs":14.5,"ref":null,"treeDeltaMs":21.75,"x":204.0,"y":120.0},"error":null}"#;

#[test]
fn click_replies_that_carry_frame_and_tree_spans_fill_them_in() {
    let dir = common::scratch("cli-stand-in");
    let mut settings = CliConfig::new(dir.join("work"));
    settings.program = stand_in(&dir, &[("click", CLICK_WITH_SPANS, 0)]);
    let primitives = [
        Primitive::TapDispatch,
        Primitive::TapToFrameChange,
        Primitive::TapToTreeDelta,
        Primitive::Settle,
    ];
    let report = run(
        CliAdapter::new(settings),
        emulator(),
        planned(&emulator(), &primitives),
        RunConfig {
            iterations: 2,
            warmup: 0,
            retries: 0,
        },
    );
    assert_eq!(report.provenance.tools[0].version.as_deref(), Some("0.2.0"));
    for (primitive, us) in [
        (Primitive::TapDispatch, 250),
        (Primitive::TapToFrameChange, 14_500),
        (Primitive::TapToTreeDelta, 21_750),
        (Primitive::Settle, 212_500),
    ] {
        let m = find(&report, primitive);
        assert_eq!(m.not_measured, None, "{primitive}");
        assert_eq!(m.clock, Some(Clock::Tool), "{primitive}");
        assert_eq!(
            m.stats.map(|s| (s.n, s.p50_us, s.max_us)),
            Some((2, us, us)),
            "{primitive}: {:?}",
            m.samples
        );
    }
    // Global flags come first, then the command, its flags, `--` and its
    // operands, as the CLI's grammar reads them.
    let log = fs::read_to_string(dir.join("mobile-agent.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines[0], "--version");
    let session = "--json --session mobile-agent-bench --device emulator-5554";
    for command in [
        "open --relaunch --env initial_tab=list -- dev.mobileagent.fixture.expo",
        "wait --timeout 10000 -- id=row-1",
        "click --timeout 10000 -- id=row-1",
        "click --timeout 10000 -- id=detail-back",
        "wait --settle",
    ] {
        let line = format!("{session} {command}");
        assert!(lines.contains(&line.as_str()), "no {line:?} in:\n{log}");
    }
}

/// The stand-in's measurements of `primitives` on the emulator's List screen.
fn stand_in_run(name: &str, click: &str, primitives: &[Primitive]) -> Report {
    let dir = common::scratch(name);
    let mut settings = CliConfig::new(dir.join("work"));
    settings.program = stand_in(&dir, &[("click", click, 0)]);
    run(
        CliAdapter::new(settings),
        emulator(),
        planned(&emulator(), primitives),
        RunConfig {
            iterations: 2,
            warmup: 0,
            retries: 0,
        },
    )
}

#[test]
fn a_cli_whose_click_does_not_watch_the_tap_leaves_both_spans_not_measured() {
    let report = stand_in_run(
        "cli-stand-in-no-watch",
        r#"{"success":true,"data":{"accepted":true,"clicked":"id=row-1","dispatchMs":0.25,"ref":null,"x":204.0,"y":120.0},"error":null}"#,
        &[
            Primitive::TapDispatch,
            Primitive::TapToFrameChange,
            Primitive::TapToTreeDelta,
        ],
    );
    let dispatch = find(&report, Primitive::TapDispatch);
    assert_eq!(dispatch.stats.map(|s| s.p50_us), Some(250));
    for (primitive, field) in [
        (Primitive::TapToFrameChange, "frameChangeMs"),
        (Primitive::TapToTreeDelta, "treeDeltaMs"),
    ] {
        let m = find(&report, primitive);
        let reason = m
            .not_measured
            .as_deref()
            .unwrap_or_else(|| panic!("{primitive} was measured: {m:?}"));
        assert!(
            reason.contains("watchedMs") && reason.contains(field) && reason.contains("8.9"),
            "{reason}"
        );
        assert_eq!((m.error.as_deref(), m.stats), (None, None), "{primitive}");
    }
}

#[test]
fn a_tap_whose_effects_never_came_fails_its_spans() {
    let report = stand_in_run(
        "cli-stand-in-no-change",
        r#"{"success":true,"data":{"accepted":true,"clicked":"id=row-1","dispatchMs":0.25,"ref":null,"watchedMs":200.5,"x":204.0,"y":120.0},"error":null}"#,
        &[Primitive::TapToFrameChange, Primitive::TapToTreeDelta],
    );
    for (primitive, error) in [
        (
            Primitive::TapToFrameChange,
            "the click on row-1 reported no frameChangeMs: no frame changed within its 200.5 ms watch",
        ),
        (
            Primitive::TapToTreeDelta,
            "the click on row-1 reported no treeDeltaMs: the tree did not change within its 200.5 ms watch",
        ),
    ] {
        let m = find(&report, primitive);
        assert_eq!(m.not_measured, None, "{primitive}");
        assert_eq!(m.stats, None, "{primitive}");
        assert_eq!(m.reliability.failures, 2, "{primitive}");
        assert_eq!(m.samples[0].errors, [error], "{primitive}");
    }
}

#[test]
fn a_failed_command_is_an_error_naming_the_command_its_code_and_its_message() {
    let dir = common::scratch("cli-failed-command");
    let mut settings = CliConfig::new(dir.join("work"));
    settings.program = stand_in(
        &dir,
        &[(
            "click",
            r#"{"success":false,"data":null,"error":"no element matches id row-1 after 10000 ms; the screen shows revision 3:\n- button \"Home\" [id=home, ref=e1]","code":"element_not_found"}"#,
            1,
        )],
    );
    let mut adapter = CliAdapter::new(settings);
    assert!(matches!(adapter.detect(), Detection::Available { .. }));
    let error = adapter
        .measure(
            &emulator(),
            &plan::scenario(Primitive::TapDispatch, Fixture::Expo),
            Primitive::TapDispatch,
        )
        .unwrap_err();
    assert!(!error.not_measured);
    assert_eq!(
        error.message,
        "mobile-agent click --timeout 10000 -- id=row-1: element_not_found: no element matches \
         id row-1 after 10000 ms; the screen shows revision 3:"
    );
}

/// Runs the three reference flows on `target` of a fake `platform` and checks
/// the screen each leaves behind.
fn flows_run_on(fake: &str, target: Target, name: &str) {
    let devices = FakeDevices::new(fake);
    let mut adapter = CliAdapter::new(devices.config(&common::scratch(name)));
    assert!(matches!(adapter.detect(), Detection::Available { .. }));
    let [login, list, big] = <[flow::Flow; 3]>::try_from(flow::reference(Fixture::Expo)).unwrap();

    let sample = adapter.run_flow(&target, &login).unwrap();
    assert_eq!((sample.clock, sample.nodes), (Clock::Wall, None));
    let status = devices.cli(&in_session(&target, &["get", "text", "id=login-status"]));
    assert_eq!(status["text"], "Welcome, agent");

    let sample = adapter.run_flow(&target, &list).unwrap();
    assert_eq!((sample.clock, sample.nodes), (Clock::Wall, None));
    // The daemon's `scroll --until` took strokes to bring row-200 on screen.
    let strokes = sample
        .detail
        .as_deref()
        .and_then(|d| d.split_once(" strokes"))
        .and_then(|(n, _)| n.parse::<u32>().ok());
    assert!(strokes.is_some_and(|n| n > 0), "{sample:?}");
    let detail = devices.cli(&in_session(&target, &["get", "text", "id=detail-200"]));
    assert_eq!(detail["text"], "Detail 200");

    let sample = adapter.run_flow(&target, &big).unwrap();
    assert_eq!((sample.clock, sample.nodes), (Clock::Wall, Some(505)));
    let count = devices.cli(&in_session(&target, &["get", "text", "id=big-count"]));
    assert_eq!(count["text"], "500 elements");
}

#[test]
fn the_reference_flows_run_through_the_cli_adapter_on_a_fake_emulator() {
    flows_run_on("android", emulator(), "cli-flows-android");
}

/// A `scroll --until` reply that found `row-200` after 7 strokes, with the
/// share of it in view that task 10.15's daemon reports, when given.
fn scroll_until_reply(visible: Option<f64>) -> String {
    let mut data = json!({
        "amount": 600.0,
        "direction": "down",
        "element": {
            "bounds": [0.0, 846.5, 411.0, 914.0],
            "enabled": true,
            "id": "row-200",
            "name": "Row 200",
            "ref": "@e31",
            "role": "button",
            "value": null
        },
        "scrolled": true,
        "strokes": 7,
        "until": "id=row-200"
    });
    if let Some(visible) = visible {
        data["visible"] = json!(visible);
    }
    json!({"success": true, "data": data, "error": null}).to_string()
}

/// Task 10.17: the list-scroll flow scrolls to its row as an agent does, with
/// one `scroll --until` whose stop condition is the daemon's, and none of
/// the harness's own settles, zero-time checks and plain strokes around it.
/// The sample notes the strokes the reply counts and, from a daemon that
/// reports it, the share of the row in view.
#[test]
fn a_scroll_to_is_one_scroll_until_as_an_agent_runs_it() {
    let session = "--json --session mobile-agent-bench --device emulator-5554";
    let expected: Vec<String> = [
        "open --relaunch -- dev.mobileagent.fixture.expo",
        "click --timeout 10000 -- id=tab-list",
        "scroll --timeout 90000 --until id=row-200 -- down 600",
        "click --timeout 10000 -- id=row-200",
        "wait --timeout 10000 --text=Detail 200",
        "get text --timeout 10000 -- id=detail-200",
    ]
    .iter()
    .map(|command| format!("{session} {command}"))
    .collect();
    for (name, visible, detail) in [
        (
            "cli-scroll-until-visible",
            Some(0.62),
            "7 strokes, visible 0.62",
        ),
        ("cli-scroll-until", None, "7 strokes"),
    ] {
        let dir = common::scratch(name);
        let mut settings = CliConfig::new(dir.join("work"));
        let scroll = scroll_until_reply(visible);
        settings.program = stand_in(
            &dir,
            &[
                ("scroll", scroll.as_str(), 0),
                (
                    "text",
                    r#"{"success":true,"data":{"text":"Detail 200"},"error":null}"#,
                    0,
                ),
            ],
        );
        let mut adapter = CliAdapter::new(settings);
        assert!(matches!(adapter.detect(), Detection::Available { .. }));
        let sample = adapter
            .run_flow(&emulator(), &flow::list_scroll(Fixture::Expo))
            .unwrap();
        let log = fs::read_to_string(dir.join("mobile-agent.log")).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines[0], "--version");
        assert_eq!(lines[1..], expected, "{name}");
        for old in ["wait --settle", "wait --timeout 0 ", "scroll -- "] {
            assert!(!log.contains(old), "{name}: {old:?} in:\n{log}");
        }
        assert_eq!(
            (sample.clock, sample.nodes, sample.detail.as_deref()),
            (Clock::Wall, None, Some(detail)),
            "{name}"
        );
    }
}

#[test]
fn the_reference_flows_run_through_the_cli_adapter_on_a_fake_simulator() {
    flows_run_on("ios", simulator(), "cli-flows-ios");
}

/// A wrapper around the real `mobile-agent` that refuses an `open` while a
/// daemon's socket is in `devices`' directory, as the agent's attach refuses
/// while the old daemon's agent still holds UI automation.
fn refusing_open_beside_a_daemon(
    devices: &FakeDevices,
    dir: &std::path::Path,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("mobile-agent");
    fs::write(
        &path,
        format!(
            r#"#!/bin/sh
case " $* " in
*" open "*)
  if [ -S '{socket}' ]; then
    echo '{{"success":false,"data":null,"error":"another process holds UI automation, likely mobile-agent-daemon","code":"automation_conflict"}}'
    exit 1
  fi ;;
esac
exec '{cli}' "$@"
"#,
            socket = devices.dir.join("daemon.sock").display(),
            cli = common::mobile_agent().display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Task 8.16: the daemon answers `shutdown` only once it has stopped its
/// targets and removed its socket, so the `open` a cold attach sends right
/// after the shutdown never meets the old daemon still holding the target,
/// even when the target takes 300 ms to stop. The `open` is refused, as the
/// agent's attach is, while a daemon's socket is still there.
#[test]
fn a_cold_attach_opens_only_once_the_old_daemon_has_let_go_of_its_targets() {
    let devices = FakeDevices::new("android").stopping(300);
    let target = emulator();
    devices.cli(&in_session(&target, &["snapshot"]));
    let dir = common::scratch("cli-cold-attach-waits");
    let mut settings = devices.config(&dir.join("work"));
    settings.program = refusing_open_beside_a_daemon(&devices, &dir);
    let mut adapter = CliAdapter::new(settings);
    assert!(matches!(adapter.detect(), Detection::Available { .. }));
    let scenario = plan::scenario(Primitive::ColdAttach, Fixture::Expo);
    for _ in 0..2 {
        let sample = adapter
            .measure(&target, &scenario, Primitive::ColdAttach)
            .unwrap();
        assert!(
            sample
                .detail
                .as_deref()
                .is_some_and(|d| d.starts_with("open settled in ")),
            "{sample:?}"
        );
    }
}

#[test]
fn cold_attach_stops_the_daemon_then_times_the_open_that_starts_another() {
    let devices = FakeDevices::new("android");
    let mut adapter = CliAdapter::new(devices.config(&common::scratch("cli-cold-attach")));
    adapter.detect();
    let scenario = plan::scenario(Primitive::ColdAttach, Fixture::Expo);
    let mut pids = vec![devices.daemon_pid()];
    for _ in 0..2 {
        let sample = adapter
            .measure(&emulator(), &scenario, Primitive::ColdAttach)
            .unwrap();
        assert_eq!(sample.clock, Clock::Wall);
        assert!(
            sample
                .detail
                .as_deref()
                .is_some_and(|d| d.starts_with("open settled in ")),
            "{sample:?}"
        );
        pids.push(devices.daemon_pid());
    }
    assert!(pids[0] != pids[1] && pids[1] != pids[2], "{pids:?}");
}

/// Task 10.10: the daemon keeps its agent, which holds an Android target's UI
/// automation, until it exits. A release returns once the daemon has let go
/// of its targets, which its `shutdown` answers only after (task 8.16), even
/// when the target takes 300 ms to stop, and keeps what `status` said about
/// the target for the report: the fake emulator gives frames and its driver
/// reports no controller.
#[test]
fn a_release_stops_the_daemon_and_returns_once_it_has_let_go_of_its_targets() {
    let devices = FakeDevices::new("android").stopping(300);
    let target = emulator();
    let mut adapter = CliAdapter::new(devices.config(&common::scratch("cli-release")));
    adapter.detect();
    let subject = Subject::Primitive(Primitive::TapDispatch);
    let scenario = plan::scenario(Primitive::TapDispatch, Fixture::Expo);
    adapter.prepare(&target, &scenario, &subject).unwrap();
    let socket = devices.dir.join("daemon.sock");
    assert!(socket.exists());
    assert_eq!(
        adapter.release(&target, &common::probe()),
        Ok(Some("stopped the daemon".to_owned()))
    );
    assert!(
        !socket.exists(),
        "the release returned before the daemon let go of its targets"
    );
    let mut described = target.clone();
    adapter.describe(&mut described);
    assert_eq!(
        (described.controller, described.frame_source),
        (None, Some(true))
    );
}

#[test]
fn a_first_snapshot_follows_a_relaunch_on_the_big_screen_and_counts_its_refs() {
    let devices = FakeDevices::new("android");
    let target = emulator();
    let mut adapter = CliAdapter::new(devices.config(&common::scratch("cli-first-snapshot")));
    adapter.detect();
    devices.cli(&in_session(
        &target,
        &["open", "dev.mobileagent.fixture.expo"],
    ));
    devices.cli(&in_session(&target, &["click", "id=tab-list"]));
    let sample = adapter
        .measure(
            &target,
            &plan::scenario(Primitive::FirstSnapshot, Fixture::Expo),
            Primitive::FirstSnapshot,
        )
        .unwrap();
    assert_eq!((sample.clock, sample.nodes), (Clock::Wall, Some(505)));
    let count = devices.cli(&in_session(&target, &["get", "text", "id=big-count"]));
    assert_eq!(count["text"], "500 elements");
}

#[test]
fn text_entry_fills_the_fixed_string_into_the_username_field() {
    let devices = FakeDevices::new("ios");
    let target = simulator();
    let mut adapter = CliAdapter::new(devices.config(&common::scratch("cli-text-entry")));
    adapter.detect();
    let scenario = plan::scenario(Primitive::TextEntry, Fixture::Expo);
    let subject = Subject::Primitive(Primitive::TextEntry);
    adapter.prepare(&target, &scenario, &subject).unwrap();
    let sample = adapter
        .measure(&target, &scenario, Primitive::TextEntry)
        .unwrap();
    assert_eq!(sample.clock, Clock::Wall);
    // Both fills read the field back, the one that empties it too.
    let detail = sample.detail.unwrap_or_default();
    let (filled, cleared) = detail.split_once("; cleared: ").unwrap_or_default();
    for note in [filled, cleared] {
        assert!(
            note.starts_with("method direct, read back as text in ") && note.ends_with(" ms"),
            "{detail}"
        );
    }
    // The field is empty again for the next fill.
    let value = devices.cli(&in_session(&target, &["get", "value", "id=login-username"]));
    assert!(value["value"].is_null() || value["value"] == "", "{value}");
}

#[test]
fn a_text_entry_that_did_not_land_fails() {
    let dir = common::scratch("cli-stand-in-text");
    let mut settings = CliConfig::new(dir.join("work"));
    settings.program = stand_in(
        &dir,
        &[
            (
                "fill",
                r#"{"success":true,"data":{"filled":"id=login-username","method":"keys","ref":"@e2"},"error":null}"#,
                0,
            ),
            (
                "value",
                r#"{"success":true,"data":{"value":"abcdefghijklm"},"error":null}"#,
                0,
            ),
        ],
    );
    let mut adapter = CliAdapter::new(settings);
    adapter.detect();
    let error = adapter
        .measure(
            &simulator(),
            &plan::scenario(Primitive::TextEntry, Fixture::Expo),
            Primitive::TextEntry,
        )
        .unwrap_err();
    assert_eq!(
        error.message,
        "login-username holds \"abcdefghijklm\" after the fill, not the fixed string"
    );
    let log = fs::read_to_string(dir.join("mobile-agent.log")).unwrap();
    let session = "--json --session mobile-agent-bench --device FAKE-IOS-1";
    for command in [
        format!("fill --timeout 10000 -- id=login-username {TEXT_ENTRY}"),
        format!("wait --timeout 10000 --text={TEXT_ENTRY}"),
        "get value --timeout 10000 -- id=login-username".to_owned(),
    ] {
        let line = format!("{session} {command}");
        assert!(log.lines().any(|l| l == line), "no {line:?} in:\n{log}");
    }
}

/// Task 10.26: a text entry's sample says how `fill` read the field back
/// (task 10.23), for the timed fill and for the one that empties the field:
/// the method, whether it entered the text again, what the read-back found
/// and after how long, and the reply's warning.
#[test]
fn a_text_entry_notes_how_each_fill_read_the_field_back() {
    let dir = common::scratch("cli-stand-in-read-back");
    let mut settings = CliConfig::new(dir.join("work"));
    let retried = json!({
        "success": true,
        "data": {"filled": "id=login-username", "method": "direct", "ref": "@e2",
                 "verified": "text", "readBackMs": 301.25, "retried": true},
        "error": null,
        "warning": "id=login-username showed \"abc\" after fill typed the text, so fill pasted it instead"
    })
    .to_string();
    let value = json!({"success": true, "data": {"value": TEXT_ENTRY}, "error": null}).to_string();
    let cleared = json!({
        "success": true,
        "data": {"filled": "id=login-username", "method": "direct", "ref": "@e2",
                 "verified": "text", "readBackMs": 120.5, "retried": false},
        "error": null
    })
    .to_string();
    settings.program = stand_in(
        &dir,
        &[
            (TEXT_ENTRY, retried.as_str(), 0),
            ("value", value.as_str(), 0),
            ("id=login-username", cleared.as_str(), 0),
        ],
    );
    let mut adapter = CliAdapter::new(settings);
    adapter.detect();
    let sample = adapter
        .measure(
            &simulator(),
            &plan::scenario(Primitive::TextEntry, Fixture::Expo),
            Primitive::TextEntry,
        )
        .unwrap();
    assert_eq!(
        sample.detail.as_deref(),
        Some(
            "method direct, retried, read back as text in 301.25 ms, warning: id=login-username \
             showed \"abc\" after fill typed the text, so fill pasted it instead; cleared: method \
             direct, read back as text in 120.5 ms"
        )
    );
}

/// Task 10.26: a flow's sample says how each of its fills read its field
/// back, a field left unread with the reply's warning.
#[test]
fn the_login_flow_notes_how_each_fill_read_its_field_back() {
    let dir = common::scratch("cli-stand-in-login-read-back");
    let mut settings = CliConfig::new(dir.join("work"));
    let username = json!({
        "success": true,
        "data": {"filled": "id=login-username", "method": "direct", "ref": "@e2",
                 "verified": "text", "readBackMs": 251.5, "retried": false},
        "error": null
    })
    .to_string();
    let password = json!({
        "success": true,
        "data": {"filled": "id=login-password", "method": "direct", "ref": "@e3",
                 "verified": null, "readBackMs": null, "retried": false},
        "error": null,
        "warning": "id=login-password left the screen after fill set the text directly, so it could not be read back"
    })
    .to_string();
    let status =
        json!({"success": true, "data": {"text": "Welcome, agent"}, "error": null}).to_string();
    settings.program = stand_in(
        &dir,
        &[
            ("id=login-username", username.as_str(), 0),
            ("id=login-password", password.as_str(), 0),
            ("text", status.as_str(), 0),
        ],
    );
    let mut adapter = CliAdapter::new(settings);
    adapter.detect();
    let sample = adapter
        .run_flow(&emulator(), &flow::login(Fixture::Expo))
        .unwrap();
    assert_eq!(
        sample.detail.as_deref(),
        Some(
            "fill id=login-username: method direct, read back as text in 251.5 ms; fill \
             id=login-password: method direct, not read back, warning: id=login-password left \
             the screen after fill set the text directly, so it could not be read back"
        )
    );
}

#[test]
fn an_absent_mobile_agent_is_skipped_with_a_note() {
    let work = common::scratch("cli-absent");
    let mut settings = CliConfig::new(&work);
    settings.search_path = vec![work.clone()];
    assert_eq!(
        CliAdapter::new(settings.clone()).detect(),
        Detection::Absent {
            note: "mobile-agent not found on PATH".into()
        }
    );
    settings.program = work.join("nowhere").join("mobile-agent");
    assert_eq!(
        CliAdapter::new(settings.clone()).detect(),
        Detection::Absent {
            note: format!("{} is not an executable file", settings.program.display())
        }
    );
}

#[test]
fn the_cli_adapter_measures_each_primitive_in_the_scenarios_it_knows() {
    let adapter = CliAdapter::new(CliConfig::new("/nonexistent"));
    let target = emulator();
    for fixture in plan::fixtures(Platform::Android) {
        for primitive in Primitive::ALL {
            let scenario = plan::scenario(primitive, fixture);
            assert!(
                adapter.supports(&target, &scenario, &Subject::Primitive(primitive)),
                "{primitive} on {fixture}"
            );
        }
    }
    let on = |screen| Scenario {
        fixture: Some(Fixture::Expo),
        screen,
    };
    let supports = |scenario: Scenario, primitive| {
        adapter.supports(&target, &scenario, &Subject::Primitive(primitive))
    };
    assert!(!supports(on(Some(Screen::Big)), Primitive::TapDispatch));
    assert!(!supports(on(Some(Screen::Big)), Primitive::Settle));
    assert!(!supports(on(Some(Screen::List)), Primitive::TextEntry));
    assert!(!supports(on(None), Primitive::WarmSnapshot));
    assert!(!supports(Scenario::default(), Primitive::ColdAttach));
    assert!(supports(on(Some(Screen::List)), Primitive::FirstSnapshot));
    assert!(adapter.supports(
        &target,
        &Scenario::default(),
        &Subject::Flow("login".into())
    ));
}
