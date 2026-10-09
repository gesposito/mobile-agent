//! The requests the runner serves, onto the shim's XCTest calls (design D20
//! to D22). Coordinates come in device pixels and go to XCTest in points.
//!
//! - `snapshot` reads the foreground application's tree through XCTest
//!   ([`xctree::read`]) and converts it with the Simulator reader's
//!   [`elements::snapshot`].
//! - `tap`, `swipe` and `key` go through XCTest's event synthesis, the
//!   fallback of the host's `dtuhidd` (D22): a tap held 50 ms unless the
//!   request says how long, a swipe that rests at its end when it asks to;
//!   a key of the HID Keyboard page (0x07) is typed as its character, and the
//!   home button's usage on the Consumer page (0x0C, usage 0x40, in the high
//!   half of the code) presses home.
//! - `text_set` types the text into the element with keyboard focus through
//!   the daemon's `_XCT_sendString`, or sets that element's value through
//!   the daemon's `_XCT_setAttribute` when it asks for the direct method,
//!   and says which it did.
//! - `screenshot` is `XCUIScreen`'s PNG at the screen's size, whatever the
//!   request asks: the host takes its screenshots through DVT (D23).

use std::collections::HashMap;

use protocol::message::{
    Call, ImageFormat, InputAck, KeyParams, Reply, Rotation, Screenshot, SwipeParams, TapParams,
    TextAck, TextMethod, TextParams,
};
use simulator_reader::elements::{self, Limits};

use crate::server::Handler;
use crate::shim::{Button, Shim};
use crate::xctree;

/// A tap's hold when the request names none, as the host's taps hold
/// (`ios::TAP_HOLD`).
pub const TAP_HOLD: f64 = 0.05;

/// The Consumer page in the high half of a `key` code: the page of the
/// hardware buttons, as the driver sends them to the runner.
pub const CONSUMER_PAGE: u32 = 0x000C_0000;

/// The runner's requests over a [`Shim`].
pub struct Dispatcher<S: Shim> {
    shim: S,
    /// Bundle identifiers already asked for, by pid.
    bundles: HashMap<i32, String>,
}

impl<S: Shim> Dispatcher<S> {
    pub fn new(shim: S) -> Dispatcher<S> {
        Dispatcher {
            shim,
            bundles: HashMap::new(),
        }
    }

    /// A position in device pixels, in points.
    fn points(&self, x: i32, y: i32) -> (f64, f64) {
        let scale = self.shim.scale();
        let scale = if scale > 0.0 { scale } else { 1.0 };
        (f64::from(x) / scale, f64::from(y) / scale)
    }

    fn tap(&self, params: &TapParams) -> Result<Reply, protocol::Error> {
        let hold = params
            .duration_ms
            .map_or(TAP_HOLD, |ms| f64::from(ms) / 1000.0);
        accepted(self.shim.tap(self.points(params.x, params.y), hold))
    }

    fn swipe(&self, params: &SwipeParams) -> Result<Reply, protocol::Error> {
        let rest = params.rest_ms.map_or(0.0, |ms| f64::from(ms) / 1000.0);
        accepted(self.shim.swipe(
            self.points(params.x1, params.y1),
            self.points(params.x2, params.y2),
            f64::from(params.duration_ms) / 1000.0,
            rest,
        ))
    }

    fn key(&self, params: &KeyParams) -> Result<Reply, protocol::Error> {
        if params.code == CONSUMER_PAGE | 0x40 {
            return accepted(self.shim.button(Button::Home));
        }
        match key_text(params.code) {
            Some(text) => accepted(self.shim.type_text(text)),
            None => Err(protocol::Error::new(
                protocol::code::INVALID_PARAMS,
                format!(
                    "key code {:#x} is neither a key of the HID Keyboard page the runner types \
                     nor the home button",
                    params.code
                ),
            )),
        }
    }

    fn text(&mut self, params: &TextParams) -> Result<Reply, protocol::Error> {
        let method = if params.mode == Some(TextMethod::DIRECT) {
            let bundle = self.foreground_bundle()?;
            self.shim
                .set_text(&bundle, &params.text)
                .map_err(internal)?;
            TextMethod::DIRECT
        } else {
            self.shim.type_text(&params.text).map_err(internal)?;
            TextMethod::KEYS
        };
        Ok(Reply::Text(TextAck { method }))
    }

    /// The bundle of the application in front.
    fn foreground_bundle(&mut self) -> Result<String, protocol::Error> {
        let pid = xctree::foreground_pid(&self.shim)?;
        xctree::bundle(&self.shim, &mut self.bundles, pid)
    }

    fn screenshot(&self) -> Result<Reply, protocol::Error> {
        let data = self.shim.screenshot().map_err(internal)?;
        let (width, height) = png_size(&data);
        Ok(Reply::Screenshot(Screenshot {
            format: ImageFormat::PNG,
            width,
            height,
            rotation: Rotation::R0,
            data,
        }))
    }
}

impl<S: Shim> Handler for Dispatcher<S> {
    fn handle(&mut self, call: Call) -> Result<Reply, protocol::Error> {
        match call {
            Call::Snapshot(params) => {
                let scale = self.shim.scale();
                xctree::read(
                    &self.shim,
                    &mut self.bundles,
                    Limits::from_params(&params),
                    scale,
                )
                .map(|read| Reply::Snapshot(elements::snapshot(&read)))
            }
            Call::Screenshot(_) => self.screenshot(),
            Call::Tap(params) => self.tap(&params),
            Call::Swipe(params) => self.swipe(&params),
            Call::Key(params) => self.key(&params),
            Call::TextSet(params) => self.text(&params),
            other => Err(protocol::Error::unknown_method(other.method())),
        }
    }
}

fn internal(message: String) -> protocol::Error {
    protocol::Error::new(protocol::code::INTERNAL, message)
}

fn accepted(result: Result<(), String>) -> Result<Reply, protocol::Error> {
    result
        .map(|()| Reply::Input(InputAck { accepted: true }))
        .map_err(internal)
}

/// The character the runner types for a key of the HID Keyboard page.
fn key_text(code: u32) -> Option<&'static str> {
    const LETTERS: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    Some(match code {
        0x04..=0x1d => LETTERS[(code - 0x04) as usize],
        0x1e..=0x27 => DIGITS[(code - 0x1e) as usize],
        0x28 => "\n",
        0x2a => "\u{8}",
        0x2b => "\t",
        0x2c => " ",
        _ => return None,
    })
}

/// The width and height a PNG's header names; zeros when it is no PNG.
fn png_size(data: &[u8]) -> (u32, u32) {
    if data.len() >= 24 && &data[1..4] == b"PNG" && &data[12..16] == b"IHDR" {
        let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        (width, height)
    } else {
        (0, 0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use protocol::message::{
        ImageFormat, InputAck, KeyParams, Rotation, ScreenshotParams, SnapshotParams, SwipeParams,
        TapParams, TextAck, TextMethod, TextParams,
    };
    use protocol::snapshot::WindowType;

    use super::*;
    use crate::shim::{Button, Foreground};

    /// What the fake shim was asked to do.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Act {
        Bundle(i32),
        Tree(i32),
        Tap((f64, f64), f64),
        Swipe((f64, f64), (f64, f64), f64, f64),
        Button(Button),
        Type(String),
        Set(String, String),
        Screenshot,
    }

    #[derive(Default)]
    pub struct State {
        pub acts: Vec<Act>,
        /// The foregrounds the shim answers in turn; the last one stays.
        pub foregrounds: VecDeque<Foreground>,
        pub bundles: HashMap<i32, String>,
        pub tree: Option<Result<String, String>>,
        pub typing: Option<String>,
        pub png: Vec<u8>,
    }

    /// A shim on a screen of scale 2 that records what it is asked.
    #[derive(Clone, Default)]
    pub struct Fake(pub Arc<Mutex<State>>);

    impl Fake {
        pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.0.lock().expect("the fake's state")
        }

        fn act(&self, act: Act) {
            self.state().acts.push(act);
        }

        pub fn acts(&self) -> Vec<Act> {
            self.state().acts.clone()
        }

        /// The fixture in front as `pid`, showing `tree`.
        pub fn showing(pid: i32, bundle: &str, tree: &str) -> Fake {
            let fake = Fake::default();
            {
                let mut state = fake.state();
                state.foregrounds.push_back(Foreground {
                    active: vec![pid],
                    system: Some(34),
                });
                state.bundles.insert(pid, bundle.to_owned());
                state.bundles.insert(34, "com.apple.springboard".to_owned());
                state.tree = Some(Ok(tree.to_owned()));
            }
            fake
        }
    }

    impl Shim for Fake {
        fn scale(&self) -> f64 {
            2.0
        }

        fn foreground(&self) -> Foreground {
            let mut state = self.state();
            if state.foregrounds.len() > 1 {
                state.foregrounds.pop_front().unwrap_or_default()
            } else {
                state.foregrounds.front().cloned().unwrap_or_default()
            }
        }

        fn bundle(&self, pid: i32) -> Option<String> {
            self.act(Act::Bundle(pid));
            self.state().bundles.get(&pid).cloned()
        }

        fn tree(&self, pid: i32) -> Result<String, String> {
            self.act(Act::Tree(pid));
            self.state()
                .tree
                .clone()
                .unwrap_or_else(|| Err("no tree".into()))
        }

        fn tap(&self, point: (f64, f64), hold: f64) -> Result<(), String> {
            self.act(Act::Tap(point, hold));
            Ok(())
        }

        fn swipe(
            &self,
            from: (f64, f64),
            to: (f64, f64),
            duration: f64,
            rest: f64,
        ) -> Result<(), String> {
            self.act(Act::Swipe(from, to, duration, rest));
            Ok(())
        }

        fn button(&self, button: Button) -> Result<(), String> {
            self.act(Act::Button(button));
            Ok(())
        }

        fn type_text(&self, text: &str) -> Result<(), String> {
            self.act(Act::Type(text.to_owned()));
            match self.state().typing.clone() {
                Some(refusal) => Err(refusal),
                None => Ok(()),
            }
        }

        fn set_text(&self, bundle: &str, text: &str) -> Result<(), String> {
            self.act(Act::Set(bundle.to_owned(), text.to_owned()));
            Ok(())
        }

        fn screenshot(&self) -> Result<Vec<u8>, String> {
            self.act(Act::Screenshot);
            Ok(self.state().png.clone())
        }
    }

    pub const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";

    pub const TREE: &str = r#"{"t":2,"f":[0,0,375,667],"l":"FixtureSwiftUI","c":[
        {"t":4,"f":[0,0,375,667],"c":[
            {"t":9,"tr":1,"f":[8,56.5,18,21],"i":"cell-1","l":"Cell 1"}]}]}"#;

    fn snapshot() -> Call {
        Call::Snapshot(SnapshotParams::default())
    }

    #[test]
    fn a_snapshot_reads_the_foreground_applications_tree_and_asks_its_bundle_once() {
        let fake = Fake::showing(812, SWIFTUI, TREE);
        let mut dispatcher = Dispatcher::new(fake.clone());
        let Ok(Reply::Snapshot(first)) = dispatcher.handle(snapshot()) else {
            panic!("a snapshot");
        };
        assert_eq!(first.windows.len(), 1);
        let window = &first.windows[0];
        assert_eq!(window.id, 812);
        assert_eq!(window.kind, WindowType::APPLICATION);
        assert_eq!(window.emits_events, Some(false));
        assert_eq!(first.node_count, 3);
        dispatcher.handle(snapshot()).expect("a second snapshot");
        assert_eq!(
            fake.acts(),
            vec![Act::Bundle(812), Act::Tree(812), Act::Tree(812)],
            "the bundle is asked once per pid"
        );
    }

    #[test]
    fn the_home_screen_is_read_from_springboard_as_system_ui() {
        let fake = Fake::showing(812, SWIFTUI, r#"{"t":2,"f":[0,0,375,667]}"#);
        fake.state().foregrounds = VecDeque::from([Foreground {
            active: vec![],
            system: Some(34),
        }]);
        let Ok(Reply::Snapshot(home)) = Dispatcher::new(fake.clone()).handle(snapshot()) else {
            panic!("a snapshot");
        };
        assert_eq!(home.windows[0].kind, WindowType::SYSTEM);
        assert_eq!(home.windows[0].id, 34);
        assert!(fake.acts().contains(&Act::Tree(34)));
        assert!(
            fake.acts().contains(&Act::Bundle(34)),
            "SpringBoard's bundle"
        );
    }

    #[test]
    fn nothing_in_front_is_no_foreground_and_can_be_repeated() {
        let fake = Fake::showing(812, SWIFTUI, TREE);
        fake.state().foregrounds = VecDeque::from([Foreground::default()]);
        let error = Dispatcher::new(fake)
            .handle(snapshot())
            .expect_err("nothing in front");
        assert_eq!(error.code, simulator_reader::codes::NO_FOREGROUND);
        assert!(error.retryable);
    }

    #[test]
    fn a_foreground_that_changes_during_the_read_is_window_changed() {
        let fake = Fake::showing(812, SWIFTUI, TREE);
        fake.state().foregrounds.push_back(Foreground {
            active: vec![900],
            system: Some(34),
        });
        let error = Dispatcher::new(fake)
            .handle(snapshot())
            .expect_err("the foreground changed");
        assert_eq!(error.code, protocol::code::WINDOW_CHANGED);
        assert!(error.retryable);
    }

    #[test]
    fn a_tree_xctest_cannot_take_is_snapshot_failed_and_an_unknown_pid_too() {
        let fake = Fake::showing(812, SWIFTUI, TREE);
        fake.state().tree = Some(Err("kAXErrorServerNotFound".into()));
        let error = Dispatcher::new(fake.clone())
            .handle(snapshot())
            .expect_err("no tree");
        assert_eq!(error.code, simulator_reader::codes::SNAPSHOT_FAILED);
        assert!(error.retryable && error.message.contains("kAXErrorServerNotFound"));

        fake.state().bundles.clear();
        let error = Dispatcher::new(fake)
            .handle(snapshot())
            .expect_err("no bundle");
        assert_eq!(error.code, simulator_reader::codes::SNAPSHOT_FAILED);
        assert!(error.message.contains("812"), "{}", error.message);
    }

    #[test]
    fn a_screenshot_is_the_png_with_the_size_its_header_names() {
        let fake = Fake::default();
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&750u32.to_be_bytes());
        png.extend_from_slice(&1334u32.to_be_bytes());
        fake.state().png = png.clone();
        let reply = Dispatcher::new(fake)
            .handle(Call::Screenshot(ScreenshotParams::default()))
            .expect("a screenshot");
        let Reply::Screenshot(shot) = reply else {
            panic!("a screenshot");
        };
        assert_eq!(shot.format, ImageFormat::PNG);
        assert_eq!((shot.width, shot.height), (750, 1334));
        assert_eq!(shot.rotation, Rotation::R0);
        assert_eq!(shot.data, png);
    }

    #[test]
    fn a_tap_goes_to_xctest_in_points_held_50_ms_or_for_its_duration() {
        let fake = Fake::default();
        let mut dispatcher = Dispatcher::new(fake.clone());
        let accepted = Ok(Reply::Input(InputAck { accepted: true }));
        let tap = |duration_ms| {
            Call::Tap(TapParams {
                x: 100,
                y: 201,
                duration_ms,
            })
        };
        assert_eq!(dispatcher.handle(tap(None)), accepted);
        assert_eq!(dispatcher.handle(tap(Some(600))), accepted);
        assert_eq!(
            fake.acts(),
            vec![Act::Tap((50.0, 100.5), 0.05), Act::Tap((50.0, 100.5), 0.6)]
        );
    }

    #[test]
    fn a_swipe_goes_in_points_with_its_duration_and_its_rest() {
        let fake = Fake::default();
        let mut dispatcher = Dispatcher::new(fake.clone());
        let swipe = |rest_ms| {
            Call::Swipe(SwipeParams {
                x1: 374,
                y1: 1040,
                x2: 374,
                y2: 640,
                duration_ms: 300,
                rest_ms,
            })
        };
        dispatcher.handle(swipe(None)).expect("a swipe");
        dispatcher.handle(swipe(Some(60))).expect("a resting swipe");
        assert_eq!(
            fake.acts(),
            vec![
                Act::Swipe((187.0, 520.0), (187.0, 320.0), 0.3, 0.0),
                Act::Swipe((187.0, 520.0), (187.0, 320.0), 0.3, 0.06)
            ]
        );
    }

    #[test]
    fn keyboard_usages_are_typed_and_the_home_usage_presses_home() {
        let fake = Fake::default();
        let mut dispatcher = Dispatcher::new(fake.clone());
        let key = |code| Call::Key(KeyParams { code, meta: None });
        for code in [0x04, 0x1d, 0x1e, 0x27, 0x28, 0x2a, 0x2c, 0x000C_0040] {
            dispatcher
                .handle(key(code))
                .unwrap_or_else(|error| panic!("{code:#x}: {error}"));
        }
        assert_eq!(
            fake.acts(),
            vec![
                Act::Type("a".into()),
                Act::Type("z".into()),
                Act::Type("1".into()),
                Act::Type("0".into()),
                Act::Type("\n".into()),
                Act::Type("\u{8}".into()),
                Act::Type(" ".into()),
                Act::Button(Button::Home),
            ]
        );
        let error = dispatcher.handle(key(0x3a)).expect_err("F1 types nothing");
        assert_eq!(error.code, protocol::code::INVALID_PARAMS);
        assert!(error.message.contains("0x3a"), "{}", error.message);
    }

    #[test]
    fn text_is_typed_or_set_on_the_focused_field_and_the_method_reported() {
        let fake = Fake::showing(812, SWIFTUI, TREE);
        let mut dispatcher = Dispatcher::new(fake.clone());
        let text = |mode| {
            Call::TextSet(TextParams {
                text: "ada".into(),
                node: None,
                mode,
            })
        };
        let method = |reply: Result<Reply, protocol::Error>| match reply {
            Ok(Reply::Text(TextAck { method })) => method,
            other => panic!("a text acknowledgement, not {other:?}"),
        };
        assert_eq!(
            method(dispatcher.handle(text(Some(TextMethod::KEYS)))),
            TextMethod::KEYS
        );
        assert_eq!(
            method(dispatcher.handle(text(Some(TextMethod::DIRECT)))),
            TextMethod::DIRECT
        );
        assert_eq!(method(dispatcher.handle(text(None))), TextMethod::KEYS);
        assert_eq!(
            fake.acts(),
            vec![
                Act::Type("ada".into()),
                Act::Bundle(812),
                Act::Set(SWIFTUI.into(), "ada".into()),
                Act::Type("ada".into()),
            ]
        );
    }
}
