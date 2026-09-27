//! Task 10.10: the harness measures an Android emulator as it is, with nothing
//! between the bench and `mobile-agent`. It opens the Android fixtures'
//! screens through the launch option they read. The tests drive the CLI
//! adapter against canned replies.

mod common;

use std::fs;

use bench::adapter::{Adapter, Detection};
use bench::cli::{CliAdapter, CliConfig};
use bench::fixture::Fixture;
use bench::harness::Subject;
use bench::plan;
use bench::primitive::Primitive;
use bench::target::{Target, TargetClass};
use common::stand_in;

fn emulator() -> Target {
    Target::new("emulator-5554", TargetClass::AndroidEmulator)
}

/// The Android fixtures read the launch intent's string extra `initial_tab`,
/// the iOS ones the environment variable `FIXTURE_INITIAL_TAB`
/// (`fixtures/CONTRACT.md`, Launch options), and `open --env` passes either,
/// as an extra on Android: each target's screens open through its own.
#[test]
fn a_screen_opens_through_the_launch_option_its_platform_s_fixtures_read() {
    for (target, option) in [
        (emulator(), "initial_tab=list"),
        (
            Target::new("FAKE-IOS-1", TargetClass::Simulator),
            "FIXTURE_INITIAL_TAB=list",
        ),
    ] {
        let dir = common::scratch(&format!("android-launch-option-{}", target.id));
        let mut settings = CliConfig::new(dir.join("work"));
        settings.program = stand_in(&dir, &[]);
        let mut adapter = CliAdapter::new(settings);
        assert!(matches!(adapter.detect(), Detection::Available { .. }));
        let subject = Subject::Primitive(Primitive::TapDispatch);
        let scenario = plan::scenario(Primitive::TapDispatch, Fixture::Expo);
        adapter.prepare(&target, &scenario, &subject).unwrap();
        let log = fs::read_to_string(dir.join("mobile-agent.log")).unwrap();
        let open = format!(
            "--json --session mobile-agent-bench --device {} open --relaunch --env {option} -- \
             dev.mobileagent.fixture.expo",
            target.id
        );
        assert!(
            log.lines().any(|line| line == open),
            "no {open:?} in:\n{log}"
        );
    }
}
