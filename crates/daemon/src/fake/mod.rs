//! The fake driver: targets that `MOBILE_AGENT_FAKE_TARGETS` (or the
//! daemon's `--fake` flag) names, each a fake device on the mirror's
//! scripted [`mirror::testing::FakeTarget`], run in real time. Every test of
//! the daemon and the CLI, and the CLI overhead measurement, runs on them.
//!
//! A fake device shows a launcher (a heading and the buttons Fixture and
//! Settings), the fixture app of `fixtures/CONTRACT.md` under the ids
//! `dev.mobileagent.fixture.expo` and the platform's native fixture, a
//! settings app, and the placeholder app `dev.mobileagent.placeholder`, one
//! screen with the shapes a React Native app showed in the production-app
//! smoke test of 2026-10-09 under placeholder names. Taps, text, keys and swipes change it
//! as the real apps change; screenshots return placeholder bytes, not an
//! image. Emulators and Simulators have a frame source and Android devices
//! none, as design D7 has it, and emulators have none either when the
//! daemon's emulator controller setting is off; `status` names an Android
//! target's input and frame paths as the Android driver names them. A fake
//! iPhone (`ios-device`) stands in for a physical one: an iOS fake whose
//! frames are screenshots, in the `ios-device` class, whose `status` names
//! its input, frames and runner as the physical iOS driver names them, and
//! which [`FakeDriver::with_readiness`] can list in a readiness state that
//! refuses an attach, as that driver lists a phone that is not ready. Each
//! attach of a fake iPhone starts a runner of its own, ending the ones a
//! lost session left on the phone first, [`FakeDriver::with_lost_runner`]
//! loses the first one's connection, its restart failing, and
//! [`FakeDriver::stall_runner`] stalls the runner serving. A latency
//! (`MOBILE_AGENT_FAKE_LATENCY_MS`) delays every change,
//! so that the time from a tap to its effects can be measured, and a frame
//! delay (`MOBILE_AGENT_FAKE_FRAME_DELAY_MS`) starts the frame stream after
//! the attach, as an emulator's starts once its subscription is up. A
//! read cost ([`FakeDriver::with_read_cost`]) makes each read of the tree
//! take that long, as a Simulator's reader and the Android agent take to
//! answer, so that a read a command makes shows in its time, and
//! [`FakeDriver::tree_reads`] counts the reads, so that a test can tell how
//! many a command made. A
//! splash (`MOBILE_AGENT_FAKE_SPLASH_MS`) shows a fixture app's launch
//! splash for that long when its process starts, an activation
//! (`MOBILE_AGENT_FAKE_ACTIVATION_MS`) holds input until the input session
//! has been active that long, as a Simulator's dtuhidd session does, and a
//! stop (`MOBILE_AGENT_FAKE_STOP_MS`) makes detaching a target take that
//! long, as stopping the Android agent does. A fling
//! (`MOBILE_AGENT_FAKE_FLING`) makes the fixture's list glide on after a
//! stroke whose finger lifts moving, as far and as long as the platform's
//! fling carries it, a tap on the list while it glides stopping it and
//! reaching nothing under it; without it, and after a stroke whose finger
//! comes to rest, the list moves as far as the finger. A typing mode
//! (`MOBILE_AGENT_FAKE_TYPING`) says how the fixture's fields take text
//! whose method the target may choose, as `fill` sends it, which they
//! otherwise take whole, as the Android agent sets it (see [`Typing`]). With
//! [`FakeDriver::with_home_returns`] the launcher takes the front back from
//! the first launches of a running app made from it, as a Simulator's home
//! screen did from an app launched during its go-home animation.
//!
//! A fake shows its keyboard while a field has focus, and with
//! [`FakeDriver::with_keyboard`] over every fixture screen, so a list can
//! lie partly under it: on Android a window over the screen from 600 points
//! down, on iOS the window the Simulator's reader gives the keyboard, from
//! its prediction bar at 539 points down. The keyboard comes up with the
//! focus, or as [`FakeDriver::with_slide`] says (see [`Slide`]): some time
//! after it, an iOS field losing the text that comes sooner, never, or with
//! its keys hidden, as under a hardware keyboard, its window drawing
//! nothing. A stroke that comes down on the
//! keyboard goes to the keyboard and moves nothing: its glide typing enters
//! a word into the field in focus, as Gboard took the strokes of a `scroll`
//! that started on its keys on a Nexus 5X
//! (`reports/10.3-physical-android.md`, finding 4).

mod app;
mod realtime;
mod tree;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use realtime::{Frames, FromDriver};
use serde_json::{Map, Value};

use crate::devices::android::report_controller;
use crate::driver::{
    AttachOptions, Attachment, Driver, DriverError, EmulatorController, Platform, Report,
    TargetClass, TargetInfo, TargetReport,
};

/// How fake targets take their time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Timing {
    /// How long a change takes to show, as a slow app or device takes.
    pub latency: Duration,
    /// How long after the attach the frame stream starts.
    pub frame_delay: Duration,
    /// How long a fixture app shows its launch splash when its process
    /// starts.
    pub splash: Duration,
    /// How long after it starts the input session is active; input before
    /// then waits for it.
    pub activation: Duration,
    /// How long detaching the target takes, as the Android agent's
    /// `shutdown` and kill take.
    pub stop: Duration,
    /// How the keyboard comes up once a field takes the focus.
    pub slide: Slide,
    /// How many launches of a running app, made while the home screen is
    /// in front, the home screen takes the front back from.
    pub home_returns: u32,
    /// How long each read of the tree takes.
    pub read_cost: Duration,
}

/// How a fake's keyboard comes up once a field takes the focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slide {
    /// With the focus.
    #[default]
    AtOnce,
    /// That long after the field took the focus. On iOS text that reaches
    /// the field before then is lost, typed or pasted, as the first keys
    /// typed into a just-focused field were lost while the Simulator's
    /// keyboard slid in, in 3 of 8 runs of the production-app smoke test of
    /// 2026-10-09; Android's text set types nothing and is never lost.
    After(Duration),
    /// Never: no keyboard window shows, and fields take text, as for a
    /// field with an input view of its own.
    Never,
    /// With its keys hidden: on iOS the keyboard's window shows with the
    /// focus but draws nothing, its keys below the screen, and fields take
    /// text at once, as on a Simulator whose hardware keyboard is connected
    /// (rn-app-fixes task 6.1); on Android no keyboard window shows.
    Hidden,
    /// As [`Slide::Hidden`] for a field that takes a text keyboard, while
    /// one that takes a number pad, the placeholder app's code fields,
    /// brings up no keyboard window at all, as a number pad does on iOS
    /// under a hardware keyboard: on the sixth production-app run the code fill
    /// waited out the keyboard wait there (rn-app-fixes task 6.5).
    HiddenPad,
}

/// How fake targets' fields take text whose method the target may choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Typing {
    /// Whole, set directly as the Android agent sets it.
    #[default]
    Whole,
    /// Key by key up to 32 characters, as a Simulator types it, and pasted
    /// whole beyond.
    Keys,
    /// As [`Typing::Keys`], each key from the fourth on landing before the
    /// field's last character, so "nobel" reads "noelb", as Bluesky's search
    /// field took it on the Simulator (`reports/10.4-rn-smoke.md`).
    Reorder,
    /// Whole, and the fields keep their first `n` characters, however text
    /// reaches them, as a field with a maximum length does.
    Max(usize),
    /// As [`Typing::Keys`] without Shift and Command: a shifted character
    /// types as its key does unshifted, "A@b" as "a2b", and a paste lands
    /// nothing, as on the sixth production-app run's Simulator with its
    /// hardware keyboard disconnected (rn-app-fixes task 6.5). On iOS such
    /// a paste into the field in focus, after the tap that readies it,
    /// leaves the edit menu that tap opened up over the screen.
    Unshifted,
}

impl Typing {
    /// Parses `whole`, `keys`, `reorder`, `unshifted` or `max:<n>`.
    pub fn parse(text: &str) -> Result<Typing, String> {
        let text = text.trim();
        match text {
            "whole" => Ok(Typing::Whole),
            "keys" => Ok(Typing::Keys),
            "reorder" => Ok(Typing::Reorder),
            "unshifted" => Ok(Typing::Unshifted),
            _ => text
                .strip_prefix("max:")
                .and_then(|n| n.parse().ok())
                .map(Typing::Max)
                .ok_or_else(|| {
                    format!(
                        "fake typing {text:?}: use whole, keys, reorder, unshifted or max:<characters>"
                    )
                }),
        }
    }

    /// Whether text whose method the target may choose is typed key by key
    /// when it is short enough, as a Simulator types it.
    pub fn types_keys(self) -> bool {
        matches!(self, Typing::Keys | Typing::Reorder | Typing::Unshifted)
    }
}

/// The runner processes fake iPhones started, across their attaches.
#[derive(Debug, Default)]
struct PhoneRunners {
    /// The pid the next runner takes.
    next_pid: u32,
    /// The runner processes on each phone, by its id.
    running: HashMap<String, Vec<u32>>,
    /// How many times each phone was attached.
    attaches: HashMap<String, u32>,
    /// The stalls [`FakeDriver::stall_runner`] put each phone's runner
    /// through since its attach.
    stalls: HashMap<String, u32>,
}

/// What a fake driver's iPhones keep across attaches: their runner
/// processes, shared by the driver's clones.
#[derive(Clone, Debug, Default)]
pub(crate) struct Runners(Arc<Mutex<PhoneRunners>>);

impl PartialEq for Runners {
    fn eq(&self, other: &Runners) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Runners {}

impl Runners {
    fn state(&self) -> MutexGuard<'_, PhoneRunners> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the runner of an attach of the phone `id`: the runners a lost
    /// session left on it are ended first, and the first attach's loses its
    /// connection `lost` after the attach, when given.
    fn start(&self, id: &str, lost: Option<Duration>) -> FakeRunner {
        let mut state = self.state();
        let attaches = state.attaches.entry(id.to_owned()).or_default();
        *attaches += 1;
        let first = *attaches == 1;
        state.next_pid += 1;
        let pid = 1000 + state.next_pid;
        let leftovers = state.running.insert(id.to_owned(), vec![pid]);
        state.stalls.remove(id);
        FakeRunner {
            runners: self.clone(),
            target: id.to_owned(),
            pid,
            leftovers: leftovers.unwrap_or_default(),
            lost: lost.filter(|_| first),
        }
    }
}

/// The reads of their trees a fake driver's targets were asked for, across
/// its attaches, shared by the driver's clones.
#[derive(Clone, Debug, Default)]
pub(crate) struct Reads(Arc<AtomicUsize>);

impl PartialEq for Reads {
    fn eq(&self, other: &Reads) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Reads {}

/// The runner of one attach of a fake iPhone.
#[derive(Debug)]
pub(crate) struct FakeRunner {
    runners: Runners,
    pub target: String,
    pub pid: u32,
    /// The runners a lost session left on the phone, ended before it
    /// started.
    pub leftovers: Vec<u32>,
    /// How long after the attach its connection is lost, its restart
    /// failing, when it is.
    pub lost: Option<Duration>,
}

impl FakeRunner {
    /// The stalls its phone's runner was put through since the attach.
    pub fn stalls(&self) -> u32 {
        self.runners
            .state()
            .stalls
            .get(&self.target)
            .copied()
            .unwrap_or(0)
    }

    /// Ends the runner as a release does, unless its connection was lost:
    /// a silent runner stays on the phone until the next attach ends it.
    pub fn release(&self, lost: bool) {
        if lost {
            return;
        }
        let mut state = self.runners.state();
        if let Some(running) = state.running.get_mut(&self.target) {
            running.retain(|&pid| pid != self.pid);
        }
    }
}

/// Fake targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeDriver {
    targets: Vec<TargetInfo>,
    timing: Timing,
    /// Whether fake emulators take the controller's path, with frames.
    controller: EmulatorController,
    /// Whether a stroke whose finger lifts moving glides the list on.
    fling: bool,
    /// How the fields take text whose method the target may choose.
    typing: Typing,
    /// Whether the keyboard stays up over every fixture screen.
    keyboard: bool,
    /// Whether fake iPhones' screenshots can be subscribed to.
    device_frames: bool,
    /// The runner processes of the fake iPhones.
    runners: Runners,
    /// How long after its first attach a fake iPhone's runner loses its
    /// connection, its restart failing, when it does.
    lost_runner: Option<Duration>,
    /// The reads of their trees the targets were asked for.
    reads: Reads,
}

impl FakeDriver {
    /// Parses a comma-separated list of `platform[:id]` entries, such as
    /// `android:emulator-5554,ios:FAKE-IOS-1`, the platform `android`, `ios`
    /// for a Simulator or `ios-device` for a physical iPhone. Without an id,
    /// Android fakes are `emulator-5554`, `emulator-5556` and so on, iOS fakes
    /// `FAKE-IOS-1`, `FAKE-IOS-2`, and fake iPhones `FAKE-IOS-DEVICE-1`,
    /// `FAKE-IOS-DEVICE-2`. An Android id starting `emulator-` is an
    /// emulator, any other a device.
    pub fn parse(spec: &str) -> Result<FakeDriver, String> {
        let mut targets: Vec<TargetInfo> = Vec::new();
        let (mut android, mut ios, mut devices) = (0u32, 0u32, 0u32);
        for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (platform, id) = match entry.split_once(':') {
                Some((platform, id)) => (platform.trim(), Some(id.trim())),
                None => (entry, None),
            };
            let device = platform.eq_ignore_ascii_case("ios-device");
            let platform = if device {
                Platform::Ios
            } else {
                Platform::parse(platform).ok_or_else(|| {
                    format!(
                        "fake target {entry:?}: the platform must be android or ios, or ios-device for a physical iPhone"
                    )
                })?
            };
            let id = match (platform, id) {
                (_, Some(id)) if !id.is_empty() => id.to_owned(),
                (Platform::Android, _) => {
                    android += 1;
                    format!("emulator-{}", 5552 + 2 * android)
                }
                (Platform::Ios, _) if device => {
                    devices += 1;
                    format!("FAKE-IOS-DEVICE-{devices}")
                }
                (Platform::Ios, _) => {
                    ios += 1;
                    format!("FAKE-IOS-{ios}")
                }
            };
            if targets.iter().any(|t| t.id == id) {
                return Err(format!("fake target {id} is listed twice"));
            }
            let validated = format!("iOS {}", ios::VALIDATED_IOS);
            let (class, name, os, state) = match platform {
                Platform::Android if id.starts_with("emulator-") => (
                    TargetClass::AndroidEmulator,
                    "Fake Android emulator",
                    "Android 16",
                    "device",
                ),
                Platform::Android => (
                    TargetClass::AndroidDevice,
                    "Fake Android device",
                    "Android 16",
                    "device",
                ),
                Platform::Ios if device => (
                    TargetClass::IosDevice,
                    "Fake physical iPhone",
                    validated.as_str(),
                    ios::Readiness::Ready.code(),
                ),
                Platform::Ios => (TargetClass::Simulator, "Fake iPhone", "iOS 26.5", "booted"),
            };
            targets.push(TargetInfo {
                id,
                platform,
                class,
                name: name.to_owned(),
                model: Some("fake".to_owned()),
                os: Some(os.to_owned()),
                state: state.to_owned(),
                advice: None,
                driver: "fake".to_owned(),
            });
        }
        Ok(FakeDriver {
            targets,
            timing: Timing::default(),
            controller: EmulatorController::Auto,
            fling: false,
            typing: Typing::Whole,
            keyboard: false,
            device_frames: true,
            runners: Runners::default(),
            lost_runner: None,
            reads: Reads::default(),
        })
    }

    /// These targets, showing each change `latency` after its cause.
    pub fn with_latency(mut self, latency: Duration) -> FakeDriver {
        self.timing.latency = latency;
        self
    }

    /// These targets, their frame streams starting `frame_delay` after the
    /// attach.
    pub fn with_frame_delay(mut self, frame_delay: Duration) -> FakeDriver {
        self.timing.frame_delay = frame_delay;
        self
    }

    /// These targets, the fixture apps showing their launch splash for
    /// `splash` when their process starts.
    pub fn with_splash(mut self, splash: Duration) -> FakeDriver {
        self.timing.splash = splash;
        self
    }

    /// These targets, their input sessions active `activation` after they
    /// start.
    pub fn with_activation(mut self, activation: Duration) -> FakeDriver {
        self.timing.activation = activation;
        self
    }

    /// These targets, each taking `stop` to detach.
    pub fn with_stop(mut self, stop: Duration) -> FakeDriver {
        self.timing.stop = stop;
        self
    }

    /// These targets under the daemon's emulator controller setting: off,
    /// emulators take the agent's path, without a frame source.
    pub fn with_emulator_controller(mut self, controller: EmulatorController) -> FakeDriver {
        self.controller = controller;
        self
    }

    /// These targets, their fixture's list gliding on after a stroke whose
    /// finger lifts moving when `fling` is set, as far and as long as the
    /// platform's fling carries it, a tap on the list while it glides
    /// stopping it.
    pub fn with_fling(mut self, fling: bool) -> FakeDriver {
        self.fling = fling;
        self
    }

    /// These targets, their fixture's fields taking text whose method the
    /// target may choose as `typing` says.
    pub fn with_typing(mut self, typing: Typing) -> FakeDriver {
        self.typing = typing;
        self
    }

    /// These targets, keeping their keyboard up over every screen of the
    /// fixture apps when `up` is set, whether or not a field has focus.
    pub fn with_keyboard(mut self, up: bool) -> FakeDriver {
        self.keyboard = up;
        self
    }

    /// These targets, their keyboard coming up as `slide` says once a field
    /// takes the focus.
    pub fn with_slide(mut self, slide: Slide) -> FakeDriver {
        self.timing.slide = slide;
        self
    }

    /// These targets, their home screen taking the front back from the first
    /// `launches` launches of a running app made while it is in front,
    /// [`realtime::HOME_RETURN`] after each, as a Simulator's home screen took
    /// it back from an app launched while its go-home animation ran
    /// (action-verdicts, task 2.4).
    pub fn with_home_returns(mut self, launches: u32) -> FakeDriver {
        self.timing.home_returns = launches;
        self
    }

    /// These targets, each read of their tree taking `cost`, in real time.
    pub fn with_read_cost(mut self, cost: Duration) -> FakeDriver {
        self.timing.read_cost = cost;
        self
    }

    /// These targets, their fake iPhones' screenshots subscribed to, or,
    /// unless `subscribed`, not.
    pub fn with_device_frames(mut self, subscribed: bool) -> FakeDriver {
        self.device_frames = subscribed;
        self
    }

    /// These targets, a fake iPhone's runner losing its connection `after`
    /// the phone's first attach, its restart failing, as a physical iPhone's
    /// runner did when its connection fell silent in the measured run
    /// (design D26 of physical-ios): from then on that attachment's tree
    /// reads and input fail `disconnected` with the physical driver's
    /// message, its status names the runner failed with no restart, and its
    /// process stays on the phone, where the next attach ends it before its
    /// own runner starts, naming it among the leftovers.
    pub fn with_lost_runner(mut self, after: Duration) -> FakeDriver {
        self.lost_runner = Some(after);
        self
    }

    /// Stalls the runner of the fake iPhone `id`'s attachment, as a physical
    /// iPhone's stalled on the 43rd emptying fill of every session in task
    /// 9.6 of physical-ios: the next tree read or text entry, which go
    /// through the runner, fails as the physical driver fails the call that
    /// finds a runner stalled (design D26), `runner_stalled`, which can be
    /// repeated, at the first stall since the attach, the runner started
    /// again and its status counting the restart; `disconnected` from the
    /// second, the session failed as a runner that died again after its
    /// restart, and a release ends the stalled runner. Taps, swipes and keys
    /// go through `dtuhidd`, which a stalled runner leaves alone.
    pub fn stall_runner(&self, id: &str) {
        *self
            .runners
            .state()
            .stalls
            .entry(id.to_owned())
            .or_default() += 1;
    }

    /// How many reads of their trees these targets were asked for, across
    /// their attaches, since the driver was made.
    pub fn tree_reads(&self) -> usize {
        self.reads.0.load(Ordering::Relaxed)
    }

    /// The runner processes on the fake iPhone `id` now, by pid.
    pub fn runners_on(&self, id: &str) -> Vec<u32> {
        self.runners
            .state()
            .running
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// These targets, the fake iPhone `id` listed in the readiness state
    /// `code` with `advice`, as the physical iOS driver lists a phone that
    /// is not ready (`crates/ios`'s readiness states and messages); an
    /// attach to it fails with `code`. Other targets are left as they are.
    pub fn with_readiness(mut self, id: &str, code: &str, advice: &str) -> FakeDriver {
        for target in &mut self.targets {
            if target.id == id && target.class == TargetClass::IosDevice {
                target.state = code.to_owned();
                target.advice = (code != ios::Readiness::Ready.code()).then(|| advice.to_owned());
            }
        }
        self
    }
}

/// What `status` shows about a fake Android emulator the switch keeps off
/// its controller: its input and frames take the agent's path, named as the
/// Android driver names them. A fake target on its default paths reports
/// nothing about them.
struct AgentPath;

impl Report for AgentPath {
    fn report(&self) -> TargetReport {
        let mut data = Map::new();
        data.insert("input".into(), "agent".into());
        data.insert("frames".into(), Value::Null);
        let mut report = TargetReport {
            lines: vec![
                "input: agent (taps, long presses and swipes)".to_owned(),
                "frames: none; screenshots come from the agent on demand".to_owned(),
            ],
            data,
        };
        report_controller(&mut report, EmulatorController::Off, true);
        report
    }
}

impl Driver for FakeDriver {
    fn name(&self) -> &str {
        "fake"
    }

    fn discover(&self) -> Result<Vec<TargetInfo>, DriverError> {
        Ok(self.targets.clone())
    }

    fn attach(
        &self,
        target: &TargetInfo,
        _options: &AttachOptions,
    ) -> Result<Attachment, DriverError> {
        // A phone that is not ready refuses a session with its state's
        // code, as the physical iOS driver's attach does.
        if target.class == TargetClass::IosDevice && target.state != ios::Readiness::Ready.code() {
            let advice = target.advice.as_deref().unwrap_or("it is not ready");
            return Err(DriverError::new(
                &target.state,
                format!("iOS device {} is not ready: {advice}", target.id),
            ));
        }
        let frames = match target.class {
            TargetClass::Simulator => Frames::Stream,
            TargetClass::IosDevice => Frames::Screenshots {
                subscribed: self.device_frames,
            },
            TargetClass::AndroidEmulator if self.controller == EmulatorController::Auto => {
                Frames::Stream
            }
            TargetClass::AndroidEmulator | TargetClass::AndroidDevice => Frames::None,
        };
        let runner = (target.class == TargetClass::IosDevice)
            .then(|| self.runners.start(&target.id, self.lost_runner));
        let mut attachment = realtime::attach(
            target.platform,
            frames,
            self.timing,
            self.fling,
            self.typing,
            self.keyboard,
            FromDriver {
                runner,
                reads: Arc::clone(&self.reads.0),
            },
        );
        if target.class == TargetClass::AndroidEmulator
            && self.controller == EmulatorController::Off
        {
            attachment.report = Some(Arc::new(AgentPath) as Arc<dyn Report>);
        }
        Ok(attachment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typing_modes_are_named_whole_keys_reorder_unshifted_or_max_with_a_length() {
        assert_eq!(Typing::parse("whole"), Ok(Typing::Whole));
        assert_eq!(Typing::parse(" keys "), Ok(Typing::Keys));
        assert_eq!(Typing::parse("reorder"), Ok(Typing::Reorder));
        assert_eq!(Typing::parse("unshifted"), Ok(Typing::Unshifted));
        assert_eq!(Typing::parse("max:8"), Ok(Typing::Max(8)));
        for bad in ["", "slow", "max:", "max:-1", "max:eight"] {
            assert!(Typing::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn fake_targets_take_default_ids_and_classes() {
        let driver = FakeDriver::parse("android, ios, android:R58M123 ,android").unwrap();
        let ids: Vec<(&str, TargetClass)> = driver
            .targets
            .iter()
            .map(|t| (t.id.as_str(), t.class))
            .collect();
        assert_eq!(
            ids,
            [
                ("emulator-5554", TargetClass::AndroidEmulator),
                ("FAKE-IOS-1", TargetClass::Simulator),
                ("R58M123", TargetClass::AndroidDevice),
                ("emulator-5556", TargetClass::AndroidEmulator),
            ]
        );
        assert!(FakeDriver::parse("windows").is_err());
        assert!(FakeDriver::parse("ios:X,android:X").is_err());
    }

    /// Task 10.12: with the emulator controller setting off, a fake
    /// emulator attaches without a frame source and reports the agent's
    /// paths, as a real emulator does; Simulators keep their frames.
    #[test]
    fn the_controller_setting_off_puts_fake_emulators_on_the_agent_path() {
        let targets = |driver: &FakeDriver| driver.targets.clone();
        for controller in [EmulatorController::Auto, EmulatorController::Off] {
            let driver = FakeDriver::parse("android, android:R58M123, ios")
                .unwrap()
                .with_emulator_controller(controller);
            let listed = targets(&driver);
            let attach = |id: &str| {
                let target = listed.iter().find(|t| t.id == id).unwrap();
                driver.attach(target, &AttachOptions::default()).unwrap()
            };
            let emulator = attach("emulator-5554");
            let off = controller == EmulatorController::Off;
            assert_eq!(emulator.backend.frames.is_some(), !off, "{controller:?}");
            match emulator.report.as_ref().map(|r| r.report()) {
                Some(report) => {
                    assert!(off, "{controller:?}: {:?}", report.data);
                    assert_eq!(report.data["input"], "agent");
                    assert!(report.data["frames"].is_null(), "{:?}", report.data);
                    assert_eq!(report.data["emulatorController"], "off");
                }
                None => assert!(!off, "an emulator on the agent's path says so"),
            }
            let device = attach("R58M123");
            assert!(device.backend.frames.is_none());
            assert!(device.report.is_none(), "{controller:?}");
            let simulator = attach("FAKE-IOS-1");
            assert!(simulator.backend.frames.is_some(), "{controller:?}");
            assert!(simulator.report.is_none());
        }
    }

    /// Task 5.1: `ios-device` stands in for a physical iPhone, its ids
    /// `FAKE-IOS-DEVICE-1`, `FAKE-IOS-DEVICE-2` unless given, listed in the
    /// `ios-device` class, ready, on the iOS the physical driver was
    /// validated on.
    #[test]
    fn ios_device_fakes_take_default_ids_and_the_ios_device_class() {
        let driver =
            FakeDriver::parse("ios-device, ios, ios-device:FAKE-IOS-DEVICE-9, ios-device").unwrap();
        let ids: Vec<(&str, TargetClass)> = driver
            .targets
            .iter()
            .map(|t| (t.id.as_str(), t.class))
            .collect();
        assert_eq!(
            ids,
            [
                ("FAKE-IOS-DEVICE-1", TargetClass::IosDevice),
                ("FAKE-IOS-1", TargetClass::Simulator),
                ("FAKE-IOS-DEVICE-9", TargetClass::IosDevice),
                ("FAKE-IOS-DEVICE-2", TargetClass::IosDevice),
            ]
        );
        let device = &driver.targets[0];
        assert_eq!(device.platform, Platform::Ios);
        assert_eq!(device.state, "ready");
        assert_eq!(device.os.as_deref(), Some("iOS 26.6.2"));
        assert_eq!(device.advice, None);
        let refused = FakeDriver::parse("ios-devices").unwrap_err();
        assert!(refused.contains("ios-device"), "{refused}");
    }

    /// Task 5.1, spec `ios-device-driver` "Device not ready": a fake iPhone
    /// can be listed in a readiness state with its advice, and an attach
    /// to it fails with the state's code, as the driver's commands do.
    #[test]
    fn an_unready_fake_iphone_is_listed_in_its_state_and_refuses_an_attach() {
        let advice = ios::Readiness::DeveloperModeOff.message();
        let driver = FakeDriver::parse("ios-device,ios-device")
            .unwrap()
            .with_readiness("FAKE-IOS-DEVICE-2", "developer_mode_off", &advice);
        let listed = driver.discover().unwrap();
        assert_eq!(listed[0].state, "ready");
        assert_eq!(listed[1].state, "developer_mode_off");
        assert_eq!(listed[1].advice.as_deref(), Some(advice.as_str()));
        match driver.attach(&listed[1], &AttachOptions::default()) {
            Ok(_) => panic!("an unready iPhone attaches"),
            Err(refused) => {
                assert_eq!(refused.code, "developer_mode_off");
                assert!(refused.message.contains("Developer Mode"), "{refused:?}");
            }
        }
        assert!(driver.attach(&listed[0], &AttachOptions::default()).is_ok());
    }

    /// Task 5.1: a fake iPhone's status names its input path, `dtuhidd`, its
    /// frames, screenshots polled at the device's rate, and the runner by its
    /// agent and restarts, as the physical driver's status does.
    #[test]
    fn a_fake_iphone_reports_dtuhidd_screenshots_and_its_runner() {
        let driver = FakeDriver::parse("ios-device").unwrap();
        let target = driver.discover().unwrap().remove(0);
        let attachment = driver.attach(&target, &AttachOptions::default()).unwrap();
        assert!(attachment.backend.frames.is_some());
        let report = attachment.report.expect("a report").report();
        let data = Value::Object(report.data.clone());
        assert_eq!(data["input"], "dtuhidd", "{data}");
        assert_eq!(data["frames"], "screenshots", "{data}");
        assert_eq!(data["runner"]["agent"], agent_ios::AGENT, "{data}");
        assert_eq!(data["runner"]["restarts"], 0, "{data}");
        // A phone on its cable, charging, as the harness's tests read it.
        assert_eq!(
            data["battery"],
            json!({"levelPct": 76, "charging": true, "externalPower": true}),
            "{data}"
        );
        let text = report.lines.join("\n");
        assert!(
            text.contains("input: dtuhidd")
                && text.contains("frames: screenshots")
                && text.contains(agent_ios::AGENT),
            "{text}"
        );
    }
}
