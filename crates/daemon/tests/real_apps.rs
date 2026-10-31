//! The tests on the default apps (design D1 of the `pass-4-fixes` change):
//! one per finding of the fourth pass on real apps, designs D2 to D9, on the
//! app that showed it, through the `mobile-agent` CLI against a daemon of
//! its own, as `devices.rs` drives the fixtures. A Simulator's Settings,
//! Maps, Calendar and Photos run when `MOBILE_AGENT_SIM_UDID` names a booted
//! Simulator, an emulator's Settings, Clock and Chrome when
//! `MOBILE_AGENT_ANDROID_SERIAL` names one; without them each test prints why
//! it skipped and passes. A test that needs the network, Maps' search and
//! Chrome's page, skips with that reason when the host reaches no server.
//!
//! Each test asserts the replies' fields and the listings, never a number of
//! milliseconds, since a shared runner's timers and load are not a developer
//! Mac's, and prints what it saw. A fresh device shows first-launch sheets
//! and permission alerts, which the tests answer as a person would. A
//! failing test names the finding it reproduced, which a task of the change
//! fixes; a test that passes on CI did not reproduce its finding there; and
//! design D1's "As found" records each verdict. The Simulator is left with the Expo fixture
//! on its Login tab, Bold Text off; the emulator's apps are stopped and its
//! dark theme off.

mod support;

use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{Daemon, TempDir};

const SIM_VAR: &str = "MOBILE_AGENT_SIM_UDID";
const SERIAL_VAR: &str = "MOBILE_AGENT_ANDROID_SERIAL";

const EXPO: &str = "dev.mobileagent.fixture.expo";

/// One test at a time drives the device.
static DEVICE: Mutex<()> = Mutex::new(());

fn device_lock() -> MutexGuard<'static, ()> {
    DEVICE.lock().unwrap_or_else(|e| e.into_inner())
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

/// Whether the host reaches a server on the internet; prints why `test` is
/// skipped when it does not.
fn network(test: &str) -> bool {
    let reached = ["example.com:80", "apple.com:443"].iter().any(|host| {
        host.to_socket_addrs().is_ok_and(|mut addrs| {
            addrs.any(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok())
        })
    });
    if !reached {
        eprintln!("skipped {test}: the host reaches no server on the internet");
    }
    reached
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

/// One CLI run in JSON mode.
#[derive(Debug)]
struct Run {
    args: Vec<String>,
    code: Option<i32>,
    reply: Value,
    stderr: String,
}

impl Run {
    fn ok(&self) -> bool {
        self.code == Some(0) && self.reply["success"] == true
    }

    /// The reply's `data`, asserting the command succeeded.
    #[track_caller]
    fn data(&self) -> Value {
        assert!(
            self.ok(),
            "`{}` failed (exit {:?}): {}\nstderr: {}",
            self.args.join(" "),
            self.code,
            self.reply,
            self.stderr
        );
        self.reply["data"].clone()
    }

    fn error(&self) -> &str {
        self.reply["error"].as_str().unwrap_or_default()
    }

    fn code(&self) -> &str {
        self.reply["code"].as_str().unwrap_or_default()
    }
}

/// A daemon without fake targets on a private socket directory, and the CLI
/// pointed at it, on one device. Dropping it shuts the daemon down.
struct Env {
    daemon: Daemon,
    home: TempDir,
    device: String,
}

impl Env {
    fn start(device: &str) -> Env {
        let mut builder = Daemon::builder().env("MOBILE_AGENT_IDLE_TIMEOUT_MS", "300000");
        if let Ok(jar) = std::env::var("MOBILE_AGENT_JAR") {
            builder = builder.env("MOBILE_AGENT_JAR", &jar);
        }
        let env = Env {
            daemon: builder.start(),
            home: TempDir::new(),
            device: device.to_owned(),
        };
        cli_bin();
        env
    }

    /// Runs `args` in JSON mode on the device and prints the reply, less a
    /// listing, which `listing` prints.
    fn run(&self, args: &[&str]) -> Run {
        let mut command = Command::new(cli_bin());
        support::clean_env(&mut command);
        command
            .args(["--json", "--device", &self.device])
            .args(args)
            .env("HOME", self.home.path())
            .env("MOBILE_AGENT_SOCKET_DIR", &self.daemon.dir)
            .env("MOBILE_AGENT_DAEMON_BIN", support::BIN)
            .current_dir(self.home.path())
            .stdin(Stdio::null());
        let output = command.output().expect("the CLI runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reply: Value = serde_json::from_str(stdout.trim_end()).unwrap_or_else(|e| {
            panic!(
                "`{}` printed no JSON ({e}): {stdout:?}; stderr: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        let mut shown = reply.clone();
        if let Some(data) = shown["data"].as_object_mut() {
            data.remove("snapshot");
            data.remove("refs");
        }
        eprintln!("$ {}\n  {shown}", args.join(" "));
        Run {
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            code: output.status.code(),
            reply,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// The listing of `snapshot` with `args`, printed.
    #[track_caller]
    fn listing(&self, args: &[&str]) -> String {
        let mut all = vec!["snapshot"];
        all.extend_from_slice(args);
        let data = self.run(&all).data();
        let listing = data["snapshot"].as_str().unwrap_or_default().to_owned();
        eprintln!(
            "  origin {}:\n{}",
            data["origin"].as_str().unwrap_or("-"),
            indent(&listing)
        );
        listing
    }

    fn interactive(&self) -> String {
        self.listing(&["-i"])
    }

    fn full(&self) -> String {
        self.listing(&[])
    }

    fn settle(&self) -> Value {
        self.run(&["wait", "--settle"]).data()
    }

    /// Answers what a fresh device puts over an app: clicks the first button
    /// of the interactive listing named exactly one of `labels`, settles and
    /// looks again, up to `rounds` times. Returns the labels clicked.
    fn dismiss(&self, labels: &[&str], rounds: usize) -> Vec<String> {
        let mut clicked = Vec::new();
        for _ in 0..rounds {
            let listing = self.interactive();
            let Some((reference, label)) = labels
                .iter()
                .find_map(|label| ref_named(&listing, label).map(|r| (r, *label)))
            else {
                break;
            };
            if self.run(&["click", &reference]).ok() {
                clicked.push(label.to_owned());
            }
            self.settle();
        }
        if !clicked.is_empty() {
            eprintln!("  dismissed {clicked:?}");
        }
        clicked
    }

    fn shutdown(mut self) {
        let _ = self.run(&["shutdown"]);
        let _ = self.daemon.wait_exit(Duration::from_secs(15));
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if self.daemon.is_running() {
            if let Ok(mut stream) = UnixStream::connect(self.daemon.socket()) {
                let _ = stream.write_all(b"{\"action\":\"shutdown\"}\n");
            }
            let _ = self.daemon.wait_exit(Duration::from_secs(15));
        }
    }
}

fn indent(listing: &str) -> String {
    listing
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The ref of the first element of a listing named exactly `name`, as `@eN`.
fn ref_named(listing: &str, name: &str) -> Option<String> {
    let quoted = format!(" \"{name}\" [");
    listing
        .lines()
        .find(|line| line.contains(&quoted) && line.contains("ref=e"))
        .and_then(line_ref)
}

/// The ref of the first line of a listing holding `needle`, as `@eN`.
fn ref_with(listing: &str, needle: &str) -> Option<String> {
    listing
        .lines()
        .find(|line| line.contains(needle) && line.contains("ref=e"))
        .and_then(line_ref)
}

/// A listing line's ref, as `@eN`.
fn line_ref(line: &str) -> Option<String> {
    let start = line.find("ref=e")? + 5;
    let digits: String = line[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then(|| format!("@e{digits}"))
}

/// Polls `done` every half second for up to `within`.
fn eventually(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

// The Simulator: Settings, Maps, Calendar and Photos.

const PREFERENCES: &str = "com.apple.Preferences";
const MAPS: &str = "com.apple.Maps";
const CALENDAR: &str = "com.apple.mobilecal";
const PHOTOS: &str = "com.apple.mobileslideshow";

/// What a fresh Simulator puts over the default apps, answered as a person
/// who keeps the defaults would answer: a feature sheet's button, a
/// permission alert's refusal.
const FIRST_LAUNCH: &[&str] = &["Continue", "Not Now", "Don’t Allow", "Don't Allow"];

fn simctl(args: &[&str]) -> String {
    Command::new("xcrun")
        .arg("simctl")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// Stops the default apps a test opens and puts the Expo fixture back on its
/// Login tab when dropped, where the shared Simulator is left.
struct SimulatorReset {
    udid: String,
    apps: &'static [&'static str],
}

impl SimulatorReset {
    fn new(udid: &str, apps: &'static [&'static str]) -> SimulatorReset {
        for app in apps {
            simctl(&["terminate", udid, app]);
        }
        SimulatorReset {
            udid: udid.to_owned(),
            apps,
        }
    }
}

impl Drop for SimulatorReset {
    fn drop(&mut self) {
        for app in self.apps {
            simctl(&["terminate", &self.udid, app]);
        }
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

/// The Simulator's iOS, as `devices` names it.
fn ios_version(env: &Env) -> String {
    let listed = env.run(&["devices"]).data();
    listed["devices"]
        .as_array()
        .and_then(|devices| devices.iter().find(|d| d["id"] == env.device.as_str()))
        .and_then(|device| device["os"].as_str())
        .unwrap_or("iOS")
        .to_owned()
}

/// The app in front, as a listing's `origin` names it.
fn origin(env: &Env) -> String {
    let data = env.run(&["snapshot", "-i"]).data();
    data["origin"].as_str().unwrap_or_default().to_owned()
}

/// Opens `app`, answering what stands in front of it as `dismiss` does. On a
/// Simulator booted minutes before, the first read can fail while
/// accessibility starts (`no_foreground`), and a permission alert that comes
/// up while open reads the app's tree ends the read (`foreground_changed`):
/// the app, or an alert over it, is waited for, and the app opened again
/// when neither comes.
fn open_past_first_launch(env: &Env, app: &str) {
    for _ in 0..3 {
        let opened = env.run(&["open", app, "--timeout", "30000"]);
        if opened.ok() {
            break;
        }
        eprintln!(
            "  open {app} answered {}: {}",
            opened.code(),
            opened.error()
        );
        let up = eventually(Duration::from_secs(20), || {
            let data = env.run(&["snapshot", "-i"]).data();
            let listing = data["snapshot"].as_str().unwrap_or_default();
            data["origin"] == app || listing.contains("Allow")
        });
        if up {
            break;
        }
    }
    env.dismiss(FIRST_LAUNCH, 6);
}

/// Taps the centre of what `selector` names by its point, where a click on
/// it is refused: iOS 27's floating toolbars count their own buttons as
/// covered (design D9a).
#[track_caller]
fn tap_box(env: &Env, selector: &str) -> Value {
    let found = env.run(&["get", "box", selector]).data();
    let x =
        found["x"].as_f64().unwrap_or_default() + found["width"].as_f64().unwrap_or_default() / 2.0;
    let y = found["y"].as_f64().unwrap_or_default()
        + found["height"].as_f64().unwrap_or_default() / 2.0;
    env.run(&["click", &format!("{x:.0}"), &format!("{y:.0}")])
        .data()
}

/// Design D2 on the Simulator's Settings: `simctl launch` right after the
/// app's termination and a press of Home failed with `NSPOSIXErrorDomain 3`,
/// ten times across two runs of the fourth pass, a retry 3 s later working
/// every time. Ten rounds of terminate, Home and `open --relaunch`; a
/// refusal is counted, and none may stand.
#[test]
fn simulator_settings_a_launch_right_after_terminate_and_home_is_not_refused() {
    let test = "simulator_settings_a_launch_right_after_terminate_and_home_is_not_refused";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[PREFERENCES]);
    let env = Env::start(&udid);
    env.run(&["open", PREFERENCES, "--timeout", "30000"]).data();
    let mut refused = Vec::new();
    let mut unsettled = 0;
    for round in 1..=10 {
        simctl(&["terminate", &udid, PREFERENCES]);
        env.run(&["press", "Home"]).data();
        let opened = env.run(&["open", PREFERENCES, "--relaunch", "--timeout", "15000"]);
        if !opened.ok() {
            refused.push(format!(
                "round {round}: {} {}",
                opened.code(),
                opened.error()
            ));
        } else if opened.reply["data"]["settled"] != true {
            unsettled += 1;
        }
    }
    eprintln!(
        "{udid}: {} of 10 launches refused, {unsettled} answered unsettled: {refused:#?}",
        refused.len()
    );
    assert!(
        refused.is_empty(),
        "D2: {} of 10 launches right after terminate and Home failed: {refused:#?}",
        refused.len()
    );
    env.shutdown();
}

/// Design D9d on a default app, the list screen under iOS 27, then the
/// switch's read-back (`real-app-fixes`): Settings' list lists its rows,
/// and after a press of Bold Text's switch `get value` reads the state the
/// switch took. Bold Text is put back off.
#[test]
fn simulator_settings_lists_its_rows_and_bold_texts_value_follows_its_switch() {
    let test = "simulator_settings_lists_its_rows_and_bold_texts_value_follows_its_switch";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[PREFERENCES]);
    let env = Env::start(&udid);
    let ios = ios_version(&env);
    env.run(&["open", PREFERENCES, "--timeout", "30000"]).data();
    env.settle();
    let rows = env.interactive();
    assert!(
        rows.contains("\"General\"") || rows.contains("\"Accessibility\""),
        "D9d: Settings' list lists none of its rows under {ios}:\n{rows}"
    );
    env.run(&["scroll", "down", "300", "--until", "text==Accessibility"]);
    env.run(&["click", "text==Accessibility"]).data();
    env.run(&["wait", "text==Display & Text Size", "--timeout", "15000"])
        .data();
    env.run(&["click", "text==Display & Text Size"]).data();
    env.run(&["wait", "text==Bold Text", "--timeout", "15000"])
        .data();
    let before = env.run(&["get", "value", "text==Bold Text"]).data();
    let was = before["checked"].as_bool();
    let clicked = env.run(&["click", "text==Bold Text"]).data();
    env.settle();
    let after = env.run(&["get", "value", "text==Bold Text"]).data();
    eprintln!(
        "{udid}: Bold Text {was:?} then {:?} after {clicked}",
        after["checked"]
    );
    if after["checked"].as_bool() == Some(true) {
        env.run(&["click", "text==Bold Text"]);
        env.settle();
    }
    assert!(
        was.is_some() && after["checked"].as_bool() == was.map(|on| !on),
        "Bold Text's value did not follow its switch: {before} then {after}"
    );
    env.shutdown();
}

/// Maps opened past its first launch with its search field's ref, and `text`
/// in the field. Its suggestions need the network.
fn maps_with(env: &Env, text: &str) -> String {
    open_past_first_launch(env, MAPS);
    env.run(&["wait", "#MapsSearchTextField", "--timeout", "20000"])
        .data();
    env.run(&["click", "#MapsSearchTextField"]).data();
    env.settle();
    env.run(&["fill", "#MapsSearchTextField", text]).data();
    env.settle();
    env.interactive()
}

/// Design D9a on the Simulator's Maps, the header cover: the search field's
/// clear button was refused as lying under the header while it showed in
/// plain view and a tap at its point hit `button "Clear"`. A click on it
/// clears the field.
#[test]
fn simulator_maps_the_search_fields_clear_button_is_not_covered_by_its_header() {
    let test = "simulator_maps_the_search_fields_clear_button_is_not_covered_by_its_header";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[MAPS]);
    let env = Env::start(&udid);
    let listing = maps_with(&env, "Lisbon");
    let full = env.full();
    let clear = ref_with(&full, "id=ClearButton")
        .or_else(|| ref_named(&full, "Clear"))
        .or_else(|| ref_named(&full, "Clear text"))
        .unwrap_or_else(|| panic!("setup: no clear button in:\n{full}\n\nsnapshot -i:\n{listing}"));
    let clicked = env.run(&["click", &clear]);
    assert!(
        clicked.ok(),
        "D9a: the click on the clear button {clear} failed {}: {}",
        clicked.code(),
        clicked.error()
    );
    let value = env.run(&["get", "value", "#MapsSearchTextField"]).data();
    assert!(
        !value["value"]
            .as_str()
            .unwrap_or_default()
            .contains("Lisbon"),
        "the clear button left the field holding {value}"
    );
    env.shutdown();
}

/// Design D8 on the Simulator's Maps: `wait --text Lisbon` resolved on
/// "Lisbon Airport (LIS)" beside a suggestion named "Lisbon". With "Lisbo" in
/// the search field, so that the field itself does not hold the text, the
/// wait names the element whose text is the whole text when the screen
/// holds one.
#[test]
fn simulator_maps_a_text_wait_prefers_the_element_whose_text_is_the_whole_text() {
    let test = "simulator_maps_a_text_wait_prefers_the_element_whose_text_is_the_whole_text";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    if !network(test) {
        return;
    }
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[MAPS]);
    let env = Env::start(&udid);
    maps_with(&env, "Lisbo");
    let found = env
        .run(&["wait", "--text", "Lisbon", "--timeout", "20000"])
        .data();
    env.settle();
    let full = env.full();
    let whole = full
        .lines()
        .any(|line| line.contains(" \"Lisbon\" [") || line.trim_end().ends_with(": Lisbon"));
    let element = &found["element"];
    eprintln!(
        "{udid}: wait --text Lisbon found {element}; the screen holds an element whose text is \
         \"Lisbon\": {whole}"
    );
    if whole {
        assert!(
            element["name"] == "Lisbon" || element["value"] == "Lisbon",
            "D8: the wait resolved on an element whose text only contains \"Lisbon\": {found}"
        );
    } else {
        eprintln!(
            "{udid}: no element's text is \"Lisbon\" alone, so the wait's choice is not judged"
        );
    }
    env.shutdown();
}

/// Design D3 on the Simulator's Maps: `type` read a field back one keystroke
/// early once in five runs, `mobileagent.de` for `mobileagent.dev`, and
/// ticked. Five times into the emptied search field, the 15 characters of
/// `Lisbon Portugal` are read back whole.
#[test]
fn simulator_maps_type_reads_the_whole_text_back_five_times() {
    let test = "simulator_maps_type_reads_the_whole_text_back_five_times";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[MAPS]);
    let env = Env::start(&udid);
    maps_with(&env, "");
    let text = "Lisbon Portugal";
    let mut short = Vec::new();
    for round in 1..=5 {
        env.run(&["fill", "#MapsSearchTextField", ""]).data();
        let typed = env.run(&["type", "#MapsSearchTextField", text]).data();
        if typed["text"] != text || typed["verified"] != "text" {
            short.push(format!("round {round}: {typed}"));
        }
    }
    assert!(
        short.is_empty(),
        "D3: {} of 5 read-backs were not the whole text: {short:#?}",
        short.len()
    );
    env.shutdown();
}

/// Design D6 on the Simulator's Photos, the listing after a system window
/// closes: right after "Don't Allow" on a permission alert that stood in
/// front of Photos, `snapshot -i` listed no interactive element while the
/// full listing held buttons. Calendar's location permission is reset so
/// that its alert comes up after its feature sheet, and Photos is opened
/// while the alert stands; right after the refusal the interactive listing
/// lists what the screen shows.
#[test]
fn simulator_photos_the_listing_right_after_an_alert_closes_lists_the_screen() {
    let test = "simulator_photos_the_listing_right_after_an_alert_closes_lists_the_screen";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[CALENDAR, PHOTOS]);
    simctl(&["privacy", &udid, "reset", "location", CALENDAR]);
    let env = Env::start(&udid);
    let opened = env.run(&["open", CALENDAR, "--timeout", "30000"]);
    if !opened.ok() {
        eventually(Duration::from_secs(30), || !origin(&env).is_empty());
    }
    env.dismiss(&["Continue", "Not Now"], 3);
    let alert = env.run(&["wait", "text==Don’t Allow", "--timeout", "20000"]);
    if !alert.ok() {
        eprintln!("skipped {test}: Calendar raised no location alert on this Simulator");
        env.shutdown();
        return;
    }
    let photos = env.run(&["open", PHOTOS, "--timeout", "30000"]);
    eprintln!(
        "  open Photos answered alert {}",
        photos.reply["data"]["alert"]
    );
    env.run(&["click", "text==Don’t Allow"]).data();
    let right_after = env.interactive();
    let settled = env.settle();
    let later = env.interactive();
    let full = env.full();
    eprintln!("{udid}: the settle after the refusal: {settled}");
    // D6 is a listing empty right after the alert that fills once the screen
    // is read again; both empty is the toolbars' cover of their own buttons.
    assert!(
        right_after.contains("ref=e") || !later.contains("ref=e"),
        "D6: right after the alert closed snapshot -i listed nothing; after a settle it listed:\n\
         {later}\nand the full listing held:\n{full}"
    );
    assert!(
        later.contains("ref=e"),
        "D9a, not D6: neither listing lists the screen, whose controls the full listing holds \
         under its toolbars at visible=0%:\n{full}"
    );
    env.shutdown();
}

/// The value `get value` reads from `selector`, empty when it has none.
fn value_of(env: &Env, selector: &str) -> String {
    let reply = env.run(&["get", "value", selector]);
    if !reply.ok() {
        return String::new();
    }
    reply.data()["value"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Turns the picker wheel `wheel` (a ref) until its value starts with
/// `wanted`, a short stroke at a time, reading it after each.
fn turn_wheel(env: &Env, wheel: &str, wanted: &str, later: impl Fn(&str) -> bool) -> bool {
    let found = env.run(&["get", "box", wheel]).data();
    let x =
        found["x"].as_f64().unwrap_or_default() + found["width"].as_f64().unwrap_or_default() / 2.0;
    let y = found["y"].as_f64().unwrap_or_default()
        + found["height"].as_f64().unwrap_or_default() / 2.0;
    for _ in 0..30 {
        let now = value_of(env, wheel);
        if now.starts_with(wanted) {
            return true;
        }
        // A stroke up brings later values in.
        let (from, to) = if later(&now) {
            (y + 20.0, y - 20.0)
        } else {
            (y - 20.0, y + 20.0)
        };
        env.run(&[
            "swipe",
            &format!("{x:.0}"),
            &format!("{from:.0}"),
            &format!("{x:.0}"),
            &format!("{to:.0}"),
        ]);
        env.settle();
    }
    value_of(env, wheel).starts_with(wanted)
}

/// Design D9c on the Simulator's Calendar, a deleted event's kept node: after
/// an event running past midnight was deleted, a node for it stayed in the
/// tree that no scroll reached. An event from 23:00 to midnight is added and
/// deleted; when the daemon's listing still holds it, a second daemon's
/// fresh read tells the replica's node, the tool's, from the app's.
#[test]
fn simulator_calendar_a_deleted_event_past_midnight_leaves_no_node() {
    let test = "simulator_calendar_a_deleted_event_past_midnight_leaves_no_node";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[CALENDAR]);
    let env = Env::start(&udid);
    open_past_first_launch(&env, CALENDAR);
    env.steady("#add-plus-button");
    tap_box(&env, "#add-plus-button");
    env.run(&["wait", "text==New Event", "--timeout", "15000"])
        .data();
    env.settle();
    let sheet = env.full();
    let title = format!("placeholder-event-{}", std::process::id());
    // Under iOS 27 the sheet's form, a list, reads empty, as Settings' list
    // does (design D9d), and the event cannot be added there.
    let field = ref_with(&sheet, "\"Title\"").unwrap_or_else(|| {
        panic!(
            "D9d before D9c: Calendar's New Event form lists none of its rows under {}, so no \
             event can be added:\n{sheet}",
            ios_version(&env)
        )
    });
    env.run(&["fill", &field, &title]).data();
    // The start's time: the row after "Starts" holds a date and a time.
    let starts = sheet
        .lines()
        .skip_while(|line| !line.contains("Starts"))
        .filter(|line| line.contains("ref=e") && (line.contains(" AM") || line.contains(" PM")))
        .find_map(line_ref)
        .unwrap_or_else(|| panic!("setup: no start time in:\n{sheet}"));
    env.run(&["click", &starts]).data();
    env.settle();
    let picker = env.full();
    // The wheels and their values: the hour's reads "10 o'clock", the
    // period's "PM"; a date wheel may come first.
    let wheels: Vec<(String, String)> = picker
        .lines()
        .filter(|line| line.contains("- combobox") && line.contains("ref=e"))
        .filter_map(|line| {
            let value = line
                .rsplit_once("]: ")
                .map_or("", |(_, value)| value)
                .trim();
            line_ref(line).map(|reference| (reference, value.to_owned()))
        })
        .collect();
    let hour = |value: &str| -> u32 {
        value
            .split(|c: char| !c.is_ascii_digit())
            .find(|part| !part.is_empty())
            .and_then(|part| part.parse().ok())
            .unwrap_or(0)
    };
    let hour_wheel = wheels
        .iter()
        .find(|(_, value)| value.contains("o'clock") || value.contains("o’clock"));
    let period_wheel = wheels
        .iter()
        .find(|(_, value)| value == "AM" || value == "PM");
    eprintln!("{udid}: the start's wheels {wheels:?}");
    let set = hour_wheel
        .is_some_and(|(wheel, _)| turn_wheel(&env, wheel, "11", |now| hour(now) % 12 < 11))
        && period_wheel
            .is_none_or(|(wheel, _)| turn_wheel(&env, wheel, "PM", |now| now.starts_with("AM")));
    let after = env.full();
    assert!(
        set,
        "setup: the start's wheels did not reach 11 PM:\n{picker}\n\nnow:\n{after}"
    );
    let save = after
        .lines()
        .find(|line| line.contains(" \"Add\" [") && !line.contains("add-plus-button"))
        .and_then(line_ref)
        .unwrap_or_else(|| panic!("setup: no Add in:\n{after}"));
    tap_box(&env, &save);
    env.run(&["wait", &format!("text={title}"), "--timeout", "15000"])
        .data();
    env.run(&["click", &format!("text={title}")]).data();
    env.run(&["wait", "text==Delete Event", "--timeout", "15000"])
        .data();
    env.run(&["click", "text==Delete Event"]).data();
    env.settle();
    let confirm = env.interactive();
    if let Some(delete) = ref_named(&confirm, "Delete Event") {
        env.run(&["click", &delete]).data();
    }
    env.settle();
    let kept = env.full().contains(&title);
    let gone = env.run(&[
        "wait",
        &format!("text={title}"),
        "--state",
        "detached",
        "--timeout",
        "10000",
    ]);
    eprintln!(
        "{udid}: after the deletion the listing holds the event: {kept}; it left the tree: {}",
        gone.ok()
    );
    if !kept && gone.ok() {
        env.shutdown();
        return;
    }
    env.shutdown();
    let fresh = Env::start(&udid);
    let read = fresh.full();
    let in_app = read.contains(&title);
    eprintln!("{udid}: a fresh daemon's read holds the event: {in_app}");
    assert!(
        in_app,
        "D9c: the replica kept the deleted event's node, which a fresh read does not hold"
    );
    eprintln!(
        "{udid}: the app keeps a node for the deleted event, which is the app's, not the tool's"
    );
    fresh.shutdown();
}

/// Design D9a's cover rule on the Simulator's Photos: under iOS 27 the
/// interactive listing counted the controls of Photos' floating toolbar
/// under the toolbar, a bar covering what it holds. The listing lists the
/// toolbar's own buttons.
#[test]
fn simulator_photos_the_toolbars_own_buttons_are_not_covered_by_it() {
    let test = "simulator_photos_the_toolbars_own_buttons_are_not_covered_by_it";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[PHOTOS]);
    let env = Env::start(&udid);
    open_past_first_launch(&env, PHOTOS);
    env.settle();
    let listing = env.interactive();
    let full = env.full();
    assert!(
        !listing.contains("under the toolbar"),
        "D9a: snapshot -i counts controls under Photos' toolbar:\n{listing}\n\nthe full listing:\n{full}"
    );
    env.shutdown();
}

/// Design D9e on the Simulator's Photos, the click's change window: a tap
/// whose effect came 486 ms after it answered `no change` in the fourth
/// pass. A tap on a control of Photos' toolbar, by its point, reports the
/// change it made; the settle after it is recorded.
#[test]
fn simulator_photos_a_tap_reports_the_change_it_made() {
    let test = "simulator_photos_a_tap_reports_the_change_it_made";
    let Some(udid) = gate(SIM_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let _reset = SimulatorReset::new(&udid, &[PHOTOS]);
    let env = Env::start(&udid);
    open_past_first_launch(&env, PHOTOS);
    env.settle();
    let full = env.full();
    // The toolbar's buttons, which the interactive listing may count under
    // the toolbar, are tapped by their point.
    let button = full
        .lines()
        .rev()
        .filter(|line| line.trim_start().starts_with("- button \"") && line.contains("ref=e"))
        .find_map(line_ref)
        .unwrap_or_else(|| panic!("setup: no button in Photos:\n{full}"));
    let found = env.run(&["get", "box", &button]).data();
    let x =
        found["x"].as_f64().unwrap_or_default() + found["width"].as_f64().unwrap_or_default() / 2.0;
    let y = found["y"].as_f64().unwrap_or_default()
        + found["height"].as_f64().unwrap_or_default() / 2.0;
    let clicked = env
        .run(&["click", &format!("{x:.0}"), &format!("{y:.0}")])
        .data();
    let settled = env.settle();
    eprintln!(
        "{udid}: the tap on {button} at ({x:.0}, {y:.0}), hit {}: frameChangeMs {}, treeDeltaMs \
         {}, watchedMs {}; the settle after it: settled {}, endedBy {}",
        clicked["hit"],
        clicked["frameChangeMs"],
        clicked["treeDeltaMs"],
        clicked["watchedMs"],
        settled["settled"],
        settled["endedBy"]
    );
    assert!(
        clicked["frameChangeMs"].is_number() || clicked["treeDeltaMs"].is_number(),
        "D9e: the tap answered no change: {clicked}"
    );
    env.shutdown();
}

// The emulator: Settings, Clock and Chrome.

const SETTINGS: &str = "com.android.settings";
const CLOCK: &str = "com.google.android.deskclock";
const CHROME: &str = "com.android.chrome";

/// The Android device `serial` through the adb server.
fn android_device(serial: &str) -> android::adb::Device {
    android::adb::AdbServer::from_env().device(serial)
}

/// What a shell command printed on the device, empty when it failed.
fn shell(device: &android::adb::Device, command: &str) -> String {
    device
        .shell(command)
        .map(|out| out.stdout_text().trim().to_owned())
        .unwrap_or_default()
}

/// Whether `package` is installed; prints why `test` is skipped when not.
fn installed(device: &android::adb::Device, package: &str, test: &str) -> bool {
    let found = shell(device, &format!("pm path {package}")).contains("package:");
    if !found {
        eprintln!(
            "skipped {test}: {package} is not installed on {}",
            device.serial()
        );
    }
    found
}

/// Stops `packages` and turns the dark theme off now and when dropped.
struct AndroidReset(android::adb::Device, &'static [&'static str]);

impl AndroidReset {
    fn new(device: &android::adb::Device, packages: &'static [&'static str]) -> AndroidReset {
        let reset = AndroidReset(device.clone(), packages);
        reset.reset();
        reset
    }

    fn reset(&self) {
        shell(&self.0, "cmd uimode night no");
        for package in self.1 {
            shell(&self.0, &format!("am force-stop {package}"));
        }
    }
}

impl Drop for AndroidReset {
    fn drop(&mut self) {
        self.reset();
    }
}

impl Env {
    /// Waits for `selector` to be on screen, then for its box to read the
    /// same twice a second apart: boxes read while a dialog or a sheet
    /// animates in are those of a frame of the animation, which a
    /// software-rendered emulator draws slowly.
    #[track_caller]
    fn steady(&self, selector: &str) {
        self.run(&["wait", selector, "--timeout", "20000"]).data();
        let mut last = None;
        let steady = eventually(Duration::from_secs(10), || {
            let reply = self.run(&["get", "box", selector]);
            let now = reply.ok().then(|| {
                let data = reply.data();
                (data["x"].to_string(), data["y"].to_string())
            });
            let same = now.is_some() && now == last;
            last = now;
            std::thread::sleep(Duration::from_millis(500));
            same
        });
        assert!(steady, "setup: {selector} never held its box");
    }
}

/// Design D9e on the emulator's Settings, the click's change window: a click
/// on Dark theme's switch answered `no change in 203 ms` in the fourth pass
/// while the theme turned on after the window. The click flips the theme,
/// and its reply names the change it made, a frame change or a tree delta.
#[test]
fn android_settings_a_click_on_dark_theme_reports_the_change_it_made() {
    let test = "android_settings_a_click_on_dark_theme_reports_the_change_it_made";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let device = android_device(&serial);
    let _reset = AndroidReset::new(&device, &[SETTINGS]);
    let env = Env::start(&serial);
    // Display & touch by its intent: taps on the main list's row did not
    // open it on CI's emulator while the list was still loading.
    shell(&device, "am start -W -a android.settings.DISPLAY_SETTINGS");
    env.steady("text=Dark theme");
    let full = env.full();
    let switch = full
        .lines()
        .find(|line| {
            line.contains("switch") && line.contains("Dark theme") && line.contains("ref=")
        })
        .or_else(|| {
            full.lines()
                .find(|line| line.contains("Dark theme") && line.contains("ref="))
        })
        .and_then(line_ref)
        .unwrap_or_else(|| panic!("setup: no Dark theme control with a ref in:\n{full}"));
    assert_eq!(
        shell(&device, "cmd uimode night"),
        "Night mode: no",
        "setup: the theme starts light"
    );

    let clicked = env.run(&["click", &switch]).data();
    assert_eq!(clicked["accepted"], true, "{clicked}");
    let flipped = eventually(Duration::from_secs(10), || {
        shell(&device, "cmd uimode night") == "Night mode: yes"
    });
    assert!(
        flipped,
        "setup: the click on {switch} did not turn the dark theme on: {clicked}"
    );
    eprintln!(
        "{serial}: the click on Dark theme: frameChangeMs {}, treeDeltaMs {}, watchedMs {}",
        clicked["frameChangeMs"], clicked["treeDeltaMs"], clicked["watchedMs"]
    );
    assert!(
        clicked["frameChangeMs"].is_number() || clicked["treeDeltaMs"].is_number(),
        "D9e: the click answered no change while it turned the dark theme on: {clicked}"
    );
    env.shutdown();
}

/// Opens Clock's new alarm and puts its time picker in keyboard mode, with
/// its boxes at rest; returns the full listing.
fn clock_time_picker_in_keyboard_mode(env: &Env) -> String {
    env.run(&["open", CLOCK, "--timeout", "30000"]).data();
    env.run(&["click", "#tab_menu_alarm"]).data();
    env.steady("text==Add alarm");
    env.run(&["click", "text==Add alarm"]).data();
    env.steady("#material_timepicker_ok_button");
    env.run(&["click", "#material_timepicker_mode_button"])
        .data();
    env.run(&[
        "wait",
        "#material_textinput_timepicker",
        "--state",
        "attached",
        "--timeout",
        "20000",
    ])
    .data();
    env.steady("#material_timepicker_ok_button");
    env.settle();
    env.full()
}

/// The full listing's line for the time picker's minute: in keyboard mode a
/// checkbox `"0 minutes"` under the minute input's view.
fn minute_line(full: &str) -> String {
    full.lines()
        .find(|line| line.contains(" minutes\"") && line.contains("ref="))
        .unwrap_or_else(|| panic!("setup: no minute element with a ref in:\n{full}"))
        .trim()
        .to_owned()
}

/// Design D9b on the emulator's Clock, the time picker's overlay: in
/// keyboard mode the minute chip lies under the minute input's view, which
/// has no ref, and a click on its ref failed `element_covered`. The click
/// on the minute's ref reaches the minute, or the listing offers a ref that
/// does.
#[test]
fn android_clock_the_time_pickers_minute_is_reachable_by_a_ref() {
    let test = "android_clock_the_time_pickers_minute_is_reachable_by_a_ref";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let device = android_device(&serial);
    if !installed(&device, CLOCK, test) {
        return;
    }
    let _reset = AndroidReset::new(&device, &[CLOCK]);
    let env = Env::start(&serial);
    let full = clock_time_picker_in_keyboard_mode(&env);
    let minute = minute_line(&full);
    let interactive = env.interactive();
    eprintln!("{serial}: the minute in the full listing: {minute}");
    let reference = line_ref(&minute).expect("the line has a ref");
    let clicked = env.run(&["click", &reference]);
    let offered = interactive.lines().any(|line| {
        line.contains("minute") && line.contains("ref=") && !line.contains("visible=0%")
    });
    assert!(
        clicked.ok() || offered,
        "D9b: the click on the minute's ref {reference} failed {} ({}), and snapshot -i offers \
         no ref for the minute:\n{interactive}",
        clicked.code(),
        clicked.error()
    );
    let _ = env.run(&["click", "#material_timepicker_cancel_button"]);
    env.shutdown();
}

/// Design D5 on the emulator's Clock: a tap by coordinates on the time
/// picker's minute answered `hit generic (no ref)`, which tells a model
/// nothing. The hit names a labeled element under the point.
#[test]
fn android_clock_a_tap_by_coordinates_names_the_labeled_element_it_hit() {
    let test = "android_clock_a_tap_by_coordinates_names_the_labeled_element_it_hit";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let device = android_device(&serial);
    if !installed(&device, CLOCK, test) {
        return;
    }
    let _reset = AndroidReset::new(&device, &[CLOCK]);
    let env = Env::start(&serial);
    let full = clock_time_picker_in_keyboard_mode(&env);
    let minute = minute_line(&full);
    let reference = line_ref(&minute).expect("the line has a ref");
    let found = env.run(&["get", "box", &reference]).data();
    let x =
        found["x"].as_f64().unwrap_or_default() + found["width"].as_f64().unwrap_or_default() / 2.0;
    let y = found["y"].as_f64().unwrap_or_default()
        + found["height"].as_f64().unwrap_or_default() / 2.0;
    let clicked = env
        .run(&["click", &format!("{x:.0}"), &format!("{y:.0}")])
        .data();
    let hit = &clicked["hit"];
    eprintln!("{serial}: a tap at ({x:.0}, {y:.0}), the minute's centre, hit {hit}");
    assert!(
        hit["role"] != "generic" || hit["name"].as_str().is_some_and(|name| !name.is_empty()),
        "D5: the tap by coordinates named a generic element without a name: {clicked}"
    );
    let _ = env.run(&["click", "#material_timepicker_cancel_button"]);
    env.shutdown();
}

/// Design D9e on the emulator's Chrome: its first run on an emulator booted
/// minutes before showed "Chrome isn't responding" dialogs in the fourth
/// pass. The first run's screens are answered, each such dialog waited
/// out and counted, and the address bar loads a page.
#[test]
fn android_chrome_the_first_run_loads_a_page_and_counts_the_dialogs_it_met() {
    let test = "android_chrome_the_first_run_loads_a_page_and_counts_the_dialogs_it_met";
    let Some(serial) = gate(SERIAL_VAR, test) else {
        return;
    };
    let _device = device_lock();
    let device = android_device(&serial);
    if !installed(&device, CHROME, test) || !network(test) {
        return;
    }
    let _reset = AndroidReset::new(&device, &[CHROME]);
    let uptime = shell(&device, "cat /proc/uptime");
    eprintln!("{serial}: uptime {uptime} (seconds since boot, idle)");
    let env = Env::start(&serial);
    env.run(&["open", CHROME, "--timeout", "60000"]).data();
    let mut dialogs = 0;
    let mut answered = Vec::new();
    let first_run = [
        "Use without an account",
        "Accept & continue",
        "No thanks",
        "No, thanks",
        "Got it",
        "Cancel",
    ];
    let mut bar = None;
    // The first run's screens come one after another, with seconds between
    // them on a software-rendered emulator, during which the listing can
    // hold nothing.
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        env.settle();
        let listing = env.interactive();
        if listing.contains("isn't responding") || listing.contains("isn’t responding") {
            dialogs += 1;
            if let Some(wait) = ref_named(&listing, "Wait") {
                env.run(&["click", &wait]);
            }
            continue;
        }
        if let Some(found) =
            ref_with(&listing, "id=url_bar").or_else(|| ref_with(&listing, "id=search_box_text"))
        {
            bar = Some(found);
            break;
        }
        match first_run
            .iter()
            .find_map(|label| ref_named(&listing, label).map(|r| (r, *label)))
        {
            Some((reference, label)) => {
                env.run(&["click", &reference]);
                answered.push(label);
            }
            None => std::thread::sleep(Duration::from_secs(2)),
        }
    }
    let bar = bar.unwrap_or_else(|| {
        panic!("setup: no address bar after the first run's screens {answered:?}")
    });
    env.run(&["click", &bar]).data();
    // The new tab page's search box gives way to the address bar; Gboard's
    // stylus onboarding, which the emulator's taps can bring up over it, is
    // cancelled.
    let focused = eventually(Duration::from_secs(30), || {
        let listing = env.interactive();
        if let Some(cancel) = ref_named(&listing, "Cancel") {
            env.run(&["click", &cancel]);
        }
        listing.contains("id=url_bar")
    });
    assert!(focused, "setup: the address bar never came up");
    env.run(&["fill", "#url_bar", "example.com"]).data();
    env.run(&["press", "Enter"]).data();
    let page = env.run(&["wait", "--text", "Example Domain", "--timeout", "30000"]);
    eprintln!(
        "{serial}: uptime at the start {uptime}; first run answered {answered:?}; {dialogs} \
         not-responding dialogs"
    );
    assert!(
        page.ok(),
        "the page's text never reached the listing after {dialogs} not-responding dialogs, so \
         the first run's dialogs are not the cause: {}",
        page.reply
    );
    env.shutdown();
}
