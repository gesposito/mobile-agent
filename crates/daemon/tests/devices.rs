//! The device drivers end to end (task 8.8): a daemon started without fake
//! targets registers the Simulator and Android drivers, or those
//! `MOBILE_AGENT_DRIVERS` names, and the `mobile-agent` CLI drives the
//! reference fixtures through them on a booted Simulator and a running
//! emulator.
//!
//! The Simulator tests run only when `MOBILE_AGENT_SIM_UDID` names a booted
//! Simulator with the SwiftUI and Expo fixtures installed, the Android tests
//! only when `MOBILE_AGENT_ANDROID_SERIAL` names an emulator or device with
//! the Compose fixture installed and the agent jar built
//! (`agent-android/build.sh`); without them each test prints why it skipped
//! and passes. For a run on the Simulator alone,
//! `MOBILE_AGENT_DRIVERS=simulator` starts the daemons without the Android
//! driver. The tests take the devices one at a time, each with a daemon of
//! its own on a private socket directory, and print what they measured.
//! The Simulator is left with the Expo fixture on its Login tab and the
//! SwiftUI fixture stopped, or both stopped after the mid-list scroll test;
//! the fixtures are stopped on the emulator.

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
        Env::start_with(&[])
    }

    /// A daemon with `vars` in its environment as well.
    fn start_with(vars: &[(&str, &str)]) -> Env {
        let mut builder = Daemon::builder().env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "300000");
        for (key, value) in vars {
            builder = builder.env(key, value);
        }
        // The test's environment reaches the daemon without its
        // MOBILE_AGENT_ variables but the drivers the run names; the agent
        // jar's location is one more to keep.
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
    assert_eq!(
        opened["settled"], true,
        "open settles once the app has shown its first tree: {opened}"
    );
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

    // The click was the first input of the Simulator's input session: its
    // watch waited for the late effects a fresh activation brings (8.11).
    assert!(
        measured.frame_change.is_some() && measured.tree_delta.is_some(),
        "the first click reports its first frame change and tree delta"
    );
    // Task 8.15: open activated the input session, so the first click did
    // not wait for it.
    assert!(
        measured.dispatch < 5.0,
        "the first click dispatched in {} ms",
        measured.dispatch
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

/// Task 8.15: right after a launch the Expo fixture's window holds only an
/// empty React Native root view, its launch splash, for a few hundred ms
/// (a snapshot then read `(empty screen)`); `open` waits past it, so the
/// snapshot right after each relaunch lists the Login screen.
#[test]
fn the_expo_fixture_shows_its_screen_not_its_splash_right_after_open() {
    let test = "the_expo_fixture_shows_its_screen_not_its_splash_right_after_open";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let _fixtures = SimulatorFixtures::new(&udid);
    let mut env = Env::start();
    for _ in 0..3 {
        let opened = env
            .on(
                &udid,
                &[
                    "open",
                    EXPO,
                    "--relaunch",
                    "--env",
                    "FIXTURE_INITIAL_TAB=login",
                ],
            )
            .data();
        let listing = listing(&env.on(&udid, &["snapshot", "-i"]).data());
        assert!(
            listing.contains("id=login-submit"),
            "the snapshot right after open ({opened}) shows the Login screen:\n{listing}"
        );
    }
    env.shutdown();
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

    // Task 8.13: open waited for the frame stream's first frame, so the
    // first click after the attach reports its first frame change.
    assert!(
        measured.frame_change.is_some(),
        "the first click reports its first frame change"
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

/// Stops the fixture apps `.1` now and when dropped.
struct Stopped(android::adb::Device, &'static [&'static str]);

impl Stopped {
    fn new(device: &android::adb::Device, packages: &'static [&'static str]) -> Stopped {
        let stopped = Stopped(device.clone(), packages);
        stopped.stop();
        stopped
    }

    fn stop(&self) {
        for package in self.1 {
            let _ = self.0.shell(&format!("am force-stop {package}"));
        }
    }
}

impl Drop for Stopped {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Panics with how to install `package` unless it is on the device.
fn require_installed(device: &android::adb::Device, package: &str) {
    let installed = device
        .shell(&format!("pm path {package}"))
        .is_ok_and(|out| out.stdout_text().contains("package:"));
    assert!(
        installed,
        "{package} is not installed on {}; build it (fixtures/build.sh) and adb install it",
        device.serial()
    );
}

/// The daemon's switch that leaves emulators' controllers unused (task
/// 10.12), `MOBILE_AGENT_EMULATOR_CONTROLLER=off`, on an emulator that
/// serves its controller: `status` names the agent's paths, no frame comes
/// from the controller, and a click goes through the agent.
#[test]
fn the_controller_switch_leaves_a_served_controller_unused() {
    let test = "the_controller_switch_leaves_a_served_controller_unused";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    if let Err(unavailable) = android::emulator::resolve(&serial) {
        eprintln!(
            "skipped {test}: {serial} serves no controller for the switch to leave unused: {}",
            unavailable.warning()
        );
        return;
    }
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    let _fixture = ComposeStopped::new(&device);
    let mut env = Env::start_with(&[("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")]);
    let opened = env.on(&serial, &["open", COMPOSE]).data();
    assert_eq!(opened["settled"], true, "{opened}");
    let login = listing(&env.on(&serial, &["snapshot", "-i"]).data());
    let tab = ref_of(&login, "tab-big");
    let clicked = env.on(&serial, &["click", &tab]).data();
    assert_eq!(clicked["accepted"], true, "{clicked}");
    assert!(
        clicked["frameChangeMs"].is_null(),
        "no frame source, so no frame change: {clicked}"
    );

    let status = env.cli(&["--json", "status"]).data();
    assert_eq!(status["daemon"]["emulatorController"], "off", "{status}");
    let target = status_of(&status, &serial);
    assert_eq!(target["frameSource"], false, "{target}");
    let driver = &target["driver"];
    assert_eq!(driver["input"], "agent", "{target}");
    assert_eq!(driver["frames"], Value::Null, "{target}");
    assert_eq!(driver["endpoint"], Value::Null, "{target}");
    assert_eq!(driver["warning"], Value::Null, "{target}");
    assert_eq!(driver["emulatorController"], "off", "{target}");
    assert_eq!(driver["framesReceived"], 0, "{target}");
    assert_eq!(
        (
            driver["inputs"]["controller"].as_u64(),
            driver["inputs"]["agent"].as_u64()
        ),
        (Some(0), Some(1)),
        "the click went through the agent: {target}"
    );
    env.shutdown();
}

/// Task 10.11, on an emulator on the agent path (the controller switch
/// off): the platform animates the Expo fixture in after a relaunch,
/// scaling it up over the launcher and sliding it in from the right over
/// another app, and reports the window at one frame of that animation, the
/// same to every read, until the animation ends. Without a frame source
/// `open` used to report the screen settled while the boxes it had were
/// those of that frame (`reports/10.1-emulator.md`, finding 2: `tab-big`
/// at x 252 to 512 instead of 164.6, and big-screen failing 46 of 50
/// iterations). Right after each `open`, over the launcher or over the
/// Compose fixture, the boxes are where the screen comes to rest, and a
/// click on the Big tab opens it.
#[test]
fn open_on_the_agent_path_waits_for_the_expo_screen_to_come_to_rest() {
    let test = "open_on_the_agent_path_waits_for_the_expo_screen_to_come_to_rest";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    require_installed(&device, EXPO);
    require_installed(&device, COMPOSE);
    let _fixtures = Stopped::new(&device, &[EXPO, COMPOSE]);
    let mut env = Env::start_with(&[("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")]);
    let box_of = |id: &str| {
        let data = env.on(&serial, &["get", "box", &format!("#{id}")]).data();
        (number(&data, "x"), number(&data, "y"))
    };
    // Where the screen comes to rest: read well after a launch.
    env.on(&serial, &["open", EXPO, "--relaunch"]).data();
    std::thread::sleep(Duration::from_secs(2));
    let rest = box_of("tab-big");
    let field_rest = box_of("login-username");

    let mut opens = Vec::new();
    let mut after = Vec::new();
    for over_compose in [false, true, false, true, false, true] {
        if over_compose {
            env.on(&serial, &["open", COMPOSE]).data();
        }
        let run = env.on(&serial, &["open", EXPO, "--relaunch"]);
        let opened = run.data();
        assert_eq!(opened["settled"], true, "{opened}");
        opens.push((over_compose, number(&opened, "elapsedMs")));
        after.push((over_compose, box_of("tab-big"), box_of("login-username")));
    }
    eprintln!(
        "{serial}, agent path: open (over the Compose fixture?, ms) {opens:?}; tab-big at rest \
         {rest:?}, login-username at rest {field_rest:?}; right after each open (over the \
         Compose fixture?, tab-big, login-username): {after:?}"
    );
    for (_, tab, field) in &after {
        assert!(
            (tab.0 - rest.0).abs() <= 1.0 && (tab.1 - rest.1).abs() <= 1.0,
            "tab-big right after open was at {tab:?}, not where it comes to rest, {rest:?}: {after:?}"
        );
        assert!(
            (field.0 - field_rest.0).abs() <= 1.0 && (field.1 - field_rest.1).abs() <= 1.0,
            "login-username right after open was at {field:?}, not {field_rest:?}: {after:?}"
        );
    }

    let listing = listing(&env.on(&serial, &["snapshot", "-i"]).data());
    let big = ref_of(&listing, "tab-big");
    env.on(&serial, &["click", &big]).data();
    let found = env
        .on(&serial, &["wait", "#cell-500", "--timeout", "5000"])
        .data();
    assert!(
        found["element"]["id"] == "cell-500",
        "the click right after open opened the Big tab: {found}"
    );
    env.shutdown();
}

/// Task 10.14, on an emulator on the agent path (the controller switch
/// off): the harness's list-scroll flow on the Expo fixture, command for
/// command as the bench's CLI adapter sends it (`crates/bench/src/cli`):
/// relaunch the app, click the List tab, then settle, look for `row-200`
/// without waiting and scroll down 600 points until it is there, click it
/// and wait for `Detail 200`. The last stroke leaves the list at its end,
/// where the platform's stretch springs back for 333 to 383 ms without an
/// event, and until it rests the list takes a touch that comes down as the
/// start of a drag: the bench's check run timed out waiting for `Detail
/// 200` in both attempts of `reports/10.11-agent-path/report.md`, while the
/// same commands by hand, seconds apart, opened the row. Each of three runs
/// must open the row with the click sent right after the check, as the
/// bench sends it. When one does not, a daemon started afresh reads the
/// screen again, which tells a tap the list took from a change the replica
/// missed.
#[test]
fn the_bench_s_list_scroll_flow_opens_the_last_row_on_the_agent_path() {
    let test = "the_bench_s_list_scroll_flow_opens_the_last_row_on_the_agent_path";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    require_installed(&device, EXPO);
    let _fixture = Stopped::new(&device, &[EXPO]);
    let agent_path = [("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")];
    let mut env = Env::start_with(&agent_path);
    // The bench's session and command lines.
    let bench = |env: &Env, args: &[&str]| {
        let mut all = vec!["--session", "mobile-agent-bench"];
        all.extend_from_slice(args);
        env.on(&serial, &all)
    };
    let mut runs = Vec::new();
    for run in 1..=3 {
        bench(&env, &["open", "--relaunch", "--", EXPO]).data();
        bench(&env, &["click", "--timeout", "10000", "--", "id=tab-list"]).data();
        let mut strokes = 0u32;
        let mut stroke_ended = None;
        let (settle, found) = loop {
            let settle = bench(&env, &["wait", "--settle"]).data();
            let check = bench(&env, &["wait", "--timeout", "0", "--", "id=row-200"]);
            if check.code == Some(0) {
                break (settle, check.data());
            }
            let reply: Value = serde_json::from_str(check.stdout.trim_end()).unwrap_or_else(|e| {
                panic!("run {run}: the check printed no JSON ({e}): {check:?}")
            });
            assert_eq!(reply["code"], "wait_timeout", "run {run}: {reply}");
            assert!(
                strokes < 50,
                "run {run}: row-200 was not on screen after 50 strokes"
            );
            let started = Instant::now();
            let stroke = bench(&env, &["scroll", "--", "down", "600"]);
            stroke.data();
            stroke_ended = Some(started + stroke.elapsed);
            strokes += 1;
        };
        let clicked = Instant::now();
        let click = bench(&env, &["click", "--timeout", "10000", "--", "id=row-200"]).data();
        let after_stroke = stroke_ended.map(|ended| ms(clicked.duration_since(ended)));
        let opened = bench(&env, &["wait", "--timeout", "10000", "--text=Detail 200"]);
        if opened.code != Some(0) {
            env.shutdown();
            let fresh = Env::start_with(&agent_path);
            let screen = listing(&fresh.on(&serial, &["snapshot", "-i"]).data());
            let shows = |id: &str| screen.contains(&format!("id={id},"));
            panic!(
                "run {run}: the click on row-200 {after_stroke:?} ms after the last of {strokes} \
                 strokes opened nothing: {}\nclick: {click}\nrow-200 at the check: {}\nsettle \
                 before it: {settle}\na fresh daemon reads detail-200 {}, row-200 {}:\n{screen}",
                opened.stdout.trim_end(),
                found["element"]["bounds"],
                shows("detail-200"),
                shows("row-200"),
            );
        }
        let shown = bench(
            &env,
            &["get", "text", "--timeout", "10000", "--", "id=detail-200"],
        )
        .data();
        assert_eq!(shown["text"], "Detail 200", "run {run}: {shown}");
        runs.push((
            strokes,
            settle["elapsedMs"].clone(),
            after_stroke,
            click["treeDeltaMs"].clone(),
        ));
    }
    eprintln!(
        "{serial}, agent path, the bench's list-scroll flow: per run (strokes, the settle after \
         the last stroke in ms, the click's start after that stroke's end in ms, the click's \
         tree delta in ms): {runs:?}"
    );
    env.shutdown();
}

/// The bounds of the element a reply's data carries, as `[left, top, right,
/// bottom]` in points.
#[track_caller]
fn element_bounds(data: &Value) -> [f64; 4] {
    let bounds = data["element"]["bounds"]
        .as_array()
        .unwrap_or_else(|| panic!("an element with bounds: {data}"));
    let at = |i: usize| bounds[i].as_f64().expect("a coordinate");
    [at(0), at(1), at(2), at(3)]
}

/// The Expo fixture's rows are 48 points tall (`fixtures/expo`).
const EXPO_ROW: f64 = 48.0;

/// Task 10.15, on an emulator on both of its paths (the controller's, with
/// frames, and the agent's), with the Expo fixture. In a frame-path run of
/// the harness's list-scroll flow the scroll stopped after seven strokes
/// with `row-200` 1.2 points tall at the list's bottom edge, all of the row
/// that Android reported, and the click hit that sliver. First the list's
/// first screen, whose last row the list cuts off: `scroll --until` on that
/// row must bring it in whole (48 points tall, which Android reports only
/// for a row the list does not cut off) with one stroke, and the click on
/// it must open it; the test prints the stroke's cost and how far the
/// content went against the stroke's aim. Then the harness's list-scroll
/// flow with `scroll --until` where the bench checks for the row itself:
/// relaunch the app, click the List tab, scroll down 600 points at a time
/// until `row-200` is mostly in view, click it and wait for `Detail 200`;
/// each run must stop with the row whole and `visible` 1, and open it.
#[test]
fn scroll_until_stops_only_once_the_row_is_mostly_in_view_on_an_emulator() {
    let test = "scroll_until_stops_only_once_the_row_is_mostly_in_view_on_an_emulator";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    require_installed(&device, EXPO);
    let _fixture = Stopped::new(&device, &[EXPO]);
    let whole = |bounds: [f64; 4]| (bounds[3] - bounds[1] - EXPO_ROW).abs() < 0.5;
    let paths: [(&str, &[(&str, &str)]); 2] = [
        ("controller", &[]),
        ("agent", &[("MOBILE_AGENT_EMULATOR_CONTROLLER", "off")]),
    ];
    for (path, vars) in paths {
        let mut env = Env::start_with(vars);
        let box_of = |id: &str| {
            let data = env.on(&serial, &["get", "box", "--", id]).data();
            let (x, y) = (number(&data, "x"), number(&data, "y"));
            [
                x,
                y,
                x + number(&data, "width"),
                y + number(&data, "height"),
            ]
        };
        let relaunch_on_the_list = || {
            env.on(&serial, &["open", "--relaunch", "--", EXPO]).data();
            env.on(
                &serial,
                &["click", "--timeout", "10000", "--", "id=tab-list"],
            )
            .data();
        };

        // The first screen's last row, which the list cuts off.
        let mut brought_in = Vec::new();
        for run in 1..=3 {
            relaunch_on_the_list();
            let screen = listing(&env.on(&serial, &["snapshot", "-i"]).data());
            let last = (1..=40)
                .rev()
                .find(|n| screen.contains(&format!("id=row-{n},")))
                .expect("rows on the list's first screen");
            let row = format!("id=row-{last}");
            let first = box_of("id=row-1");
            let before = box_of(&row);
            assert!(
                !whole(before),
                "{path} path, run {run}: the list cuts off row {last}, the last row on its first \
                 screen: {before:?}"
            );
            let scroll = env.on(
                &serial,
                &[
                    "scroll",
                    "down",
                    "600",
                    "--until",
                    &row,
                    "--timeout",
                    "30000",
                ],
            );
            let data = scroll.data();
            let after = element_bounds(&data);
            assert!(
                data["strokes"] == 1 && whole(after) && data["visible"] == 1.0,
                "{path} path, run {run}: one stroke brings row {last} in whole: {data}"
            );
            env.on(&serial, &["click", "--timeout", "10000", "--", &row])
                .data();
            env.on(
                &serial,
                &[
                    "wait",
                    "--timeout",
                    "10000",
                    &format!("--text=Detail {last}"),
                ],
            )
            .data();
            // The stroke aims to carry the row's reported middle to the
            // list's, from row 1's top to where the list cut the row off.
            let aim = (before[1] + before[3]) / 2.0 - (first[1] + before[3]) / 2.0;
            let carried = before[1] - after[1];
            brought_in.push((
                last,
                (before[3] - before[1]).round(),
                ms(scroll.elapsed).round(),
                aim.round(),
                carried.round(),
            ));
        }

        // The harness's list-scroll flow.
        let mut runs = Vec::new();
        for run in 1..=3 {
            relaunch_on_the_list();
            let scroll = env.on(
                &serial,
                &[
                    "scroll",
                    "down",
                    "600",
                    "--until",
                    "id=row-200",
                    "--timeout",
                    "30000",
                ],
            );
            let data = scroll.data();
            let bounds = element_bounds(&data);
            assert!(
                whole(bounds) && data["visible"] == 1.0,
                "{path} path, run {run}: row-200 whole and in view where the scroll stopped: {data}"
            );
            env.on(
                &serial,
                &["click", "--timeout", "10000", "--", "id=row-200"],
            )
            .data();
            env.on(
                &serial,
                &["wait", "--timeout", "10000", "--text=Detail 200"],
            )
            .data();
            runs.push((
                data["strokes"].clone(),
                ms(scroll.elapsed).round(),
                bounds[1],
            ));
        }
        eprintln!(
            "{serial}, {path} path: scroll down 600 --until on the first screen's cut-off row, per \
             run (row, points of it shown, ms, the stroke's aim and the content's carry in \
             points) {brought_in:?}; scroll down 600 --until row-200 after relaunching on the \
             List tab, per run (strokes, ms, the row's top where it stopped) {runs:?}"
        );
        env.shutdown();
    }
}

/// Runs of `scroll --until` on a row from each fixture's first screen.
const MID_LIST_RUNS: u32 = 4;

/// One run of `scroll down 600 --until` on a row, from the top of a
/// fixture's List tab.
#[derive(Debug)]
struct UntilRun {
    app: &'static str,
    row: u32,
    /// The reply's `data`, or its `code` and `error` when it failed.
    reply: Value,
    ok: bool,
    elapsed: f64,
}

/// [`MID_LIST_RUNS`] runs on each of `rows` for each of `apps` on `device`,
/// each after relaunching the app and clicking its List tab, as the
/// harness's list-scroll flow starts.
fn scroll_until_runs(
    env: &Env,
    device: &str,
    apps: &[&'static str],
    rows: &[u32],
) -> Vec<UntilRun> {
    let mut runs = Vec::new();
    for &app in apps {
        for &row in rows {
            for _ in 0..MID_LIST_RUNS {
                env.on(device, &["open", "--relaunch", "--", app]).data();
                env.on(
                    device,
                    &["click", "--timeout", "10000", "--", "id=tab-list"],
                )
                .data();
                let until = format!("id=row-{row}");
                let run = env.on(
                    device,
                    &[
                        "scroll",
                        "down",
                        "600",
                        "--until",
                        &until,
                        "--timeout",
                        "60000",
                    ],
                );
                let reply: Value = serde_json::from_str(run.stdout.trim_end())
                    .unwrap_or_else(|e| panic!("{app} row-{row}: no JSON ({e}): {run:?}"));
                let ok = run.code == Some(0) && reply["success"] == true;
                runs.push(UntilRun {
                    app,
                    row,
                    reply: if ok {
                        reply["data"].clone()
                    } else {
                        json!({"code": reply["code"], "error": reply["error"], "data": reply["data"]})
                    },
                    ok,
                    elapsed: ms(run.elapsed),
                });
            }
        }
    }
    runs
}

/// Prints every run, per app and row: whether it found the row, its
/// strokes, its time, the share of the row in view and the stroke's length;
/// then asserts that every run found the row mostly in view.
fn report_until_runs(device: &str, runs: &[UntilRun]) {
    for run in runs {
        let data = &run.reply;
        eprintln!(
            "{device} {} row-{}: {} strokes {} in {:.0} ms, visible {}, amount {}{}",
            run.app,
            run.row,
            if run.ok { "found" } else { "FAILED" },
            if run.ok {
                &data["strokes"]
            } else {
                &data["data"]["strokes"]
            },
            run.elapsed,
            data["visible"],
            data["amount"],
            if run.ok {
                String::new()
            } else {
                format!(": {} {}", data["code"], data["error"])
            },
        );
    }
    let failed: Vec<&UntilRun> = runs
        .iter()
        .filter(|run| !run.ok || run.reply["visible"].as_f64().is_none_or(|v| v < 0.5))
        .collect();
    assert!(
        failed.is_empty(),
        "{} of {} runs did not stop with the row mostly in view: {failed:#?}",
        failed.len(),
        runs.len()
    );
}

/// Task 10.16, on an emulator (the controller's path) with the Expo and
/// Compose fixtures: `scroll --until` must find a row in the middle of the
/// list. It looks for the row between strokes, and a 600-point stroke over
/// 300 ms whose finger lifted moving carried the lists 1,103 to 1,319
/// points, more than the roughly 780 they show, so the row could pass
/// through the list between two looks and the scroll failed at the list's
/// end with `element_not_found`. Each run relaunches the fixture, clicks
/// its List tab and scrolls down 600 points at a time until `row-100`, or
/// `row-200` at the list's end, is mostly in view; every run must find its
/// row. The test prints each run's strokes and time.
#[test]
fn scroll_until_finds_a_mid_list_row_on_an_emulator() {
    let test = "scroll_until_finds_a_mid_list_row_on_an_emulator";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let device = android_device(&serial);
    android_quiet(&device);
    require_installed(&device, EXPO);
    require_installed(&device, COMPOSE);
    let _fixtures = Stopped::new(&device, &[EXPO, COMPOSE]);
    let mut env = Env::start();
    let runs = scroll_until_runs(&env, &serial, &[EXPO, COMPOSE], &[100, 200]);
    env.shutdown();
    report_until_runs(&serial, &runs);
}

/// Terminates the Simulator's fixtures now and when dropped, leaving it
/// with neither running.
struct SimulatorStopped(String);

impl SimulatorStopped {
    fn new(udid: &str) -> SimulatorStopped {
        let stopped = SimulatorStopped(udid.to_owned());
        stopped.stop();
        stopped
    }

    fn stop(&self) {
        for app in [SWIFTUI, EXPO] {
            simctl(&["terminate", &self.0, app]);
        }
    }
}

impl Drop for SimulatorStopped {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Task 10.16, on a Simulator with the SwiftUI and Expo fixtures: as on the
/// emulator, `scroll --until` must find `row-100` and `row-200`. A 600-point
/// stroke over 300 ms whose finger lifted moving carried the Expo list
/// 1,522 to 1,608 points on the iOS 26.5 Simulator, about twice what it
/// shows. The test leaves both fixtures stopped.
#[test]
fn scroll_until_finds_a_mid_list_row_on_a_simulator() {
    let test = "scroll_until_finds_a_mid_list_row_on_a_simulator";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _devices = devices_lock();
    let _fixtures = SimulatorStopped::new(&udid);
    let mut env = Env::start();
    let runs = scroll_until_runs(&env, &udid, &[SWIFTUI, EXPO], &[100, 200]);
    env.shutdown();
    report_until_runs(&udid, &runs);
}
