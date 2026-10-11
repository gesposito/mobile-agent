//! The device tests' setups on the fake device (task 9.9): the helpers of
//! `tests/common/swiftui.rs`, which the device tests run on the phone, run
//! here through the driver's own launch, runner session, `dtuhidd` taps and
//! text entry, on the fake device's SwiftUI fixture (`fake_fixture.rs`),
//! whose reads come in the shapes the phone gave them.

use std::path::Path;
use std::time::Duration;

use protocol::message::{Call, Reply, TextMethod};

use crate::driver::{Input, Lifecycle, Session};
use crate::error::Result;
use crate::macos::driver::tests::{FakeHost, PHONE, core};
use crate::macos::driver::{Core, Gesture};
use crate::macos::fake_fixture::{FakeFixture, TAB_IDENTIFIERS_FROM, Tab};
use crate::model::{
    AttachOptions, Attached, Button, Dispatch, Installed, Key, LaunchOptions, Launched, Point,
    Released, RunnerSpec, Swipe, Terminated,
};
use crate::touch::TAP_HOLD;

// The device tests use helpers these tests do not.
#[allow(dead_code)]
#[path = "../../tests/common/swiftui.rs"]
mod swiftui;

impl Lifecycle for Core<FakeHost> {
    fn install(&self, target: &str, app: &Path) -> Result<Installed> {
        Core::<FakeHost>::install(self, target, app)
    }

    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched> {
        Core::<FakeHost>::launch(self, target, bundle_id, options)
    }

    fn terminate(&self, target: &str, bundle_id: &str) -> Result<Terminated> {
        Core::<FakeHost>::terminate(self, target, bundle_id)
    }

    fn running(&self, target: &str, bundle_id: &str) -> Result<Option<u32>> {
        Core::<FakeHost>::running(self, target, bundle_id)
    }
}

impl Session for Core<FakeHost> {
    fn attach(&self, target: &str, options: &AttachOptions) -> Result<Attached> {
        Core::<FakeHost>::attach(self, target, options)
    }

    fn call(&self, target: &str, call: Call, timeout: Duration) -> Result<Reply> {
        Core::<FakeHost>::call(self, target, call, timeout)
    }

    fn release(&self, target: &str) -> Result<Released> {
        Core::<FakeHost>::release(self, target)
    }
}

impl Input for Core<FakeHost> {
    fn tap(&self, target: &str, point: Point) -> Result<Dispatch> {
        self.input(target, Gesture::Press(point, TAP_HOLD))
    }

    fn press(&self, target: &str, point: Point, hold: Duration) -> Result<Dispatch> {
        self.input(target, Gesture::Press(point, hold))
    }

    fn swipe(&self, target: &str, swipe: &Swipe) -> Result<Dispatch> {
        self.input(target, Gesture::Swipe(*swipe))
    }

    fn button(&self, target: &str, button: Button) -> Result<Dispatch> {
        self.input(target, Gesture::Button(button))
    }

    fn key(&self, target: &str, key: Key) -> Result<Dispatch> {
        self.input(target, Gesture::Key(key))
    }

    fn keys(&self, target: &str, keys: &[Key]) -> Result<Dispatch> {
        self.input(target, Gesture::Keys(keys.to_vec()))
    }
}

const VERSION: &str = "0.1.0";
/// The harness's text entry, as the emptying fills type it.
const TEXT_ENTRY: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// The phone with the runner installed and the SwiftUI fixture shown.
fn phone() -> Core<FakeHost> {
    let host = FakeHost::ready();
    host.device.install_runner(VERSION);
    host.device.show_fixture(FakeFixture::swiftui());
    core(host)
}

/// The setup of `input::one_hundred_twenty_emptying_fills_in_one_session`,
/// which failed twice on the phone before any fill (task 9.7's device
/// check): the runner's read after the one that showed `row-1` had no
/// `tab-login` yet. Here the fixture lists the tab bar without identifiers
/// in that read too, and the setup reaches the Login tab's username field,
/// focused and empty, where the fills type and empty it.
#[test]
fn the_emptying_fills_setup_waits_for_the_tab_bar_swiftui_identifies_after_its_rows() {
    let core = phone();
    swiftui::on_the_list_with(&core, PHONE, AttachOptions::new(RunnerSpec::new(VERSION)));
    swiftui::on_the_focused_username(&core, PHONE);
    {
        let fixture = core.host.device.fixture();
        let fixture = fixture.as_ref().expect("the fixture is shown");
        assert_eq!(fixture.tab(), Tab::Login);
        assert!(fixture.focused(), "the username field has keyboard focus");
        assert_eq!(fixture.username(), "");
        assert!(
            fixture.reads() > TAB_IDENTIFIERS_FROM,
            "{} reads since the launch",
            fixture.reads()
        );
    }
    for (text, mode) in [(TEXT_ENTRY, TextMethod::KEYS), ("", TextMethod::DIRECT)] {
        let (method, _) = swiftui::enter(&core, PHONE, text, Some(mode));
        assert_eq!(method, mode);
        let fixture = core.host.device.fixture();
        assert_eq!(fixture.as_ref().map(FakeFixture::username), Some(text));
    }
    core.release(PHONE).expect("releases");
}
