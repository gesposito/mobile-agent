//! Helpers shared by the integration tests.
//!
//! Every integration test drives a real Simulator, so each one first checks
//! `MOBILE_AGENT_SIM_UDID`. When it is unset, as in CI, the test prints why it
//! skipped and passes. Set it to the UDID of a booted iPhone 17 Simulator with
//! both fixtures installed and on the Expo fixture's Login tab:
//!
//! ```sh
//! MOBILE_AGENT_SIM_UDID=<udid> cargo test -p simulator
//! ```
//!
//! Tests that create their own devices install the fixtures from
//! `fixtures/*/build`, which `just fixtures-build` produces, or from the paths
//! in `MOBILE_AGENT_SWIFTUI_APP` and `MOBILE_AGENT_EXPO_APP`. Those devices are
//! deleted when the test ends, whether it passed or not.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use simulator::{
    ForegroundTree, Frame, FrameStream, Frames, ImageFormat, LaunchOptions, Lifecycle,
    ScreenshotOptions, SimulatorDriver, Tree, TreeOptions,
};

/// The variable that enables the integration tests.
pub const UDID_VARIABLE: &str = "MOBILE_AGENT_SIM_UDID";

pub const SWIFTUI_BUNDLE: &str = "dev.mobileagent.fixture.swiftui";
pub const EXPO_BUNDLE: &str = "dev.mobileagent.fixture.expo";

/// The prefix of every device a test creates, so leftovers are easy to find.
pub const DEVICE_PREFIX: &str = "mobile-agent-simtest";

/// The booted Simulator to drive, or `None` after printing why the test is
/// skipped.
pub fn gated(test: &str) -> Option<String> {
    match std::env::var(UDID_VARIABLE) {
        Ok(udid) if !udid.trim().is_empty() => Some(udid.trim().to_string()),
        _ => {
            eprintln!(
                "{test}: skipped; set {UDID_VARIABLE} to the UDID of a booted Simulator \
                 with the fixtures installed"
            );
            None
        }
    }
}

/// Serializes the tests of one binary that change what a Simulator shows.
/// Cargo runs test binaries one after another, so this covers the rest.
pub fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn driver() -> SimulatorDriver {
    SimulatorDriver::open().expect("the driver opens")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/simulator sits two levels below the workspace root")
        .to_path_buf()
}

fn fixture_app(variable: &str, relative: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join(relative));
    assert!(
        path.is_dir(),
        "{} does not exist; run `just fixtures-build` or set {variable}",
        path.display()
    );
    path
}

/// The SwiftUI fixture's application bundle.
pub fn swiftui_app() -> PathBuf {
    fixture_app(
        "MOBILE_AGENT_SWIFTUI_APP",
        "fixtures/swiftui/build/Build/Products/Debug-iphonesimulator/FixtureSwiftUI.app",
    )
}

/// The Expo fixture's application bundle.
pub fn expo_app() -> PathBuf {
    fixture_app(
        "MOBILE_AGENT_EXPO_APP",
        "fixtures/expo/build/Build/Products/Release-iphonesimulator/FixtureExpo.app",
    )
}

/// A name no other test run uses.
pub fn unique(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{label}-{}-{nanos}", std::process::id())
}

/// The device's data directory in the default device set.
pub fn data_path(udid: &str) -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME is set");
    PathBuf::from(home)
        .join("Library/Developer/CoreSimulator/Devices")
        .join(udid)
        .join("data")
}

/// The device's accessibility preferences file.
pub fn accessibility_preferences(udid: &str) -> PathBuf {
    data_path(udid).join("Library/Preferences/com.apple.Accessibility.plist")
}

/// The value at `key` of a property list, as `plutil -extract ... raw` prints it.
pub fn plist_value(file: &Path, key: &str) -> Option<String> {
    let output = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(file)
        .output()
        .expect("plutil runs");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The device `MOBILE_AGENT_SIM_UDID` names, whose runtime the devices the
/// tests create run: the newest runtime installed may not take an iPhone 17.
fn tested_device() -> String {
    std::env::var(UDID_VARIABLE)
        .ok()
        .map(|udid| udid.trim().to_string())
        .filter(|udid| !udid.is_empty())
        .unwrap_or_else(|| panic!("{UDID_VARIABLE} names the device the tests drive"))
}

/// A device made for one test, shut down and deleted when dropped.
pub struct TemporaryDevice<'a> {
    driver: &'a SimulatorDriver,
    pub udid: String,
    pub name: String,
    deleted: bool,
}

impl<'a> TemporaryDevice<'a> {
    /// An iPhone 17 on the runtime of the device the tests drive.
    pub fn create(driver: &'a SimulatorDriver, label: &str) -> TemporaryDevice<'a> {
        let name = unique(&format!("{DEVICE_PREFIX}-{label}"));
        let device = driver
            .create_device_like(&name, "iPhone 17", &tested_device())
            .expect("the device is created");
        assert_eq!(device.name, name);
        TemporaryDevice {
            driver,
            udid: device.udid,
            name,
            deleted: false,
        }
    }

    /// Shuts the device down and deletes it, failing the test on an error.
    pub fn delete(mut self) {
        self.driver
            .shutdown(&self.udid)
            .expect("the device shuts down");
        self.driver
            .delete_device(&self.udid)
            .expect("the device is deleted");
        self.deleted = true;
    }
}

impl Drop for TemporaryDevice<'_> {
    fn drop(&mut self) {
        if self.deleted {
            return;
        }
        if let Err(error) = self.driver.shutdown(&self.udid) {
            eprintln!("shutting down {} failed: {error}", self.udid);
        }
        if let Err(error) = self.driver.delete_device(&self.udid) {
            eprintln!(
                "deleting {} failed: {error}; delete it with `xcrun simctl delete {}`",
                self.udid, self.udid
            );
        }
    }
}

/// A device line of `xcrun simctl list devices`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimctlDevice {
    pub udid: String,
    pub name: String,
    pub state: String,
}

/// The devices `xcrun simctl list devices` reports. Its lines read
/// `    <name> (<UDID>) (<state>)`, and names may contain parentheses, so the
/// UDID is found by its shape.
pub fn simctl_devices() -> Vec<SimctlDevice> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices"])
        .output()
        .expect("xcrun simctl runs");
    assert!(output.status.success(), "xcrun simctl list devices failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_simctl_line)
        .collect()
}

pub fn parse_simctl_line(line: &str) -> Option<SimctlDevice> {
    let bytes = line.as_bytes();
    let start = (0..line.len()).find(|&i| {
        bytes[i] == b'('
            && line.get(i + 1..i + 37).is_some_and(is_uuid)
            && bytes.get(i + 37) == Some(&b')')
    })?;
    let name = line[..start].trim().to_string();
    let udid = line[start + 1..start + 37].to_string();
    let state = line[start + 38..]
        .trim_start()
        .strip_prefix('(')?
        .split(')')
        .next()?
        .to_string();
    Some(SimctlDevice { udid, name, state })
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// The command line and environment of a host process, from `ps -wwE`.
/// Simulator applications are ordinary processes of this user.
pub fn process_environment(pid: i32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-wwE", "-o", "command=", "-p"])
        .arg(pid.to_string())
        .output()
        .expect("ps runs");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn process_alive(pid: i32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Polls `condition` every 50 ms until it holds or `timeout` passes.
pub fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Relaunches the Expo fixture on `tab` (`login`, `list`, `big`, `anim` or
/// `system`) and waits until its first screen has had time to draw.
pub fn show_expo(driver: &SimulatorDriver, udid: &str, tab: &str) {
    driver
        .launch(
            udid,
            EXPO_BUNDLE,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", tab)
                .terminating_running(),
        )
        .expect("the Expo fixture launches");
    thread::sleep(Duration::from_millis(1500));
}

/// Puts the Expo fixture back on its Login tab when dropped, which is where
/// the shared Simulator must be left.
pub struct ExpoOnLogin<'a> {
    pub driver: &'a SimulatorDriver,
    pub udid: String,
}

impl Drop for ExpoOnLogin<'_> {
    fn drop(&mut self) {
        let restored = self.driver.launch(
            &self.udid,
            EXPO_BUNDLE,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", "login")
                .terminating_running(),
        );
        if let Err(error) = restored {
            eprintln!("could not put the Expo fixture back on Login: {error}");
        }
    }
}

/// Every frame the stream delivers within `window`.
pub fn collect(stream: &FrameStream, window: Duration) -> Vec<Frame> {
    let deadline = Instant::now() + window;
    let mut frames = Vec::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if let Some(frame) = stream.next(remaining) {
            frames.push(frame);
        }
    }
    frames
}

/// Reads frames until none arrives for `quiet`, and returns the last one
/// seen, `last` when none came. Fails when the display keeps changing for
/// `timeout`.
pub fn drain_until_quiet(
    stream: &FrameStream,
    last: Option<Frame>,
    quiet: Duration,
    timeout: Duration,
) -> Option<Frame> {
    let deadline = Instant::now() + timeout;
    let mut last = last;
    loop {
        match stream.next(quiet) {
            Some(frame) => last = Some(frame),
            None => return last,
        }
        assert!(
            Instant::now() < deadline,
            "the display did not settle within {timeout:?}"
        );
    }
}

/// A baseline JPEG's dimensions from its SOF0 segment.
pub fn jpeg_dimensions(jpeg: &[u8]) -> Option<(u32, u32)> {
    if jpeg.get(..2) != Some(&[0xFF, 0xD8]) || jpeg.get(jpeg.len() - 2..) != Some(&[0xFF, 0xD9]) {
        return None;
    }
    let mut at = 2;
    while at + 9 <= jpeg.len() {
        let marker = jpeg[at + 1];
        let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if marker == 0xC0 {
            let height = u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]);
            let width = u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]);
            return Some((u32::from(width), u32::from(height)));
        }
        at += 2 + length;
    }
    None
}

/// User plus system CPU time of this process so far.
pub fn process_cpu() -> Duration {
    // SAFETY: getrusage fills a zeroed rusage for RUSAGE_SELF.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let time = |t: libc::timeval| {
        Duration::new(t.tv_sec as u64, 0) + Duration::from_micros(t.tv_usec as u64)
    };
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// Where the Expo fixture's controls sit on an iPhone 17 (402x874 points),
/// as spike S3 measured them.
pub mod expo {
    use simulator::Point;

    pub const USERNAME: Point = Point { x: 201.0, y: 113.0 };
    pub const PASSWORD: Point = Point { x: 201.0, y: 168.0 };
    pub const SIGN_IN: Point = Point { x: 201.0, y: 218.0 };
    /// Blank space between the status line and the tab bar.
    pub const BLANK: Point = Point { x: 201.0, y: 520.0 };
    pub const TAB_LOGIN: Point = Point { x: 40.2, y: 812.0 };
    pub const TAB_LIST: Point = Point { x: 120.6, y: 812.0 };
    /// The status line under Sign in, in points: "Signed out" at launch.
    pub const STATUS_ROWS: (f64, f64) = (258.0, 288.0);
    /// The two text fields, in points.
    pub const FIELD_ROWS: (f64, f64) = (90.0, 190.0);
    /// The list between the status bar and the tab bar, in points.
    pub const LIST_ROWS: (f64, f64) = (80.0, 770.0);
}

/// A packed BGRA screenshot scaled to 1000 pixels on its longer side.
pub struct Raw {
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u8>,
}

impl Raw {
    /// The pixel row of a y coordinate in points on a 874-point screen.
    pub fn row(&self, points: f64) -> usize {
        ((points / 874.0) * self.height as f64).round() as usize
    }

    fn luma(&self, x: usize, y: usize) -> f64 {
        let at = (y * self.width + x) * 4;
        let [b, g, r] = [self.bgra[at], self.bgra[at + 1], self.bgra[at + 2]];
        0.114 * f64::from(b) + 0.587 * f64::from(g) + 0.299 * f64::from(r)
    }
}

pub fn raw_screenshot(driver: &SimulatorDriver, udid: &str) -> Raw {
    let shot = driver
        .screenshot(
            udid,
            &ScreenshotOptions {
                max_dimension: 1000,
                format: ImageFormat::Bgra,
                ..ScreenshotOptions::default()
            },
        )
        .expect("a raw screenshot");
    assert_eq!(shot.format, ImageFormat::Bgra);
    assert_eq!(
        shot.data.len(),
        shot.width as usize * shot.height as usize * 4
    );
    Raw {
        width: shot.width as usize,
        height: shot.height as usize,
        bgra: shot.data,
    }
}

/// The mean absolute brightness difference of two screenshots over the rows
/// between `rows.0` and `rows.1` points.
pub fn difference(a: &Raw, b: &Raw, rows: (f64, f64)) -> f64 {
    let (top, bottom) = (a.row(rows.0), a.row(rows.1));
    let mut total = 0.0;
    let mut count = 0usize;
    for y in top..bottom {
        for x in 0..a.width {
            total += (a.luma(x, y) - b.luma(x, y)).abs();
            count += 1;
        }
    }
    total / count.max(1) as f64
}

/// How far the content between `rows` moved up from `before` to `after`:
/// the pixel shift at which the two agree best, the difference there, and
/// the difference without a shift.
pub fn vertical_shift(before: &Raw, after: &Raw, rows: (f64, f64)) -> (usize, f64, f64) {
    let (top, bottom) = (before.row(rows.0), before.row(rows.1));
    let compare = |shift: usize| {
        let mut total = 0.0;
        let mut count = 0usize;
        for y in (top..bottom.saturating_sub(shift)).step_by(2) {
            for x in (0..before.width).step_by(3) {
                total += (after.luma(x, y) - before.luma(x, y + shift)).abs();
                count += 1;
            }
        }
        total / count.max(1) as f64
    };
    let unshifted = compare(0);
    let span = bottom - top;
    let (shift, best) = (8..span * 3 / 4)
        .map(|shift| (shift, compare(shift)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("some shift is compared");
    (shift, best, unshifted)
}

/// A new subscription, read until the display has been quiet for 700 ms,
/// and the last frame it delivered.
pub fn settle(driver: &SimulatorDriver, udid: &str) -> (FrameStream, Frame) {
    let stream = driver
        .frames(udid, &simulator::FrameOptions::default())
        .expect("the display is subscribed");
    let settled = drain_until_quiet(
        &stream,
        None,
        Duration::from_millis(700),
        Duration::from_secs(15),
    )
    .expect("a subscription delivers the frame on screen");
    (stream, settled)
}

/// The first frame within `timeout` whose content differs from `from`.
pub fn wait_for_change(stream: &FrameStream, from: &Frame, timeout: Duration) -> Option<Frame> {
    let deadline = Instant::now() + timeout;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if let Some(frame) = stream.next(remaining)
            && frame.content_hash != from.content_hash
        {
            return Some(frame);
        }
    }
    None
}

/// The fresh device of this test binary, if it made one.
static FRESH: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// Deletes the fresh device this binary created when the binary exits.
extern "C" fn delete_fresh_device() {
    let fresh = FRESH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some((udid, true)) = fresh {
        for command in ["shutdown", "delete"] {
            let _ = Command::new("xcrun")
                .args(["simctl", command, &udid])
                .output();
        }
    }
}

/// An iPhone 17 booted for the first time with both fixtures installed, on
/// the runtime of the device the tests drive, shared by the tests of one
/// binary and deleted when it exits.
/// `MOBILE_AGENT_SIM_FRESH_UDID` names an existing one to use instead, which
/// is then left alone.
pub fn fresh_device(driver: &SimulatorDriver) -> String {
    let mut fresh = FRESH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((udid, _)) = fresh.as_ref() {
        return udid.clone();
    }
    if let Ok(udid) = std::env::var("MOBILE_AGENT_SIM_FRESH_UDID")
        && !udid.trim().is_empty()
    {
        *fresh = Some((udid.trim().to_string(), false));
        return udid.trim().to_string();
    }
    let name = unique(&format!("{DEVICE_PREFIX}-fresh"));
    let device = driver
        .create_device_like(&name, "iPhone 17", &tested_device())
        .expect("the fresh device is created");
    *fresh = Some((device.udid.clone(), true));
    // SAFETY: registers a plain function with no captured state.
    unsafe { libc::atexit(delete_fresh_device) };
    driver.boot(&device.udid).expect("the fresh device boots");
    driver
        .install(&device.udid, &expo_app())
        .expect("the Expo fixture installs");
    driver
        .install(&device.udid, &swiftui_app())
        .expect("the SwiftUI fixture installs");
    device.udid
}

/// The host's one-minute load average.
pub fn load_average() -> f64 {
    let mut loads = [0.0f64; 3];
    // SAFETY: `loads` has room for the three averages asked for.
    let count = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    if count < 1 { 0.0 } else { loads[0] }
}

/// Waits up to `timeout` for the one-minute load average to fall below
/// `limit`, as the spikes did before timed steps, and returns it.
pub fn wait_for_quiet_host(limit: f64, timeout: Duration) -> f64 {
    let deadline = Instant::now() + timeout;
    loop {
        let load = load_average();
        if load < limit || Instant::now() > deadline {
            return load;
        }
        thread::sleep(Duration::from_secs(2));
    }
}

/// Whether dtuhidd has started on the device's current boot: its launchd
/// job's run count, read in the guest as spike S3 did.
pub fn dtuhidd_started(udid: &str) -> bool {
    let output = Command::new("xcrun")
        .args([
            "simctl",
            "spawn",
            udid,
            "launchctl",
            "print",
            "system/com.apple.coredevice.dtuhidd",
        ])
        .output()
        .expect("xcrun simctl spawn runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("runs = "))
        .any(|runs| runs.trim() != "0")
}

/// Runs `devicectl device pasteboard` with `arguments` on the device,
/// feeding it `input`, and returns what it printed.
pub fn devicectl_pasteboard(udid: &str, arguments: &[&str], input: Option<&str>) -> String {
    use std::io::Write as _;
    use std::process::Stdio;
    let mut child = Command::new("xcrun")
        .args(["devicectl", "device", "pasteboard"])
        .args(arguments)
        .args(["-d", udid])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("devicectl runs");
    if let Some(input) = input {
        child
            .stdin
            .take()
            .expect("a stdin")
            .write_all(input.as_bytes())
            .expect("devicectl reads its input");
    }
    let output = child.wait_with_output().expect("devicectl finishes");
    assert!(output.status.success(), "devicectl {arguments:?} failed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The focused field's text: a sentinel on the device pasteboard, then
/// Command-A and Command-C in the field, then the pasteboard read back.
pub fn field_text(driver: &SimulatorDriver, udid: &str) -> String {
    use simulator::{Input, Key, Modifiers};
    devicectl_pasteboard(udid, &["copy"], Some("sentinel: nothing was copied"));
    let command = Modifiers {
        command: true,
        ..Modifiers::default()
    };
    for usage in [0x04, 0x06] {
        driver
            .key(
                udid,
                Key {
                    usage,
                    modifiers: command,
                },
            )
            .expect("a shortcut");
        thread::sleep(Duration::from_millis(300));
    }
    devicectl_pasteboard(udid, &["paste"], None)
}

/// Relaunches the SwiftUI fixture on `tab` and returns its pid.
pub fn show_swiftui(driver: &SimulatorDriver, udid: &str, tab: &str) -> i32 {
    driver
        .launch(
            udid,
            SWIFTUI_BUNDLE,
            &LaunchOptions::new()
                .env("FIXTURE_INITIAL_TAB", tab)
                .terminating_running(),
        )
        .expect("the SwiftUI fixture launches")
        .pid
}

/// Reads the tree until `ready` accepts it, retrying the failures a launch
/// causes (no foreground yet, a snapshot the runtime refused), for up to
/// `timeout`.
pub fn snapshot_until(
    driver: &SimulatorDriver,
    udid: &str,
    timeout: Duration,
    mut ready: impl FnMut(&ForegroundTree) -> bool,
) -> ForegroundTree {
    let start = Instant::now();
    loop {
        match driver.snapshot(udid, &TreeOptions::default()) {
            Ok(tree) if ready(&tree) => return tree,
            Ok(tree) if start.elapsed() > timeout => panic!(
                "no acceptable tree within {timeout:?}; the last one, from {}, held {} nodes: {:?}",
                tree.backend,
                tree.snapshot.node_count,
                identifiers(&tree)
            ),
            Err(error) if !error.retryable() || start.elapsed() > timeout => {
                panic!("the tree read failed: {error}")
            }
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Every accessibility identifier in a read, in order.
pub fn identifiers(tree: &ForegroundTree) -> Vec<String> {
    tree.nodes()
        .into_iter()
        .filter_map(|(_, node)| node.res_id.clone())
        .collect()
}

/// Forgets whether `bundle` may use location, so it asks again.
pub fn reset_location(udid: &str, bundle: &str) {
    let status = Command::new("xcrun")
        .args(["simctl", "privacy", udid, "reset", "location", bundle])
        .status()
        .expect("simctl runs");
    assert!(status.success(), "simctl privacy reset location failed");
}

/// The bundle identifiers installed on a device, as `simctl listapps` lists
/// them.
pub fn installed_bundles(udid: &str) -> Vec<String> {
    let output = Command::new("xcrun")
        .args(["simctl", "listapps", udid])
        .output()
        .expect("simctl runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("    ") && !line.starts_with("     "))
        .filter_map(|line| line.trim().split_once(" ="))
        .map(|(key, _)| key.trim_matches('"').to_string())
        .collect()
}

/// Whether `a` and `b`, frames in points, are within `tolerance` points.
pub fn close(a: [f64; 4], b: [f64; 4], tolerance: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
}

/// How far the brightness of the rows between `rows.0` and `rows.1` points
/// strays from its mean: about 0 for a blank screen.
pub fn spread(raw: &Raw, rows: (f64, f64)) -> f64 {
    let (top, bottom) = (raw.row(rows.0), raw.row(rows.1));
    let values: Vec<f64> = (top..bottom)
        .flat_map(|y| (0..raw.width).map(move |x| (x, y)))
        .map(|(x, y)| raw.luma(x, y))
        .collect();
    let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
    values.iter().map(|v| (v - mean).abs()).sum::<f64>() / values.len().max(1) as f64
}
