//! Handing a target from one tool to the next. An Android device has one UI
//! automation owner at a time, and a tool can keep holding the role between
//! its commands: `mobile-agent`'s daemon keeps its agent until it exits, and
//! agent-device keeps a snapshot helper until its session is closed. Tools
//! can also leave device state behind, such as a test input method selected
//! for text entry. So before another tool drives a target, and once the run
//! is over, the harness asks the tool that drove it last to let go
//! ([`Adapter::release`]) and puts an Android target's input method back to
//! the one the run found. Hand-offs happen between series, never inside one,
//! and each is recorded with its duration.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::adapter::Adapter;
use crate::provenance::Probe;
use crate::stats::micros;
use crate::target::{Platform, Target};

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
    /// How long the hand-off took: the tool's release and the reset of the
    /// device together.
    pub elapsed_us: u64,
    /// What the tool's release did; `None` when the tool held nothing.
    pub released: Option<String>,
    /// On Android, the input method a tool left selected and the one set
    /// back; `None` when the input method was the one the run found.
    pub input_method: Option<String>,
    /// Why the release or the reset failed, when one did. The run goes on,
    /// and the next tool meets the target as it was left.
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

    /// The release of `target` by `tools[from]`, then the reset of its input
    /// method, timed together and recorded.
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
        let released = tools[from].release(target).unwrap_or_else(|e| {
            errors.push(e.message);
            None
        });
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

    /// What the tools and the device were asked to do, in order.
    type Log = Rc<RefCell<Vec<String>>>;

    /// A tool that notes each release in the log, and fails its release when
    /// told to.
    struct Holder {
        name: &'static str,
        log: Log,
        fails: bool,
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

        fn release(&mut self, target: &Target) -> Result<Option<String>, AdapterError> {
            self.log
                .borrow_mut()
                .push(format!("{} released {}", self.name, target.id));
            if self.fails {
                return Err(AdapterError::new(format!("{} would not let go", self.name)));
            }
            Ok(Some(format!("{} let go", self.name)))
        }
    }

    /// An Android device's input method, as `adb` reads and sets it; the
    /// commands it was asked to run go to the log.
    struct Device {
        ime: RefCell<Option<String>>,
        log: Log,
        /// `ime set` runs and changes nothing.
        stuck: bool,
    }

    impl Probe for Device {
        fn output(&self, program: &str, args: &[&str]) -> Option<String> {
            self.log
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            match args {
                [
                    _,
                    _,
                    "shell",
                    "settings",
                    "get",
                    "secure",
                    "default_input_method",
                ] => self.ime.borrow().clone(),
                [_, _, "shell", "ime", "set", ime] => {
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
            }) as Box<dyn Adapter>
        };
        let tools = vec![tool("first", false), tool("second", true)];
        let device = Device {
            ime: RefCell::new(ime.map(str::to_owned)),
            log: Rc::clone(&log),
            stuck,
        };
        (tools, device, log)
    }

    fn emulator() -> Target {
        Target::new("emulator-5554", TargetClass::AndroidEmulator)
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
    fn a_simulator_s_input_method_is_never_read() {
        let (mut tools, device, log) = setup(Some(STOCK), false);
        let target = Target::new("SIM-1", TargetClass::Simulator);
        let mut handoffs = HandOffs::new(&device, std::slice::from_ref(&target));
        handoffs.take(&mut tools, &device, &target, 0);
        handoffs.finish(&mut tools, &device, std::slice::from_ref(&target));
        assert!(
            !log.borrow().iter().any(|line| line.starts_with("adb ")),
            "{:?}",
            log.borrow()
        );
    }
}
