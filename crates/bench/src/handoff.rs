//! Handing a target from one tool to the next. An Android device has one UI
//! automation owner at a time, and a tool can keep holding the role between
//! its commands: `mobile-agent`'s daemon keeps its agent until it exits, and
//! agent-device keeps a snapshot helper until its session is closed. Tools
//! can also leave device state behind, such as a test input method selected
//! for text entry, or processes on the host, such as agent-device's daemon,
//! with its XCTest runner and snapshot bridge on a Simulator. So before
//! another tool drives a target, and once the run is over, the harness asks
//! the tool that drove it last to let go ([`Adapter::release`]), waits until
//! nothing the tool keeps for the target is left running
//! ([`Adapter::left_running`]), ending itself what the release should have
//! stopped and did not reach, and puts an Android target's input method
//! back to the one the run found. Hand-offs happen between series, never
//! inside one, and each is recorded with its duration.

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::adapter::{Adapter, Left};
use crate::provenance::Probe;
use crate::stats::micros;
use crate::target::{Platform, Target};

/// How long a hand-off waits for what a tool left running to end after its
/// release. On an iOS 26.5 Simulator at a load of 21, agent-device's daemon,
/// runner and bridge were gone by the time its `daemon stop --clean`
/// returned, under 0.5 s after it started (`crates/bench/README.md`,
/// Hand-offs between tools). `daemon stop` itself waits up to 12 s for the
/// daemon, and a bridge whose daemon has gone ends 60 s after its last
/// request.
const EXIT_WAIT: Duration = Duration::from_secs(30);

/// How often a hand-off asks what is still running while it waits.
const EXIT_POLL: Duration = Duration::from_millis(100);

/// How long a stray has after the hand-off's SIGTERM before it is sent
/// SIGKILL: as long as agent-device's own `daemon stop` gives its daemon. On
/// the API 36 emulator a SIGTERM ended a daemon `daemon stop` did not reach
/// in 305 ms, closing its session on the way out (`crates/bench/README.md`,
/// Hand-offs between tools).
const KILL_AFTER: Duration = Duration::from_secs(10);

/// One hand-off of a target from the tool that drove it to the next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandOff {
    /// The [`Target::id`].
    pub target: String,
    /// The tool that let go of the target.
    pub from: String,
    /// The tool about to drive it; `None` for the release once the run is
    /// over.
    pub to: Option<String>,
    /// How long the hand-off took: the tool's release, the wait for what it
    /// left running and the reset of the device together.
    pub elapsed_us: u64,
    /// What the tool's release did; `None` when the tool held nothing.
    pub released: Option<String>,
    /// On Android, the input method a tool left selected and the one set
    /// back; `None` when the input method was the one the run found.
    pub input_method: Option<String>,
    /// Why the release or the reset failed, a stray the hand-off ended and
    /// how, or what the tool left running past the wait, when there was
    /// one. The run goes on, and the next tool meets the target as it was
    /// left.
    pub error: Option<String>,
}

/// Which tool drives each target, the input method each Android target had
/// when the run started, and the hand-offs so far.
pub(crate) struct HandOffs {
    /// The index of the tool that drove each target last, by target id, in
    /// the order the targets were first driven.
    holders: Vec<(String, usize)>,
    /// The input method of each Android target when the run started.
    input_methods: HashMap<String, String>,
    pub(crate) done: Vec<HandOff>,
    /// How long a hand-off waits for what a tool left running: [`EXIT_WAIT`].
    exit_wait: Duration,
    /// How long a stray has after SIGTERM before SIGKILL: [`KILL_AFTER`].
    kill_after: Duration,
}

/// A stray the hand-off is ending: when it sent SIGTERM, and whether it has
/// sent SIGKILL since.
struct Ending {
    left: Left,
    termed: Instant,
    killed: bool,
}

impl HandOffs {
    /// No target driven yet; reads the input method of each Android target
    /// in `targets` through `probe`.
    pub(crate) fn new(probe: &dyn Probe, targets: &[Target]) -> Self {
        let input_methods = targets
            .iter()
            .filter(|target| target.class.platform() == Platform::Android)
            .filter_map(|target| Some((target.id.clone(), input_method(probe, &target.id)?)))
            .collect();
        HandOffs {
            holders: Vec::new(),
            input_methods,
            done: Vec::new(),
            exit_wait: EXIT_WAIT,
            kill_after: KILL_AFTER,
        }
    }

    /// Makes `tools[next]` the tool that drives `target`, after the tool that
    /// drove it until now, if another, let go of it.
    pub(crate) fn take(
        &mut self,
        tools: &mut [Box<dyn Adapter>],
        probe: &dyn Probe,
        target: &Target,
        next: usize,
    ) {
        let holder = self.holders.iter_mut().find(|(id, _)| *id == target.id);
        match holder {
            Some((_, holder)) if *holder == next => {}
            Some((_, holder)) => {
                let from = std::mem::replace(holder, next);
                self.hand_over(tools, probe, target, from, Some(next));
            }
            None => self.holders.push((target.id.clone(), next)),
        }
    }

    /// Has the tool that drove each target last let go of it, once the run
    /// is over.
    pub(crate) fn finish(
        &mut self,
        tools: &mut [Box<dyn Adapter>],
        probe: &dyn Probe,
        targets: &[Target],
    ) {
        for (id, holder) in std::mem::take(&mut self.holders) {
            if let Some(target) = targets.iter().find(|target| target.id == id) {
                self.hand_over(tools, probe, target, holder, None);
            }
        }
    }

    /// The release of `target` by `tools[from]`, the wait for what it left
    /// running, then the reset of its input method, timed together and
    /// recorded.
    fn hand_over(
        &mut self,
        tools: &mut [Box<dyn Adapter>],
        probe: &dyn Probe,
        target: &Target,
        from: usize,
        to: Option<usize>,
    ) {
        let started = Instant::now();
        let mut errors = Vec::new();
        let released = tools[from].release(target, probe).unwrap_or_else(|e| {
            errors.push(e.message);
            None
        });
        if let Err(e) = self.await_exit(tools[from].as_ref(), probe, target) {
            errors.push(e);
        }
        let input_method = self.reset_input_method(probe, target).unwrap_or_else(|e| {
            errors.push(e);
            None
        });
        let elapsed_us = micros(started.elapsed());
        self.done.push(HandOff {
            target: target.id.clone(),
            from: tools[from].name().to_owned(),
            to: to.map(|to| tools[to].name().to_owned()),
            elapsed_us,
            released,
            input_method,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
        });
    }

    /// Waits until `tool` has nothing left running for `target`, asking every
    /// [`EXIT_POLL`] for at most `exit_wait`. A stray, a process the release
    /// should have stopped and did not reach, is sent SIGTERM through `probe`
    /// as soon as it is listed, and SIGKILL once it has outlived SIGTERM by
    /// `kill_after`. A stray is an error naming it and what was sent, however
    /// it ended, since the release left it behind; any other process still
    /// running once the wait is over is an error naming it, as is a failure
    /// to tell.
    fn await_exit(
        &self,
        tool: &dyn Adapter,
        probe: &dyn Probe,
        target: &Target,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut ending: Vec<Ending> = Vec::new();
        loop {
            let left = tool.left_running(target, probe)?;
            for stray in left.iter().filter(|left| left.stray) {
                match ending
                    .iter_mut()
                    .find(|ending| ending.left.pid == stray.pid)
                {
                    None => {
                        signal(probe, "TERM", stray.pid);
                        ending.push(Ending {
                            left: stray.clone(),
                            termed: Instant::now(),
                            killed: false,
                        });
                    }
                    Some(ending)
                        if !ending.killed && ending.termed.elapsed() >= self.kill_after =>
                    {
                        signal(probe, "KILL", stray.pid);
                        ending.killed = true;
                    }
                    Some(_) => {}
                }
            }
            let waited = started.elapsed();
            if left.is_empty() || waited >= self.exit_wait {
                return self.what_was_left(tool.name(), &left, &ending);
            }
            thread::sleep(EXIT_POLL.min(self.exit_wait - waited));
        }
    }

    /// The error of a wait that ended with `left` still running for `tool`,
    /// after the hand-off sent signals to the strays `ending`; none when
    /// nothing is left and there were no strays.
    fn what_was_left(&self, tool: &str, left: &[Left], ending: &[Ending]) -> Result<(), String> {
        let running = |pid: u32| left.iter().any(|left| left.pid == pid);
        let mut errors: Vec<String> = ending
            .iter()
            .map(|ending| {
                format!(
                    "{tool} left {} running after its release; the hand-off sent it SIGTERM{}, \
                     and it {}",
                    ending.left,
                    if ending.killed {
                        format!(", then SIGKILL after {}", spoken(self.kill_after))
                    } else {
                        String::new()
                    },
                    if running(ending.left.pid) {
                        format!(
                            "was still running {} after its release",
                            spoken(self.exit_wait)
                        )
                    } else {
                        "exited".to_owned()
                    }
                )
            })
            .collect();
        let rest: Vec<String> = left
            .iter()
            .filter(|left| !ending.iter().any(|ending| ending.left.pid == left.pid))
            .map(Left::to_string)
            .collect();
        if !rest.is_empty() {
            errors.push(format!(
                "{tool} left {} running {} after its release",
                rest.join(", "),
                spoken(self.exit_wait)
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Selects the input method `target` had when the run started, when a
    /// tool left another one selected. Returns what it set back, if anything.
    fn reset_input_method(
        &self,
        probe: &dyn Probe,
        target: &Target,
    ) -> Result<Option<String>, String> {
        let Some(found) = self.input_methods.get(&target.id) else {
            return Ok(None);
        };
        let left = input_method(probe, &target.id)
            .ok_or_else(|| format!("cannot read the input method of {}", target.id))?;
        if left == *found {
            return Ok(None);
        }
        let set = probe.output("adb", &["-s", &target.id, "shell", "ime", "set", found]);
        match input_method(probe, &target.id) {
            Some(now) if now == *found => {
                Ok(Some(format!("{left} left selected, {found} set back")))
            }
            now => Err(format!(
                "{left} left selected, and `ime set {found}` {}, leaving {}",
                if set.is_some() { "ran" } else { "failed" },
                now.as_deref().unwrap_or("no input method")
            )),
        }
    }
}

/// Sends `signal`, `TERM` or `KILL`, to the host's process `pid` with
/// `kill` through `probe`. Whether it took is told by the next listing.
fn signal(probe: &dyn Probe, signal: &str, pid: u32) {
    probe.output("kill", &[&format!("-{signal}"), &pid.to_string()]);
}

/// `duration` in whole seconds, such as `30 s`, else in milliseconds.
fn spoken(duration: Duration) -> String {
    if duration.subsec_millis() == 0 && duration.as_secs() > 0 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

/// The input method selected on the Android target `serial`, as
/// `settings get secure default_input_method` reads it, such as
/// `com.android.inputmethod.latin/.LatinIME`; `None` when it cannot be read.
fn input_method(probe: &dyn Probe, serial: &str) -> Option<String> {
    probe
        .output(
            "adb",
            &[
                "-s",
                serial,
                "shell",
                "settings",
                "get",
                "secure",
                "default_input_method",
            ],
        )
        .map(|text| text.trim().to_owned())
        .filter(|ime| !ime.is_empty() && ime != "null")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::adapter::{AdapterError, Detection, Role, Scenario};
    use crate::harness::Subject;
    use crate::target::TargetClass;
    use serde_json::json;

    /// What the tools and the device were asked to do, in order.
    type Log = Rc<RefCell<Vec<String>>>;

    /// The host's process listing, as a [`Holder`] asks the probe for it.
    const PS: &str = "ps -A -ww -o pid=,command=";

    /// A tool that notes each release in the log, and fails its release when
    /// told to. When it keeps processes on the host, it finds what it left
    /// running in the host's process listing, where a daemon is a stray: its
    /// release should have stopped it.
    struct Holder {
        name: &'static str,
        log: Log,
        fails: bool,
        on_host: bool,
    }

    impl Adapter for Holder {
        fn name(&self) -> &str {
            self.name
        }

        fn role(&self) -> Role {
            Role::Baseline
        }

        fn detect(&mut self) -> Detection {
            Detection::Available { version: None }
        }

        fn supports(&self, _: &Target, _: &Scenario, _: &Subject) -> bool {
            true
        }

        fn release(
            &mut self,
            target: &Target,
            _: &dyn Probe,
        ) -> Result<Option<String>, AdapterError> {
            self.log
                .borrow_mut()
                .push(format!("{} released {}", self.name, target.id));
            if self.fails {
                return Err(AdapterError::new(format!("{} would not let go", self.name)));
            }
            Ok(Some(format!("{} let go", self.name)))
        }

        fn left_running(&self, _: &Target, probe: &dyn Probe) -> Result<Vec<Left>, String> {
            if !self.on_host {
                return Ok(Vec::new());
            }
            let listing = probe
                .output("ps", &["-A", "-ww", "-o", "pid=,command="])
                .ok_or("ps failed")?;
            let mine = format!(" of {}", self.name);
            Ok(listing
                .lines()
                .filter_map(|line| line.strip_suffix(&mine))
                .filter_map(|line| line.split_once(' '))
                .map(|(pid, name)| Left {
                    name: name.to_owned(),
                    pid: pid.parse().unwrap(),
                    stray: name == "daemon",
                    device: false,
                })
                .collect())
        }
    }

    /// A process a tool runs on the host: its pid and name, how many process
    /// listings after the tool's last release it is still listed, or `None`
    /// for as long as no signal ends it, and the weaker of the signals that
    /// end it, `TERM` or `KILL`, or `None` for neither.
    struct Running {
        tool: &'static str,
        pid: u32,
        name: &'static str,
        outlives: Option<usize>,
        ends_on: Option<&'static str>,
    }

    /// A host and the Android device on it. The device's input method is as
    /// `adb` reads and sets it. The host runs `processes`, and `kill` sends
    /// them signals. A physical iPhone on it, `PHONE-1`, runs `on_phone`,
    /// which CoreDevice lists and ends: `devicectl device process terminate`
    /// sends SIGTERM, with `--kill` SIGKILL. The commands it was asked to run
    /// go to the log.
    struct Device {
        ime: RefCell<Option<String>>,
        log: Log,
        /// `ime set` runs and changes nothing.
        stuck: bool,
        processes: Vec<Running>,
        on_phone: Vec<Running>,
    }

    impl Device {
        /// `ps`: each process its tool has not released, or released at most
        /// as many listings ago, this one included, as it outlives a release
        /// by, and that no signal sent since has ended.
        fn listing(&self) -> String {
            let log = self.log.borrow();
            self.processes
                .iter()
                .filter(|process| {
                    let released = format!("{} released ", process.tool);
                    let Some(at) = log.iter().rposition(|l| l.starts_with(&released)) else {
                        return true;
                    };
                    let since = log[at..].iter().filter(|l| *l == PS).count();
                    let ended = match process.ends_on {
                        Some("TERM") => ["TERM", "KILL"].as_slice(),
                        Some(_) => ["KILL"].as_slice(),
                        None => [].as_slice(),
                    }
                    .iter()
                    .any(|signal| log[at..].contains(&format!("kill -{signal} {}", process.pid)));
                    !ended && process.outlives.is_none_or(|outlives| since <= outlives)
                })
                .map(|process| format!("{} {} of {}", process.pid, process.name, process.tool))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// `devicectl device info processes` on the iPhone: launchd and each
        /// process of `on_phone` its tool has not released, or released at
        /// most as many phone listings ago, this one included, as it
        /// outlives a release by, and that no termination sent since has
        /// ended.
        fn phone_listing(&self) -> String {
            let log = self.log.borrow();
            let mut processes =
                vec![json!({"executable": "file:///sbin/launchd", "processIdentifier": 1})];
            for process in &self.on_phone {
                let released = format!("{} released ", process.tool);
                if let Some(at) = log.iter().rposition(|l| l.starts_with(&released)) {
                    let since = log[at..].iter().filter(|l| *l == PHONE_PS).count();
                    let term = terminate(process.pid, false);
                    let kill = terminate(process.pid, true);
                    let ended = match process.ends_on {
                        Some("TERM") => log[at..].iter().any(|l| *l == term || *l == kill),
                        Some(_) => log[at..].contains(&kill),
                        None => false,
                    };
                    if ended || process.outlives.is_some_and(|outlives| since > outlives) {
                        continue;
                    }
                }
                processes.push(json!({
                    "executable": format!(
                        "file:///private/var/containers/Bundle/Application/1/{0}.app/{0}",
                        process.name
                    ),
                    "processIdentifier": process.pid
                }));
            }
            json!({"info": {"outcome": "success"}, "result": {"runningProcesses": processes}})
                .to_string()
        }
    }

    /// The iPhone's process listing, as the hand-off asks the probe for it.
    const PHONE_PS: &str = "xcrun devicectl device info processes --device PHONE-1 --json-output -";

    /// The termination of `pid` on the iPhone, as the hand-off asks the
    /// probe for it: SIGTERM, or SIGKILL when `kill`.
    fn terminate(pid: u32, kill: bool) -> String {
        format!(
            "xcrun devicectl device process terminate --device PHONE-1 --pid {pid}{} --json-output -",
            if kill { " --kill" } else { "" }
        )
    }

    impl Probe for Device {
        fn output(&self, program: &str, args: &[&str]) -> Option<String> {
            self.log
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            match (program, args) {
                ("ps", ["-A", "-ww", "-o", "pid=,command="]) => Some(self.listing()),
                ("kill", [signal, _]) if signal.starts_with('-') => Some(String::new()),
                (
                    "xcrun",
                    [
                        "devicectl",
                        "device",
                        "info",
                        "processes",
                        "--device",
                        "PHONE-1",
                        "--json-output",
                        "-",
                    ],
                ) => Some(self.phone_listing()),
                ("xcrun", ["devicectl", "device", "process", "terminate", ..]) => {
                    Some(json!({"info": {"outcome": "success"}}).to_string())
                }
                (
                    "adb",
                    [
                        _,
                        _,
                        "shell",
                        "settings",
                        "get",
                        "secure",
                        "default_input_method",
                    ],
                ) => self.ime.borrow().clone(),
                ("adb", [_, _, "shell", "ime", "set", ime]) => {
                    if !self.stuck {
                        *self.ime.borrow_mut() = Some((*ime).to_owned());
                    }
                    Some(format!("Input method {ime} selected for user #0"))
                }
                _ => None,
            }
        }
    }

    const STOCK: &str = "com.android.inputmethod.latin/.LatinIME";
    const TEST_IME: &str = "com.callstack.agentdevice.imehelper/.TestInputMethodService";

    /// Two tools, the second of which fails its release, and a device with
    /// `ime` selected, whose `ime set` changes nothing when `stuck`.
    fn setup(ime: Option<&str>, stuck: bool) -> (Vec<Box<dyn Adapter>>, Device, Log) {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let tool = |name: &'static str, fails: bool| {
            Box::new(Holder {
                name,
                log: Rc::clone(&log),
                fails,
                on_host: false,
            }) as Box<dyn Adapter>
        };
        let tools = vec![tool("first", false), tool("second", true)];
        let device = Device {
            ime: RefCell::new(ime.map(str::to_owned)),
            log: Rc::clone(&log),
            stuck,
            processes: Vec::new(),
            on_phone: Vec::new(),
        };
        (tools, device, log)
    }

    /// `from` drives the iPhone `PHONE-1`, then `to` does; `from` leaves its
    /// runner app, pid 812, on the phone for `outlives` phone listings after
    /// its release, or until a termination `ends_on` names ends it.
    fn on_a_phone(
        from: &'static str,
        to: &'static str,
        runner: &'static str,
        outlives: Option<usize>,
        ends_on: Option<&'static str>,
    ) -> (Vec<Box<dyn Adapter>>, Device, Log) {
        let (_, mut device, log) = setup(None, false);
        let tool = |name: &'static str| {
            Box::new(Holder {
                name,
                log: Rc::clone(&log),
                fails: false,
                on_host: false,
            }) as Box<dyn Adapter>
        };
        device.on_phone = vec![Running {
            tool: from,
            pid: 812,
            name: runner,
            outlives,
            ends_on,
        }];
        (vec![tool(from), tool(to)], device, log)
    }

    fn phone() -> Target {
        Target::new("PHONE-1", TargetClass::IosDevice)
    }

    /// On a physical iPhone each tool's runner holds the phone's one
    /// automation session, and a runner left there makes the next tool's
    /// wait for the session rather than fail. After a tool's release the
    /// hand-off lists the phone's processes, ends that tool's runner app if
    /// it is still there, through CoreDevice since it is no host process,
    /// waits until it is gone, and names what it did.
    #[test]
    fn a_runner_left_on_an_iphone_is_ended_through_coredevice_before_the_next_tool_drives_it() {
        let (mut tools, device, log) = on_a_phone(
            "agent-device",
            "mobile-agent",
            "AgentDeviceRunnerUITests-Runner",
            None,
            Some("TERM"),
        );
        let target = phone();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        log.borrow_mut()
            .push("mobile-agent drives PHONE-1".to_owned());
        assert_eq!(
            log.borrow()[..],
            [
                "agent-device released PHONE-1".to_owned(),
                PHONE_PS.to_owned(),
                terminate(812, false),
                PHONE_PS.to_owned(),
                "mobile-agent drives PHONE-1".to_owned(),
            ]
        );
        assert_eq!(
            handoffs.done[0].error.as_deref(),
            Some(
                "agent-device left runner app on the iPhone (pid 812) running after its \
                 release; the hand-off sent it SIGTERM, and it exited"
            )
        );
        assert!(kills(&log).is_empty(), "no host process was signalled");
    }

    /// A runner that outlives SIGTERM on the phone gets SIGKILL through
    /// CoreDevice once the bound has passed.
    #[test]
    fn a_runner_that_outlives_sigterm_on_an_iphone_gets_sigkill_through_coredevice() {
        let (mut tools, device, log) = on_a_phone(
            "webdriveragent",
            "mobile-agent",
            "WebDriverAgentRunner-Runner",
            None,
            Some("KILL"),
        );
        let target = phone();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.kill_after = Duration::from_millis(50);
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        let terminations: Vec<String> = log
            .borrow()
            .iter()
            .filter(|l| l.contains(" terminate "))
            .cloned()
            .collect();
        assert_eq!(terminations, [terminate(812, false), terminate(812, true)]);
        assert_eq!(
            handoffs.done[0].error.as_deref(),
            Some(
                "webdriveragent left runner app on the iPhone (pid 812) running after its \
                 release; the hand-off sent it SIGTERM, then SIGKILL after 50 ms, and it exited"
            )
        );
    }

    /// `mobile-agent`'s daemon ends its runner as it shuts down, so the
    /// hand-off finds nothing of it on the phone and records no error; a
    /// tool that keeps no runner there, such as Maestro, has nothing looked
    /// for on the phone.
    #[test]
    fn a_runner_its_release_ended_leaves_nothing_to_end_on_the_iphone() {
        let (mut tools, device, log) = on_a_phone(
            "mobile-agent",
            "agent-device",
            "MobileAgentRunner-Runner",
            Some(0),
            None,
        );
        let target = phone();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        assert_eq!(
            log.borrow()[..],
            ["mobile-agent released PHONE-1", PHONE_PS]
        );
        assert_eq!(handoffs.done[0].error, None);

        let (mut tools, device, log) = on_a_phone(
            "maestro",
            "mobile-agent",
            "MobileAgentRunner-Runner",
            None,
            None,
        );
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        assert_eq!(log.borrow()[..], ["maestro released PHONE-1"]);
        assert_eq!(handoffs.done[0].error, None);
    }

    /// Two tools on a Simulator, the first of which runs a daemon, a runner
    /// and a bridge on the host, which end 0, 1 and `bridge` listings after
    /// its release.
    fn on_host(bridge: Option<usize>) -> (Vec<Box<dyn Adapter>>, Device, Log) {
        let (mut tools, mut device, log) = setup(None, false);
        tools[0] = Box::new(Holder {
            name: "first",
            log: Rc::clone(&log),
            fails: false,
            on_host: true,
        });
        let running = |pid, name, outlives| Running {
            tool: "first",
            pid,
            name,
            outlives,
            ends_on: None,
        };
        device.processes = vec![
            running(101, "daemon", Some(0)),
            running(102, "runner", Some(1)),
            running(103, "bridge", bridge),
        ];
        (tools, device, log)
    }

    /// Two tools on a Simulator, the first of which leaves its daemon running
    /// after its release, a stray that `ends_on` ends.
    fn with_a_stray(ends_on: Option<&'static str>) -> (Vec<Box<dyn Adapter>>, Device, Log) {
        let (tools, mut device, log) = on_host(Some(0));
        device.processes[0].outlives = None;
        device.processes[0].ends_on = ends_on;
        (tools, device, log)
    }

    /// The signals the hand-off sent, as the log has them.
    fn kills(log: &Log) -> Vec<String> {
        log.borrow()
            .iter()
            .filter(|line| line.starts_with("kill "))
            .cloned()
            .collect()
    }

    fn emulator() -> Target {
        Target::new("emulator-5554", TargetClass::AndroidEmulator)
    }

    fn simulator() -> Target {
        Target::new("SIM-1", TargetClass::Simulator)
    }

    #[test]
    fn a_target_changes_hands_only_when_another_tool_drives_it_and_once_the_run_is_over() {
        let (mut tools, device, log) = setup(None, false);
        let target = emulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        for next in [0, 0, 1, 1, 0] {
            handoffs.take(&mut tools, &device, &target, next);
        }
        handoffs.finish(&mut tools, &device, std::slice::from_ref(&target));
        let released: Vec<String> = log
            .borrow()
            .iter()
            .filter(|line| line.contains(" released "))
            .cloned()
            .collect();
        assert_eq!(
            released,
            [
                "first released emulator-5554",
                "second released emulator-5554",
                "first released emulator-5554"
            ]
        );
        let moves: Vec<(&str, Option<&str>)> = handoffs
            .done
            .iter()
            .map(|h| (h.from.as_str(), h.to.as_deref()))
            .collect();
        assert_eq!(
            moves,
            [
                ("first", Some("second")),
                ("second", Some("first")),
                ("first", None)
            ]
        );
        assert_eq!(handoffs.done[0].released.as_deref(), Some("first let go"));
        // A release that fails is recorded, and the run goes on.
        assert_eq!(handoffs.done[1].released, None);
        assert_eq!(
            handoffs.done[1].error.as_deref(),
            Some("second would not let go")
        );
        // With no input method to read, none is reset.
        assert!(handoffs.done.iter().all(|h| h.input_method.is_none()));
    }

    #[test]
    fn an_android_target_gets_back_the_input_method_the_run_found() {
        let (mut tools, device, log) = setup(Some(STOCK), false);
        let target = emulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        // The first tool leaves its test input method selected.
        *device.ime.borrow_mut() = Some(TEST_IME.to_owned());
        handoffs.take(&mut tools, &device, &target, 1);
        assert_eq!(device.ime.borrow().as_deref(), Some(STOCK));
        assert_eq!(
            handoffs.done[0].input_method,
            Some(format!("{TEST_IME} left selected, {STOCK} set back"))
        );
        assert_eq!(handoffs.done[0].error, None);
        assert!(
            log.borrow()
                .contains(&format!("adb -s emulator-5554 shell ime set {STOCK}")),
            "{:?}",
            log.borrow()
        );
        // The input method was the one found: nothing is set.
        handoffs.finish(&mut tools, &device, std::slice::from_ref(&target));
        assert_eq!(handoffs.done[1].input_method, None);
        let sets = log
            .borrow()
            .iter()
            .filter(|l| l.contains(" ime set "))
            .count();
        assert_eq!(sets, 1);
    }

    #[test]
    fn an_input_method_that_cannot_be_set_back_is_an_error_naming_what_was_left() {
        let (mut tools, device, _) = setup(Some(STOCK), true);
        let target = emulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        *device.ime.borrow_mut() = Some(TEST_IME.to_owned());
        handoffs.finish(&mut tools, &device, std::slice::from_ref(&target));
        assert_eq!(
            handoffs.done[0].error,
            Some(format!(
                "{TEST_IME} left selected, and `ime set {STOCK}` ran, leaving {TEST_IME}"
            ))
        );
    }

    #[test]
    fn a_tool_s_processes_are_gone_before_the_next_tool_takes_the_target() {
        let (mut tools, device, log) = on_host(Some(2));
        let target = simulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        log.borrow_mut().push("second drives SIM-1".to_owned());
        // After the release the hand-off listed the host's processes until
        // the slowest, the bridge, was gone, and only then returned.
        assert_eq!(
            log.borrow()[..],
            ["first released SIM-1", PS, PS, PS, "second drives SIM-1"]
        );
        assert_eq!(handoffs.done[0].released.as_deref(), Some("first let go"));
        assert_eq!(handoffs.done[0].error, None);
    }

    #[test]
    fn processes_that_outlive_the_wait_are_an_error_naming_what_was_left() {
        let (mut tools, device, _) = on_host(None);
        let target = simulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.exit_wait = Duration::from_millis(50);
        handoffs.take(&mut tools, &device, &target, 0);
        let started = Instant::now();
        handoffs.take(&mut tools, &device, &target, 1);
        assert!(started.elapsed() >= Duration::from_millis(50));
        // The run goes on: the release is recorded, and so is what it left.
        assert_eq!(handoffs.done[0].released.as_deref(), Some("first let go"));
        assert_eq!(
            handoffs.done[0].error.as_deref(),
            Some("first left bridge (pid 103) running 50 ms after its release")
        );
    }

    /// Task 10.27: a daemon the tool's own stop did not reach, such as
    /// agent-device's daemon whose `daemon.json` a timed-out request deleted,
    /// idled for minutes when the hand-off only reported it. The hand-off
    /// sends such a stray SIGTERM as soon as it lists it, waits for it as for
    /// the rest, and names what it did in the hand-off's error, since the
    /// release left the stray behind.
    #[test]
    fn a_stray_is_sent_sigterm_at_once_and_the_error_says_it_exited() {
        let (mut tools, device, log) = with_a_stray(Some("TERM"));
        let target = simulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        log.borrow_mut().push("second drives SIM-1".to_owned());
        assert_eq!(
            log.borrow()[..],
            [
                "first released SIM-1",
                PS,
                "kill -TERM 101",
                PS,
                "second drives SIM-1"
            ]
        );
        assert_eq!(handoffs.done[0].released.as_deref(), Some("first let go"));
        assert_eq!(
            handoffs.done[0].error.as_deref(),
            Some(
                "first left daemon (pid 101) running after its release; the hand-off sent \
                 it SIGTERM, and it exited"
            )
        );
    }

    /// A stray still running `kill_after` past its SIGTERM gets SIGKILL, as
    /// agent-device's own `daemon stop` does to its daemon after 10 s.
    #[test]
    fn a_stray_that_outlives_sigterm_is_sent_sigkill_once_the_bound_has_passed() {
        let (mut tools, device, log) = with_a_stray(Some("KILL"));
        let target = simulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.kill_after = Duration::from_millis(50);
        handoffs.take(&mut tools, &device, &target, 0);
        let started = Instant::now();
        handoffs.take(&mut tools, &device, &target, 1);
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert_eq!(kills(&log), ["kill -TERM 101", "kill -KILL 101"]);
        // After SIGKILL the hand-off listed the host once more and found
        // nothing left.
        assert_eq!(
            log.borrow()[log.borrow().len() - 2..],
            ["kill -KILL 101", PS]
        );
        assert_eq!(
            handoffs.done[0].error.as_deref(),
            Some(
                "first left daemon (pid 101) running after its release; the hand-off sent \
                 it SIGTERM, then SIGKILL after 50 ms, and it exited"
            )
        );
    }

    /// A stray that outlives both signals is still running when the wait
    /// ends, which the error says after what was sent.
    #[test]
    fn a_stray_that_outlives_both_signals_is_an_error_saying_what_was_sent() {
        let (mut tools, device, log) = with_a_stray(None);
        let target = simulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.kill_after = Duration::from_millis(20);
        handoffs.exit_wait = Duration::from_millis(100);
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.take(&mut tools, &device, &target, 1);
        assert_eq!(kills(&log), ["kill -TERM 101", "kill -KILL 101"]);
        assert_eq!(
            handoffs.done[0].error.as_deref(),
            Some(
                "first left daemon (pid 101) running after its release; the hand-off sent \
                 it SIGTERM, then SIGKILL after 20 ms, and it was still running 100 ms \
                 after its release"
            )
        );
    }

    #[test]
    fn a_simulator_s_input_method_is_never_read() {
        let (mut tools, device, log) = setup(Some(STOCK), false);
        let target = simulator();
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.finish(&mut tools, &device, std::slice::from_ref(&target));
        assert!(
            !log.borrow().iter().any(|line| line.starts_with("adb ")),
            "{:?}",
            log.borrow()
        );
    }

    /// Only a physical iPhone has its processes listed through CoreDevice,
    /// even when the tool that let go keeps a runner on one.
    #[test]
    fn a_simulator_or_an_emulator_has_no_phone_processes_listed() {
        for target in [simulator(), emulator()] {
            let (mut tools, device, log) = on_a_phone(
                "agent-device",
                "mobile-agent",
                "AgentDeviceRunnerUITests-Runner",
                None,
                None,
            );
            let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
            handoffs.take(&mut tools, &device, &target, 0);
            handoffs.take(&mut tools, &device, &target, 1);
            assert!(
                !log.borrow().iter().any(|line| line.starts_with("xcrun ")),
                "{:?}",
                log.borrow()
            );
        }
    }
}
