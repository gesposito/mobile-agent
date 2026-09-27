//! How the agent is started and recognized on the device: its per-launch
//! socket name, the shell command that starts it detached, the lines it
//! prints once it listens or when UI automation is taken, and its entries in
//! the process list.

use std::fmt;
use std::time::Duration;

/// The class `app_process` starts.
pub const MAIN_CLASS: &str = "dev.mobileagent.agent.Main";

/// The name the agent gives itself in the process list, followed by its
/// socket name: `mobile-agent <socket>`.
pub const PROCESS_NAME: &str = "mobile-agent";

/// The environment variable that carries the per-launch token, which keeps
/// it out of the process list.
pub const TOKEN_VARIABLE: &str = "MOBILE_AGENT_TOKEN";

/// Mode of the pushed jar: read-only for everyone. adbd copies the owner's
/// bits to group and other, so 0o444 is the one mode that arrives unwidened
/// and never writable (task 5.1 measured 0o644 arriving as 0666).
pub const JAR_MODE: u32 = 0o444;

/// Lists every process with its command line, where the agent appears as
/// `mobile-agent <socket>`.
pub const PS_COMMAND: &str = "ps -A -o PID,ARGS";

/// Where the jar of the launch whose socket is `socket` is pushed.
pub fn remote_jar(socket: &str) -> String {
    let _ = socket;
    todo!("task 5.2")
}

/// `bytes` random bytes from the operating system as lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    let _ = bytes;
    todo!("task 5.2")
}

/// A per-launch socket name that also says which agent listens on it:
/// `mobile-agent-<version>-p<protocol>-<id>`. The process list shows it, so
/// the agent state of a target is known without connecting, which would
/// take the agent's connection away from whoever holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketName {
    /// The agent version, with characters a socket name cannot hold replaced
    /// by `_`.
    pub version: String,
    pub protocol: u16,
    /// Random, unique per launch.
    pub id: String,
}

impl SocketName {
    /// A name for a launch of `version` speaking `protocol`, with `id`.
    pub fn new(version: &str, protocol: u16, id: &str) -> SocketName {
        let _ = (version, protocol, id);
        todo!("task 5.2")
    }

    /// A name for a new launch of `version` speaking `protocol`, with a
    /// random id.
    pub fn generate(version: &str, protocol: u16) -> SocketName {
        SocketName::new(version, protocol, &random_hex(6))
    }

    /// Parses a name this driver made; `None` for any other name.
    pub fn parse(name: &str) -> Option<SocketName> {
        let _ = name;
        todo!("task 5.2")
    }

    /// Whether this name says the agent is `version` speaking `protocol`.
    pub fn is(&self, version: &str, protocol: u16) -> bool {
        let _ = (version, protocol);
        todo!("task 5.2")
    }
}

impl fmt::Display for SocketName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("task 5.2")
    }
}

/// The shell command that starts the agent from the jar at `jar`, listening
/// on `socket`, and returns at once, leaving it running.
///
/// - The token is read from the command's standard input, so it appears in
///   no command line; the caller writes it followed by a newline.
/// - The agent ignores SIGHUP and SIGPIPE and is started in the background
///   of a subshell, so it is reparented to init. Dropping the stream that
///   launched it (adbd sends SIGHUP) and the end of that stream's pipes
///   (later writes fail with EPIPE) leave it running.
/// - Its standard input is `/dev/null`; standard output and error stay on
///   the stream, which carries the ready or conflict line.
/// - `--idle-exit-ms` bounds how long it outlives a host that went away,
///   `--force` takes UI automation over from a known automation client, and
///   `extra` arguments are passed on quoted.
pub fn launch_command(
    jar: &str,
    socket: &str,
    idle_exit: Duration,
    force: bool,
    extra: &[String],
) -> String {
    let _ = (jar, socket, idle_exit, force, extra);
    todo!("task 5.2")
}

/// A process named in an agent's report: `uiautomator` and its pid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    /// A short name such as `uiautomator`, `mobile-agent` or `am-instrument`.
    pub name: String,
    pub pid: Option<u32>,
}

impl fmt::Display for Holder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.pid {
            Some(pid) => write!(f, "{} (pid {pid})", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// The line the agent prints once it holds UI automation and listens:
/// `ready socket=<name> pid=<pid> uid=<uid> protocol=<n> capabilities=0x<bits>
/// ownership=held connect_ms=<ms> window_wait_ms=<ms> windows=<n>
/// active=<package> forced=<name:pid or ->`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyLine {
    pub socket: String,
    pub pid: u32,
    pub uid: Option<u32>,
    pub protocol: Option<u16>,
    pub capabilities: Option<u32>,
    /// How long taking UI automation took on the device.
    pub connect_ms: Option<u64>,
    /// How long the agent waited for an active window.
    pub window_wait_ms: Option<u64>,
    pub windows: Option<u32>,
    /// The package of the active window, when there was one.
    pub active: Option<String>,
    /// The holder `--force` stopped to take UI automation over.
    pub forced: Option<Holder>,
}

/// Parses a ready line; `None` for any other line.
pub fn parse_ready(line: &str) -> Option<ReadyLine> {
    let _ = line;
    todo!("task 5.2")
}

/// The line the agent prints before exiting with status 3 when another
/// process holds UI automation: `conflict pid=<pid> holder=<name>
/// holder_pid=<pid or -> message=<text>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictLine {
    /// The agent that could not take the role.
    pub agent_pid: Option<u32>,
    /// The likely holder, when the process and instrumentation lists name
    /// one.
    pub holder: Option<Holder>,
    /// The agent's explanation, to the end of the line.
    pub message: String,
}

/// Parses a conflict line; `None` for any other line.
pub fn parse_conflict(line: &str) -> Option<ConflictLine> {
    let _ = line;
    todo!("task 5.2")
}

/// An agent process found in the process list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningAgent {
    pub pid: u32,
    /// The socket it listens on.
    pub socket: String,
    /// The socket name parsed, when this driver made it; it says the agent's
    /// version and protocol.
    pub name: Option<SocketName>,
}

/// The agents in [`PS_COMMAND`]'s output: processes named `mobile-agent
/// <socket>`, and `app_process` running the agent's main class where the
/// process could not rename itself. Wrappers such as `timeout` and the shell
/// that started them are left out.
pub fn parse_agents(ps: &str) -> Vec<RunningAgent> {
    let _ = ps;
    todo!("task 5.2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_name_carries_the_agent_version_and_protocol() {
        let name = SocketName::new("0.1.0", 1, "3f9a1c2b7d4e");
        assert_eq!(name.to_string(), "mobile-agent-0.1.0-p1-3f9a1c2b7d4e");
        assert_eq!(SocketName::parse(&name.to_string()), Some(name.clone()));
        assert!(name.is("0.1.0", 1));
        assert!(!name.is("0.1.1", 1));
        assert!(!name.is("0.1.0", 2));
    }

    #[test]
    fn versions_with_hyphens_and_build_metadata_survive_the_name() {
        let name = SocketName::new("1.2.0-rc.1+abc", 12, "00ff");
        let text = name.to_string();
        assert_eq!(text, "mobile-agent-1.2.0-rc.1_abc-p12-00ff");
        assert!(
            text.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)),
            "{text}"
        );
        let parsed = SocketName::parse(&text).unwrap();
        assert!(parsed.is("1.2.0-rc.1+abc", 12));
        assert_eq!(parsed.version, "1.2.0-rc.1_abc");
    }

    #[test]
    fn names_other_launchers_chose_do_not_parse() {
        for other in [
            "mobile-agent-3f9a1c2b7d4e",
            "mobile-agent-it-foo-1-2",
            "mobile-agent-0.1.0-px-1a",
            "scrcpy",
            "mobile-agent--p1-1a",
        ] {
            assert_eq!(SocketName::parse(other), None, "{other}");
        }
    }

    #[test]
    fn generated_names_differ_and_fit_the_agents_socket_rule() {
        let a = SocketName::generate("0.1.0", 1);
        let b = SocketName::generate("0.1.0", 1);
        assert_ne!(a, b);
        let text = a.to_string();
        assert!(text.len() <= 100, "{text}");
        assert_eq!(a.id.len(), 12);
        assert!(a.id.chars().all(|c| c.is_ascii_hexdigit()), "{}", a.id);
    }

    #[test]
    fn random_hex_has_twice_the_bytes_in_digits_and_varies() {
        let a = random_hex(16);
        assert_eq!(a.len(), 32);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(a, random_hex(16));
    }

    #[test]
    fn the_jar_goes_to_a_per_launch_path_in_data_local_tmp() {
        assert_eq!(
            remote_jar("mobile-agent-0.1.0-p1-00ff"),
            "/data/local/tmp/mobile-agent-0.1.0-p1-00ff.jar"
        );
    }

    #[test]
    fn the_launch_command_keeps_the_token_off_the_command_line_and_detaches() {
        let command = launch_command(
            "/data/local/tmp/a.jar",
            "mobile-agent-0.1.0-p1-00ff",
            Duration::from_secs(30),
            false,
            &[],
        );
        // The token comes from standard input into a variable.
        assert!(command.starts_with("read -r t && "), "{command}");
        assert!(command.contains("MOBILE_AGENT_TOKEN=\"$t\""), "{command}");
        // SIGHUP and SIGPIPE ignored, reparented to init, stdin from /dev/null.
        assert!(command.contains("trap '' HUP PIPE"), "{command}");
        assert!(command.contains("</dev/null &)"), "{command}");
        assert!(
            command.contains(
                "CLASSPATH='/data/local/tmp/a.jar' exec app_process / \
                 dev.mobileagent.agent.Main serve --socket 'mobile-agent-0.1.0-p1-00ff' \
                 --idle-exit-ms 30000"
            ),
            "{command}"
        );
        assert!(!command.contains("--force"), "{command}");
    }

    #[test]
    fn force_and_extra_arguments_are_passed_quoted() {
        let command = launch_command(
            "/data/local/tmp/a.jar",
            "s",
            Duration::from_millis(1500),
            true,
            &["--trace".to_owned(), "it's".to_owned()],
        );
        assert!(
            command.contains("--idle-exit-ms 1500 --force '--trace' 'it'\\''s' </dev/null &)"),
            "{command}"
        );
    }

    #[test]
    fn a_ready_line_gives_the_pid_socket_and_forced_holder() {
        let line = "ready socket=mobile-agent-0.1.0-p1-00ff pid=812 uid=2000 protocol=1 \
                    capabilities=0x37 ownership=held connect_ms=12 window_wait_ms=66 windows=2 \
                    active=com.google.android.apps.nexuslauncher forced=-";
        let ready = parse_ready(line).unwrap();
        assert_eq!(
            ready,
            ReadyLine {
                socket: "mobile-agent-0.1.0-p1-00ff".into(),
                pid: 812,
                uid: Some(2000),
                protocol: Some(1),
                capabilities: Some(0x37),
                connect_ms: Some(12),
                window_wait_ms: Some(66),
                windows: Some(2),
                active: Some("com.google.android.apps.nexuslauncher".into()),
                forced: None,
            }
        );
        let forced = parse_ready(
            "ready socket=s pid=9 uid=2000 protocol=1 capabilities=0x37 ownership=held \
             connect_ms=1 window_wait_ms=0 windows=1 active=- forced=uiautomator:4702",
        )
        .unwrap();
        assert_eq!(forced.active, None);
        assert_eq!(
            forced.forced,
            Some(Holder {
                name: "uiautomator".into(),
                pid: Some(4702)
            })
        );
        assert_eq!(
            parse_ready("ready socket=s"),
            None,
            "a ready line needs a pid"
        );
        assert_eq!(parse_ready("warning cannot rename the process"), None);
    }

    #[test]
    fn a_conflict_line_names_the_holder_and_keeps_the_whole_message() {
        let line = "conflict pid=4711 holder=uiautomator holder_pid=4702 message=UI automation \
                    is held by another process, likely uiautomator (pid 4702: uiautomator); stop \
                    it, or run the agent with --force to take the role over";
        let conflict = parse_conflict(line).unwrap();
        assert_eq!(conflict.agent_pid, Some(4711));
        assert_eq!(
            conflict.holder,
            Some(Holder {
                name: "uiautomator".into(),
                pid: Some(4702)
            })
        );
        assert!(
            conflict
                .message
                .starts_with("UI automation is held by another process")
        );
        assert!(conflict.message.ends_with("to take the role over"));

        let unknown = parse_conflict(
            "conflict pid=5 holder=- holder_pid=- message=UI automation is held by a process",
        )
        .unwrap();
        assert_eq!(unknown.holder, None);
        let instrumentation =
            parse_conflict("conflict pid=5 holder=instrumentation holder_pid=- message=held")
                .unwrap();
        assert_eq!(
            instrumentation.holder,
            Some(Holder {
                name: "instrumentation".into(),
                pid: None
            })
        );
        assert_eq!(parse_conflict("ready socket=s pid=1"), None);
    }

    #[test]
    fn agents_are_found_in_the_process_list_by_name_or_main_class() {
        let ps = "  PID ARGS\n\
                  \x20   1 init second_stage\n\
                  \x20 812 mobile-agent mobile-agent-0.1.0-p1-00ff\n\
                  \x20 900 mobile-agent mobile-agent-3f9a1c2b7d4e\n\
                  \x20 901 app_process / dev.mobileagent.agent.Main serve --socket s1 --idle-exit-ms 10\n\
                  \x20 902 timeout -s KILL 600 app_process / dev.mobileagent.agent.Main serve --socket s2\n\
                  \x20 903 sh -c read -r t && (trap '' HUP PIPE; exec app_process / dev.mobileagent.agent.Main serve --socket s3 &)\n\
                  \x20 904 uiautomator events\n\
                  \x20 905 grep mobile-agent\n";
        let agents = parse_agents(ps);
        assert_eq!(
            agents,
            vec![
                RunningAgent {
                    pid: 812,
                    socket: "mobile-agent-0.1.0-p1-00ff".into(),
                    name: Some(SocketName::new("0.1.0", 1, "00ff")),
                },
                RunningAgent {
                    pid: 900,
                    socket: "mobile-agent-3f9a1c2b7d4e".into(),
                    name: None,
                },
                RunningAgent {
                    pid: 901,
                    socket: "s1".into(),
                    name: None,
                },
            ]
        );
    }
}
