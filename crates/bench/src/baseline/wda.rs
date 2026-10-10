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
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::BaselineConfig;
use crate::adapter::{Adapter, AdapterError, Detection, Left, Role, Sample, Scenario};
use crate::end_state::{EndState, Reading};
use crate::flow::Flow;
use crate::harness::Subject;
use crate::provenance::Probe;
use crate::target::Target;

/// The tool's name in reports.
pub const NAME: &str = "webdriveragent";

/// The bundle identifier of the WebDriverAgent runner the README's build
/// installs: its test bundle is `dev.mobileagent.wda.runner`, under the
/// team's wildcard App ID.
pub const RUNNER_BUNDLE_ID: &str = "dev.mobileagent.wda.runner.xctrunner";

/// The port Appium listens on, on the loopback.
pub const APPIUM_PORT: u16 = 4723;

/// Why WebDriverAgent is skipped on a run without a physical iPhone.
pub const NO_IPHONE_NOTE: &str = "";

/// How long a scroll-to swipes before it fails: Maestro's
/// `scrollUntilVisible` timeout in the flows ([`super::maestro::SCROLL_TIMEOUT_MS`]).
pub const SCROLL_TIMEOUT: Duration = Duration::from_millis(super::maestro::SCROLL_TIMEOUT_MS);

/// How long an assertion waits.
pub const ASSERT_TIMEOUT: Duration = Duration::from_secs(10);

/// The W3C key of an element reference.
const ELEMENT: &str = "element-6066-11e4-a52e-4f735466cecf";

/// The capabilities of the session on the iPhone `udid`, attached to the
/// WebDriverAgent `wda_url` serves.
pub fn capabilities(udid: &str, wda_url: &str) -> Value {
    let _ = (udid, wda_url, ELEMENT);
    json!({})
}

/// The localhost URL in a line the helper printed, `wda <url>`.
pub fn helper_url(line: &str) -> Option<String> {
    let _ = line;
    None
}

/// Arguments for `appium` serving on the loopback at `port`, logging to
/// `log`.
pub fn appium_args(port: u16, log: &std::path::Path) -> Vec<OsString> {
    let _ = (port, log);
    Vec::new()
}

/// The XCUITest driver's version and install path in what `appium driver
/// list --installed --json` printed.
pub fn driver(stdout: &str) -> Result<(String, Option<PathBuf>), String> {
    let _ = stdout;
    Err(String::new())
}

/// Reads the tree `mobile: source` gives in its JSON form: every node
/// counts, an element's identifier is its `rawIdentifier` and its text its
/// `value`, else its `label`.
pub fn read_source(tree: &Value) -> Reading {
    let _ = tree;
    Reading::default()
}

/// What WebDriverAgent's adapter still runs on the host for the iPhone
/// `udid` after its release: its helper and Appium on `port`. `programs` is
/// the host's processes as `ps` lists them with
/// [`super::agent_device::PS_PROGRAMS`], and `details` lists those whose
/// pids it is given with their command lines.
pub fn left_running(
    programs: &str,
    udid: &str,
    port: u16,
    details: impl FnOnce(&[u32]) -> String,
) -> Vec<Left> {
    let _ = (programs, udid, port, details);
    Vec::new()
}

/// WebDriverAgent through Appium's XCUITest driver.
pub struct WebDriverAgent {
    config: BaselineConfig,
}

impl WebDriverAgent {
    pub fn new(config: BaselineConfig) -> Self {
        WebDriverAgent { config }
    }
}

impl Adapter for WebDriverAgent {
    fn name(&self) -> &str {
        NAME
    }

    fn role(&self) -> Role {
        Role::Baseline
    }

    fn detect(&mut self) -> Detection {
        let _ = &self.config;
        Detection::Absent {
            note: String::new(),
        }
    }

    fn supports(&self, _target: &Target, _scenario: &Scenario, _subject: &Subject) -> bool {
        false
    }

    fn run_flow(&mut self, _target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        Err(AdapterError::new(format!(
            "{NAME} did not run {}",
            flow.name
        )))
    }

    fn end_state(&mut self, _target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        Err(AdapterError::new(format!(
            "{NAME} did not read {}",
            flow.name
        )))
    }

    fn release(
        &mut self,
        _target: &Target,
        _probe: &dyn Probe,
    ) -> Result<Option<String>, AdapterError> {
        Ok(None)
    }

    fn left_running(&self, _target: &Target, _probe: &dyn Probe) -> Result<Vec<Left>, String> {
        Ok(Vec::new())
    }
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
