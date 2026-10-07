//! Task 9.3: baseline adapters for Maestro, maestro-runner and agent-device.
//! An absent tool is noted as skipped and the tools that ran are still
//! compared. Fake executables stand in for the tools, found through a search
//! path the test controls, never the host's `PATH`.

mod common;

use std::cell::{Cell, RefCell};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use bench::adapter::{Adapter, AdapterError, Clock, Detection, Role, Sample, Scenario};
use bench::baseline::{BaselineConfig, agent_device, baselines, maestro};
use bench::fake::{FakeAdapter, FakeScript, FakeSeries};
use bench::fixture::Fixture;
use bench::flow::{Flow, Selector, Step};
use bench::harness::{Harness, RunConfig, Subject};
use bench::primitive::Primitive;
use bench::provenance::{Probe, ScriptedProbe, ToolStatus};
use bench::report::{Outcome, Report};
use bench::target::{Target, TargetClass};
use serde_json::Value;

const ITERATIONS: u32 = 5;
const WARMUP: u32 = 1;

/// A tool that answers `--version`, logs its arguments next to itself and
/// then runs `body`.
fn fake_tool(dir: &Path, name: &str, version: &str, body: &str) {
    let path = dir.join(name);
    fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo '{version}'; exit 0; fi\n\
             echo \"$@\" >> \"${{0%/*}}/{name}.log\"\n\
             {body}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

const MAESTRO_PASSES: &str = "echo '[Passed] tab-list'";
const AGENT_DEVICE_PASSES: &str =
    r#"echo '{"success":true,"data":{"total":3,"executed":3,"totalDurationMs":42,"results":[]}}'"#;

/// Opens the List tab of the Expo fixture.
fn tab_flow() -> Flow {
    Flow {
        name: "tab-list".into(),
        fixture: Fixture::Expo,
        steps: vec![
            Step::Launch,
            Step::Tap(Selector::id("tab-list")),
            Step::AssertVisible(Selector::id("row-1")),
        ],
        count_nodes: false,
        golden: Default::default(),
    }
}

/// `mobile-agent`, scripted, running the flow on the emulator.
fn script() -> FakeScript {
    FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![Target::new("emulator-5554", TargetClass::AndroidEmulator)],
        series: vec![FakeSeries::new(
            "emulator-5554",
            Some(Fixture::Expo),
            None,
            Subject::Flow("tab-list".into()),
            vec![300.0, 310.0, 320.0],
        )],
        flows: vec![tab_flow()],
    }
}

struct Setup {
    dir: PathBuf,
    bin: PathBuf,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = common::scratch(name);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        Setup { dir, bin }
    }

    fn config(&self) -> BaselineConfig {
        BaselineConfig {
            search_path: vec![self.bin.clone()],
            work_dir: self.dir.join("work"),
            flow_timeout: Duration::from_secs(30),
            version_timeout: Duration::from_secs(10),
        }
    }

    fn run(&self) -> Report {
        let fake = FakeAdapter::new(script());
        let mut harness = Harness::new(
            RunConfig {
                iterations: ITERATIONS,
                warmup: WARMUP,
                retries: 0,
            },
            fake.targets(),
            fake.series(),
        )
        .flows(fake.flows())
        .tool(Box::new(fake));
        for baseline in baselines(&self.config()) {
            harness = harness.tool(baseline);
        }
        harness
            .probe(Box::new(common::probe()), "macos")
            .run()
            .unwrap()
    }

    fn log(&self, tool: &str) -> String {
        fs::read_to_string(self.bin.join(format!("{tool}.log"))).unwrap_or_default()
    }
}

#[test]
fn an_absent_tool_is_noted_as_skipped_and_the_tools_that_ran_are_still_compared() {
    let setup = Setup::new("baselines-one-missing");
    fake_tool(&setup.bin, "maestro", "2.10.0", MAESTRO_PASSES);
    fake_tool(&setup.bin, "agent-device", "0.21.14", AGENT_DEVICE_PASSES);
    let report = setup.run();

    let tools = &report.provenance.tools;
    let tool = |name: &str| tools.iter().find(|t| t.name == name).unwrap();
    assert_eq!(tool("maestro-runner").status, ToolStatus::Skipped);
    assert_eq!(
        tool("maestro-runner").note.as_deref(),
        Some("maestro-runner not found on PATH")
    );
    assert_eq!(tool("maestro").status, ToolStatus::Available);
    assert_eq!(tool("maestro").version.as_deref(), Some("2.10.0"));
    assert_eq!(tool("maestro").role, Role::Baseline);
    assert_eq!(tool("agent-device").version.as_deref(), Some("0.21.14"));

    let measured: Vec<&str> = report
        .measurements
        .iter()
        .map(|m| m.tool.as_str())
        .collect();
    assert_eq!(measured, ["mobile-agent", "maestro", "agent-device"]);
    for m in &report.measurements {
        assert_eq!(m.subject, Subject::Flow("tab-list".into()));
        assert_eq!(m.stats.map(|s| s.n), Some(ITERATIONS), "{}", m.tool);
    }
    let agent_device = &report.measurements[2];
    assert_eq!(agent_device.clock, Some(Clock::Wall));
    assert_eq!(
        agent_device.samples[0].detail.as_deref(),
        Some("agent-device reported 42 ms")
    );

    let md = report.to_markdown();
    let tools_table = common::table(&md, "Tools");
    let runner = tools_table
        .rows
        .iter()
        .find(|r| r[0] == "maestro-runner")
        .unwrap();
    assert_eq!(tools_table.cell(runner, "Status"), "skipped");
    assert_eq!(
        tools_table.cell(runner, "Note"),
        "maestro-runner not found on PATH"
    );
    let comparison = common::table(&md, "Comparison");
    assert_eq!(
        comparison.header,
        [
            "Target",
            "Fixture",
            "Screen",
            "Subject",
            "mobile-agent",
            "maestro",
            "agent-device"
        ]
    );
    assert_eq!(comparison.rows.len(), 1);
    assert_eq!(comparison.rows[0][3], "flow tab-list");
    assert!(
        md.contains("Skipped: maestro-runner (maestro-runner not found on PATH)."),
        "{md}"
    );
    let json: Value = serde_json::from_str(&report.to_json()).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &json);

    // Each tool received the flow in its documented shape, once per iteration.
    let work = setup.dir.join("work");
    let yaml = work.join("maestro-tab-list-emulator-5554.yaml");
    let maestro_log = setup.log("maestro");
    assert_eq!(maestro_log.lines().count(), (WARMUP + ITERATIONS) as usize);
    assert_eq!(
        maestro_log.lines().next().unwrap(),
        format!("--device emulator-5554 test {}", yaml.display())
    );
    assert_eq!(
        fs::read_to_string(&yaml).unwrap(),
        maestro::flow_yaml(&tab_flow(), "dev.mobileagent.fixture.expo")
    );
    let steps = work.join("agent-device-tab-list-emulator-5554.json");
    assert_eq!(
        setup.log("agent-device").lines().next().unwrap(),
        format!(
            "batch --steps-file {} --on-error stop --json --platform android \
             --serial emulator-5554 --session mobile-agent-bench",
            steps.display()
        )
    );
    assert_eq!(
        fs::read_to_string(&steps).unwrap(),
        agent_device::steps(
            &tab_flow(),
            "dev.mobileagent.fixture.expo",
            bench::target::Platform::Android
        )
    );
}

#[test]
fn every_baseline_is_skipped_when_none_is_installed() {
    let setup = Setup::new("baselines-none");
    let report = setup.run();
    for name in ["maestro", "maestro-runner", "agent-device"] {
        let tool = report
            .provenance
            .tools
            .iter()
            .find(|t| t.name == name)
            .unwrap();
        assert_eq!(tool.status, ToolStatus::Skipped);
        assert_eq!(tool.note, Some(format!("{name} not found on PATH")));
    }
    assert!(report.measurements.iter().all(|m| m.tool == "mobile-agent"));
    assert!(report.to_markdown().contains(
        "Skipped: maestro (maestro not found on PATH); maestro-runner (maestro-runner not \
         found on PATH); agent-device (agent-device not found on PATH)."
    ));
}

#[test]
fn a_flow_the_tool_fails_is_a_failed_iteration_with_its_output() {
    let setup = Setup::new("baselines-maestro-fails");
    fake_tool(
        &setup.bin,
        "maestro",
        "2.10.0",
        "echo 'Running tab-list'; echo 'Assertion is false: id: row-1 is visible' >&2; exit 1",
    );
    let report = setup.run();
    let m = report
        .measurements
        .iter()
        .find(|m| m.tool == "maestro")
        .unwrap();
    assert_eq!(m.stats, None);
    assert!(m.samples.iter().all(|s| s.outcome == Outcome::Fail));
    let error = &m.samples[1].errors[0];
    assert!(error.contains("exit status 1"), "{error}");
    assert!(
        error.contains("Assertion is false: id: row-1 is visible"),
        "{error}"
    );
    let comparison = common::table(&report.to_markdown(), "Comparison");
    let column = comparison
        .header
        .iter()
        .position(|h| h == "maestro")
        .unwrap();
    assert_eq!(comparison.rows[0][column], "no samples");
}

#[test]
fn agent_device_failures_are_read_from_its_json_reply() {
    let setup = Setup::new("baselines-agent-device-fails");
    fake_tool(
        &setup.bin,
        "agent-device",
        "0.21.14",
        r#"echo '{"success":false,"error":{"code":"ELEMENT_NOT_FOUND","message":"no element matches"},"data":{"results":[{"step":1,"command":"open","ok":true},{"step":2,"command":"press","ok":false}]}}'; exit 1"#,
    );
    let report = setup.run();
    let m = report
        .measurements
        .iter()
        .find(|m| m.tool == "agent-device")
        .unwrap();
    assert_eq!(m.stats, None);
    assert_eq!(
        m.samples[0].errors,
        ["step 2 (press) failed: ELEMENT_NOT_FOUND: no element matches"]
    );
}

/// `PATH` holding the fake tools and the system directories the harness's
/// own probes need, and none of the places a real baseline is installed.
fn hermetic_path(bin: &Path) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

#[test]
fn a_run_through_the_binary_includes_the_installed_baselines() {
    let setup = Setup::new("baselines-binary-run");
    fake_tool(&setup.bin, "maestro", "2.10.0", MAESTRO_PASSES);
    let script_path = setup.dir.join("script.json");
    fs::write(&script_path, serde_json::to_string(&script()).unwrap()).unwrap();
    let out = setup.dir.join("report");
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("run")
        .arg("--fake")
        .arg(&script_path)
        .arg("--out")
        .arg(&out)
        .args(["--iterations", "3", "--warmup", "0"])
        .env("PATH", hermetic_path(&setup.bin))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    let report = Report::from_json(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    let skipped: Vec<&str> = report
        .provenance
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Skipped)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(skipped, ["maestro-runner", "agent-device"]);
    let measured: Vec<&str> = report
        .measurements
        .iter()
        .map(|m| m.tool.as_str())
        .collect();
    assert_eq!(measured, ["mobile-agent", "maestro"]);
    assert!(
        out.join("flows/maestro-tab-list-emulator-5554.yaml")
            .is_file()
    );
}

#[test]
fn the_baselines_command_lists_each_tool_and_notes_absent_ones() {
    let setup = Setup::new("baselines-command");
    fake_tool(&setup.bin, "maestro", "2.10.0", MAESTRO_PASSES);
    let output = Command::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
        .arg("baselines")
        .env("PATH", hermetic_path(&setup.bin))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let table = common::table(&format!("# Baselines\n\n{stdout}"), "Baselines");
    let row = |name: &str| table.rows.iter().find(|r| r[0] == name).unwrap().clone();
    assert_eq!(row("maestro"), ["maestro", "available", "2.10.0", "—"]);
    assert_eq!(
        row("maestro-runner"),
        [
            "maestro-runner",
            "skipped",
            "—",
            "maestro-runner not found on PATH"
        ]
    );
    assert_eq!(
        row("agent-device"),
        [
            "agent-device",
            "skipped",
            "—",
            "agent-device not found on PATH"
        ]
    );
}

/// What happened on the host, in order: the processes `ps` listed, and the
/// moments the tool under test took the target.
type Events = Rc<RefCell<Vec<String>>>;

/// A process on the host: its pid, its program as `ps -o comm=` names it,
/// its arguments and its environment.
type Process = (u32, &'static str, &'static str, &'static str);

/// agent-device's processes for the Simulator `SIM-1` and the state
/// directory `/state`, and how many listings after a `daemon stop` each is
/// still listed: its daemon, its XCTest runner's `xcodebuild`, its snapshot
/// bridge's `simctl spawn`, the bridge in the Simulator, and the runner app
/// in the Simulator.
const ON_SIMULATOR: [(Process, u32); 5] = [
    (
        (
            401,
            "/opt/homebrew/bin/node",
            "/opt/homebrew/lib/node_modules/agent-device/dist/src/internal/daemon.js",
            "AGENT_DEVICE_STATE_DIR=/state AGENT_DEVICE_DAEMON_SERVER_MODE=socket",
        ),
        0,
    ),
    (
        (
            402,
            "/Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild",
            "test-without-building -only-testing AgentDeviceRunnerUITests/RunnerTests/testCommand \
             -xctestrun /Users/<user>/.agent-device/apple-runner/derived/ios-simulator/cache-1/\
             Build/Products/AgentDeviceRunner.env.session-SIM-1-owner-401-1-1.xctestrun \
             -destination platform=iOS Simulator,id=SIM-1",
            "AGENT_DEVICE_STATE_DIR=/state",
        ),
        1,
    ),
    (
        (
            403,
            "/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/bin/simctl",
            "spawn SIM-1 /Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge serve \
             /tmp/agent-device-ax-1-401-1/snapshot.sock --idle-timeout 60 --exit-on-disconnect false",
            "AGENT_DEVICE_STATE_DIR=/state",
        ),
        0,
    ),
    (
        (
            404,
            "/Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge",
            "serve /tmp/agent-device-ax-1-401-1/snapshot.sock --idle-timeout 60 \
             --exit-on-disconnect false",
            "SIMULATOR_DEVICE_NAME=bench SIMULATOR_UDID=SIM-1",
        ),
        3,
    ),
    (
        (
            405,
            "/Users/<user>/Library/Developer/CoreSimulator/Devices/SIM-1/data/Containers/Bundle/\
             Application/1/AgentDeviceRunnerUITests-Runner.app/AgentDeviceRunnerUITests-Runner",
            "",
            "SIMULATOR_DEVICE_NAME=bench SIMULATOR_UDID=SIM-1",
        ),
        2,
    ),
];

/// agent-device's process on the host for the emulator `emulator-5554` and
/// the state directory `/state`: its daemon, gone at the first listing after
/// a `daemon stop`, which returns once its daemon has exited. Its snapshot
/// helper's `adb … am instrument -w` ends with the session's `close`.
const ON_EMULATOR: [(Process, u32); 1] = [(
    (
        401,
        "/opt/homebrew/bin/node",
        "/opt/homebrew/lib/node_modules/agent-device/dist/src/internal/daemon.js",
        "AGENT_DEVICE_STATE_DIR=/state AGENT_DEVICE_DAEMON_SERVER_MODE=socket",
    ),
    0,
)];

/// Processes that are not agent-device's for `SIM-1` and `/state`, and never
/// end: the daemon of another state directory, another Simulator's runner
/// and bridge, and a shell whose command names everything agent-device's do.
const OTHER_PROCESSES: [Process; 5] = [
    (1, "/sbin/launchd", "", ""),
    (
        501,
        "/opt/homebrew/bin/node",
        "/opt/homebrew/lib/node_modules/agent-device/dist/src/internal/daemon.js",
        "AGENT_DEVICE_STATE_DIR=/other AGENT_DEVICE_DAEMON_SERVER_MODE=socket",
    ),
    (
        502,
        "/Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild",
        "test-without-building -only-testing AgentDeviceRunnerUITests/RunnerTests/testCommand \
         -destination platform=iOS Simulator,id=SIM-2",
        "",
    ),
    (
        503,
        "/Users/<user>/.agent-device/snapshot-source/1/snapshot-bridge",
        "serve /tmp/agent-device-ax-2-501-1/snapshot.sock --idle-timeout 60 \
         --exit-on-disconnect false",
        "SIMULATOR_DEVICE_NAME=other SIMULATOR_UDID=SIM-2",
    ),
    (
        504,
        "/bin/zsh",
        "-c grep -e /dist/src/internal/daemon.js -e AgentDeviceRunner -e SIM-1 \
         -e '/snapshot-bridge serve ' -e xcodebuild",
        "AGENT_DEVICE_STATE_DIR=/state SIMULATOR_UDID=SIM-1",
    ),
];

/// agent-device's input-method helper, which stays in an Android guest as a
/// cached process after `close` and `daemon stop`.
const IME_HELPER: &str = "com.callstack.agentdevice.imehelper";

/// The host agent-device drives a target from. Once a batch has run, `ps`
/// lists agent-device's processes, `agent_device`, beside the others; after
/// a `daemon stop` that followed the last batch, each of agent-device's
/// processes ends as `agent_device` says. Each listing of the host's
/// programs is noted in `events` with agent-device's processes on it.
///
/// On an emulator, `serial`, the host also answers `adb`: the guest's
/// processes, which hold agent-device's input-method helper from the first
/// batch on until a force-stop of its package after the last batch, and
/// the force-stops, each noted in `events`. Every other command is answered
/// as `common::probe()` answers it.
struct Host {
    tool_log: PathBuf,
    events: Events,
    agent_device: &'static [(Process, u32)],
    listings_since_stop: Cell<u32>,
    serial: Option<&'static str>,
    /// Each package force-stopped in the guest, and how many batches had run
    /// by then.
    force_stopped: RefCell<Vec<(String, usize)>>,
    scripted: ScriptedProbe,
}

impl Host {
    /// Whether agent-device has run a batch, and whether a `daemon stop`
    /// came after the last one.
    fn ran_and_stopped(&self) -> (bool, bool) {
        let log = fs::read_to_string(&self.tool_log).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        let last_batch = lines.iter().rposition(|line| line.starts_with("batch "));
        let last_stop = lines
            .iter()
            .rposition(|line| line.starts_with("daemon stop"));
        (last_batch.is_some(), last_stop > last_batch)
    }

    /// How many batches agent-device has run.
    fn batches(&self) -> usize {
        let log = fs::read_to_string(&self.tool_log).unwrap_or_default();
        log.lines()
            .filter(|line| line.starts_with("batch "))
            .count()
    }

    /// `adb shell ps -A -o PID,NAME` in the guest: a header, then each
    /// process and its name.
    fn guest(&self) -> String {
        let batches = self.batches();
        let helper = batches > 0
            && !self
                .force_stopped
                .borrow()
                .iter()
                .any(|(package, at)| package == IME_HELPER && *at == batches);
        self.events.borrow_mut().push(if helper {
            format!("the guest runs {IME_HELPER}")
        } else {
            "the guest runs none of agent-device's helpers".to_owned()
        });
        let mut listing =
            "  PID NAME\n    1 init\n  462 zygote64\n25756 dev.mobileagent.fixture.expo".to_owned();
        if helper {
            listing.push_str(&format!("\n30694 {IME_HELPER}"));
        }
        listing
    }

    /// `adb shell am force-stop <package>`, which prints nothing.
    fn force_stop(&self, package: &str) -> String {
        self.events
            .borrow_mut()
            .push(format!("adb force-stops {package}"));
        self.force_stopped
            .borrow_mut()
            .push((package.to_owned(), self.batches()));
        String::new()
    }

    /// The processes alive now.
    fn alive(&self) -> Vec<Process> {
        let agent_device: Vec<Process> = match self.ran_and_stopped() {
            (false, _) => Vec::new(),
            (true, stopped) => self
                .agent_device
                .iter()
                .filter(|(_, outlives)| !stopped || self.listings_since_stop.get() <= *outlives)
                .map(|(process, _)| *process)
                .collect(),
        };
        OTHER_PROCESSES
            .iter()
            .copied()
            .chain(agent_device)
            .collect()
    }

    /// `ps -A -o pid=,comm=`: each process alive and its program.
    fn programs(&self) -> String {
        if self.ran_and_stopped() == (true, true) {
            self.listings_since_stop
                .set(self.listings_since_stop.get() + 1);
        } else {
            self.listings_since_stop.set(0);
        }
        let alive = self.alive();
        let theirs: Vec<String> = alive
            .iter()
            .filter(|(pid, ..)| self.agent_device.iter().any(|((own, ..), _)| own == pid))
            .map(|(pid, ..)| pid.to_string())
            .collect();
        self.events.borrow_mut().push(if theirs.is_empty() {
            "ps lists none of agent-device's processes".to_owned()
        } else {
            format!("ps lists {}", theirs.join(", "))
        });
        alive
            .iter()
            .map(|(pid, program, ..)| format!("{pid:>5} {program}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `ps -E -ww -o pid=,command= -p <pids>`: each of those processes still
    /// alive, its command and then its environment; nothing, as `ps` exits 1,
    /// when none is.
    fn details(&self, pids: &str) -> Option<String> {
        let asked: Vec<u32> = pids.split(',').filter_map(|p| p.parse().ok()).collect();
        let lines: Vec<String> = self
            .alive()
            .iter()
            .filter(|(pid, ..)| asked.contains(pid))
            .map(|(pid, program, args, env)| format!("{pid:>5} {program} {args} {env}"))
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

impl Probe for Host {
    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        match (program, args) {
            ("ps", ["-A", "-o", "pid=,comm="]) => Some(self.programs()),
            ("ps", ["-E", "-ww", "-o", "pid=,command=", "-p", pids]) => self.details(pids),
            ("adb", ["-s", serial, "shell", "ps", "-A", "-o", "PID,NAME"])
                if Some(*serial) == self.serial =>
            {
                Some(self.guest())
            }
            ("adb", ["-s", serial, "shell", "am", "force-stop", package])
                if Some(*serial) == self.serial =>
            {
                Some(self.force_stop(package))
            }
            _ => self.scripted.output(program, args),
        }
    }
}

/// The fake tool under test, noting in `events` when it prepares a series,
/// which is when it takes the target.
struct Watched {
    inner: FakeAdapter,
    events: Events,
}

impl Adapter for Watched {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn role(&self) -> Role {
        self.inner.role()
    }

    fn detect(&mut self) -> Detection {
        self.inner.detect()
    }

    fn supports(&self, target: &Target, scenario: &Scenario, subject: &Subject) -> bool {
        self.inner.supports(target, scenario, subject)
    }

    fn prepare(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        subject: &Subject,
    ) -> Result<(), AdapterError> {
        self.events
            .borrow_mut()
            .push(format!("mobile-agent prepares {subject}"));
        self.inner.prepare(target, scenario, subject)
    }

    fn measure(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        primitive: Primitive,
    ) -> Result<Sample, AdapterError> {
        self.inner.measure(target, scenario, primitive)
    }

    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        self.inner.run_flow(target, flow)
    }
}

/// agent-device's `close` of the bench's session on `SIM-1`.
const CLOSE_ON_SIM: &str = "close --json --platform ios --udid SIM-1 --session mobile-agent-bench";

/// agent-device's `close` of the bench's session on `emulator-5554`.
const CLOSE_ON_EMULATOR: &str =
    "close --json --platform android --serial emulator-5554 --session mobile-agent-bench";

/// A `close` that ended the bench's session.
const CLOSED: &str = "echo '{\"success\":true,\"data\":{\"session\":\"mobile-agent-bench\"}}'";

/// A `close` that failed.
const CLOSE_FAILS: &str = "echo '{\"success\":false,\"error\":{\"code\":\"COMMAND_FAILED\",\"message\":\"the session did not end\"}}'; exit 1";

/// On the fake host of `target`, `SIM-1` or `emulator-5554`, the tool under
/// test runs the tab-list flow, then agent-device does, whose `close` runs
/// `close`, a shell command; then the tool under test takes the target back
/// for its process start. Returns the report, what happened on the host and
/// the commands agent-device was given besides its batches.
fn hand_back(name: &str, target: Target, close: &str) -> (Report, Vec<String>, Vec<String>) {
    let setup = Setup::new(name);
    fake_tool(
        &setup.bin,
        "agent-device",
        "0.21.14",
        &format!(
            "case \"$1\" in\n\
             batch) echo '{}' ;;\n\
             close) {close} ;;\n\
             daemon) echo '{{\"success\":true,\"data\":{{\"stopped\":true,\"mode\":\"graceful\",\"clean\":true}}}}' ;;\n\
             session) echo '{{\"success\":true,\"data\":{{\"stateDir\":\"/state\"}}}}' ;;\n\
             esac",
            AGENT_DEVICE_PASSES
                .trim_start_matches("echo '")
                .trim_end_matches('\'')
        ),
    );
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let host = Host {
        tool_log: setup.bin.join("agent-device.log"),
        events: Rc::clone(&events),
        agent_device: match target.class {
            TargetClass::Simulator => &ON_SIMULATOR,
            _ => &ON_EMULATOR,
        },
        listings_since_stop: Cell::new(0),
        serial: (target.class != TargetClass::Simulator).then_some("emulator-5554"),
        force_stopped: RefCell::new(Vec::new()),
        scripted: common::probe(),
    };
    // The tool under test runs the flow, then agent-device does; then the
    // tool under test takes the target back for its process start.
    let fake = FakeAdapter::new(FakeScript {
        tool: "mobile-agent".into(),
        version: Some("0.1.0".into()),
        role: Role::UnderTest,
        targets: vec![target.clone()],
        series: vec![
            FakeSeries::new(
                &target.id,
                Some(Fixture::Expo),
                None,
                Subject::Flow("tab-list".into()),
                vec![300.0],
            ),
            FakeSeries::new(
                &target.id,
                None,
                None,
                Subject::Primitive(Primitive::CliStart),
                vec![4.0],
            ),
        ],
        flows: vec![tab_flow()],
    });
    let mut harness = Harness::new(
        RunConfig {
            iterations: 2,
            warmup: 0,
            retries: 0,
        },
        fake.targets(),
        fake.series(),
    )
    .flows(fake.flows())
    .tool(Box::new(Watched {
        inner: fake,
        events: Rc::clone(&events),
    }));
    for baseline in baselines(&setup.config()) {
        harness = harness.tool(baseline);
    }
    let report = harness.probe(Box::new(host), "macos").run().unwrap();
    let commands = setup
        .log("agent-device")
        .lines()
        .filter(|l| !l.starts_with("batch "))
        .map(str::to_owned)
        .collect();
    let events = events.borrow().clone();
    (report, events, commands)
}

/// Task 10.18: on the Simulator, `agent-device close` ends only the session.
/// agent-device's daemon keeps its XCTest runner and its snapshot bridge,
/// which idled for five minutes into the next tool's series in
/// `reports/10.2-simulator.md` (finding 2). Handing the Simulator on,
/// agent-device closes its session, then stops its daemon with `--clean`,
/// which also ends the runner the daemon would leave running, and the run
/// lists the host's processes until none of agent-device's daemon, runner
/// and bridge is left before the next tool takes the Simulator.
#[test]
fn agent_device_s_daemon_runner_and_bridge_are_gone_before_the_next_tool_takes_the_simulator() {
    let (report, events, commands) =
        hand_back("baselines-agent-device-simulator", simulator(), CLOSED);
    let agent_device = report
        .measurements
        .iter()
        .find(|m| m.tool == "agent-device")
        .unwrap();
    assert_eq!(agent_device.stats.map(|s| s.n), Some(2));
    // agent-device closed its session, then stopped its daemon.
    let close = commands.iter().position(|c| c == CLOSE_ON_SIM);
    let stop = commands
        .iter()
        .position(|c| c == "daemon stop --clean --json");
    assert!(
        close.is_some() && stop.is_some() && close < stop,
        "agent-device was asked to {commands:?}"
    );
    let handoff = report
        .handoffs
        .iter()
        .find(|h| h.from == "agent-device")
        .unwrap();
    assert_eq!(handoff.to.as_deref(), Some("mobile-agent"));
    assert_eq!(
        handoff.released.as_deref(),
        Some("closed agent-device session mobile-agent-bench and stopped its daemon")
    );
    assert_eq!(handoff.error, None);
    // The hand-off listed the host's processes until the slowest of
    // agent-device's, the bridge's process in the Simulator, was gone, and
    // only then did the tool under test take the Simulator.
    assert_eq!(
        events,
        [
            "mobile-agent prepares flow tab-list",
            "ps lists 402, 404, 405",
            "ps lists 404, 405",
            "ps lists 404",
            "ps lists none of agent-device's processes",
            "mobile-agent prepares cli-start",
        ]
    );
}

/// A `close` that fails leaves the daemon to its stop: it ends its sessions
/// as it stops, so the release still stops it and the hand-off still waits
/// for its processes, and the failure is recorded.
#[test]
fn a_close_that_fails_on_the_simulator_still_stops_agent_device_s_daemon() {
    let (report, events, commands) = hand_back(
        "baselines-agent-device-close-fails",
        simulator(),
        CLOSE_FAILS,
    );
    assert_eq!(commands[..2], [CLOSE_ON_SIM, "daemon stop --clean --json"]);
    let handoff = report
        .handoffs
        .iter()
        .find(|h| h.from == "agent-device")
        .unwrap();
    assert_eq!(handoff.released, None);
    let error = handoff.error.as_deref().unwrap_or_default();
    assert!(
        error.starts_with("agent-device close failed: COMMAND_FAILED: the session did not end"),
        "{error}"
    );
    assert_eq!(
        events.last().map(String::as_str),
        Some("mobile-agent prepares cli-start")
    );
    assert_eq!(
        events[events.len() - 2],
        "ps lists none of agent-device's processes"
    );
}

/// Task 10.27: on Android too, `agent-device close` ends only the session,
/// and agent-device's daemon idled on the host for five minutes into the
/// next tool's series (`reports/10.26-remeasure.md`, finding 5), while its
/// input-method helper stayed in the guest. Handing the emulator on,
/// agent-device closes its session, then stops its daemon as on a
/// Simulator, the release force-stops the helper, which neither command
/// ends, and the run lists the host's processes until the daemon is gone
/// before the next tool takes the emulator.
#[test]
fn agent_device_s_daemon_and_helper_are_gone_before_the_next_tool_takes_the_emulator() {
    let (report, events, commands) =
        hand_back("baselines-agent-device-emulator", emulator(), CLOSED);
    let agent_device = report
        .measurements
        .iter()
        .find(|m| m.tool == "agent-device")
        .unwrap();
    assert_eq!(agent_device.stats.map(|s| s.n), Some(2));
    // agent-device closed its session, then stopped its daemon.
    assert!(
        commands.starts_with(&[
            CLOSE_ON_EMULATOR.to_owned(),
            "daemon stop --clean --json".to_owned()
        ]),
        "agent-device was asked to {commands:?}"
    );
    let handoff = report
        .handoffs
        .iter()
        .find(|h| h.from == "agent-device")
        .unwrap();
    assert_eq!(handoff.to.as_deref(), Some("mobile-agent"));
    assert_eq!(
        handoff.released.as_deref(),
        Some(
            "closed agent-device session mobile-agent-bench, stopped its daemon and \
             force-stopped its input-method helper"
        )
    );
    assert_eq!(handoff.error, None);
    // Once the daemon was stopped, the release force-stopped the helper it
    // found in the guest; the hand-off then listed the host's processes,
    // found none of agent-device's, and only then did the tool under test
    // take the emulator.
    assert_eq!(
        events,
        [
            "mobile-agent prepares flow tab-list",
            "the guest runs com.callstack.agentdevice.imehelper",
            "adb force-stops com.callstack.agentdevice.imehelper",
            "ps lists none of agent-device's processes",
            "mobile-agent prepares cli-start",
        ]
    );
}

/// As on a Simulator, a `close` that fails on the emulator leaves the daemon
/// to its stop, which ends its sessions as it stops, and the helper is
/// force-stopped all the same.
#[test]
fn a_close_that_fails_on_the_emulator_still_stops_agent_device_s_daemon() {
    let (report, events, commands) = hand_back(
        "baselines-agent-device-emulator-close-fails",
        emulator(),
        CLOSE_FAILS,
    );
    assert!(
        commands.starts_with(&[
            CLOSE_ON_EMULATOR.to_owned(),
            "daemon stop --clean --json".to_owned()
        ]),
        "agent-device was asked to {commands:?}"
    );
    let handoff = report
        .handoffs
        .iter()
        .find(|h| h.from == "agent-device")
        .unwrap();
    assert_eq!(handoff.released, None);
    let error = handoff.error.as_deref().unwrap_or_default();
    assert!(
        error.starts_with("agent-device close failed: COMMAND_FAILED: the session did not end"),
        "{error}"
    );
    assert_eq!(
        events[events.len() - 3..],
        [
            "adb force-stops com.callstack.agentdevice.imehelper",
            "ps lists none of agent-device's processes",
            "mobile-agent prepares cli-start"
        ]
    );
}

fn simulator() -> Target {
    Target::new("SIM-1", TargetClass::Simulator)
}

fn emulator() -> Target {
    Target::new("emulator-5554", TargetClass::AndroidEmulator)
}

/// A run of `series` on the script's emulator, with the tab-list flow.
fn check(series: Vec<bench::harness::Series>) -> Result<Report, String> {
    let fake = FakeAdapter::new(script());
    Harness::new(RunConfig::default(), fake.targets(), series)
        .flows(fake.flows())
        .tool(Box::new(fake))
        .probe(Box::new(common::probe()), "macos")
        .run()
}

#[test]
fn a_series_the_run_cannot_carry_out_is_refused_before_anything_runs() {
    use bench::fixture::Screen;
    use bench::harness::Series;
    let series = |fixture, screen, subject| Series {
        target: "emulator-5554".into(),
        fixture,
        screen,
        subject,
    };
    let unknown_flow = check(vec![series(
        Some(Fixture::Expo),
        None,
        Subject::Flow("checkout".into()),
    )])
    .unwrap_err();
    assert!(unknown_flow.contains("flow checkout"), "{unknown_flow}");
    let wrong_fixture = check(vec![series(
        Some(Fixture::Compose),
        None,
        Subject::Flow("tab-list".into()),
    )])
    .unwrap_err();
    assert!(wrong_fixture.contains("expo"), "{wrong_fixture}");
    let with_screen = check(vec![series(
        Some(Fixture::Expo),
        Some(Screen::List),
        Subject::Flow("tab-list".into()),
    )]);
    assert!(with_screen.is_err());
    let swiftui_on_android = check(vec![series(
        Some(Fixture::SwiftUi),
        Some(Screen::Big),
        Subject::Primitive(bench::primitive::Primitive::WarmSnapshot),
    )])
    .unwrap_err();
    assert!(
        swiftui_on_android.contains("swiftui fixture on emulator-5554"),
        "{swiftui_on_android}"
    );
}
