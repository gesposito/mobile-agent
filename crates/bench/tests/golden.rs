//! Task 9.4: golden end states. Each reference flow names what its final
//! screen shows when the flow did its work; every adapter reads the final
//! screen through its own tool and reports the same summary, and a run times a
//! flow through a tool only once that tool's reading matches the golden. The
//! tool under test runs as the scripted fake and as the CLI adapter on the
//! daemon's fake targets; the baselines run as fake tools that print their
//! tools' own hierarchy shapes.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use bench::adapter::{Adapter, Detection, Role};
use bench::baseline::{BaselineConfig, BaselineTool, baselines};
use bench::cli::CliAdapter;
use bench::end_state::EndState;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow::{self, Flow, Step};
use bench::harness::{Harness, RunConfig, Subject};
use bench::report::{Measurement, Report};
use bench::target::{Platform, Target, TargetClass};
use common::FakeDevices;
use serde_json::{Value, json};

/// An element as every tool reads it: its identifier and the text it shows.
type Shown = (Option<String>, String);

/// What the Expo fixture shows at the end of the reference flow `name`.
fn end_screen(name: &str) -> Vec<Shown> {
    let mut screen: Vec<Shown> = Vec::new();
    let mut add = |id: &str, text: &str| screen.push((Some(id.to_owned()), text.to_owned()));
    match name {
        "login" => {
            add("login-title", "Login");
            add("login-username", "agent");
            add("login-password", "•••••••");
            add("login-submit", "Sign in");
            add("login-status", "Welcome, agent");
        }
        "list-scroll" => {
            add("detail-200", "Detail 200");
            add("detail-back", "Back");
        }
        "big-screen" => {
            add("big-count", "500 elements");
            for n in 1..=500 {
                add(&format!("cell-{n}"), &format!("Cell {n}"));
            }
        }
        _ => {}
    }
    for (id, label) in [
        ("tab-login", "Login"),
        ("tab-list", "List"),
        ("tab-big", "Big"),
        ("tab-anim", "Anim"),
        ("tab-system", "System"),
    ] {
        add(id, label);
    }
    screen.push((None, "12:30".to_owned()));
    screen
}

/// `maestro hierarchy` for `screen` on `platform`: the root, the app's
/// window, then a node per element with Maestro's attributes for the
/// platform: the text in `text` on Android, the label in `accessibilityText`
/// on iOS.
fn maestro_hierarchy(screen: &[Shown], platform: Platform) -> String {
    let node = |id: &str, text: &str| match platform {
        Platform::Android => json!({
            "attributes": {"text": text, "resource-id": id, "bounds": "[0,0][10,10]",
                           "class": "android.widget.TextView", "enabled": "true"},
            "children": [], "clickable": false, "enabled": true, "focused": false,
            "checked": false, "selected": false
        }),
        Platform::Ios => json!({
            "attributes": {"accessibilityText": text, "title": "", "value": "", "text": "",
                           "hintText": "", "resource-id": id, "bounds": "[0,0][10,10]",
                           "enabled": "true", "focused": "false", "selected": "false",
                           "checked": "false"},
            "children": [], "enabled": true
        }),
    };
    let elements: Vec<Value> = screen
        .iter()
        .map(|(id, text)| node(id.as_deref().unwrap_or(""), text))
        .collect();
    let root = json!({"attributes": {}, "children": [{
        "attributes": {"resource-id": "", "text": "", "bounds": "[0,0][1080,2400]"},
        "children": elements
    }]});
    serde_json::to_string_pretty(&root).unwrap()
}

/// `maestro-runner hierarchy` for `screen`: its normalized tree.
fn runner_hierarchy(screen: &[Shown]) -> String {
    let elements: Vec<Value> = screen
        .iter()
        .map(|(id, text)| {
            let mut node = json!({"type": "TextView", "text": text,
                                  "bounds": {"x": 0, "y": 0, "width": 10, "height": 10}});
            if let Some(id) = id {
                node["id"] = id.as_str().into();
            }
            node
        })
        .collect();
    let root = json!({"type": "Application", "text": "FixtureExpo", "children": elements});
    serde_json::to_string_pretty(&root).unwrap()
}

/// `agent-device snapshot --json` for `screen`: an application node, a
/// window, then a node per element, printed as agent-device prints JSON.
fn agent_device_snapshot(screen: &[Shown]) -> String {
    let mut nodes = vec![
        json!({"index": 0, "type": "Application", "label": "FixtureExpo", "depth": 0, "ref": "e1"}),
        json!({"index": 1, "type": "Window", "depth": 1, "parentIndex": 0, "ref": "e2"}),
    ];
    for (i, (id, text)) in screen.iter().enumerate() {
        let mut node = json!({"index": i + 2, "type": "StaticText", "label": text, "depth": 2,
                              "parentIndex": 1, "ref": format!("e{}", i + 3),
                              "rect": {"x": 0, "y": 0, "width": 10, "height": 10}});
        if let Some(id) = id {
            node["identifier"] = id.as_str().into();
        }
        nodes.push(node);
    }
    let reply = json!({"success": true, "data": {
        "nodes": nodes, "truncated": false,
        "appName": "FixtureExpo", "appBundleId": "dev.mobileagent.fixture.expo"
    }});
    serde_json::to_string_pretty(&reply).unwrap()
}

/// Fake Maestro, maestro-runner and agent-device in one directory. A flow
/// run keeps the end screen of the flow its file names; a hierarchy read
/// (`hierarchy`, or agent-device's `snapshot`) prints the screen kept, in the
/// tool's shape. Each tool logs its arguments to `<tool>.log`.
struct Tools {
    dir: PathBuf,
    bin: PathBuf,
}

impl Tools {
    /// The tools on `platform`, showing `screen(flow)` after each flow.
    fn new(name: &str, platform: Platform, screen: impl Fn(&str) -> Vec<Shown>) -> Self {
        let dir = common::scratch(name);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let tools = Tools { dir, bin };
        let flows = ["login", "list-scroll", "big-screen"];
        for flow in flows {
            let shown = screen(flow);
            tools.write(
                &format!("maestro-{flow}.out"),
                &maestro_hierarchy(&shown, platform),
            );
            tools.write(
                &format!("maestro-runner-{flow}.out"),
                &runner_hierarchy(&shown),
            );
            tools.write(
                &format!("agent-device-{flow}.out"),
                &agent_device_snapshot(&shown),
            );
        }
        tools.write(
            "agent-device-batch.out",
            r#"{"success":true,"data":{"total":3,"executed":3,"totalDurationMs":42,"results":[]}}"#,
        );
        let maestro_passes = "echo '[Passed] flow'";
        let batch_passes = "cat \"$dir/agent-device-batch.out\"";
        tools.tool("maestro", "2.10.0", "hierarchy", maestro_passes);
        tools.tool("maestro-runner", "1.1.0", "hierarchy", maestro_passes);
        tools.tool("agent-device", "0.21.14", "snapshot", batch_passes);
        tools
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.bin.join(name), contents).unwrap();
    }

    fn tool(&self, name: &str, version: &str, read: &str, passes: &str) {
        let mut runs = String::new();
        for flow in ["login", "list-scroll", "big-screen"] {
            runs.push_str(&format!(
                "*-{flow}-*) cp \"$dir/{name}-{flow}.out\" \"$dir/{name}-screen.out\"; {passes} ;;\n"
            ));
        }
        let path = self.bin.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 dir=\"${{0%/*}}\"\n\
                 if [ \"$1\" = \"--version\" ]; then echo '{version}'; exit 0; fi\n\
                 echo \"$@\" >> \"$dir/{name}.log\"\n\
                 case \" $* \" in\n\
                 *\" {read} \"*) cat \"$dir/{name}-screen.out\" ;;\n\
                 {runs}\
                 *) {passes} ;;\n\
                 esac\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn config(&self) -> BaselineConfig {
        BaselineConfig {
            search_path: vec![self.bin.clone()],
            work_dir: self.dir.join("work"),
            flow_timeout: Duration::from_secs(30),
            version_timeout: Duration::from_secs(10),
        }
    }

    /// Each line `tool` logged, reduced to what it did: `run <flow>` for a
    /// flow run, `read` for a hierarchy read.
    fn log(&self, tool: &str) -> Vec<String> {
        let log = fs::read_to_string(self.bin.join(format!("{tool}.log"))).unwrap_or_default();
        log.lines()
            .map(|line| {
                if line.contains(" hierarchy") || line.starts_with("snapshot ") {
                    "read".to_owned()
                } else {
                    let flow = ["login", "list-scroll", "big-screen", "tab"]
                        .into_iter()
                        .find(|flow| line.contains(&format!("-{flow}-")))
                        .unwrap_or("?");
                    format!("run {flow}")
                }
            })
            .collect()
    }
}

fn emulator() -> Target {
    Target::new("emulator-5554", TargetClass::AndroidEmulator)
}

fn simulator() -> Target {
    Target::new("FAKE-IOS-1", TargetClass::Simulator)
}

/// The fake tool under test on `target`, running the reference flows of Expo
/// in 300 ms, 546 nodes on the 500-cell screen; `end_state` scripts what its
/// run of `flow` reports.
fn fake_under_test(target: &Target, end_state: Option<(&str, EndState)>) -> FakeAdapter {
    let series = flow::reference(Fixture::Expo)
        .iter()
        .map(|flow| {
            let mut series = FakeSeries::new(
                &target.id,
                Some(Fixture::Expo),
                None,
                Subject::Flow(flow.name.clone()),
                vec![300.0],
            );
            series.nodes = flow.count_nodes.then_some(546);
            series.end_state = end_state
                .as_ref()
                .filter(|(name, _)| *name == flow.name)
                .map(|(_, state)| state.clone());
            series
        })
        .collect();
    FakeAdapter::new(FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![target.clone()],
        series,
        flows: flow::reference(Fixture::Expo),
    })
}

/// Every adapter on `target` of `platform` reads each reference flow's
/// golden: the fake tool under test, the CLI adapter on a fake target, and
/// the three baselines.
fn every_adapter_reads_the_golden(platform: Platform, fake: &str, target: Target) {
    let name = format!("golden-{}", platform.id());
    let tools = Tools::new(&name, platform, end_screen);
    let devices = FakeDevices::new(fake);
    let mut adapters: Vec<Box<dyn Adapter>> = vec![
        Box::new(fake_under_test(&target, None)),
        Box::new(CliAdapter::new(
            devices.config(&common::scratch(&format!("{name}-cli"))),
        )),
    ];
    adapters.extend(baselines(&tools.config()));
    let names: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
    assert_eq!(
        names,
        [
            "mobile-agent",
            "mobile-agent",
            "maestro",
            "maestro-runner",
            "agent-device"
        ]
    );
    for adapter in &mut adapters {
        assert!(
            matches!(adapter.detect(), Detection::Available { .. }),
            "{}",
            adapter.name()
        );
    }
    for flow in flow::reference(Fixture::Expo) {
        let states: Vec<EndState> = adapters
            .iter_mut()
            .map(|adapter| {
                adapter
                    .check_flow(&target, &flow)
                    .unwrap_or_else(|e| panic!("{} on {}: {e}", adapter.name(), flow.name))
            })
            .collect();
        for (adapter, state) in adapters.iter().zip(&states) {
            assert_eq!(
                *state,
                flow.golden,
                "{} on {} read {state}",
                adapter.name(),
                flow.name
            );
        }
    }
}

#[test]
fn every_adapter_reports_the_golden_end_state_of_each_flow_on_android() {
    every_adapter_reads_the_golden(Platform::Android, "android", emulator());
}

#[test]
fn every_adapter_reports_the_golden_end_state_of_each_flow_on_ios() {
    every_adapter_reads_the_golden(Platform::Ios, "ios", simulator());
}

fn run(fake: FakeAdapter, tools: Option<&Tools>, config: RunConfig) -> Report {
    let mut harness = Harness::new(config, fake.targets(), fake.series())
        .flows(fake.flows())
        .tool(Box::new(fake));
    if let Some(tools) = tools {
        for baseline in baselines(&tools.config()) {
            harness = harness.tool(baseline);
        }
    }
    harness
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap()
}

fn find<'a>(report: &'a Report, tool: &str, flow: &str) -> &'a Measurement {
    report
        .measurements
        .iter()
        .find(|m| m.tool == tool && m.subject == Subject::Flow(flow.into()))
        .unwrap_or_else(|| panic!("no {tool} measurement of {flow}"))
}

fn golden(name: &str) -> EndState {
    flow::reference(Fixture::Expo)
        .into_iter()
        .find(|f| f.name == name)
        .unwrap()
        .golden
}

const ITERATIONS: u32 = 2;
const WARMUP: u32 = 1;

#[test]
fn a_run_times_each_flow_only_after_every_tool_read_its_golden() {
    let tools = Tools::new("golden-run", Platform::Android, end_screen);
    let report = run(
        fake_under_test(&emulator(), None),
        Some(&tools),
        RunConfig {
            iterations: ITERATIONS,
            warmup: WARMUP,
            retries: 0,
        },
    );
    assert_eq!(report.measurements.len(), 12);
    for m in &report.measurements {
        let Subject::Flow(name) = &m.subject else {
            panic!("{m:?}")
        };
        assert_eq!(m.end_state.as_ref(), Some(&golden(name)), "{}", m.tool);
        assert_eq!(m.error, None, "{} {name}", m.tool);
        assert_eq!(m.stats.map(|s| s.n), Some(ITERATIONS), "{} {name}", m.tool);
    }
    // Each tool ran and read every flow once, before any flow was timed.
    let runs = (1 + WARMUP + ITERATIONS) as usize;
    for tool in ["maestro", "maestro-runner", "agent-device"] {
        let log = tools.log(tool);
        assert_eq!(
            log[..6],
            [
                "run login",
                "read",
                "run list-scroll",
                "read",
                "run big-screen",
                "read"
            ],
            "{tool}: {log:?}"
        );
        for flow in ["login", "list-scroll", "big-screen"] {
            let count = log.iter().filter(|l| **l == format!("run {flow}")).count();
            assert_eq!(count, runs, "{tool} ran {flow} {count} times: {log:?}");
        }
    }
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(
        json["measurements"][0]["end_state"],
        json!({"texts": {"login-status": "Welcome, agent"}})
    );
    let back = Report::from_json(&report.to_json()).unwrap();
    assert_eq!(back, report);
}

#[test]
fn a_tool_whose_end_state_differs_from_the_golden_is_not_timed() {
    let tools = Tools::new("golden-differs", Platform::Android, |flow| {
        let mut screen = end_screen(flow);
        if flow == "login" {
            for (id, text) in &mut screen {
                if id.as_deref() == Some("login-status") {
                    *text = "Invalid credentials".to_owned();
                }
            }
        }
        screen
    });
    let config = RunConfig {
        iterations: ITERATIONS,
        warmup: WARMUP,
        retries: 1,
    };
    let report = run(fake_under_test(&emulator(), None), Some(&tools), config);
    for tool in ["maestro", "maestro-runner", "agent-device"] {
        let login = find(&report, tool, "login");
        let error = login.error.as_deref().unwrap_or_default();
        assert!(
            error.starts_with("the end state differs from the golden")
                && error
                    .contains("login-status shows \"Invalid credentials\", not \"Welcome, agent\""),
            "{tool}: {error}"
        );
        assert_eq!(login.stats, None, "{tool}");
        assert!(login.samples.is_empty(), "{tool}");
        assert_eq!(login.reliability.failures, ITERATIONS, "{tool}");
        assert_eq!(
            login.end_state,
            Some(EndState::default().with_text("login-status", "Invalid credentials")),
            "{tool}"
        );
        // The check ran the flow once and once more on retry, and no timed
        // run followed; the other flows were checked and timed.
        let log = tools.log(tool);
        let logins = log.iter().filter(|l| *l == "run login").count();
        assert_eq!(logins, 2, "{tool}: {log:?}");
        let list = find(&report, tool, "list-scroll");
        assert_eq!(list.stats.map(|s| s.n), Some(ITERATIONS), "{tool}");
    }
    let under_test = find(&report, "mobile-agent", "login");
    assert_eq!(under_test.stats.map(|s| s.n), Some(ITERATIONS));
    let md = report.to_markdown();
    assert!(
        md.contains(
            "- maestro on emulator-5554, flow login: the end state differs from the golden"
        ),
        "{md}"
    );
    let comparison = common::table(&md, "Comparison");
    let row = comparison
        .rows
        .iter()
        .find(|r| comparison.cell(r, "Subject") == "flow login")
        .unwrap();
    assert_eq!(comparison.cell(row, "maestro"), "no samples");
}

#[test]
fn the_tool_under_test_is_held_to_the_golden_too() {
    let absent = EndState {
        present: [("detail-200".to_owned(), false)].into(),
        ..EndState::default()
    };
    let report = run(
        fake_under_test(&emulator(), Some(("list-scroll", absent))),
        None,
        RunConfig {
            iterations: ITERATIONS,
            warmup: WARMUP,
            retries: 0,
        },
    );
    let list = find(&report, "mobile-agent", "list-scroll");
    assert_eq!(
        list.error.as_deref(),
        Some("the end state differs from the golden: detail-200 is absent")
    );
    assert_eq!(list.stats, None);
    let login = find(&report, "mobile-agent", "login");
    assert_eq!(login.end_state, Some(golden("login")));
    assert_eq!(login.stats.map(|s| s.n), Some(ITERATIONS));
}

#[test]
fn a_flow_without_a_golden_is_timed_unchecked() {
    let tools = Tools::new("golden-none", Platform::Android, end_screen);
    let tab = Flow {
        name: "tab".into(),
        fixture: Fixture::Expo,
        steps: vec![Step::Launch],
        count_nodes: false,
        golden: EndState::default(),
    };
    let fake = FakeAdapter::new(FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![emulator()],
        series: vec![FakeSeries::new(
            "emulator-5554",
            Some(Fixture::Expo),
            None,
            Subject::Flow("tab".into()),
            vec![300.0],
        )],
        flows: vec![tab],
    });
    let report = run(
        fake,
        Some(&tools),
        RunConfig {
            iterations: ITERATIONS,
            warmup: 0,
            retries: 0,
        },
    );
    for m in &report.measurements {
        assert_eq!(m.end_state, None, "{}", m.tool);
        assert_eq!(m.stats.map(|s| s.n), Some(ITERATIONS), "{}", m.tool);
    }
    for tool in [
        BaselineTool::Maestro,
        BaselineTool::MaestroRunner,
        BaselineTool::AgentDevice,
    ] {
        let log = tools.log(tool.name());
        assert_eq!(log, ["run tab", "run tab"], "{}", tool.name());
    }
}
