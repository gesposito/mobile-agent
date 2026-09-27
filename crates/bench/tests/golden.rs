//! Task 9.4: golden end states. Each reference flow names what its final
//! screen shows when the flow did its work; every adapter reads the final
//! screen through its own tool and reports the same summary, and a run times a
//! flow through a tool only once that tool's reading matches the golden. The
//! tool under test runs as the scripted fake and as the CLI adapter on the
//! daemon's fake targets; the baselines run as fake tools that print their
//! tools' own hierarchy shapes.

mod common;

use bench::adapter::{Adapter, Detection, Role};
use bench::baseline::{BaselineTool, baselines};
use bench::cli::CliAdapter;
use bench::end_state::EndState;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow::{self, Flow, Step};
use bench::harness::{Harness, RunConfig, Subject};
use bench::report::{Measurement, Report};
use bench::target::{Platform, Target, TargetClass};
use common::FakeDevices;
use common::tools::{Tools, end_screen};
use serde_json::{Value, json};

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

/// Task 10.10: a tool whose golden check failed does not time the flow, so
/// it does not drive the target for that series and is not handed it: on
/// the emulator, agent-device's failed list-scroll checks cost a hand-off to
/// it and a release of a session it no longer had, each time.
#[test]
fn a_tool_that_does_not_time_a_flow_is_not_handed_the_target_for_it() {
    let tools = Tools::new("golden-no-hand-off", Platform::Android, |flow| {
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
    let report = run(
        fake_under_test(&emulator(), None),
        Some(&tools),
        RunConfig {
            iterations: ITERATIONS,
            warmup: WARMUP,
            retries: 0,
        },
    );
    // Every tool checks every flow; then login is timed through the tool
    // under test alone, the other flows through every tool.
    let baselines = ["maestro", "maestro-runner", "agent-device"];
    let mut order = Vec::new();
    for _ in 0..3 {
        order.push("mobile-agent");
        order.extend(baselines);
    }
    order.push("mobile-agent");
    for _ in 0..2 {
        order.push("mobile-agent");
        order.extend(baselines);
    }
    let mut expected: Vec<(&str, Option<&str>)> = order
        .windows(2)
        .filter(|pair| pair[0] != pair[1])
        .map(|pair| (pair[0], Some(pair[1])))
        .collect();
    expected.push((order[order.len() - 1], None));
    let handed: Vec<(&str, Option<&str>)> = report
        .handoffs
        .iter()
        .map(|h| (h.from.as_str(), h.to.as_deref()))
        .collect();
    assert_eq!(handed, expected);
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

#[test]
fn the_large_screen_records_the_node_count_each_tool_returned() {
    let tools = Tools::new("golden-nodes", Platform::Android, end_screen);
    let report = run(
        fake_under_test(&emulator(), None),
        Some(&tools),
        RunConfig {
            iterations: ITERATIONS,
            warmup: WARMUP,
            retries: 0,
        },
    );
    // 507 elements on the 500-cell screen: the count, 500 cells, 5 tabs and
    // the clock. Maestro adds its root and the window, maestro-runner its
    // root, agent-device the application and the window.
    assert_eq!(end_screen("big-screen").len(), 507);
    for (tool, nodes) in [
        ("mobile-agent", 546),
        ("maestro", 509),
        ("maestro-runner", 508),
        ("agent-device", 509),
    ] {
        let big = find(&report, tool, "big-screen");
        assert_eq!(
            big.nodes.map(|n| (n.min, n.max)),
            Some((nodes, nodes)),
            "{tool}: {:?}",
            big.samples
        );
        assert!(
            big.samples.iter().all(|s| s.nodes == Some(nodes)),
            "{tool}: {:?}",
            big.samples
        );
        for flow in ["login", "list-scroll"] {
            assert_eq!(find(&report, tool, flow).nodes, None, "{tool} {flow}");
        }
    }
    // The count is read after each timed run of the flow, outside its
    // timing, as the check reads the screen.
    let runs = (1 + WARMUP + ITERATIONS) as usize;
    for tool in ["maestro", "maestro-runner", "agent-device"] {
        let log = tools.log(tool);
        let big_runs: Vec<usize> = log
            .iter()
            .enumerate()
            .filter(|(_, l)| *l == "run big-screen")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(big_runs.len(), runs, "{tool}: {log:?}");
        for i in big_runs {
            assert_eq!(
                log.get(i + 1).map(String::as_str),
                Some("read"),
                "{tool}: {log:?}"
            );
        }
    }
    let md = report.to_markdown();
    let table = common::table(&md, "Measurements");
    for (tool, nodes) in [
        ("maestro", "509"),
        ("maestro-runner", "508"),
        ("agent-device", "509"),
    ] {
        let row = table
            .rows
            .iter()
            .find(|r| {
                table.cell(r, "Tool") == tool && table.cell(r, "Subject") == "flow big-screen"
            })
            .unwrap();
        assert_eq!(table.cell(row, "Nodes"), nodes);
    }
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &json);
}
