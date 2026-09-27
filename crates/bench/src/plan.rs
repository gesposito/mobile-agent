//! The run plan for the tool under test: the scenario each primitive is
//! measured in, and the flows each fixture runs.

use crate::adapter::Scenario;
use crate::fixture::Fixture;
use crate::flow::Flow;
use crate::harness::Series;
use crate::primitive::Primitive;
use crate::target::{Platform, Target};

/// Every fixture, in report order.
pub const FIXTURES: [Fixture; 3] = [Fixture::SwiftUi, Fixture::Expo, Fixture::Compose];

/// The fixtures `platform` has.
pub fn fixtures(platform: Platform) -> Vec<Fixture> {
    let _ = platform;
    Vec::new()
}

/// Where `primitive` is measured with `fixture`.
pub fn scenario(primitive: Primitive, fixture: Fixture) -> Scenario {
    let _ = (primitive, fixture);
    Scenario::default()
}

/// The series on `target` with `fixtures`.
pub fn series(target: &Target, fixtures: &[Fixture]) -> Vec<Series> {
    let _ = (target, fixtures);
    Vec::new()
}

/// The reference flows of `fixtures`.
pub fn flows(fixtures: &[Fixture]) -> Vec<Flow> {
    let _ = fixtures;
    Vec::new()
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
