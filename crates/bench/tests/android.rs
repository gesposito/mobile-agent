//! Task 10.10: the harness measures an Android emulator as it is, with nothing
//! between the bench and `mobile-agent`. It opens the Android fixtures'
//! screens through the launch option they read, records whether the
//! emulator's controller answered, and lists tap to frame change as not
//! measured on a target that gives no frames. The tests drive the CLI adapter
//! against canned replies, and the scripted fake as the tool under test.

mod common;

use std::fs;
use std::path::Path;

use bench::adapter::{Adapter, Detection};
use bench::cli::{CliAdapter, CliConfig};
use bench::fake::FakeAdapter;
use bench::fixture::Fixture;
use bench::gates::{GateSet, GateStatus};
use bench::harness::{Harness, RunConfig, Series, Subject};
use bench::plan;
use bench::primitive::Primitive;
use bench::report::Report;
use bench::target::{Target, TargetClass};
use common::stand_in;
use serde_json::{Value, json};

fn emulator() -> Target {
    Target::new("emulator-5554", TargetClass::AndroidEmulator)
}

/// The Android fixtures read the launch intent's string extra `initial_tab`,
/// the iOS ones the environment variable `FIXTURE_INITIAL_TAB`
/// (`fixtures/CONTRACT.md`, Launch options), and `open --env` passes either,
/// as an extra on Android: each target's screens open through its own.
#[test]
fn a_screen_opens_through_the_launch_option_its_platform_s_fixtures_read() {
    for (target, option) in [
        (emulator(), "initial_tab=list"),
        (
            Target::new("FAKE-IOS-1", TargetClass::Simulator),
            "FIXTURE_INITIAL_TAB=list",
        ),
    ] {
        let dir = common::scratch(&format!("android-launch-option-{}", target.id));
        let mut settings = CliConfig::new(dir.join("work"));
        settings.program = stand_in(&dir, &[]);
        let mut adapter = CliAdapter::new(settings);
        assert!(matches!(adapter.detect(), Detection::Available { .. }));
        let subject = Subject::Primitive(Primitive::TapDispatch);
        let scenario = plan::scenario(Primitive::TapDispatch, Fixture::Expo);
        adapter.prepare(&target, &scenario, &subject).unwrap();
        let log = fs::read_to_string(dir.join("mobile-agent.log")).unwrap();
        let open = format!(
            "--json --session mobile-agent-bench --device {} open --relaunch --env {option} -- \
             dev.mobileagent.fixture.expo",
            target.id
        );
        assert!(
            log.lines().any(|line| line == open),
            "no {open:?} in:\n{log}"
        );
    }
}

/// The warning the daemon gives for an emulator started without `-grpc`.
const NO_CONTROLLER: &str = "emulator control endpoint unavailable for emulator-5554 (no \
    discovery file names console port 5554): taps and frames fall back to the agent. Start the \
    emulator with `-grpc 8554 -grpc-use-token` to use the fast path, or enable it without a \
    restart through the emulator console command `grpc 8554`, for example `adb -s emulator-5554 \
    emu grpc 8554`.";

/// A `status` reply for a daemon serving from `socket` with the emulator
/// attached, as the daemon's Android driver reports it: with the controller
/// when `controller` says so, taps and frames through it; without it, taps
/// through the agent and no frame stream.
fn status(socket: &Path, controller: bool) -> String {
    let driver = if controller {
        json!({"agent": "mobile-agent-android 0.2.0", "input": "emulator controller",
               "frames": "emulator controller", "screenshots": "agent", "keys": "agent",
               "endpoint": "http://127.0.0.1:8554", "keyboard": false, "warning": null,
               "framesReceived": 67, "controllerError": null, "frameError": null})
    } else {
        json!({"agent": "mobile-agent-android 0.2.0", "input": "agent", "frames": null,
               "screenshots": "agent", "keys": "agent", "endpoint": null, "keyboard": null,
               "warning": NO_CONTROLLER, "framesReceived": 0, "controllerError": null,
               "frameError": null})
    };
    json!({"success": true, "error": null, "data": {
        "daemon": {"pid": 4242, "version": "0.2.0", "socket": socket},
        "targets": [{"id": "emulator-5554", "platform": "android", "class": "android-emulator",
                     "name": "Pixel 9", "state": "device", "attached": true,
                     "frameSource": controller, "app": "dev.mobileagent.fixture.expo",
                     "claimedBy": "mobile-agent-bench", "driver": driver}],
        "sessions": []
    }})
    .to_string()
}

/// The tap primitives on the emulator's List screen, run through the CLI
/// adapter against canned replies: `status` as [`status`] gives it and each
/// click answered with `click`.
fn tap_run(name: &str, controller: bool, click: &str) -> Report {
    let dir = common::scratch(name);
    let reply = status(&dir.join("daemon.sock"), controller);
    let mut settings = CliConfig::new(dir.join("work"));
    settings.program = stand_in(&dir, &[("status", reply.as_str(), 0), ("click", click, 0)]);
    let series: Vec<Series> = plan::series(&emulator(), &[Fixture::Expo])
        .into_iter()
        .filter(|s| {
            matches!(
                s.subject,
                Subject::Primitive(
                    Primitive::TapDispatch
                        | Primitive::TapToFrameChange
                        | Primitive::TapToTreeDelta
                )
            )
        })
        .collect();
    Harness::new(
        RunConfig {
            iterations: 2,
            warmup: 0,
            retries: 0,
        },
        vec![emulator()],
        series,
    )
    .tool(Box::new(CliAdapter::new(settings)))
    .probe(Box::new(common::probe()), "macos")
    .run()
    .unwrap()
}

fn primitive<'a>(json: &'a Value, primitive: &str) -> &'a Value {
    json["measurements"]
        .as_array()
        .and_then(|all| all.iter().find(|m| m["primitive"] == primitive))
        .unwrap_or_else(|| panic!("no {primitive} in {json}"))
}

/// The emulator's row of the markdown's targets table: its controller and
/// frame source cells.
fn targets_row(md: &str) -> (String, String) {
    let table = common::table(md, "Targets");
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Target") == "emulator-5554")
        .unwrap_or_else(|| panic!("no emulator-5554 row in {md}"));
    (
        table.cell(row, "Emulator controller").to_owned(),
        table.cell(row, "Frame source").to_owned(),
    )
}

/// With the controller off the daemon has no frame stream, so a click's reply
/// carries a tree delta and no frame change: tap to frame change cannot be
/// measured there, and is listed as not measured with the daemon's reason,
/// its gate too, rather than failed.
#[test]
fn a_tap_on_an_emulator_whose_controller_is_off_has_no_frame_change_and_is_not_measured() {
    let report = tap_run(
        "android-controller-off",
        false,
        r#"{"success":true,"data":{"accepted":true,"clicked":"id=row-1","dispatchMs":2.25,"ref":null,"treeDeltaMs":226.5,"watchedMs":430.5,"x":204.0,"y":120.0},"error":null}"#,
    );
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    let frame = primitive(&json, "tap-to-frame-change");
    let reason = frame["not_measured"]
        .as_str()
        .unwrap_or_else(|| panic!("tap-to-frame-change was not listed as not measured: {frame}"));
    assert!(
        reason.starts_with(
            "emulator-5554 gives mobile-agent no frames, so a tap on it has no frame change to time"
        ) && reason.contains("-grpc 8554"),
        "{reason}"
    );
    assert_eq!(frame["error"], Value::Null, "{frame}");
    assert_eq!(frame["reliability"]["failures"], 0, "{frame}");
    assert_eq!(frame["samples"], json!([]), "{frame}");
    for (name, us) in [("tap-dispatch", 2_250), ("tap-to-tree-delta", 226_500)] {
        let m = primitive(&json, name);
        assert_eq!(m["stats"]["n"], 2, "{name}: {m}");
        assert_eq!(m["stats"]["p50_us"], us, "{name}: {m}");
    }

    // The report says the controller was off and the target gave no frames.
    let target = &json["provenance"]["targets"][0];
    assert_eq!(
        (&target["controller"], &target["frame_source"]),
        (&json!("off"), &json!(false)),
        "{target}"
    );
    let md = report.to_markdown();
    assert_eq!(targets_row(&md), ("off".to_owned(), "no".to_owned()));
    assert!(
        md.contains(&format!(
            "- mobile-agent on emulator-5554, tap-to-frame-change: {reason}"
        )),
        "{md}"
    );
    common::assert_every_markdown_number_is_in_the_json(&md, &json);

    // The spec's tap-to-frame gate reads not measured and does not fail the run.
    let verdict = GateSet::defaults().evaluate(&report);
    let gate = verdict
        .rows
        .iter()
        .find(|row| row.gate == "android-emulator tap-to-frame-change")
        .unwrap();
    assert_eq!(gate.status, GateStatus::NotMeasured, "{}", verdict.table());
    assert!(verdict.passed, "{}", verdict.table());
}

/// With the controller on, the same click carries its frame change, which is
/// measured, and the report says the controller answered.
#[test]
fn the_report_records_that_the_emulator_s_controller_answered() {
    let report = tap_run(
        "android-controller-on",
        true,
        r#"{"success":true,"data":{"accepted":true,"clicked":"id=row-1","dispatchMs":2.25,"frameChangeMs":31.75,"ref":null,"treeDeltaMs":226.5,"watchedMs":430.5,"x":204.0,"y":120.0},"error":null}"#,
    );
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    let frame = primitive(&json, "tap-to-frame-change");
    assert_eq!(frame["not_measured"], Value::Null, "{frame}");
    assert_eq!(frame["stats"]["p50_us"], 31_750, "{frame}");
    let target = &json["provenance"]["targets"][0];
    assert_eq!(
        (&target["controller"], &target["frame_source"]),
        (&json!("on"), &json!(true)),
        "{target}"
    );
    assert_eq!(
        targets_row(&report.to_markdown()),
        ("on".to_owned(), "yes".to_owned())
    );
}

/// A run of the scripted fake on an emulator the script describes as
/// `emulator`, a target in the report's JSON shape.
fn fake_run(emulator: Value) -> Report {
    let script = json!({
        "tool": "mobile-agent",
        "version": "0.2.0",
        "targets": [emulator],
        "series": [{"target": "emulator-5554", "fixture": "compose", "screen": "list",
                    "primitive": "tap-to-frame-change", "ms": [31.75, 45.5]}]
    });
    let fake = FakeAdapter::from_json(&script.to_string()).unwrap();
    Harness::new(RunConfig::default(), fake.targets(), fake.series())
        .tool(Box::new(fake))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap()
}

/// Two runs on one emulator, one with its controller on and one with it off,
/// are two environments: the controller changes the tap-to-frame-change gate's
/// span, so the provenance tells them apart, as it tells a target with frames
/// from one without.
#[test]
fn provenance_tells_a_run_with_the_emulator_s_controller_from_one_without() {
    let target = |controller: &str, frames: bool| {
        json!({"id": "emulator-5554", "class": "android-emulator",
               "controller": controller, "frame_source": frames})
    };
    let on = fake_run(target("on", true));
    assert_eq!(
        on.provenance
            .environment_differences(&fake_run(target("on", true)).provenance),
        Vec::<String>::new()
    );
    assert_eq!(
        on.provenance
            .environment_differences(&fake_run(target("off", true)).provenance),
        ["targets[emulator-5554].controller"]
    );
    assert_eq!(
        on.provenance
            .environment_differences(&fake_run(target("off", false)).provenance),
        [
            "targets[emulator-5554].controller",
            "targets[emulator-5554].frame_source"
        ]
    );
    let md = on.to_markdown();
    assert_eq!(targets_row(&md), ("on".to_owned(), "yes".to_owned()));
    let back = Report::from_json(&on.to_json()).unwrap();
    assert_eq!(back, on);
}
