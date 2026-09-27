//! Task 9.4: the benchmark-harness spec's flows, described once in the flow
//! type for every fixture: login, list scroll and the 500-node screen, each
//! with the golden end state it must reach. The baselines render them in their
//! tools' shapes, and a run carries the same flow on several fixtures.

mod common;

use bench::adapter::Role;
use bench::baseline::{agent_device, maestro};
use bench::end_state::EndState;
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow::{self, Direction, Flow, Selector, Step};
use bench::harness::{Harness, RunConfig, Series, Subject};
use bench::target::{Platform, Target, TargetClass};
use serde_json::{Value, json};

const FIXTURES: [Fixture; 3] = [Fixture::SwiftUi, Fixture::Expo, Fixture::Compose];

#[test]
fn the_login_flow_signs_in_and_ends_on_the_welcome_text() {
    for fixture in FIXTURES {
        assert_eq!(
            flow::login(fixture),
            Flow {
                name: "login".into(),
                fixture,
                steps: vec![
                    Step::Launch,
                    Step::Fill {
                        target: Selector::id("login-username"),
                        text: "agent".into(),
                    },
                    Step::Fill {
                        target: Selector::id("login-password"),
                        text: "hunter2".into(),
                    },
                    Step::Tap(Selector::id("login-submit")),
                    Step::AssertText {
                        id: "login-status".into(),
                        text: "Welcome, agent".into(),
                    },
                ],
                count_nodes: false,
                golden: EndState::default().with_text("login-status", "Welcome, agent"),
            }
        );
    }
}

#[test]
fn the_list_scroll_flow_scrolls_to_row_200_and_opens_its_detail() {
    for fixture in FIXTURES {
        assert_eq!(
            flow::list_scroll(fixture),
            Flow {
                name: "list-scroll".into(),
                fixture,
                steps: vec![
                    Step::Launch,
                    Step::Tap(Selector::id("tab-list")),
                    Step::ScrollTo {
                        target: Selector::id("row-200"),
                        direction: Direction::Down,
                    },
                    Step::Tap(Selector::id("row-200")),
                    Step::AssertText {
                        id: "detail-200".into(),
                        text: "Detail 200".into(),
                    },
                ],
                count_nodes: false,
                golden: EndState::default().with_present("detail-200"),
            }
        );
    }
}

#[test]
fn the_500_node_flow_opens_the_big_tab_and_records_its_node_count() {
    for fixture in FIXTURES {
        assert_eq!(
            flow::big_screen(fixture),
            Flow {
                name: "big-screen".into(),
                fixture,
                steps: vec![
                    Step::Launch,
                    Step::Tap(Selector::id("tab-big")),
                    Step::AssertVisible(Selector::id("cell-500")),
                ],
                count_nodes: true,
                golden: EndState::default().with_count("cell", 500),
            }
        );
    }
}

#[test]
fn the_reference_flows_come_in_report_order_and_differ_only_in_their_fixture() {
    let expo = flow::reference(Fixture::Expo);
    let names: Vec<&str> = expo.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, [flow::LOGIN, flow::LIST_SCROLL, flow::BIG_SCREEN]);
    assert_eq!(names, ["login", "list-scroll", "big-screen"]);
    for fixture in FIXTURES {
        let flows = flow::reference(fixture);
        assert_eq!(
            flows,
            [
                flow::login(fixture),
                flow::list_scroll(fixture),
                flow::big_screen(fixture)
            ]
        );
        for (flow, same) in flows.iter().zip(&expo) {
            assert_eq!(flow.fixture, fixture);
            assert_eq!(
                Flow {
                    fixture: Fixture::Expo,
                    ..flow.clone()
                },
                *same
            );
        }
    }
}

#[test]
fn a_flow_and_its_golden_read_and_write_their_documented_json() {
    let login = json!({
        "name": "login",
        "fixture": "compose",
        "steps": [
            "launch",
            {"fill": {"target": {"id": "login-username"}, "text": "agent"}},
            {"fill": {"target": {"id": "login-password"}, "text": "hunter2"}},
            {"tap": {"id": "login-submit"}},
            {"assert-text": {"id": "login-status", "text": "Welcome, agent"}}
        ],
        "count_nodes": false,
        "golden": {"texts": {"login-status": "Welcome, agent"}}
    });
    let flow: Flow = serde_json::from_value(login.clone()).unwrap();
    assert_eq!(flow, flow::login(Fixture::Compose));
    assert_eq!(serde_json::to_value(&flow).unwrap(), login);

    let goldens: Vec<Value> = flow::reference(Fixture::Expo)
        .iter()
        .map(|f| serde_json::to_value(f).unwrap()["golden"].clone())
        .collect();
    assert_eq!(
        goldens,
        [
            json!({"texts": {"login-status": "Welcome, agent"}}),
            json!({"present": {"detail-200": true}}),
            json!({"counts": {"cell": 500}}),
        ]
    );
    // A flow without a golden leaves the key out and reads back empty.
    let bare: Flow =
        serde_json::from_str(r#"{"name": "tab", "fixture": "expo", "steps": ["launch"]}"#).unwrap();
    assert!(bare.golden.is_empty());
    assert!(serde_json::to_value(&bare).unwrap().get("golden").is_none());
}

#[test]
fn maestro_runs_each_reference_flow_as_one_flow_file() {
    let app = "dev.mobileagent.fixture.expo";
    assert_eq!(
        maestro::flow_yaml(&flow::login(Fixture::Expo), app),
        r#"appId: dev.mobileagent.fixture.expo
---
- launchApp
- tapOn:
    id: "login-username"
- inputText: "agent"
- tapOn:
    id: "login-password"
- inputText: "hunter2"
- tapOn:
    id: "login-submit"
- assertVisible:
    id: "login-status"
    text: "Welcome, agent"
"#
    );
    assert_eq!(
        maestro::flow_yaml(&flow::list_scroll(Fixture::Expo), app),
        r#"appId: dev.mobileagent.fixture.expo
---
- launchApp
- tapOn:
    id: "tab-list"
- scrollUntilVisible:
    element:
      id: "row-200"
    direction: DOWN
- tapOn:
    id: "row-200"
- assertVisible:
    id: "detail-200"
    text: "Detail 200"
"#
    );
    assert_eq!(
        maestro::flow_yaml(&flow::big_screen(Fixture::Expo), app),
        r#"appId: dev.mobileagent.fixture.expo
---
- launchApp
- tapOn:
    id: "tab-big"
- assertVisible:
    id: "cell-500"
"#
    );
}

#[test]
fn agent_device_runs_each_reference_flow_as_one_batch() {
    let steps = |flow: &Flow| -> Value {
        let app = flow.fixture.app_id(Platform::Android).unwrap();
        serde_json::from_str(&agent_device::steps(flow, app, Platform::Android)).unwrap()
    };
    let by_id = |id: &str| json!({"kind": "selector", "selector": format!("id=\"{id}\"")});
    let open = json!({"command": "open", "input": {
        "app": "dev.mobileagent.fixture.compose", "platform": "android", "relaunch": true
    }});
    assert_eq!(
        steps(&flow::login(Fixture::Compose)),
        json!([
            open,
            {"command": "fill", "input": {"target": by_id("login-username"), "text": "agent"}},
            {"command": "fill", "input": {"target": by_id("login-password"), "text": "hunter2"}},
            {"command": "press", "input": {"target": by_id("login-submit")}},
            {"command": "wait", "input": {"kind": "text", "text": "Welcome, agent", "timeoutMs": 10000}},
            {"command": "is", "input": {"predicate": "text", "selector": "id=\"login-status\"", "value": "Welcome, agent"}}
        ])
    );
    assert_eq!(
        steps(&flow::list_scroll(Fixture::Compose)),
        json!([
            open,
            {"command": "press", "input": {"target": by_id("tab-list")}},
            {"command": "scroll", "input": {"direction": "down", "until": "id=\"row-200\""}},
            {"command": "press", "input": {"target": by_id("row-200")}},
            {"command": "wait", "input": {"kind": "text", "text": "Detail 200", "timeoutMs": 10000}},
            {"command": "is", "input": {"predicate": "text", "selector": "id=\"detail-200\"", "value": "Detail 200"}}
        ])
    );
    assert_eq!(
        steps(&flow::big_screen(Fixture::Compose)),
        json!([
            open,
            {"command": "press", "input": {"target": by_id("tab-big")}},
            {"command": "wait", "input": {"kind": "selector", "selector": "id=\"cell-500\"", "timeoutMs": 10000}}
        ])
    );
}

/// The fake tool under test on a Simulator and an emulator, with the
/// reference flows of every fixture, measuring `series` for 300 ms each.
fn fake(series: &[(&str, Fixture, &str)]) -> FakeAdapter {
    FakeAdapter::new(FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![
            Target::new("SIM-1", TargetClass::Simulator),
            Target::new("emulator-5554", TargetClass::AndroidEmulator),
        ],
        series: series
            .iter()
            .map(|(target, fixture, name)| {
                let mut series = FakeSeries::new(
                    target,
                    Some(*fixture),
                    None,
                    Subject::Flow((*name).into()),
                    vec![300.0],
                );
                series.nodes = (*name == flow::BIG_SCREEN).then_some(546);
                series
            })
            .collect(),
        flows: FIXTURES.into_iter().flat_map(flow::reference).collect(),
    })
}

fn run(fake: FakeAdapter) -> Result<bench::report::Report, String> {
    Harness::new(
        RunConfig {
            iterations: 3,
            warmup: 1,
            retries: 0,
        },
        fake.targets(),
        fake.series(),
    )
    .flows(fake.flows())
    .tool(Box::new(fake))
    .probe(Box::new(common::probe()), "macos")
    .run()
}

#[test]
fn a_run_measures_the_same_flow_on_several_fixtures() {
    let planned = [
        ("SIM-1", Fixture::SwiftUi, "login"),
        ("SIM-1", Fixture::Expo, "login"),
        ("emulator-5554", Fixture::Compose, "login"),
        ("emulator-5554", Fixture::Expo, "login"),
        ("emulator-5554", Fixture::Compose, "big-screen"),
    ];
    let report = run(fake(&planned)).unwrap();
    let measured: Vec<(&str, Option<Fixture>, String)> = report
        .measurements
        .iter()
        .map(|m| (m.target.as_str(), m.fixture, m.subject.to_string()))
        .collect();
    assert_eq!(
        measured,
        planned
            .iter()
            .map(|(target, fixture, name)| (*target, Some(*fixture), format!("flow {name}")))
            .collect::<Vec<_>>()
    );
    for m in &report.measurements {
        assert_eq!(m.error, None, "{m:?}");
        assert_eq!(m.stats.map(|s| s.n), Some(3), "{m:?}");
    }
    let big = report.measurements.last().unwrap();
    assert_eq!(big.nodes.map(|n| n.max), Some(546));
}

#[test]
fn a_run_refuses_two_flows_with_one_name_on_one_fixture() {
    let fake = fake(&[("emulator-5554", Fixture::Expo, "login")]);
    let mut flows = fake.flows();
    flows.push(flow::login(Fixture::Expo));
    let error = Harness::new(RunConfig::default(), fake.targets(), fake.series())
        .flows(flows)
        .tool(Box::new(fake))
        .probe(Box::new(common::probe()), "macos")
        .run()
        .unwrap_err();
    assert!(error.contains("login") && error.contains("expo"), "{error}");
}

#[test]
fn a_flow_series_must_name_a_fixture_the_flow_has() {
    let fake = fake(&[]);
    let series = |fixture| Series {
        target: "emulator-5554".into(),
        fixture: Some(fixture),
        screen: None,
        subject: Subject::Flow("login".into()),
    };
    let only_expo = vec![flow::login(Fixture::Expo)];
    let error = Harness::new(
        RunConfig::default(),
        fake.targets(),
        vec![series(Fixture::Compose)],
    )
    .flows(only_expo)
    .tool(Box::new(fake))
    .probe(Box::new(common::probe()), "macos")
    .run()
    .unwrap_err();
    assert!(error.contains("login") && error.contains("expo"), "{error}");
}
