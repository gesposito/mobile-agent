//! WebDriverAgent on a physical iPhone, driven through Appium's XCUITest
//! driver. WebDriverAgent is built from the driver's own copy, signed for the
//! team with a bundle identifier under `dev.mobileagent.` and installed on
//! the phone beforehand (`crates/bench/README.md`, Baselines).
//!
//! On the first flow on an iPhone the adapter starts WebDriverAgent through
//! the phone's test manager with its helper process (`mobile-agent-bench
//! wda-session`, [`super::wda_xctest`]), which prints the localhost URL it
//! forwards to WebDriverAgent's port through usbmuxd; it starts Appium on
//! [`APPIUM_PORT`] and opens one session attached to that URL
//! (`appium:webDriverAgentUrl`), so Appium neither builds nor launches
//! WebDriverAgent and needs no RemoteXPC tunnel. A flow is then a sequence of
//! WebDriver requests to Appium, timed by wall clock from the first
//! request's start to the last one's end:
//!
//! - launch: `mobile: terminateApp`, then `mobile: launchApp`;
//! - tap: the element found by accessibility identifier (or a predicate on
//!   its label for a text), then `click`;
//! - fill: `click` on the field, then `value` with the text;
//! - scroll-to: until the element is found and displayed, `mobile: swipe`
//!   up (down for an upward scroll), within [`SCROLL_TIMEOUT`];
//! - assertions wait, as Maestro's do, up to [`ASSERT_TIMEOUT`]: a visible
//!   element until it is found and displayed, a text until the element's
//!   `text` is it.
//!
//! The screen is read with `mobile: source` in its JSON form, whose nodes
//! carry the accessibility identifier as `rawIdentifier`. The release deletes
//! the session, stops Appium and the helper, which ends WebDriverAgent's
//! test, and ends WebDriverAgent's runner app on the phone, which outlives
//! its test manager session.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    BaselineConfig, check_phone_is_free, end_phone_runner, find_executable, json_document,
    last_line, version_from,
};
use crate::adapter::{Adapter, AdapterError, Clock, Detection, Left, Role, Sample, Scenario};
use crate::end_state::{Element, EndState, Reading};
use crate::flow::{Direction, Flow, Selector, Step};
use crate::harness::Subject;
use crate::process;
use crate::provenance::Probe;
use crate::target::{Platform, Target, TargetClass};

/// The tool's name in reports.
pub const NAME: &str = "webdriveragent";

/// The bundle identifier of the WebDriverAgent runner the README's build
/// installs: its test bundle is `dev.mobileagent.wda.runner`, under the
/// team's wildcard App ID.
pub const RUNNER_BUNDLE_ID: &str = "dev.mobileagent.wda.runner.xctrunner";

/// The port Appium listens on, on the loopback.
pub const APPIUM_PORT: u16 = 4723;

/// Why WebDriverAgent is skipped on a run without a physical iPhone.
pub const NO_IPHONE_NOTE: &str =
    "WebDriverAgent runs here on a physical iPhone only, and the run has none";

/// How long a scroll-to swipes before it fails: Maestro's
/// `scrollUntilVisible` timeout in the flows ([`super::maestro::SCROLL_TIMEOUT_MS`]).
pub const SCROLL_TIMEOUT: Duration = Duration::from_millis(super::maestro::SCROLL_TIMEOUT_MS);

/// How long an assertion waits, and a tap or a fill for its element.
pub const ASSERT_TIMEOUT: Duration = Duration::from_secs(10);

/// The W3C key of an element reference.
const ELEMENT: &str = "element-6066-11e4-a52e-4f735466cecf";

/// How long the helper has to print WebDriverAgent's URL: the test manager
/// stopped answering its handshake for a minute or two after runners were
/// ended mid-start (design D20), and a first start can wait 60 s on the
/// phone's "Enable UI Automation" approval.
const HELPER_TIMEOUT: Duration = Duration::from_secs(150);

/// How long Appium has to answer `/status` once started: about 2 s on the
/// host this was written on.
const APPIUM_TIMEOUT: Duration = Duration::from_secs(60);

/// The deadline of one WebDriver request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the release waits for Appium and the helper to exit once asked,
/// before it kills them.
const EXIT_WAIT: Duration = Duration::from_secs(10);

/// How long a polling step waits between two tries.
const POLL: Duration = Duration::from_millis(50);

/// The capabilities of the session on the iPhone `udid`, attached to the
/// WebDriverAgent `wda_url` serves: Appium neither builds nor launches
/// WebDriverAgent, captures no log, which on iOS 18 and later takes the
/// RemoteXPC tunnel, resets nothing and keeps the session for the run.
pub fn capabilities(udid: &str, wda_url: &str) -> Value {
    json!({"capabilities": {"alwaysMatch": {
        "platformName": "iOS",
        "appium:automationName": "XCUITest",
        "appium:udid": udid,
        "appium:webDriverAgentUrl": wda_url,
        "appium:skipLogCapture": true,
        "appium:noReset": true,
        "appium:newCommandTimeout": 0
    }}})
}

/// The localhost URL in a line the helper printed, `wda <url>`.
pub fn helper_url(line: &str) -> Option<String> {
    line.trim()
        .strip_prefix("wda ")
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

/// Arguments for `appium` serving on the loopback at `port`, logging to
/// `log`.
pub fn appium_args(port: u16, log: &Path) -> Vec<OsString> {
    vec![
        "--address".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string().into(),
        "--log-no-colors".into(),
        "--log".into(),
        log.as_os_str().to_owned(),
    ]
}

/// The XCUITest driver's version and install path in what `appium driver
/// list --installed --json` printed.
pub fn driver(stdout: &str) -> Result<(String, Option<PathBuf>), String> {
    let missing = |why: String| format!("Appium's XCUITest driver is not installed: {why}");
    let listed = json_document(stdout).ok_or_else(|| {
        missing(format!(
            "appium driver list printed no JSON{}",
            last_line(stdout)
                .map(|line| format!(": {line}"))
                .unwrap_or_default()
        ))
    })?;
    let driver = &listed["xcuitest"];
    if driver.is_null() {
        return Err(missing(
            "appium driver list --installed names no xcuitest".into(),
        ));
    }
    let version = driver["version"]
        .as_str()
        .ok_or_else(|| missing("appium names xcuitest without a version".into()))?;
    Ok((
        version.to_owned(),
        driver["installPath"].as_str().map(PathBuf::from),
    ))
}

/// Reads the tree `mobile: source` gives in its JSON form: every node
/// counts, an element's identifier is its `rawIdentifier` and its text its
/// `value`, else its `label`.
pub fn read_source(tree: &Value) -> Reading {
    let text = |node: &Value, key: &str| {
        node[key]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut reading = Reading::default();
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        reading.elements.push(Element {
            id: text(node, "rawIdentifier"),
            text: text(node, "value").or_else(|| text(node, "label")),
        });
        if let Some(children) = node["children"].as_array() {
            stack.extend(children.iter().rev());
        }
    }
    reading.nodes = u32::try_from(reading.elements.len()).unwrap_or(u32::MAX);
    reading
}

/// What WebDriverAgent's adapter still runs on the host for the iPhone
/// `udid` after its release: its helper and Appium on `port`. `programs` is
/// the host's processes as `ps` lists them with
/// [`super::agent_device::PS_PROGRAMS`], and `details` lists those whose
/// pids it is given with their command lines. The release stops both
/// itself, so either is a stray.
pub fn left_running(
    programs: &str,
    udid: &str,
    port: u16,
    details: impl FnOnce(&[u32]) -> String,
) -> Vec<Left> {
    let candidates: Vec<u32> = programs
        .lines()
        .filter_map(pid_and_rest)
        .filter(|(_, program)| {
            matches!(
                program.rsplit('/').next(),
                Some("mobile-agent-bench" | "node")
            )
        })
        .map(|(pid, _)| pid)
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let helper = format!("--udid {udid}");
    let port = format!("--port {port}");
    let mut left: Vec<Left> = details(&candidates)
        .lines()
        .filter_map(pid_and_rest)
        .filter(|(pid, _)| candidates.contains(pid))
        .filter_map(|(pid, line)| {
            let name = if line.contains(" wda-session ") && has_word(line, &helper) {
                "helper"
            } else if line.contains("appium ") && has_word(line, &port) {
                "Appium"
            } else {
                return None;
            };
            Some(Left {
                name: name.to_owned(),
                pid,
                stray: true,
                device: false,
            })
        })
        .collect();
    left.sort_by_key(|left| left.pid);
    left
}

/// A line `ps` printed for `pid=,…`, as the pid and the rest.
fn pid_and_rest(line: &str) -> Option<(u32, &str)> {
    let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
    Some((pid.parse().ok()?, rest.trim_start()))
}

/// Whether `line` holds `words` followed by the end or a space.
fn has_word(line: &str, words: &str) -> bool {
    line.match_indices(words).any(|(at, _)| {
        line[at + words.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    })
}

/// A minimal HTTP/1.1 client for the WebDriver requests to Appium on the
/// loopback, one connection per request.
mod http {
    use super::*;

    /// Sends `method path`, with `body` as JSON, to the loopback's `port`,
    /// and returns the reply's status and its JSON.
    pub(super) fn call(
        port: u16,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(u16, Value), String> {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
            .map_err(|e| format!("cannot reach 127.0.0.1:{port}: {e}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .map_err(|e| format!("cannot set a deadline on 127.0.0.1:{port}: {e}"))?;
        let body = body.map(Value::to_string).unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
             Content-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
        .map_err(|e| format!("cannot send {method} {path}: {e}"))?;
        let mut reply = Vec::new();
        stream
            .read_to_end(&mut reply)
            .map_err(|e| format!("no reply to {method} {path}: {e}"))?;
        let reply = String::from_utf8_lossy(&reply);
        let (head, body) = reply
            .split_once("\r\n\r\n")
            .ok_or_else(|| format!("{method} {path} got no HTTP reply"))?;
        let status: u16 = head
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| format!("{method} {path} got no HTTP status"))?;
        let chunked = head.lines().any(|line| {
            line.to_ascii_lowercase()
                .starts_with("transfer-encoding: chunked")
        });
        let body = if chunked {
            unchunk(body)
        } else {
            body.to_owned()
        };
        let value = serde_json::from_str(body.trim()).unwrap_or(Value::Null);
        Ok((status, value))
    }

    /// A chunked body put back together.
    fn unchunk(mut body: &str) -> String {
        let mut whole = String::new();
        while let Some((size, rest)) = body.split_once("\r\n") {
            let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
                break;
            };
            if size == 0 || rest.len() < size {
                break;
            }
            whole.push_str(&rest[..size]);
            body = rest[size..].trim_start_matches("\r\n");
        }
        whole
    }
}

/// What the adapter runs for the iPhone it drives: the helper holding
/// WebDriverAgent's test, Appium, and Appium's session.
struct Running {
    target: String,
    helper: Child,
    /// The helper's standard input, which the release closes.
    stdin: Option<ChildStdin>,
    appium: Child,
    session: String,
}

/// WebDriverAgent through Appium's XCUITest driver.
pub struct WebDriverAgent {
    config: BaselineConfig,
    /// `appium`, once detected.
    appium: Option<PathBuf>,
    running: Option<Running>,
}

impl WebDriverAgent {
    pub fn new(config: BaselineConfig) -> Self {
        WebDriverAgent {
            config,
            appium: None,
            running: None,
        }
    }

    /// A WebDriver request to Appium: the reply's `value`, or the W3C error
    /// it carries.
    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let (status, reply) =
            http::call(self.config.appium_port, method, path, body, REQUEST_TIMEOUT)?;
        if status == 200 {
            return Ok(reply["value"].clone());
        }
        let error = &reply["value"];
        Err(format!(
            "{method} {path} failed with {status}: {}{}",
            error["error"].as_str().unwrap_or("an error"),
            error["message"]
                .as_str()
                .map(|m| format!(": {}", m.lines().next().unwrap_or_default()))
                .unwrap_or_default()
        ))
    }

    /// The session's path for `rest`.
    fn path(&self, rest: &str) -> Result<String, String> {
        let running = self
            .running
            .as_ref()
            .ok_or("WebDriverAgent has no session")?;
        Ok(format!("/session/{}{rest}", running.session))
    }

    /// Runs `script`, one of Appium's `mobile:` commands, with `args`.
    fn execute(&self, script: &str, args: Value) -> Result<Value, String> {
        self.request(
            "POST",
            &self.path("/execute/sync")?,
            Some(&json!({"script": script, "args": [args]})),
        )
    }

    /// The element `selector` finds, if one is on the screen.
    fn find(&self, selector: &Selector) -> Result<Option<String>, String> {
        let body = match selector {
            Selector::Id(id) => json!({"using": "accessibility id", "value": id}),
            Selector::Text(text) => {
                let quoted = text.replace('\\', "\\\\").replace('"', "\\\"");
                json!({"using": "-ios predicate string",
                       "value": format!("label == \"{quoted}\" OR value == \"{quoted}\"")})
            }
        };
        let path = self.path("/element")?;
        let (status, reply) = http::call(
            self.config.appium_port,
            "POST",
            &path,
            Some(&body),
            REQUEST_TIMEOUT,
        )?;
        match status {
            200 => reply["value"][ELEMENT]
                .as_str()
                .map(|id| Some(id.to_owned()))
                .ok_or_else(|| format!("POST {path} answered no element")),
            404 if reply["value"]["error"] == "no such element" => Ok(None),
            _ => Err(format!(
                "POST {path} failed with {status}: {}",
                reply["value"]["message"].as_str().unwrap_or("an error")
            )),
        }
    }

    /// Whether the element `id` is displayed.
    fn displayed(&self, id: &str) -> Result<bool, String> {
        Ok(self.request(
            "GET",
            &self.path(&format!("/element/{id}/displayed"))?,
            None,
        )? == true)
    }

    /// The element `selector` finds once it is on the screen, within
    /// `timeout`, displayed when `visible`.
    fn wait_for(
        &self,
        selector: &Selector,
        visible: bool,
        timeout: Duration,
    ) -> Result<String, String> {
        let started = Instant::now();
        loop {
            if let Some(id) = self.find(selector)?
                && (!visible || self.displayed(&id)?)
            {
                return Ok(id);
            }
            if started.elapsed() >= timeout {
                return Err(format!(
                    "{} was not {} within {} s",
                    describe(selector),
                    if visible { "visible" } else { "found" },
                    timeout.as_secs()
                ));
            }
            thread::sleep(POLL);
        }
    }

    /// One run of `flow`'s steps on the app `app_id`, timed from the first
    /// request's start to the last one's end; the sample notes the swipes
    /// its scroll-to's took.
    fn run_steps(&self, flow: &Flow, app_id: &str) -> Result<Sample, String> {
        let started = Instant::now();
        let mut swipes = 0_u32;
        for step in &flow.steps {
            match step {
                Step::Launch => {
                    self.execute("mobile: terminateApp", json!({"bundleId": app_id}))?;
                    self.execute("mobile: launchApp", json!({"bundleId": app_id}))?;
                }
                Step::Tap(selector) => {
                    let id = self.wait_for(selector, false, ASSERT_TIMEOUT)?;
                    self.request(
                        "POST",
                        &self.path(&format!("/element/{id}/click"))?,
                        Some(&json!({})),
                    )?;
                }
                Step::Fill { target, text } => {
                    let id = self.wait_for(target, false, ASSERT_TIMEOUT)?;
                    self.request(
                        "POST",
                        &self.path(&format!("/element/{id}/click"))?,
                        Some(&json!({})),
                    )?;
                    self.request(
                        "POST",
                        &self.path(&format!("/element/{id}/value"))?,
                        Some(&json!({"text": text})),
                    )?;
                }
                Step::ScrollTo { target, direction } => {
                    let toward = match direction {
                        Direction::Down => "up",
                        Direction::Up => "down",
                    };
                    let begun = Instant::now();
                    loop {
                        if let Some(id) = self.find(target)?
                            && self.displayed(&id)?
                        {
                            break;
                        }
                        if begun.elapsed() >= SCROLL_TIMEOUT {
                            return Err(format!(
                                "{} was not visible after {swipes} swipes in {} s",
                                describe(target),
                                SCROLL_TIMEOUT.as_secs()
                            ));
                        }
                        self.execute("mobile: swipe", json!({"direction": toward}))?;
                        swipes += 1;
                    }
                }
                Step::AssertVisible(selector) => {
                    self.wait_for(selector, true, ASSERT_TIMEOUT)?;
                }
                Step::AssertText { id, text } => {
                    let begun = Instant::now();
                    loop {
                        let shown = match self.find(&Selector::Id(id.clone()))? {
                            Some(element) => self
                                .request(
                                    "GET",
                                    &self.path(&format!("/element/{element}/text"))?,
                                    None,
                                )?
                                .as_str()
                                .map(str::to_owned),
                            None => None,
                        };
                        if shown.as_deref() == Some(text.as_str()) {
                            break;
                        }
                        if begun.elapsed() >= ASSERT_TIMEOUT {
                            return Err(format!(
                                "{id} showed {} rather than {text:?} after {} s",
                                shown.map_or_else(|| "nothing".to_owned(), |s| format!("{s:?}")),
                                ASSERT_TIMEOUT.as_secs()
                            ));
                        }
                        thread::sleep(POLL);
                    }
                }
            }
        }
        Ok(Sample {
            elapsed: started.elapsed(),
            clock: Clock::Wall,
            nodes: None,
            detail: (swipes > 0).then(|| format!("{swipes} swipes")),
        })
    }

    /// The screen, as `mobile: source` gives it in its JSON form.
    fn read_screen(&self) -> Result<Reading, String> {
        let tree = self.execute("mobile: source", json!({"format": "json"}))?;
        let tree = match tree {
            Value::String(text) => serde_json::from_str(&text)
                .map_err(|e| format!("mobile: source gave no JSON tree: {e}"))?,
            tree => tree,
        };
        if !tree.is_object() {
            return Err("mobile: source gave no tree".to_owned());
        }
        Ok(read_source(&tree))
    }

    /// Starts what the adapter runs for the iPhone `target`, unless it runs
    /// already: once no other tool's runner holds the phone, the helper,
    /// which starts WebDriverAgent through the phone's test manager, then
    /// Appium, then Appium's session attached to WebDriverAgent.
    fn ensure(&mut self, target: &Target) -> Result<(), AdapterError> {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.target == target.id)
        {
            return Ok(());
        }
        let appium = self
            .appium
            .clone()
            .ok_or_else(|| AdapterError::new(format!("{NAME} was not detected")))?;
        check_phone_is_free(&self.config.search_path, target, NAME)?;
        fs::create_dir_all(&self.config.work_dir).map_err(|e| {
            AdapterError::new(format!(
                "cannot create {}: {e}",
                self.config.work_dir.display()
            ))
        })?;
        let (mut helper, stdin, url) = self.start_helper(target).map_err(AdapterError::new)?;
        let mut appium = match self.start_appium(&appium, target) {
            Ok(appium) => appium,
            Err(e) => {
                let _ = helper.kill();
                let _ = helper.wait();
                return Err(AdapterError::new(e));
            }
        };
        let session = self
            .request("POST", "/session", Some(&capabilities(&target.id, &url)))
            .and_then(|value| {
                value["sessionId"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "Appium's new session has no id".to_owned())
            });
        match session {
            Ok(session) => {
                self.running = Some(Running {
                    target: target.id.clone(),
                    helper,
                    stdin: Some(stdin),
                    appium,
                    session,
                });
                Ok(())
            }
            Err(e) => {
                for child in [&mut appium, &mut helper] {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(AdapterError::new(format!(
                    "Appium did not attach to WebDriverAgent: {e}"
                )))
            }
        }
    }

    /// The helper started for `target`, its standard input, and the URL it
    /// printed once WebDriverAgent answered.
    fn start_helper(&self, target: &Target) -> Result<(Child, ChildStdin, String), String> {
        let (program, args) = self
            .config
            .wda_helper
            .split_first()
            .ok_or("no helper to start WebDriverAgent with")?;
        let log = self
            .config
            .work_dir
            .join(format!("wda-helper-{}.log", target.id));
        let stderr =
            File::create(&log).map_err(|e| format!("cannot create {}: {e}", log.display()))?;
        let mut helper = Command::new(program)
            .args(args)
            .args(["--udid", &target.id, "--bundle-id", RUNNER_BUNDLE_ID])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", Path::new(program).display()))?;
        let stdin = helper
            .stdin
            .take()
            .ok_or("the helper has no standard input")?;
        let stdout = helper
            .stdout
            .take()
            .ok_or("the helper has no standard output")?;
        let (lines, received) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let started = Instant::now();
        let failed = |helper: &mut Child, why: String| {
            let _ = helper.kill();
            let _ = helper.wait();
            let said = fs::read_to_string(&log).unwrap_or_default();
            match last_line(&said) {
                Some(line) => format!("{why}: {line}"),
                None => why,
            }
        };
        loop {
            let left = HELPER_TIMEOUT.saturating_sub(started.elapsed());
            match received.recv_timeout(left) {
                Ok(line) => {
                    if let Some(url) = helper_url(&line) {
                        return Ok((helper, stdin, url));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(failed(
                        &mut helper,
                        "the helper ended before WebDriverAgent answered".to_owned(),
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(failed(
                        &mut helper,
                        format!(
                            "WebDriverAgent did not answer within {} s",
                            HELPER_TIMEOUT.as_secs()
                        ),
                    ));
                }
            }
        }
    }

    /// Appium started on the configured port, once it answers `/status`.
    fn start_appium(&self, appium: &Path, target: &Target) -> Result<Child, String> {
        let log = self
            .config
            .work_dir
            .join(format!("appium-{}.log", target.id));
        let mut child = Command::new(appium)
            .args(appium_args(self.config.appium_port, &log))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", appium.display()))?;
        let started = Instant::now();
        loop {
            if let Ok((200, _)) = http::call(
                self.config.appium_port,
                "GET",
                "/status",
                None,
                Duration::from_secs(5),
            ) {
                return Ok(child);
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "Appium exited with {status} before it served; its log is {}",
                    log.display()
                ));
            }
            if started.elapsed() >= APPIUM_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Appium did not serve on port {} within {} s",
                    self.config.appium_port,
                    APPIUM_TIMEOUT.as_secs()
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// `id=…` or `text=…`, for errors.
fn describe(selector: &Selector) -> String {
    match selector {
        Selector::Id(id) => format!("id={id}"),
        Selector::Text(text) => format!("text={text:?}"),
    }
}

/// Waits up to [`EXIT_WAIT`] for `child` to exit, then kills it; returns
/// whether it exited by itself.
fn reap(child: &mut Child) -> bool {
    let started = Instant::now();
    while started.elapsed() < EXIT_WAIT {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        thread::sleep(POLL);
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

impl Adapter for WebDriverAgent {
    fn name(&self) -> &str {
        NAME
    }

    fn role(&self) -> Role {
        Role::Baseline
    }

    /// Available with `appium` on the search path and its XCUITest driver
    /// installed, on a run with a physical iPhone; the version is
    /// WebDriverAgent's, from the driver's copy, with Appium's and the
    /// driver's.
    fn detect(&mut self) -> Detection {
        let targets = &self.config.targets;
        if !targets.is_empty() && !targets.contains(&TargetClass::IosDevice) {
            return Detection::Absent {
                note: NO_IPHONE_NOTE.to_owned(),
            };
        }
        let Some(appium) = find_executable("appium", &self.config.search_path) else {
            return Detection::Absent {
                note: "appium not found on PATH".to_owned(),
            };
        };
        let run = |args: &[&str]| {
            process::run(
                Command::new(&appium).args(args),
                self.config.version_timeout,
            )
            .ok()
            .filter(|finished| finished.status.success())
            .map(|finished| finished.stdout)
        };
        let appium_version = run(&["--version"]).and_then(|out| version_from(&out));
        let listed = run(&["driver", "list", "--installed", "--json"]).unwrap_or_default();
        let (driver_version, installed) = match driver(&listed) {
            Ok(found) => found,
            Err(note) => return Detection::Absent { note },
        };
        let wda_version = installed
            .map(|dir| dir.join("node_modules/appium-webdriveragent/package.json"))
            .and_then(|file| fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|package| package["version"].as_str().map(str::to_owned));
        self.appium = Some(appium);
        Detection::Available {
            version: Some(format!(
                "{} (Appium {}, XCUITest driver {driver_version})",
                wda_version.as_deref().unwrap_or("unknown"),
                appium_version.as_deref().unwrap_or("unknown")
            )),
        }
    }

    /// Flows, on a physical iPhone.
    fn supports(&self, target: &Target, _scenario: &Scenario, subject: &Subject) -> bool {
        matches!(subject, Subject::Flow(_)) && target.class == TargetClass::IosDevice
    }

    fn prepare(
        &mut self,
        target: &Target,
        _scenario: &Scenario,
        _subject: &Subject,
    ) -> Result<(), AdapterError> {
        self.ensure(target)
    }

    /// Runs the flow, timed, then, for a flow that records its node count,
    /// reads the source once more outside the timing.
    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        self.ensure(target)?;
        let app = app_id(flow)?;
        let mut sample = self.run_steps(flow, app).map_err(AdapterError::new)?;
        if flow.count_nodes {
            let reading = self.read_screen().map_err(|e| {
                AdapterError::new(format!(
                    "the flow ran, but its node count was not read: {e}"
                ))
            })?;
            sample.nodes = Some(reading.nodes);
        }
        Ok(sample)
    }

    fn end_state(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        self.ensure(target)?;
        let reading = self
            .read_screen()
            .map_err(|e| AdapterError::new(format!("{NAME} read no screen: {e}")))?;
        Ok(flow.golden.observe(&reading.elements))
    }

    /// One run of the flow's steps, then one read of the screen.
    fn check_flow(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        self.ensure(target)?;
        self.run_steps(flow, app_id(flow)?)
            .map_err(AdapterError::new)?;
        self.end_state(target, flow)
    }

    /// Deletes Appium's session, stops Appium with SIGTERM and the helper by
    /// closing its standard input, which ends WebDriverAgent's test, and
    /// ends WebDriverAgent's runner app on the iPhone, which outlives its
    /// test manager session, through CoreDevice ([`end_phone_runner`]).
    fn release(
        &mut self,
        target: &Target,
        probe: &dyn Probe,
    ) -> Result<Option<String>, AdapterError> {
        let Some(mut running) = self.running.take_if(|running| running.target == target.id) else {
            return Ok(None);
        };
        let mut done = Vec::new();
        let mut errors = Vec::new();
        match http::call(
            self.config.appium_port,
            "DELETE",
            &format!("/session/{}", running.session),
            None,
            REQUEST_TIMEOUT,
        ) {
            Ok((200, _)) => done.push("deleted its Appium session".to_owned()),
            Ok((status, reply)) => errors.push(format!(
                "Appium did not delete its session: {status} {}",
                reply["value"]["message"].as_str().unwrap_or_default()
            )),
            Err(e) => errors.push(format!("Appium did not delete its session: {e}")),
        }
        let _ = Command::new("kill")
            .args(["-TERM", &running.appium.id().to_string()])
            .status();
        let appium_exited = reap(&mut running.appium);
        drop(running.stdin.take());
        let helper_exited = reap(&mut running.helper);
        if !appium_exited {
            errors.push(format!(
                "Appium outlived SIGTERM by {} s and was killed",
                EXIT_WAIT.as_secs()
            ));
        }
        if !helper_exited {
            errors.push(format!(
                "the helper outlived its closed input by {} s and was killed",
                EXIT_WAIT.as_secs()
            ));
        }
        done.push("stopped Appium and the helper holding WebDriverAgent's test".to_owned());
        match end_phone_runner(probe, &target.id, NAME) {
            Ok(true) => done.push("ended WebDriverAgent's runner app on the iPhone".to_owned()),
            Ok(false) => {}
            Err(e) => errors.push(e),
        }
        if !errors.is_empty() {
            return Err(AdapterError::new(errors.join("; ")));
        }
        Ok(Some(match done.as_slice() {
            [only] => only.clone(),
            [first, second] => format!("{first} and {second}"),
            [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
            [] => unreachable!("the release always stops Appium"),
        }))
    }

    /// What the adapter still runs for the iPhone on the host: its helper
    /// and Appium ([`left_running`]); WebDriverAgent's runner app on the
    /// phone is the hand-off's to look for.
    fn left_running(&self, target: &Target, probe: &dyn Probe) -> Result<Vec<Left>, String> {
        if target.class != TargetClass::IosDevice {
            return Ok(Vec::new());
        }
        let programs = probe
            .output("ps", &super::agent_device::PS_PROGRAMS)
            .ok_or("cannot list the host's processes with ps")?;
        Ok(left_running(
            &programs,
            &target.id,
            self.config.appium_port,
            |pids| {
                let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
                probe
                    .output("ps", &super::agent_device::ps_details(&pids.join(",")))
                    .unwrap_or_default()
            },
        ))
    }
}

/// The iOS bundle identifier of the app `flow` drives.
fn app_id(flow: &Flow) -> Result<&'static str, AdapterError> {
    flow.fixture.app_id(Platform::Ios).ok_or_else(|| {
        AdapterError::new(format!("the {} fixture does not run on iOS", flow.fixture))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::end_state::Element;
    use std::path::Path;

    #[test]
    fn the_session_attaches_to_the_running_webdriveragent_and_launches_nothing() {
        let caps = capabilities("PHONE-1", "http://127.0.0.1:53546");
        assert_eq!(
            caps,
            json!({"capabilities": {"alwaysMatch": {
                "platformName": "iOS",
                "appium:automationName": "XCUITest",
                "appium:udid": "PHONE-1",
                "appium:webDriverAgentUrl": "http://127.0.0.1:53546",
                "appium:skipLogCapture": true,
                "appium:noReset": true,
                "appium:newCommandTimeout": 0
            }}})
        );
    }

    #[test]
    fn the_helper_s_line_names_webdriveragent_s_localhost_url() {
        assert_eq!(
            helper_url("wda http://127.0.0.1:53546\n").as_deref(),
            Some("http://127.0.0.1:53546")
        );
        assert_eq!(helper_url("starting"), None);
        assert_eq!(helper_url("wda "), None);
    }

    #[test]
    fn appium_serves_on_the_loopback_and_logs_to_a_file() {
        let args: Vec<String> = appium_args(4723, Path::new("/out/flows/appium-PHONE-1.log"))
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "--address",
                "127.0.0.1",
                "--port",
                "4723",
                "--log-no-colors",
                "--log",
                "/out/flows/appium-PHONE-1.log"
            ]
        );
    }

    #[test]
    fn the_xcuitest_driver_is_read_from_appium_s_installed_list() {
        let listed = r#"{
    "xcuitest": {
        "automationName": "XCUITest",
        "pkgName": "appium-xcuitest-driver",
        "version": "12.16.0",
        "installType": "npm",
        "installPath": "/Users/<user>/.appium/node_modules/appium-xcuitest-driver",
        "installed": true
    }
}"#;
        assert_eq!(
            driver(listed),
            Ok((
                "12.16.0".to_owned(),
                Some(PathBuf::from(
                    "/Users/<user>/.appium/node_modules/appium-xcuitest-driver"
                ))
            ))
        );
        let error = driver("{}").unwrap_err();
        assert!(error.contains("XCUITest driver"), "{error}");
        assert!(driver("- Listing installed drivers").is_err());
    }

    #[test]
    fn a_source_gives_identifiers_values_or_labels_and_its_node_count() {
        let tree = json!({
            "type": "Application", "label": "FixtureSwiftUI", "rawIdentifier": null, "value": null,
            "children": [{
                "type": "Window", "label": null, "rawIdentifier": null, "value": null,
                "children": [
                    {"type": "TextField", "label": "Username", "rawIdentifier": "login-username", "value": "agent"},
                    {"type": "StaticText", "label": "Welcome, agent", "rawIdentifier": "login-status", "value": "Welcome, agent", "children": []},
                    {"type": "Button", "label": "List", "rawIdentifier": "tab-list", "value": ""},
                    {"type": "Image", "label": "List", "rawIdentifier": "list.bullet", "value": null}
                ]
            }]
        });
        assert_eq!(
            read_source(&tree),
            Reading {
                elements: vec![
                    Element::new(None, Some("FixtureSwiftUI")),
                    Element::new(None, None),
                    Element::new(Some("login-username"), Some("agent")),
                    Element::new(Some("login-status"), Some("Welcome, agent")),
                    Element::new(Some("tab-list"), Some("List")),
                    Element::new(Some("list.bullet"), Some("List")),
                ],
                nodes: 6,
            }
        );
    }

    /// The host's processes while the adapter drives `PHONE-1` with Appium on
    /// 4723: its helper and Appium, and the same for another phone and port.
    const PROGRAMS: &str = "    1 /sbin/launchd
  601 /work/target/release/mobile-agent-bench
  602 /opt/node/bin/node
  603 /work/target/release/mobile-agent-bench
  604 /opt/node/bin/node
  605 /bin/zsh";

    const DETAILS: &str = "  601 /work/target/release/mobile-agent-bench wda-session --udid PHONE-1 --bundle-id dev.mobileagent.wda.runner.xctrunner
  602 node /opt/node/bin/appium --address 127.0.0.1 --port 4723 --log-no-colors --log /out/flows/appium-PHONE-1.log
  603 /work/target/release/mobile-agent-bench wda-session --udid PHONE-2 --bundle-id dev.mobileagent.wda.runner.xctrunner
  604 node /opt/node/bin/appium --address 127.0.0.1 --port 4800 --log-no-colors --log /out/flows/appium-PHONE-2.log";

    #[test]
    fn what_is_left_running_is_the_helper_and_appium_of_the_phone_and_port() {
        let left = left_running(PROGRAMS, "PHONE-1", 4723, |pids| {
            assert_eq!(pids, [601, 602, 603, 604]);
            DETAILS.to_owned()
        });
        let named: Vec<String> = left.iter().map(Left::to_string).collect();
        assert_eq!(named, ["helper (pid 601)", "Appium (pid 602)"]);
        // The release stops both itself, so either is a stray once it ran.
        assert!(left.iter().all(|l| l.stray && !l.device));
        assert!(
            left_running("    1 /sbin/launchd", "PHONE-1", 4723, |_| panic!("asked")).is_empty()
        );
    }
}
