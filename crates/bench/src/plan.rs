//! The run plan for the tool under test: the scenario each primitive is
//! measured in and the flows each fixture runs. Fixtures and screens are those
//! of `fixtures/CONTRACT.md`.

use crate::adapter::Scenario;
use crate::fixture::{Fixture, Screen};
use crate::flow::{self, Flow};
use crate::harness::{Series, Subject};
use crate::primitive::Primitive;
use crate::target::{Platform, Target, TargetClass};

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

/// The series of `planned` that measure one of `primitives` or time one of
/// the flows `flows` names, in plan order; every series of `planned` when
/// both are empty.
pub fn only(planned: Vec<Series>, primitives: &[Primitive], flows: &[String]) -> Vec<Series> {
    if primitives.is_empty() && flows.is_empty() {
        return planned;
    }
    planned
        .into_iter()
        .filter(|series| match &series.subject {
            Subject::Primitive(primitive) => primitives.contains(primitive),
            Subject::Flow(name) => flows.contains(name),
        })
        .collect()
}

/// The targets `ids` names among `connected`, in the order named and each
/// once, with the fixtures each runs: `chosen` when given, else every fixture
/// of its platform (SwiftUI and Expo on a Simulator, Expo and Compose on
/// Android). Fails when `ids` names no target, on an id no connected target
/// has, naming the connected ones, and on a chosen fixture a target's
/// platform lacks.
pub fn select(
    connected: &[Target],
    ids: &[String],
    chosen: &[Fixture],
) -> Result<Vec<(Target, Vec<Fixture>)>, String> {
    if ids.is_empty() {
        return Err("name a target to measure".to_owned());
    }
    let mut selected: Vec<(Target, Vec<Fixture>)> = Vec::new();
    for id in ids {
        if selected.iter().any(|(target, _)| target.id == *id) {
            continue;
        }
        let target = connected
            .iter()
            .find(|target| target.id == *id)
            .ok_or_else(|| {
                let listed: Vec<String> = connected
                    .iter()
                    .map(|target| format!("{} ({})", target.id, target.class))
                    .collect();
                format!(
                    "no connected target is {id}; mobile-agent devices lists {}",
                    joined(&listed)
                )
            })?;
        let platform = target.class.platform();
        let available = fixtures(platform);
        let mut runs = Vec::new();
        for &fixture in chosen {
            if fixture.app_id(platform).is_none() {
                let names: Vec<String> = available.iter().map(ToString::to_string).collect();
                let article = match target.class {
                    TargetClass::Simulator => "a",
                    TargetClass::AndroidEmulator | TargetClass::AndroidDevice => "an",
                };
                return Err(format!(
                    "the {fixture} fixture does not run on {}, {article} {}; its fixtures are {}",
                    target.id,
                    target.class,
                    joined(&names)
                ));
            }
            if !runs.contains(&fixture) {
                runs.push(fixture);
            }
        }
        if runs.is_empty() {
            runs = available;
        }
        selected.push((target.clone(), runs));
    }
    Ok(selected)
}

/// `items` as a phrase: `a`, `a and b`, `a, b and c`; `none` for no items.
fn joined(items: &[String]) -> String {
    match items {
        [] => "none".to_owned(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
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

    fn connected() -> Vec<Target> {
        vec![
            Target::new("emulator-5554", TargetClass::AndroidEmulator),
            Target::new("SIM-1", TargetClass::Simulator),
            Target::new("R58M123", TargetClass::AndroidDevice),
        ]
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn each_named_target_runs_its_platform_s_fixtures_by_default() {
        let chosen = select(&connected(), &ids(&["SIM-1", "emulator-5554"]), &[]).unwrap();
        let chosen: Vec<(&str, Vec<Fixture>)> = chosen
            .iter()
            .map(|(target, fixtures)| (target.id.as_str(), fixtures.clone()))
            .collect();
        assert_eq!(
            chosen,
            [
                ("SIM-1", vec![Fixture::SwiftUi, Fixture::Expo]),
                ("emulator-5554", vec![Fixture::Expo, Fixture::Compose]),
            ]
        );
    }

    #[test]
    fn chosen_fixtures_run_on_every_named_target() {
        let chosen = select(
            &connected(),
            &ids(&["emulator-5554", "R58M123"]),
            &[Fixture::Compose],
        )
        .unwrap();
        assert!(
            chosen
                .iter()
                .all(|(_, fixtures)| *fixtures == [Fixture::Compose])
        );
        assert_eq!(chosen[1].0, connected()[2]);
    }

    #[test]
    fn a_target_nobody_connected_is_refused_naming_the_connected_ones() {
        let error = select(&connected(), &ids(&["emulator-5556"]), &[]).unwrap_err();
        assert_eq!(
            error,
            "no connected target is emulator-5556; mobile-agent devices lists emulator-5554 \
             (android-emulator), SIM-1 (simulator) and R58M123 (android-device)"
        );
        let error = select(&[], &ids(&["SIM-1"]), &[]).unwrap_err();
        assert_eq!(
            error,
            "no connected target is SIM-1; mobile-agent devices lists none"
        );
        assert!(select(&connected(), &[], &[]).is_err());
    }

    #[test]
    fn named_flows_keep_only_their_series_in_plan_order() {
        let target = Target::new("SIM-1", TargetClass::Simulator);
        let planned = series(&target, &[Fixture::SwiftUi, Fixture::Expo]);
        let kept = |names: &[&str]| -> Vec<(Option<Fixture>, String)> {
            let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
            only(planned.clone(), &[], &names)
                .iter()
                .map(|s| (s.fixture, s.subject.to_string()))
                .collect()
        };
        let on = |fixture, name: &str| (Some(fixture), format!("flow {name}"));
        assert_eq!(
            kept(&[flow::LIST_SCROLL]),
            [
                on(Fixture::SwiftUi, "list-scroll"),
                on(Fixture::Expo, "list-scroll")
            ]
        );
        assert_eq!(
            kept(&[flow::BIG_SCREEN, flow::LOGIN]),
            [
                on(Fixture::SwiftUi, "login"),
                on(Fixture::SwiftUi, "big-screen"),
                on(Fixture::Expo, "login"),
                on(Fixture::Expo, "big-screen"),
            ]
        );
        assert_eq!(only(planned.clone(), &[], &[]), planned);
    }

    #[test]
    fn named_primitives_keep_their_series_beside_the_named_flows_in_plan_order() {
        let target = Target::new("SIM-1", TargetClass::Simulator);
        let planned = series(&target, &[Fixture::SwiftUi, Fixture::Expo]);
        let kept = |primitives: &[Primitive], flows: &[&str]| -> Vec<(Option<Fixture>, String)> {
            let flows: Vec<String> = flows.iter().map(|name| (*name).to_owned()).collect();
            only(planned.clone(), primitives, &flows)
                .iter()
                .map(|s| (s.fixture, s.subject.to_string()))
                .collect()
        };
        let on = |fixture, subject: &str| (Some(fixture), subject.to_owned());
        assert_eq!(
            kept(&[Primitive::TextEntry], &[]),
            [
                on(Fixture::SwiftUi, "text-entry"),
                on(Fixture::Expo, "text-entry")
            ]
        );
        assert_eq!(
            kept(&[Primitive::TextEntry], &[flow::LOGIN]),
            [
                on(Fixture::SwiftUi, "text-entry"),
                on(Fixture::SwiftUi, "flow login"),
                on(Fixture::Expo, "text-entry"),
                on(Fixture::Expo, "flow login"),
            ]
        );
        assert_eq!(
            kept(&[Primitive::Settle, Primitive::CliStart], &[]),
            [
                (None, "cli-start".to_owned()),
                on(Fixture::SwiftUi, "settle"),
                on(Fixture::Expo, "settle")
            ]
        );
    }

    #[test]
    fn a_fixture_a_target_s_platform_lacks_is_refused() {
        let error =
            select(&connected(), &ids(&["emulator-5554"]), &[Fixture::SwiftUi]).unwrap_err();
        assert_eq!(
            error,
            "the swiftui fixture does not run on emulator-5554, an android-emulator; its \
             fixtures are expo and compose"
        );
    }

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
