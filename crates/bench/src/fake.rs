//! A scripted adapter: each attempt returns the next scripted duration or an
//! injected failure. Tests drive the harness with it, and `run --fake` loads a
//! script from JSON to exercise the harness without a device.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::adapter::{Adapter, AdapterError, Clock, Detection, Role, Sample, Scenario};
use crate::fixture::{Fixture, Screen};
use crate::harness::{Series, Subject};
use crate::primitive::Primitive;
use crate::target::Target;

/// A fake tool: its identity, the targets it reports and one entry per series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FakeScript {
    /// The tool name reports show.
    pub tool: String,
    pub version: Option<String>,
    #[serde(default = "under_test")]
    pub role: Role,
    pub targets: Vec<Target>,
    pub series: Vec<FakeSeries>,
}

fn under_test() -> Role {
    Role::UnderTest
}

fn tool_clock() -> Clock {
    Clock::Tool
}

/// The scripted behaviour of one series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FakeSeries {
    pub target: String,
    pub fixture: Option<Fixture>,
    pub screen: Option<Screen>,
    #[serde(flatten)]
    pub subject: Subject,
    /// Durations in milliseconds, one per attempt in order, starting over when
    /// exhausted. A failing attempt consumes its entry too.
    pub ms: Vec<f64>,
    /// Zero-based attempt numbers within the series, warm-up included, that fail.
    #[serde(default)]
    pub fail: Vec<u32>,
    pub nodes: Option<u32>,
    #[serde(default = "tool_clock")]
    pub clock: Clock,
}

impl FakeSeries {
    /// A series that never fails and reports no node count.
    pub fn new(
        target: &str,
        fixture: Option<Fixture>,
        screen: Option<Screen>,
        subject: Subject,
        ms: Vec<f64>,
    ) -> Self {
        FakeSeries {
            target: target.to_owned(),
            fixture,
            screen,
            subject,
            ms,
            fail: Vec::new(),
            nodes: None,
            clock: Clock::Tool,
        }
    }
}

/// The adapter a [`FakeScript`] describes.
#[derive(Clone, Debug)]
pub struct FakeAdapter {
    script: FakeScript,
    attempts: Vec<u32>,
}

impl FakeAdapter {
    pub fn new(script: FakeScript) -> Self {
        let attempts = vec![0; script.series.len()];
        FakeAdapter { script, attempts }
    }

    /// Loads a script from JSON.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text)
            .map(FakeAdapter::new)
            .map_err(|e| format!("fake script: {e}"))
    }

    /// The targets the script declares.
    pub fn targets(&self) -> Vec<Target> {
        self.script.targets.clone()
    }

    /// One series per scripted series, in script order.
    pub fn series(&self) -> Vec<Series> {
        self.script
            .series
            .iter()
            .map(|s| Series {
                target: s.target.clone(),
                fixture: s.fixture,
                screen: s.screen,
                subject: s.subject.clone(),
            })
            .collect()
    }

    fn find(&self, target: &Target, scenario: &Scenario, subject: &Subject) -> Option<usize> {
        self.script.series.iter().position(|s| {
            s.target == target.id
                && s.fixture == scenario.fixture
                && s.screen == scenario.screen
                && s.subject == *subject
        })
    }

    fn attempt(&mut self, index: usize) -> Result<Sample, AdapterError> {
        let attempt = self.attempts[index];
        self.attempts[index] += 1;
        let series = &self.script.series[index];
        if series.fail.contains(&attempt) {
            return Err(AdapterError::new(format!(
                "injected failure at attempt {attempt}"
            )));
        }
        if series.ms.is_empty() {
            return Err(AdapterError::new("the scripted series has no durations"));
        }
        let ms = series.ms[attempt as usize % series.ms.len()];
        Ok(Sample {
            elapsed: Duration::from_micros((ms * 1000.0).round() as u64),
            clock: series.clock,
            nodes: series.nodes,
            detail: None,
        })
    }
}

impl Adapter for FakeAdapter {
    fn name(&self) -> &str {
        &self.script.tool
    }

    fn role(&self) -> Role {
        self.script.role
    }

    fn detect(&mut self) -> Detection {
        Detection::Available {
            version: self.script.version.clone(),
        }
    }

    fn supports(&self, target: &Target, scenario: &Scenario, subject: &Subject) -> bool {
        self.find(target, scenario, subject).is_some()
    }

    fn measure(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        primitive: Primitive,
    ) -> Result<Sample, AdapterError> {
        let subject = Subject::Primitive(primitive);
        let index = self.find(target, scenario, &subject).ok_or_else(|| {
            AdapterError::new(format!("no scripted series for {subject} on {}", target.id))
        })?;
        self.attempt(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::TargetClass;

    fn adapter(fail: Vec<u32>) -> FakeAdapter {
        let mut series = FakeSeries::new(
            "SIM-1",
            Some(Fixture::SwiftUi),
            Some(Screen::Big),
            Subject::Primitive(Primitive::WarmSnapshot),
            vec![1.5, 2.25],
        );
        series.fail = fail;
        series.nodes = Some(546);
        FakeAdapter::new(FakeScript {
            tool: "mobile-agent".into(),
            version: Some("0.1.0".into()),
            role: Role::UnderTest,
            targets: vec![Target::new("SIM-1", TargetClass::Simulator)],
            series: vec![series],
        })
    }

    fn big() -> Scenario {
        Scenario {
            fixture: Some(Fixture::SwiftUi),
            screen: Some(Screen::Big),
        }
    }

    #[test]
    fn durations_are_consumed_in_order_and_repeat() {
        let mut fake = adapter(vec![]);
        let target = Target::new("SIM-1", TargetClass::Simulator);
        let elapsed: Vec<_> = (0..3)
            .map(|_| {
                fake.measure(&target, &big(), Primitive::WarmSnapshot)
                    .unwrap()
                    .elapsed
            })
            .collect();
        assert_eq!(elapsed, [1_500, 2_250, 1_500].map(Duration::from_micros));
    }

    #[test]
    fn injected_failures_hit_the_numbered_attempts() {
        let mut fake = adapter(vec![1]);
        let target = Target::new("SIM-1", TargetClass::Simulator);
        let results: Vec<_> = (0..3)
            .map(|_| {
                fake.measure(&target, &big(), Primitive::WarmSnapshot)
                    .is_ok()
            })
            .collect();
        assert_eq!(results, [true, false, true]);
    }

    #[test]
    fn only_scripted_series_are_supported() {
        let fake = adapter(vec![]);
        let target = Target::new("SIM-1", TargetClass::Simulator);
        assert!(fake.supports(
            &target,
            &big(),
            &Subject::Primitive(Primitive::WarmSnapshot)
        ));
        assert!(!fake.supports(&target, &big(), &Subject::Primitive(Primitive::Screenshot)));
        assert!(!fake.supports(
            &target,
            &Scenario::default(),
            &Subject::Primitive(Primitive::WarmSnapshot)
        ));
    }

    #[test]
    fn a_script_loads_from_json() {
        let fake = FakeAdapter::from_json(
            r#"{"tool": "mobile-agent", "version": "0.1.0",
                "targets": [{"id": "SIM-1", "class": "simulator"}],
                "series": [{"target": "SIM-1", "fixture": "swiftui", "screen": "big",
                            "primitive": "warm-snapshot", "ms": [120.5], "nodes": 546}]}"#,
        )
        .unwrap();
        assert_eq!(fake.role(), Role::UnderTest);
        assert_eq!(fake.targets()[0].class, TargetClass::Simulator);
        assert_eq!(
            fake.series()[0].subject,
            Subject::Primitive(Primitive::WarmSnapshot)
        );
    }
}
