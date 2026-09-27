//! Path selection (android-driver requirement "Emulator fast path"; design
//! D7): whether taps, swipes and frames go through the emulator's controller
//! or the agent, and the status that says which.
//!
//! - On an emulator whose control endpoint resolves and answers, taps,
//!   swipes and the frame stream use the controller.
//! - Without one, input goes through the agent, there is no frame stream,
//!   and the status carries the warning that names the launch flag and the
//!   console command that enables the endpoint without a restart.
//! - Keys and text always go through the agent: the controller accepts and
//!   drops keys unless the AVD declares a keyboard, and even then it cannot
//!   say whether the platform accepted them. The status reports whether the
//!   AVD declares one.
//! - Screenshots go through the agent, which returns them upright in every
//!   display orientation.

use std::fmt;

use super::ControllerLink;
use crate::emulator::{self, Endpoint, Options};

/// Whether to use the emulator's controller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ControllerChoice {
    /// Resolve the endpoint from the emulator's discovery file and use it
    /// when it answers.
    #[default]
    Auto,
    /// Never use it.
    Off,
    /// Use this endpoint.
    Endpoint(Endpoint),
}

/// Where a kind of input or output goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// The emulator's gRPC controller: the fast path.
    Controller,
    /// The resident agent.
    Agent,
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Path::Controller => "emulator controller",
            Path::Agent => "agent",
        })
    }
}

/// The paths chosen for a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub emulator: bool,
    /// Taps, long presses and swipes.
    pub input: Path,
    /// The frame stream; `None` when there is none, as on devices and
    /// emulators without the controller.
    pub frames: Option<Path>,
    pub screenshots: Path,
    /// Keys and text.
    pub keys: Path,
    /// Whether the AVD declares a keyboard (`hw.keyboard`), as the controller
    /// reports it; `None` without a controller.
    pub keyboard: Option<bool>,
    /// The controller's address, when it is used.
    pub endpoint: Option<String>,
    /// Why the fast path is not used on an emulator, and how to enable it.
    pub warning: Option<String>,
}

impl Paths {
    /// Everything through the agent; `warning` says why, on an emulator.
    pub fn agent(emulator: bool, warning: Option<String>) -> Paths {
        let _ = (emulator, warning);
        todo!("task 5.5")
    }

    /// Taps, swipes and frames through `link`'s controller at `endpoint`.
    pub fn controller(link: &ControllerLink, endpoint: Option<&str>) -> Paths {
        let _ = (link, endpoint);
        todo!("task 5.5")
    }
}

/// The controller to use, if any, and the paths that follow.
#[derive(Debug)]
pub struct Selection {
    pub link: Option<ControllerLink>,
    pub paths: Paths,
}

/// Selects the paths for the target `serial`, an emulator when `emulator`
/// says so: the controller as `choice` says when it connects within
/// `options`' deadlines, else the agent, with the warning on an emulator
/// whose controller was wanted.
pub fn select_paths(
    serial: &str,
    emulator: bool,
    choice: &ControllerChoice,
    options: Options,
) -> Selection {
    let _ = (serial, emulator, choice, options, emulator::resolve);
    todo!("task 5.5")
}

/// What a session reports about its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub serial: String,
    /// The agent name and version its hello carried.
    pub agent: String,
    pub protocol: u16,
    pub pid: u32,
    pub socket: String,
    /// Whether the agent was reused rather than launched.
    pub reused: bool,
    /// The paths as they are now: input moves to the agent when the
    /// controller fails during the session.
    pub paths: Paths,
    /// Inputs sent through the controller and through the agent.
    pub inputs: (u64, u64),
    pub frames_received: u64,
    /// Why the controller stopped taking input, once it failed.
    pub controller_error: Option<String>,
    /// The last failure of the frame stream.
    pub frame_error: Option<String>,
    /// Why the connection to the agent closed, once it has.
    pub disconnected: Option<String>,
}

impl Status {
    /// The status as lines of text: the agent, then one line per path, then
    /// the warnings.
    pub fn lines(&self) -> Vec<String> {
        todo!("task 5.5")
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lines().join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn quick() -> Options {
        Options {
            deadline: Duration::from_millis(500),
            connect_deadline: Duration::from_millis(500),
        }
    }

    /// An endpoint on a port nothing listens on.
    fn closed_endpoint(serial: &str) -> Endpoint {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        Endpoint {
            serial: serial.into(),
            uri: format!("http://127.0.0.1:{port}"),
            token: Some("token".into()),
            console_port: Some(5554),
            discovery_file: None,
        }
    }

    fn status(paths: Paths) -> Status {
        Status {
            serial: "emulator-5554".into(),
            agent: "mobile-agent-android 0.1.0".into(),
            protocol: 1,
            pid: 812,
            socket: "mobile-agent-0.1.0-p1-00ff".into(),
            reused: false,
            paths,
            inputs: (0, 0),
            frames_received: 0,
            controller_error: None,
            frame_error: None,
            disconnected: None,
        }
    }

    fn controller_paths(keyboard: bool) -> Paths {
        Paths {
            emulator: true,
            input: Path::Controller,
            frames: Some(Path::Controller),
            screenshots: Path::Agent,
            keys: Path::Agent,
            keyboard: Some(keyboard),
            endpoint: Some("http://127.0.0.1:8554".into()),
            warning: None,
        }
    }

    #[test]
    fn a_device_uses_the_agent_without_a_warning() {
        let selection = select_paths("R58M123ABC", false, &ControllerChoice::Auto, quick());
        assert!(selection.link.is_none());
        assert_eq!(selection.paths, Paths::agent(false, None));
        assert_eq!(selection.paths.input, Path::Agent);
        assert_eq!(selection.paths.frames, None);
        assert_eq!(selection.paths.warning, None);
    }

    #[test]
    fn an_emulator_whose_controller_is_turned_off_uses_the_agent_without_a_warning() {
        let selection = select_paths("emulator-5554", true, &ControllerChoice::Off, quick());
        assert!(selection.link.is_none());
        assert_eq!(selection.paths, Paths::agent(true, None));
    }

    /// Spec scenario "Control endpoint disabled", its selection: a closed
    /// port stands for an emulator started without the control flag.
    #[test]
    fn an_emulator_whose_controller_does_not_answer_falls_back_with_the_launch_flag() {
        let endpoint = closed_endpoint("emulator-5554");
        let started = Instant::now();
        let selection = select_paths(
            "emulator-5554",
            true,
            &ControllerChoice::Endpoint(endpoint),
            quick(),
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(selection.link.is_none());
        let paths = &selection.paths;
        assert_eq!(
            (paths.input, paths.frames, paths.keys),
            (Path::Agent, None, Path::Agent)
        );
        let warning = paths.warning.as_deref().expect("a warning");
        assert!(warning.contains("-grpc 8554 -grpc-use-token"), "{warning}");
        assert!(warning.contains("`grpc 8554`"), "{warning}");
        assert!(warning.contains("without a restart"), "{warning}");
    }

    #[test]
    fn the_agent_paths_send_everything_to_the_agent() {
        let paths = Paths::agent(true, Some("no endpoint".into()));
        assert_eq!(paths.input, Path::Agent);
        assert_eq!(paths.frames, None);
        assert_eq!(paths.screenshots, Path::Agent);
        assert_eq!(paths.keys, Path::Agent);
        assert_eq!(paths.keyboard, None);
        assert_eq!(paths.endpoint, None);
        assert_eq!(paths.warning.as_deref(), Some("no endpoint"));
    }

    /// Spec scenario "Control endpoint enabled", its status: the fast path
    /// is named, and keys stay with the agent with the keyboard reported.
    #[test]
    fn the_status_names_the_fast_path_and_the_keyboard() {
        let text = status(controller_paths(false)).to_string();
        assert!(
            text.contains("input: emulator controller at http://127.0.0.1:8554"),
            "{text}"
        );
        assert!(text.contains("frames: emulator controller"), "{text}");
        assert!(text.contains("screenshots: agent"), "{text}");
        assert!(text.contains("keys and text: agent"), "{text}");
        assert!(text.contains("declares no keyboard"), "{text}");
        assert!(!text.contains("warning"), "{text}");
        let with_keyboard = status(controller_paths(true)).to_string();
        assert!(
            with_keyboard.contains("declares a keyboard"),
            "{with_keyboard}"
        );
    }

    /// Spec scenario "Control endpoint disabled", its status: the agent path
    /// is named and the warning names the launch flag.
    #[test]
    fn the_status_names_the_agent_path_and_carries_the_warning() {
        let warning = emulator::Unavailable::new("emulator-5554", Some(5554), "refused").warning();
        let text = status(Paths::agent(true, Some(warning))).to_string();
        assert!(text.contains("input: agent"), "{text}");
        assert!(text.contains("frames: none"), "{text}");
        assert!(text.contains("screenshots: agent"), "{text}");
        assert!(text.contains("warning: "), "{text}");
        assert!(text.contains("-grpc 8554 -grpc-use-token"), "{text}");
    }

    #[test]
    fn the_status_names_the_agent_its_version_and_whether_it_was_reused() {
        let mut reused = status(Paths::agent(false, None));
        reused.reused = true;
        let first = reused.lines().remove(0);
        assert!(
            first.starts_with("emulator-5554: mobile-agent-android 0.1.0"),
            "{first}"
        );
        assert!(first.contains("protocol 1"), "{first}");
        assert!(first.contains("pid 812"), "{first}");
        assert!(first.contains("reused"), "{first}");
    }

    #[test]
    fn a_controller_failure_and_a_lost_agent_are_reported() {
        let mut failed = status(controller_paths(false));
        failed.controller_error = Some("sendTouch: no answer within 2s".into());
        failed.paths.input = Path::Agent;
        failed.disconnected = Some("the agent is disconnected (eof)".into());
        let text = failed.to_string();
        assert!(text.contains("input: agent"), "{text}");
        assert!(text.contains("sendTouch: no answer within 2s"), "{text}");
        assert!(text.contains("disconnected"), "{text}");
    }
}
