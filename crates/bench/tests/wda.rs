//! Task 6.3: WebDriverAgent on a physical iPhone, through Appium's XCUITest
//! driver. Fakes stand in for everything: Appium's executable and server
//! (`common::appium`), the helper that starts WebDriverAgent through the
//! phone's test manager, `xcrun devicectl` for the phone's processes, and the
//! host the hand-off asks.

mod common;

use std::cell::{Cell, RefCell};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use bench::adapter::{Adapter, Detection, Role};
use bench::baseline::wda::{self, WebDriverAgent};
use bench::baseline::{BaselineConfig, baselines};
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow;
use bench::harness::{Harness, RunConfig, Subject};
use bench::primitive::Primitive;
use bench::provenance::{Probe, ScriptedProbe, ToolStatus};
use bench::report::Report;
use bench::target::{Target, TargetClass};
use common::appium::FakeAppium;
use serde_json::json;

const ITERATIONS: u32 = 2;

/// The URL the fake helper prints for WebDriverAgent.
const WDA_URL: &str = "http://127.0.0.1:59999";

/// The pid of WebDriverAgent's runner app on the phone.
const RUNNER_PID: u32 = 7920;

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A scratch directory with fake `appium`, a fake helper and a fake
/// `xcrun` whose phone runs `held` besides launchd.
struct Setup {
    dir: PathBuf,
    bin: PathBuf,
}

impl Setup {
    fn new(name: &str, held: &[(&str, u32)], with_driver: bool) -> Setup {
        let dir = common::scratch(name);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let driver = dir.join("driver");
        fs::create_dir_all(driver.join("node_modules/appium-webdriveragent")).unwrap();
        fs::write(
            driver.join("node_modules/appium-webdriveragent/package.json"),
            r#"{"name": "appium-webdriveragent", "version": "16.14.1"}"#,
        )
        .unwrap();
        let drivers = if with_driver {
            json!({"xcuitest": {"automationName": "XCUITest", "pkgName": "appium-xcuitest-driver",
                                "version": "12.16.0", "installType": "npm",
                                "installPath": driver, "installed": true}})
        } else {
            json!({})
        };
        fs::write(bin.join("drivers.json"), drivers.to_string()).unwrap();
        executable(
            &bin.join("appium"),
            "#!/bin/sh\n\
             dir=\"${0%/*}\"\n\
             case \"$*\" in\n\
             --version) echo 3.8.0 ;;\n\
             'driver list --installed --json') cat \"$dir/drivers.json\" ;;\n\
             *) echo \"$@\" >> \"$dir/appium.log\"\n\
                trap 'echo ended >> \"$dir/appium.log\"; exit 0' TERM\n\
                while :; do sleep 0.1; done ;;\n\
             esac\n",
        );
        executable(
            &bin.join("wda-helper"),
            &format!(
                "#!/bin/sh\n\
                 dir=\"${{0%/*}}\"\n\
                 echo \"$@\" >> \"$dir/wda-helper.log\"\n\
                 echo 'wda {WDA_URL}'\n\
                 cat > /dev/null\n\
                 echo ended >> \"$dir/wda-helper.log\"\n"
            ),
        );
        let mut processes =
            vec![json!({"executable": "file:///sbin/launchd", "processIdentifier": 1})];
        for (runner, pid) in held {
            processes.push(json!({
                "executable": format!("file:///private/var/containers/Bundle/Application/2/{runner}.app/{runner}"),
                "processIdentifier": pid
            }));
        }
        fs::write(
            bin.join("phone.json"),
            json!({"info": {"outcome": "success"}, "result": {"runningProcesses": processes}})
                .to_string(),
        )
        .unwrap();
        executable(
            &bin.join("xcrun"),
            "#!/bin/sh\n\
             echo \"$@\" >> \"${0%/*}/xcrun.log\"\n\
             case \"$*\" in\n\
             'devicectl device info processes '*) cat \"${0%/*}/phone.json\" ;;\n\
             *) exit 1 ;;\n\
             esac\n",
        );
        Setup { dir, bin }
    }

    fn config(&self, port: u16) -> BaselineConfig {
        BaselineConfig {
            search_path: vec![self.bin.clone()],
            work_dir: self.dir.join("work"),
            flow_timeout: Duration::from_secs(30),
            version_timeout: Duration::from_secs(10),
            targets: vec![TargetClass::IosDevice],
            ios_signing: None,
            wda_helper: vec![self.bin.join("wda-helper").into()],
            appium_port: port,
        }
    }

    fn log(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.bin.join(format!("{name}.log")))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// The host the hand-off and the release ask: nothing of WebDriverAgent's
/// left on it, and the phone `PHONE-1`, where WebDriverAgent's runner app
/// runs once the helper has started it, until CoreDevice terminates it.
struct Host {
    bin: PathBuf,
    terminated: Cell<bool>,
    events: Rc<RefCell<Vec<String>>>,
    scripted: ScriptedProbe,
}

impl Probe for Host {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        match (program, args) {
            ("ps", ["-A", "-o", "pid=,comm="]) => Some("    1 /sbin/launchd".to_owned()),
            (
                "xcrun",
                [
                    "devicectl",
                    "device",
                    "info",
                    "processes",
                    "--device",
                    "PHONE-1",
                    ..,
                ],
            ) => {
                let started = self.bin.join("wda-helper.log").exists();
                let runs = started && !self.terminated.get();
                self.events.borrow_mut().push(if runs {
                    "the iPhone runs WebDriverAgentRunner-Runner".to_owned()
                } else {
                    "the iPhone runs no runner".to_owned()
                });
                let mut processes =
                    vec![json!({"executable": "file:///sbin/launchd", "processIdentifier": 1})];
                if runs {
                    processes.push(json!({
                        "executable": "file:///private/var/containers/Bundle/Application/3/\
                                       WebDriverAgentRunner-Runner.app/WebDriverAgentRunner-Runner",
                        "processIdentifier": RUNNER_PID
                    }));
                }
                Some(
                    json!({"info": {"outcome": "success"}, "result": {"runningProcesses": processes}})
                        .to_string(),
                )
            }
            (
                "xcrun",
                [
                    "devicectl",
                    "device",
                    "process",
                    "terminate",
                    "--device",
                    "PHONE-1",
                    "--pid",
                    pid,
                    "--json-output",
                    "-",
                ],
            ) if *pid == RUNNER_PID.to_string() => {
                self.terminated.set(true);
                self.events
                    .borrow_mut()
                    .push(format!("CoreDevice terminates {pid}"));
                Some(r#"{"info": {"outcome": "success"}}"#.to_owned())
            }
            _ => self.scripted.output(program, args),
        }
    }
}

fn phone() -> Target {
    Target::new("PHONE-1", TargetClass::IosDevice)
}

/// A run on `PHONE-1`: the fake tool under test and the three reference
/// flows on the SwiftUI fixture, then CLI process start, with the baselines
/// `setup` installs and the fake Appium `appium` serving.
fn run(setup: &Setup, appium: &FakeAppium) -> (Report, Vec<String>) {
    let target = phone();
    let flows = flow::reference(Fixture::SwiftUi);
    let mut series: Vec<FakeSeries> = flows
        .iter()
        .map(|f| {
            FakeSeries::new(
                &target.id,
                Some(Fixture::SwiftUi),
                None,
                Subject::Flow(f.name.clone()),
                vec![900.0],
            )
        })
        .collect();
    series.push(FakeSeries::new(
        &target.id,
        None,
        None,
        Subject::Primitive(Primitive::CliStart),
        vec![4.0],
    ));
    let fake = FakeAdapter::new(FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![target],
        series,
        flows,
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut harness = Harness::new(
        RunConfig {
            iterations: ITERATIONS,
            warmup: 0,
            retries: 0,
        },
        fake.targets(),
        fake.series(),
    )
    .flows(fake.flows())
    .tool(Box::new(fake));
    for baseline in baselines(&setup.config(appium.port)) {
        harness = harness.tool(baseline);
    }
    let host = Host {
        bin: setup.bin.clone(),
        terminated: Cell::new(false),
        events: Rc::clone(&events),
        scripted: common::probe(),
    };
    let report = harness.probe(Box::new(host), "macos").run().unwrap();
    let events = events.borrow().clone();
    (report, events)
}

/// The reference flows run through WebDriverAgent on the iPhone: the helper
/// starts WebDriverAgent through the phone's test manager, Appium attaches a
/// session to the URL the helper prints, each flow's golden check reads the
/// golden from WebDriverAgent's source, the flows are timed, and the
/// 500-cell screen's node count is the source's. The release deletes the
/// session, stops Appium and the helper, and ends WebDriverAgent's runner
/// app on the phone through CoreDevice, before the tool under test takes
/// the phone back.
#[test]
fn the_reference_flows_run_through_webdriveragent_on_an_iphone_and_its_release_ends_its_runner() {
    let setup = Setup::new("wda-flows", &[], true);
    let appium = FakeAppium::start();
    let (report, events) = run(&setup, &appium);

    let tool = report
        .provenance
        .tools
        .iter()
        .find(|t| t.name == "webdriveragent")
        .unwrap();
    assert_eq!(tool.status, ToolStatus::Available, "{tool:?}");
    assert_eq!(
        tool.version.as_deref(),
        Some("16.14.1 (Appium 3.8.0, XCUITest driver 12.16.0)")
    );
    for name in [flow::LOGIN, flow::LIST_SCROLL, flow::BIG_SCREEN] {
        let m = report
            .measurements
            .iter()
            .find(|m| m.tool == "webdriveragent" && m.subject == Subject::Flow(name.into()))
            .unwrap_or_else(|| panic!("no {name} through webdriveragent"));
        assert_eq!(m.error, None, "{name}: {m:?}");
        assert_eq!(m.stats.map(|s| s.n), Some(ITERATIONS), "{name}");
        let golden = flow::reference(Fixture::SwiftUi)
            .into_iter()
            .find(|f| f.name == name)
            .unwrap()
            .golden;
        assert_eq!(m.end_state.as_ref(), Some(&golden), "{name}");
        if name == flow::BIG_SCREEN {
            // The application, its window, the count, 500 cells and 5 tabs.
            assert_eq!(m.nodes.map(|n| (n.min, n.max)), Some((508, 508)));
        }
    }

    // Appium served on the loopback at the configured port, and its one
    // session attached to the WebDriverAgent the helper started.
    let appium_log = setup.log("appium");
    assert_eq!(
        appium_log[0],
        format!(
            "--address 127.0.0.1 --port {} --log-no-colors --log {}",
            appium.port,
            setup.dir.join("work/appium-PHONE-1.log").display()
        )
    );
    assert_eq!(appium.capabilities(), wda::capabilities("PHONE-1", WDA_URL));
    let helper_log = setup.log("wda-helper");
    assert_eq!(
        helper_log,
        [
            "--udid PHONE-1 --bundle-id dev.mobileagent.wda.runner.xctrunner",
            "ended"
        ]
    );
    let requests = appium.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.starts_with("POST /session") && !r.contains("/session/"))
            .count(),
        1,
        "one session for the run: {requests:?}"
    );
    assert!(requests.contains(&"POST execute mobile: swipe up".to_owned()));
    assert_eq!(
        requests.last().map(String::as_str),
        Some("DELETE /session/S1")
    );
    assert!(!appium.session_open());
    assert_eq!(appium_log.last().map(String::as_str), Some("ended"));

    let handoff = report
        .handoffs
        .iter()
        .find(|h| h.from == "webdriveragent")
        .unwrap();
    assert_eq!(
        handoff.released.as_deref(),
        Some(
            "deleted its Appium session, stopped Appium and the helper holding \
             WebDriverAgent's test, and ended WebDriverAgent's runner app on the iPhone"
        )
    );
    assert_eq!(handoff.error, None);
    let after = events
        .iter()
        .position(|e| *e == format!("CoreDevice terminates {RUNNER_PID}"))
        .unwrap_or_else(|| panic!("the runner app was not ended: {events:?}"));
    assert_eq!(events[after + 1], "the iPhone runs no runner");
}

/// One automation session per iPhone: with another tool's runner on the
/// phone WebDriverAgent's would wait for the session, so the adapter starts
/// nothing there and each flow's series fails at once, naming the runner.
#[test]
fn webdriveragent_does_not_start_on_an_iphone_another_tool_s_runner_holds() {
    let setup = Setup::new("wda-held", &[("MobileAgentRunner-Runner", 4242)], true);
    let appium = FakeAppium::start();
    let (report, _) = run(&setup, &appium);
    let failed: Vec<&str> = report
        .measurements
        .iter()
        .filter(|m| m.tool == "webdriveragent")
        .map(|m| m.error.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(failed.len(), 3);
    for error in failed {
        assert!(
            error.contains(
                "the iPhone's automation session is held by mobile-agent's runner \
                 MobileAgentRunner-Runner (pid 4242)"
            ),
            "{error}"
        );
    }
    assert!(setup.log("wda-helper").is_empty(), "the helper started");
    assert!(appium.requests().is_empty(), "Appium was asked");
}

/// WebDriverAgent needs Appium, its XCUITest driver and a physical iPhone in
/// the run; without any of them it is skipped with the reason.
#[test]
fn webdriveragent_is_skipped_without_appium_its_driver_or_an_iphone() {
    let setup = Setup::new("wda-detect", &[], true);
    let mut adapter = WebDriverAgent::new(setup.config(1));
    assert_eq!(
        adapter.detect(),
        Detection::Available {
            version: Some("16.14.1 (Appium 3.8.0, XCUITest driver 12.16.0)".into())
        }
    );
    let config = BaselineConfig {
        targets: vec![TargetClass::Simulator, TargetClass::AndroidEmulator],
        ..setup.config(1)
    };
    assert_eq!(
        WebDriverAgent::new(config).detect(),
        Detection::Absent {
            note: wda::NO_IPHONE_NOTE.to_owned()
        }
    );
    assert_eq!(
        wda::NO_IPHONE_NOTE,
        "WebDriverAgent runs here on a physical iPhone only, and the run has none"
    );

    let no_driver = Setup::new("wda-detect-no-driver", &[], false);
    let Detection::Absent { note } = WebDriverAgent::new(no_driver.config(1)).detect() else {
        panic!("detected without its driver");
    };
    assert!(
        note.starts_with("Appium's XCUITest driver is not installed"),
        "{note}"
    );

    let empty = common::scratch("wda-detect-no-appium");
    let config = BaselineConfig {
        search_path: vec![empty.clone()],
        ..setup.config(1)
    };
    assert_eq!(
        WebDriverAgent::new(config).detect(),
        Detection::Absent {
            note: "appium not found on PATH".into()
        }
    );
}
