//! The device drivers end to end (task 8.8): a daemon started without fake
//! targets registers the Simulator and Android drivers, and the
//! `mobile-agent` CLI drives the reference fixtures through them on a booted
//! Simulator and a running emulator.
//!
//! The Simulator tests run only when `MOBILE_AGENT_SIM_UDID` names a booted
//! Simulator with the SwiftUI and Expo fixtures installed, the Android tests
//! only when `MOBILE_AGENT_ANDROID_SERIAL` names an emulator or device with
//! the Compose fixture installed and the agent jar built
//! (`agent-android/build.sh`); without them each test prints why it skipped
//! and passes. The tests take the devices one at a time, each with a daemon
//! of its own on a private socket directory, and print what they measured.
//! The Simulator is left with the Expo fixture on its Login tab and the
//! SwiftUI fixture stopped; the Compose fixture is stopped on the emulator.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Daemon, TempDir};

const SIM_VAR: &str = "MOBILE_AGENT_SIM_UDID";
const SERIAL_VAR: &str = "MOBILE_AGENT_ANDROID_SERIAL";

const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";
const EXPO: &str = "dev.mobileagent.fixture.expo";
const COMPOSE: &str = "dev.mobileagent.fixture.compose";

/// The fixtures' Big tab holds exactly this many cells (`fixtures/CONTRACT.md`).
const CELLS: usize = 500;

/// One test at a time drives the devices, since an Android device has one
/// UI automation owner and each test runs a daemon of its own.
static DEVICES: Mutex<()> = Mutex::new(());

fn devices_lock() -> MutexGuard<'static, ()> {
    DEVICES.lock().unwrap_or_else(|e| e.into_inner())
}

/// The value of `var`, or `None` after printing why `test` is skipped.
fn gate(var: &str, test: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
        _ => {
            eprintln!("skipped {test}: set {var} to run it against a real target");
            None
        }
    }
}

/// The `mobile-agent` binary, built beside the daemon under test.
fn cli_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = Path::new(support::BIN)
            .parent()
            .expect("the daemon lives in a directory");
        let mut cargo = Command::new(env!("CARGO"));
        cargo
            .args(["build", "--quiet", "-p", "cli", "--bin", "mobile-agent"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if dir.file_name().is_some_and(|name| name == "release") {
            cargo.arg("--release");
        }
        let status = cargo.status().expect("cargo runs");
        assert!(status.success(), "cargo build -p cli failed");
        dir.join("mobile-agent")
    })
    .clone()
}

/// One CLI run.
#[derive(Debug)]
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

impl Run {
    /// The reply's `data`, asserting the command succeeded.
    #[track_caller]
    fn data(&self) -> Value {
        let reply: Value = serde_json::from_str(self.stdout.trim_end()).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}): {:?}; stderr: {}",
                self.stdout, self.stderr
            )
        });
        assert!(
            self.code == Some(0) && reply["success"] == true,
            "the command failed (exit {:?}): {reply}\nstderr: {}",
            self.code,
            self.stderr
        );
        reply["data"].clone()
    }
}

/// A daemon without fake targets on a private socket directory, and the CLI
/// pointed at it. Dropping it shuts the daemon down.
struct Env {
    daemon: Daemon,
    home: TempDir,
    /// The daemon executable the CLI starts and compares versions with.
    binary: PathBuf,
}

impl Env {
    fn start() -> Env {
        let mut builder = Daemon::builder().env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "300000");
        // The test's environment reaches the daemon without its
        // MOBILE_AGENT_ variables; the agent jar's location is one to keep.
        if let Ok(jar) = std::env::var("MOBILE_AGENT_JAR") {
            builder = builder.env("MOBILE_AGENT_JAR", &jar);
        }
        let env = Env {
            daemon: builder.start(),
            home: TempDir::new(),
            binary: PathBuf::from(support::BIN),
        };
        cli_bin();
        env
    }

    /// A daemon run from a copy of the binary outside the source tree, as an
    /// installed daemon runs, with no `MOBILE_AGENT_JAR`.
    fn start_installed() -> Env {
        let home = TempDir::new();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).expect("a bin directory");
        let binary = bin.join("mobile-agent-daemon");
        std::fs::copy(support::BIN, &binary).expect("the daemon is copied");
        let daemon = Daemon::builder()
            .binary(&binary)
            .env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "300000")
            .start();
        cli_bin();
        Env {
            daemon,
            home,
            binary,
        }
    }

    fn cli(&self, args: &[&str]) -> Run {
        let mut command = Command::new(cli_bin());
        support::clean_env(&mut command);
        command
            .args(args)
            .env("HOME", self.home.path())
            .env("MOBILE_AGENT_SOCKET_DIR", &self.daemon.dir)
            .env("MOBILE_AGENT_DAEMON_BIN", &self.binary)
            .current_dir(self.home.path())
            .stdin(Stdio::null());
        let started = Instant::now();
        let output = command.output().expect("the CLI runs");
        Run {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            elapsed: started.elapsed(),
        }
    }

    /// Runs `args` in JSON mode on `device`.
    fn on(&self, device: &str, args: &[&str]) -> Run {
        let mut all = vec!["--json", "--device", device];
        all.extend_from_slice(args);
        self.cli(&all)
    }

    /// Sends `shutdown` through the CLI and waits for the daemon to exit.
    fn shutdown(&mut self) -> Duration {
        let started = Instant::now();
        let data = self.cli(&["--json", "shutdown"]).data();
        assert_eq!(data["stopped"], true, "{data}");
        let status = self
            .daemon
            .wait_exit(Duration::from_secs(15))
            .expect("the daemon exits after shutdown");
        assert!(status.success(), "the daemon exits cleanly: {status}");
        started.elapsed()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if self.daemon.is_running() {
            let _ = support::raw_request(&self.daemon.socket(), r#"{"action":"shutdown"}"#);
            let _ = self.daemon.wait_exit(Duration::from_secs(15));
        }
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// The median of `samples`, in milliseconds.
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// The ref a listing gives the element with identifier `id`, as `@eN`.
#[track_caller]
fn ref_of(listing: &str, id: &str) -> String {
    let needle = format!("id={id},");
    let line = listing
        .lines()
        .find(|line| line.contains(&needle))
        .unwrap_or_else(|| panic!("no element {id} in:\n{listing}"));
    let start = line.find("ref=e").expect("the line has a ref") + 4;
    let digits: String = line[start + 1..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    format!("@e{digits}")
}

/// The distinct `cell-N` identifiers a listing gives refs to.
fn cells(listing: &str) -> usize {
    let mut ids: Vec<&str> = listing
        .lines()
        .filter(|line| line.contains("ref=e"))
        .filter_map(|line| {
            let start = line.find("id=cell-")? + 3;
            line[start..].split([',', ']']).next()
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// The listing of a `snapshot -i` reply.
fn listing(data: &Value) -> String {
    data["snapshot"].as_str().unwrap_or_default().to_owned()
}

#[track_caller]
fn number(data: &Value, key: &str) -> f64 {
    data[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} is not a number in {data}"))
}

/// The entry `status` gives the target `id`.
#[track_caller]
fn status_of(status: &Value, id: &str) -> Value {
    status["targets"]
        .as_array()
        .and_then(|targets| targets.iter().find(|t| t["id"] == id))
        .cloned()
        .unwrap_or_else(|| panic!("status lists no target {id}: {status}"))
}

/// The host's processes whose command line contains `needle`.
fn host_processes(needle: &str) -> Vec<String> {
    let output = Command::new("ps")
        .args(["-axo", "pid,command"])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_owned)
        .collect()
}

/// Waits up to `within` for `done`.
fn eventually(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A JPEG or PNG file's dimensions: the JPEG's start-of-frame, or the PNG's
/// header.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        return Some((be(16), be(20)));
    }
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while at + 9 < bytes.len() {
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if matches!(marker, 0xC0..=0xC3) {
            let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]);
            let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]);
            return Some((u32::from(width), u32::from(height)));
        }
        at += 2 + length;
    }
    None
}

/// What one fixture run measured, printed at its end.
#[derive(Default)]
struct Measured {
    open: f64,
    open_settled: Option<bool>,
    open_to_first_snapshot: f64,
    warm_cli: f64,
    warm_socket: f64,
    dispatch: f64,
    frame_change: Option<f64>,
    tree_delta: Option<f64>,
    watched: f64,
    settle: f64,
    ended_by: String,
    screenshot: (u32, u32),
    shutdown: f64,
}

impl Measured {
    fn print(&self, target: &str) {
        let optional = |value: Option<f64>| {
            value.map_or_else(|| "absent".to_owned(), |v| format!("{v:.1} ms"))
        };
        eprintln!(
            "{target}: open {:.1} ms (settled: {:?}); open to first snapshot {:.1} ms; \
             warm snapshot p50 {:.1} ms through the CLI, {:.1} ms on the socket; \
             click dispatch {:.3} ms, first frame change {}, first tree delta {}, watched {:.1} ms; \
             settle {:.1} ms ended by {}; screenshot {}x{}; shutdown {:.1} ms",
            self.open,
            self.open_settled,
            self.open_to_first_snapshot,
            self.warm_cli,
            self.warm_socket,
            self.dispatch,
            optional(self.frame_change),
            optional(self.tree_delta),
            self.watched,
            self.settle,
            self.ended_by,
            self.screenshot.0,
            self.screenshot.1,
            self.shutdown,
        );
    }
}

/// Opens `app` on the Big tab, then checks the snapshot, a tap on the Login
/// tab, a settle wait and a screenshot, recording what they took.
fn big_then_login(env: &Env, device: &str, open: &[&str], dir: &Path) -> Measured {
    let mut measured = Measured::default();
    let started = Instant::now();
    let run = env.on(device, open);
    let opened = run.data();
    measured.open = ms(run.elapsed);
    measured.open_settled = opened["settled"].as_bool();
    let first = env.on(device, &["snapshot", "-i"]).data();
    measured.open_to_first_snapshot = ms(started.elapsed());
    let big = listing(&first);
    assert_eq!(
        cells(&big),
        CELLS,
        "the Big tab lists every cell with a ref; verdict {}:\n{big}",
        first["verdict"]
    );
    assert_eq!(first["target"], device, "{first}");

    let mut warm = Vec::new();
    for _ in 0..5 {
        let run = env.on(device, &["snapshot", "-i"]);
        assert_eq!(cells(&listing(&run.data())), CELLS);
        warm.push(ms(run.elapsed));
    }
    measured.warm_cli = median(warm);
    let mut socket = Vec::new();
    for _ in 0..10 {
        let started = Instant::now();
        let reply = env.daemon.request(json!({
            "action": "snapshot", "device": device, "interactive": true,
        }));
        socket.push(ms(started.elapsed()));
        assert_eq!(reply["success"], true, "{reply}");
    }
    measured.warm_socket = median(socket);

    let tab = ref_of(&big, "tab-login");
    let clicked = env.on(device, &["click", &tab]).data();
    assert_eq!(clicked["accepted"], true, "{clicked}");
    measured.dispatch = number(&clicked, "dispatchMs");
    measured.frame_change = clicked["frameChangeMs"].as_f64();
    measured.tree_delta = clicked["treeDeltaMs"].as_f64();
    measured.watched = number(&clicked, "watchedMs");
    let login = listing(&env.on(device, &["snapshot", "-i"]).data());
    assert!(
        login.contains("id=login-username") && cells(&login) == 0,
        "the tap on the Login tab changed the tree to the Login screen:\n{login}"
    );

    let settled = env.on(device, &["wait", "--settle"]).data();
    let ended = settled["endedBy"].as_str().unwrap_or_default().to_owned();
    assert!(
        ["tree", "frame", "both", "none", "timeout"].contains(&ended.as_str()),
        "wait --settle names the signal it ended on: {settled}"
    );
    measured.settle = number(&settled, "elapsedMs");
    measured.ended_by = ended;

    let path = dir.join("screen.jpg");
    let path_text = path.to_string_lossy().into_owned();
    let shot = env.on(device, &["screenshot", &path_text]).data();
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("no screenshot at {path_text}: {e}"));
    let size = image_size(&bytes).unwrap_or_else(|| panic!("the screenshot is no image: {shot}"));
    assert!(
        size.0 > 0 && size.1 > size.0,
        "a portrait image: {size:?}, {shot}"
    );
    assert_eq!(
        (shot["width"].as_u64(), shot["height"].as_u64()),
        (Some(u64::from(size.0)), Some(u64::from(size.1))),
        "the reply gives the image's size: {shot}"
    );
    measured.screenshot = size;
    measured
}

/// Stops the SwiftUI fixture now and puts the Expo fixture back on its
/// Login tab when dropped, where the shared Simulator is left.
struct SimulatorFixtures {
    udid: String,
}

impl SimulatorFixtures {
    fn new(udid: &str) -> SimulatorFixtures {
        simctl(&["terminate", udid, SWIFTUI]);
        SimulatorFixtures {
            udid: udid.to_owned(),
        }
    }
}

impl Drop for SimulatorFixtures {
    fn drop(&mut self) {
        simctl(&["terminate", &self.udid, SWIFTUI]);
        let restored = Command::new("xcrun")
            .args([
                "simctl",
                "launch",
                "--terminate-running-process",
                &self.udid,
                EXPO,
            ])
            .env("SIMCTL_CHILD_FIXTURE_INITIAL_TAB", "login")
            .output();
        if !restored.is_ok_and(|out| out.status.success()) {
            eprintln!("could not put the Expo fixture back on its Login tab");
        }
    }
}

fn simctl(args: &[&str]) {
    let _ = Command::new("xcrun")
        .arg("simctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The Android device `serial` through the adb server.
fn android_device(serial: &str) -> android::adb::Device {
    android::adb::AdbServer::from_env().device(serial)
}

/// The agent processes on the device.
fn android_agents(device: &android::adb::Device) -> Vec<android::agent::RunningAgent> {
    device
        .shell(android::agent::PS_COMMAND)
        .map(|out| android::agent::parse_agents(&out.stdout_text()))
        .unwrap_or_default()
}

/// Waits, up to a minute, until no agent runs on the shared device and
/// nothing holds UI automation, as the Android driver's own tests do.
fn android_quiet(device: &android::adb::Device) {
    let quiet = eventually(Duration::from_secs(60), || {
        let held = device
            .shell("dumpsys accessibility")
            .map(|out| out.stdout_text().contains("Ui Automation"))
            .unwrap_or(false);
        android_agents(device).is_empty() && !held
    });
    assert!(
        quiet,
        "the shared device stayed busy for a minute: agents {:?}",
        android_agents(device)
    );
}

/// Stops the Compose fixture now and when dropped.
struct ComposeStopped(android::adb::Device);

impl ComposeStopped {
    fn new(device: &android::adb::Device) -> ComposeStopped {
        let _ = device.shell(&format!("am force-stop {COMPOSE}"));
        ComposeStopped(device.clone())
    }
}

impl Drop for ComposeStopped {
    fn drop(&mut self) {
        let _ = self.0.shell(&format!("am force-stop {COMPOSE}"));
    }
}

/// Agent-cli spec, Command grammar: `devices` lists the Simulator and the
/// emulator the device drivers found, each with its platform and state.
#[test]
fn devices_lists_the_simulator_and_the_emulator_with_platform_and_state() {
    let test = "devices_lists_the_simulator_and_the_emulator_with_platform_and_state";
    let udid = gate(SIM_VAR, test);
    let serial = gate(SERIAL_VAR, test);
    if udid.is_none() && serial.is_none() {
        return;
    }
    let _devices = devices_lock();
    let env = Env::start();
    let listed = env.cli(&["--json", "devices"]).data();
    let devices = listed["devices"].as_array().cloned().unwrap_or_default();
    let find = |id: &str| {
        devices
            .iter()
            .find(|d| d["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("devices does not list {id}: {listed}"))
    };
    if let Some(udid) = &udid {
        let simulator = find(udid);
        assert_eq!(simulator["platform"], "ios", "{simulator}");
        assert_eq!(simulator["class"], "simulator", "{simulator}");
        assert_eq!(simulator["state"], "booted", "{simulator}");
        assert!(
            simulator["os"]
                .as_str()
                .is_some_and(|os| os.starts_with("iOS")),
            "{simulator}"
        );
    }
    if let Some(serial) = &serial {
        let android = find(serial);
        assert_eq!(android["platform"], "android", "{android}");
        assert!(
            ["android-emulator", "android-device"]
                .contains(&android["class"].as_str().unwrap_or_default()),
            "{android}"
        );
        assert_eq!(android["state"], "device", "{android}");
    }
    let human = env.cli(&["devices"]);
    assert_eq!(human.code, Some(0), "{human:?}");
    for id in udid.iter().chain(serial.iter()) {
        let line = human
            .stdout
            .lines()
            .find(|line| line.starts_with(id.as_str()))
            .unwrap_or_else(|| panic!("no line for {id} in:\n{}", human.stdout));
        let state = if Some(id) == udid.as_ref() {
            "booted"
        } else {
            "device"
        };
        assert!(line.contains(state), "the line names the state: {line}");
    }
}

/// On a Simulator: `open` launches the SwiftUI fixture on its Big tab with
/// its launch environment, `snapshot -i` lists its 500 cells, a click on the
/// Login tab reports its dispatch and changes the tree, `wait --settle`
/// names its ending signal, `screenshot` saves an image, `status` names the
/// input transport and the reader, and `shutdown` leaves no reader running.
#[test]
fn the_swiftui_fixture_is_driven_through_the_cli_on_a_simulator() {
    let test = "the_swiftui_fixture_is_driven_through_the_cli_on_a_simulator";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let _fixtures = SimulatorFixtures::new(&udid);
    let dir = TempDir::new();
    let mut env = Env::start();
    let mut measured = big_then_login(
        &env,
        &udid,
        &["open", SWIFTUI, "--env", "FIXTURE_INITIAL_TAB=big"],
        dir.path(),
    );

    let status = env.cli(&["--json", "status"]).data();
    let target = status_of(&status, &udid);
    assert_eq!(target["attached"], true, "{target}");
    let driver = &target["driver"];
    assert!(
        ["dtuhidd", "Indigo"].contains(&driver["input"].as_str().unwrap_or_default()),
        "status names the Simulator's input transport: {target}"
    );
    assert_eq!(driver["tree"], "simulator-reader", "{target}");
    assert!(
        driver["reader"]["readyMs"].as_f64().is_some()
            && driver["reader"]["pid"].as_i64().is_some(),
        "status names the spawned reader and its readiness time: {target}"
    );
    let lines = driver["lines"].as_array().cloned().unwrap_or_default();
    assert!(
        lines
            .iter()
            .any(|l| l.as_str().is_some_and(|l| l.starts_with("input: "))),
        "{target}"
    );

    let daemon = env.daemon.pid();
    let readers = format!("mobile-agent-reader-{daemon}-");
    assert!(
        !host_processes(&readers).is_empty(),
        "the reader runs while the daemon is attached"
    );
    measured.shutdown = ms(env.shutdown());
    assert!(
        eventually(Duration::from_secs(5), || host_processes(&readers)
            .is_empty()),
        "no reader of the daemon runs after shutdown: {:?}",
        host_processes(&readers)
    );
    measured.print(&format!("Simulator {udid}"));
}

/// On an emulator: `open` launches the Compose fixture by package with the
/// `initial_tab` extra, `snapshot -i` lists its 500 cells, a click on the
/// Login tab reports its dispatch and changes the tree, `wait --settle`
/// names its ending signal, `screenshot` saves an image, `status` names the
/// input path, and `shutdown` leaves no agent on the device.
#[test]
fn the_compose_fixture_is_driven_through_the_cli_on_an_emulator() {
    let test = "the_compose_fixture_is_driven_through_the_cli_on_an_emulator";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    let _fixture = ComposeStopped::new(&device);
    let dir = TempDir::new();
    // An installed daemon: its own copy of the binary and the agent built
    // into it, neither the source tree nor MOBILE_AGENT_JAR (task 8.11).
    let mut env = Env::start_installed();
    let mut measured = big_then_login(
        &env,
        &serial,
        &["open", COMPOSE, "--env", "initial_tab=big"],
        dir.path(),
    );

    let status = env.cli(&["--json", "status"]).data();
    let target = status_of(&status, &serial);
    assert_eq!(target["attached"], true, "{target}");
    let driver = &target["driver"];
    assert!(
        ["emulator controller", "agent"].contains(&driver["input"].as_str().unwrap_or_default()),
        "status names the Android input path: {target}"
    );
    assert_eq!(
        driver["agentJar"], "embedded",
        "the installed daemon deploys the agent built into it: {target}"
    );
    let socket = driver["socket"].as_str().unwrap_or_default().to_owned();
    let pid = driver["pid"].as_u64().unwrap_or_default();
    assert!(
        !socket.is_empty() && pid > 0,
        "status names the agent: {target}"
    );
    let human = env.cli(&["status"]);
    assert!(
        human.stdout.contains("input: "),
        "the human status prints the driver's lines:\n{}",
        human.stdout
    );
    let ours = |agents: &[android::agent::RunningAgent]| {
        agents
            .iter()
            .any(|a| u64::from(a.pid) == pid && a.socket == socket)
    };
    assert!(
        ours(&android_agents(&device)),
        "the agent runs while attached"
    );

    measured.shutdown = ms(env.shutdown());
    let agents = android_agents(&device);
    assert!(
        !ours(&agents),
        "the daemon's agent is gone after shutdown: {agents:?}"
    );
    assert!(
        agents.is_empty(),
        "no agent process is left on the device after shutdown: {agents:?}"
    );
    measured.print(&format!("Android {serial}, input {}", driver["input"]));
}

/// Task 8.11: `--force` reaches the deploy conflict. A second daemon finds
/// the first one's agent holding UI automation and is refused with
/// `automation_conflict`, naming `--force`; the same command with `--force`
/// takes UI automation over, and no agent is left once both shut down.
#[test]
fn force_takes_ui_automation_over_from_another_daemons_agent() {
    let test = "force_takes_ui_automation_over_from_another_daemons_agent";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    let _fixture = ComposeStopped::new(&device);
    let mut first = Env::start();
    let open = ["open", COMPOSE, "--env", "initial_tab=login"];
    first.on(&serial, &open).data();
    let holder = status_of(&first.cli(&["--json", "status"]).data(), &serial)["driver"]["pid"]
        .as_u64()
        .expect("the first daemon's agent");

    let mut second = Env::start();
    let refused = second.on(&serial, &["snapshot", "-i"]);
    let reply: Value = serde_json::from_str(refused.stdout.trim_end()).expect("a JSON reply");
    assert_eq!(reply["code"], "automation_conflict", "{reply}");
    let message = reply["error"].as_str().unwrap_or_default();
    assert!(message.contains("--force"), "{message}");

    let forced = second
        .cli(&["--json", "--force", "--device", &serial, "snapshot", "-i"])
        .data();
    assert!(
        listing(&forced).contains("id=login-username"),
        "the forced daemon reads the screen: {forced}"
    );
    let taker = status_of(&second.cli(&["--json", "status"]).data(), &serial)["driver"]["pid"]
        .as_u64()
        .expect("the second daemon's agent");
    assert_ne!(taker, holder, "a new agent took UI automation over");
    let agents = android_agents(&device);
    assert!(
        agents.iter().all(|a| u64::from(a.pid) != holder),
        "the first daemon's agent was stopped: {agents:?}"
    );

    second.shutdown();
    first.shutdown();
    let agents = android_agents(&device);
    assert!(agents.is_empty(), "no agent is left: {agents:?}");
}

/// Puts the device's rotation settings back when dropped.
struct RotationRestored {
    device: android::adb::Device,
    accelerometer: String,
    user: String,
}

impl RotationRestored {
    fn new(device: &android::adb::Device) -> RotationRestored {
        let get = |key: &str| {
            device
                .shell(&format!("settings get system {key}"))
                .map(|out| out.stdout_text().trim().to_owned())
                .unwrap_or_default()
        };
        RotationRestored {
            device: device.clone(),
            accelerometer: get("accelerometer_rotation"),
            user: get("user_rotation"),
        }
    }

    /// Turns the display by `quarter_turns` and waits until it reports the
    /// size of that rotation.
    fn turn(&self, quarter_turns: u32) {
        let _ = self.device.shell(&format!(
            "settings put system accelerometer_rotation 0; settings put system user_rotation {quarter_turns}"
        ));
        let landscape = quarter_turns % 2 == 1;
        let turned = eventually(Duration::from_secs(5), || {
            let shown = self
                .device
                .shell("dumpsys window displays")
                .map(|out| out.stdout_text())
                .unwrap_or_default();
            let cur = shown
                .split_whitespace()
                .find_map(|word| word.strip_prefix("cur="))
                .and_then(|size| size.split_once('x'))
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
            cur.is_some_and(|(w, h)| (w > h) == landscape)
        });
        assert!(
            turned,
            "the display did not turn {quarter_turns} quarter turns"
        );
    }
}

impl Drop for RotationRestored {
    fn drop(&mut self) {
        let value = |v: &str, default: &str| {
            if v.is_empty() || v == "null" {
                default.to_owned()
            } else {
                v.to_owned()
            }
        };
        let _ = self.device.shell(&format!(
            "settings put system user_rotation {}; settings put system accelerometer_rotation {}",
            value(&self.user, "0"),
            value(&self.accelerometer, "0")
        ));
    }
}

/// Task 8.11: the Android display's size follows the current rotation, so
/// a swipe no element anchors crosses the middle of the rotated screen.
#[test]
fn the_android_display_follows_its_rotation() {
    let test = "the_android_display_follows_its_rotation";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    let _fixture = ComposeStopped::new(&device);
    let rotation = RotationRestored::new(&device);
    let mut env = Env::start();
    env.on(&serial, &["open", COMPOSE, "--env", "initial_tab=login"])
        .data();
    let from = |reply: &Value| {
        let from = reply["from"].as_array().cloned().unwrap_or_default();
        (
            from.first().and_then(Value::as_f64).unwrap_or_default(),
            from.get(1).and_then(Value::as_f64).unwrap_or_default(),
        )
    };
    let upright = env.on(&serial, &["swipe", "up", "10"]).data();
    let (x, y) = from(&upright);
    assert!(
        x < y,
        "portrait: the middle is lower than it is wide: {upright}"
    );
    rotation.turn(1);
    let turned = env.on(&serial, &["swipe", "up", "10"]).data();
    let (x, y) = from(&turned);
    assert!(
        x > y,
        "landscape: the swipe crosses the middle of the rotated screen: {turned}"
    );
    drop(rotation);
    env.shutdown();
}

/// Doctor (task 8.7) with the device drivers: the Xcode compatibility table
/// holds the rows the Simulator driver supplies.
#[test]
fn doctor_shows_the_xcode_compatibility_table_the_simulator_driver_fills() {
    let test = "doctor_shows_the_xcode_compatibility_table_the_simulator_driver_fills";
    if gate(SIM_VAR, test).is_none() {
        return;
    }
    let _devices = devices_lock();
    let env = Env::start();
    let data = env.cli(&["--json", "doctor"]).data();
    let drivers: Vec<&str> = data["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|c| c["name"] == "drivers"))
        .and_then(|check| check["message"].as_str())
        .map(|message| message.split(", ").collect())
        .unwrap_or_default();
    assert!(drivers.contains(&"simulator"), "{data}");
    let rows = data["compatibility"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for component in ["Xcode", "CoreSimulator", "input transport", "tree reader"] {
        assert!(
            rows.iter().any(|row| row["subject"]
                .as_str()
                .is_some_and(|s| s.starts_with(component))),
            "no {component} row in {rows:?}"
        );
    }
    assert!(
        rows.iter().any(|row| row["feature"]
            .as_str()
            .is_some_and(|f| f.contains("lookup:error:"))),
        "the table lists the private calls: {rows:?}"
    );
    let check = data["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|c| c["name"] == "xcode-compatibility"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(check["status"], "ok", "{check}");
    let human = env.cli(&["doctor"]);
    assert!(
        human.stdout.contains("Xcode compatibility:") && !human.stdout.contains("(no rows"),
        "{}",
        human.stdout
    );
}
