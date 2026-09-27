//! The run plan for the tool under test: the scenario each primitive is
//! measured in and the flows each fixture runs. Fixtures and screens are those
//! of `fixtures/CONTRACT.md`.

use crate::adapter::Scenario;
use crate::fixture::{Fixture, Screen};
use crate::flow::{self, Flow};
use crate::harness::{Series, Subject};
use crate::primitive::Primitive;
use crate::target::{Platform, Target};

/// Every fixture, in report order.
pub const FIXTURES: [Fixture; 3] = [Fixture::SwiftUi, Fixture::Expo, Fixture::Compose];

/// The fixtures `platform` has, in report order: SwiftUI and Expo on iOS,
/// Expo and Compose on Android.
pub fn fixtures(platform: Platform) -> Vec<Fixture> {
    FIXTURES
        .into_iter()
        .filter(|fixture| fixture.app_id(platform).is_some())
        .collect()
}

/// Where `primitive` is measured with `fixture`. CLI process start involves no
/// app; cold attach opens the fixture on whatever screen it shows; snapshots
/// and the screenshot use the 500-cell screen, the spec's reference screen;
/// taps and settle detection the List screen, whose rows open a detail; text
/// entry the Login screen's username field.
pub fn scenario(primitive: Primitive, fixture: Fixture) -> Scenario {
    let on = |screen| Scenario {
        fixture: Some(fixture),
        screen: Some(screen),
    };
    match primitive {
        Primitive::CliStart => Scenario::default(),
        Primitive::ColdAttach => Scenario {
            fixture: Some(fixture),
            screen: None,
        },
        Primitive::FirstSnapshot | Primitive::WarmSnapshot | Primitive::Screenshot => {
            on(Screen::Big)
        }
        Primitive::TapDispatch
        | Primitive::TapToFrameChange
        | Primitive::TapToTreeDelta
        | Primitive::Settle => on(Screen::List),
        Primitive::TextEntry => on(Screen::Login),
    }
}

/// The series on `target` with `fixtures`: CLI process start once, then for
/// each fixture every other primitive in its scenario, in [`Primitive::ALL`]
/// order, and the reference flows.
pub fn series(target: &Target, fixtures: &[Fixture]) -> Vec<Series> {
    let series = |fixture, screen, subject| Series {
        target: target.id.clone(),
        fixture,
        screen,
        subject,
    };
    let mut planned = vec![series(None, None, Subject::Primitive(Primitive::CliStart))];
    for &fixture in fixtures {
        for primitive in Primitive::ALL {
            if primitive != Primitive::CliStart {
                let at = scenario(primitive, fixture);
                planned.push(series(at.fixture, at.screen, Subject::Primitive(primitive)));
            }
        }
        for flow in flow::reference(fixture) {
            planned.push(series(Some(fixture), None, Subject::Flow(flow.name)));
        }
    }
    planned
}

/// The reference flows of `fixtures`, which the plan's flow series name.
pub fn flows(fixtures: &[Fixture]) -> Vec<Flow> {
    fixtures.iter().copied().flat_map(flow::reference).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Screen;
    use crate::flow;
    use crate::harness::Subject;
    use crate::target::TargetClass;

    #[test]
    fn each_platform_has_its_fixtures_in_report_order() {
        assert_eq!(fixtures(Platform::Ios), [Fixture::SwiftUi, Fixture::Expo]);
        assert_eq!(
            fixtures(Platform::Android),
            [Fixture::Expo, Fixture::Compose]
        );
    }

    #[test]
    fn each_primitive_has_its_planned_scenario() {
        let on = |screen| Scenario {
            fixture: Some(Fixture::Compose),
            screen,
        };
        let planned = |primitive| scenario(primitive, Fixture::Compose);
        assert_eq!(planned(Primitive::CliStart), Scenario::default());
        assert_eq!(planned(Primitive::ColdAttach), on(None));
        for primitive in [
            Primitive::FirstSnapshot,
            Primitive::WarmSnapshot,
            Primitive::Screenshot,
        ] {
            assert_eq!(planned(primitive), on(Some(Screen::Big)), "{primitive}");
        }
        for primitive in [
            Primitive::TapDispatch,
            Primitive::TapToFrameChange,
            Primitive::TapToTreeDelta,
            Primitive::Settle,
        ] {
            assert_eq!(planned(primitive), on(Some(Screen::List)), "{primitive}");
        }
        assert_eq!(planned(Primitive::TextEntry), on(Some(Screen::Login)));
    }

    #[test]
    fn a_target_measures_process_start_once_then_each_fixture_s_primitives_and_flows() {
        let target = Target::new("emulator-5554", TargetClass::AndroidEmulator);
        let series = series(&target, &[Fixture::Expo, Fixture::Compose]);
        assert!(series.iter().all(|s| s.target == "emulator-5554"));
        let subjects: Vec<(Option<Fixture>, String)> = series
            .iter()
            .map(|s| (s.fixture, s.subject.to_string()))
            .collect();
        let mut expected = vec![(None, "cli-start".to_owned())];
        for fixture in [Fixture::Expo, Fixture::Compose] {
            for primitive in Primitive::ALL.into_iter().skip(1) {
                expected.push((Some(fixture), primitive.to_string()));
            }
            for name in ["login", "list-scroll", "big-screen"] {
                expected.push((Some(fixture), format!("flow {name}")));
            }
        }
        assert_eq!(subjects, expected);
        for s in &series {
            match &s.subject {
                Subject::Primitive(primitive) => {
                    let planned = scenario(*primitive, s.fixture.unwrap_or(Fixture::Expo));
                    assert_eq!(s.scenario(), planned, "{}", s.subject);
                }
                Subject::Flow(_) => assert_eq!(s.screen, None),
            }
        }
        assert_eq!(
            flows(&[Fixture::Expo, Fixture::Compose]),
            [
                flow::reference(Fixture::Expo),
                flow::reference(Fixture::Compose)
            ]
            .concat()
        );
    }
}
