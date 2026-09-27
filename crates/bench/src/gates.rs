//! Latency gates: per target class, a statistic of one primitive's latency that
//! must stay under a limit. The defaults are the benchmark-harness spec's; a
//! gate file replaces them. Gates judge only the tool under test.

use serde::{Deserialize, Serialize};

use crate::adapter::Role;
use crate::fixture::{Fixture, Screen};
use crate::harness::Subject;
use crate::markdown::{Table, or_dash};
use crate::primitive::Primitive;
use crate::report::{Measurement, Report};
use crate::stats::{Summary, ms};
use crate::target::TargetClass;

/// Which statistic of a series a gate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Statistic {
    P50,
    P95,
    Max,
}

impl Statistic {
    pub fn id(self) -> &'static str {
        match self {
            Statistic::P50 => "p50",
            Statistic::P95 => "p95",
            Statistic::Max => "max",
        }
    }

    /// The statistic's value in `summary`, in microseconds.
    pub fn of(self, summary: &Summary) -> u64 {
        match self {
            Statistic::P50 => summary.p50_us,
            Statistic::P95 => summary.p95_us,
            Statistic::Max => summary.max_us,
        }
    }
}

fn p95() -> Statistic {
    Statistic::P95
}

/// One gate. It applies to every measurement of the tool under test on a
/// target of `class` whose primitive matches and whose fixture and screen
/// match where the gate names them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gate {
    pub class: TargetClass,
    pub primitive: Primitive,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<Fixture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<Screen>,
    #[serde(default = "p95")]
    pub statistic: Statistic,
    /// The statistic must be strictly below this many milliseconds.
    pub limit_ms: f64,
}

impl Gate {
    /// A p95 gate on every fixture and screen.
    pub fn new(class: TargetClass, primitive: Primitive, limit_ms: f64) -> Self {
        Gate {
            class,
            primitive,
            fixture: None,
            screen: None,
            statistic: Statistic::P95,
            limit_ms,
        }
    }

    /// The same gate restricted to one fixture's screen.
    pub fn on(mut self, fixture: Option<Fixture>, screen: Screen) -> Self {
        self.fixture = fixture;
        self.screen = Some(screen);
        self
    }

    /// Class, primitive, and the fixture and screen when the gate names them,
    /// such as `simulator warm-snapshot swiftui big`.
    pub fn name(&self) -> String {
        let mut name = format!("{} {}", self.class, self.primitive);
        if let Some(fixture) = self.fixture {
            name.push_str(&format!(" {fixture}"));
        }
        if let Some(screen) = self.screen {
            name.push_str(&format!(" {screen}"));
        }
        name
    }

    /// The limit in whole microseconds.
    pub fn limit_us(&self) -> u64 {
        (self.limit_ms * 1000.0).round() as u64
    }

    /// Whether the gate judges `measurement`, ignoring which tool made it.
    pub fn applies_to(&self, measurement: &Measurement) -> bool {
        measurement.target_class == self.class
            && measurement.subject == Subject::Primitive(self.primitive)
            && self.fixture.is_none_or(|f| measurement.fixture == Some(f))
            && self.screen.is_none_or(|s| measurement.screen == Some(s))
    }
}

/// The gates a run is judged by. As a file: `{"gates": [...]}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateSet {
    pub gates: Vec<Gate>,
}

impl GateSet {
    /// The benchmark-harness spec's defaults. The spec states p95 for the
    /// snapshot, dispatch and tap-to-frame gates and no statistic for CLI start
    /// and cold attach, which get p95 too.
    pub fn defaults() -> Self {
        use Primitive::{CliStart, ColdAttach, TapDispatch, TapToFrameChange, WarmSnapshot};
        use TargetClass::{AndroidDevice, AndroidEmulator, Simulator};
        GateSet {
            gates: vec![
                Gate::new(Simulator, CliStart, 20.0),
                Gate::new(Simulator, ColdAttach, 2000.0),
                Gate::new(Simulator, WarmSnapshot, 150.0).on(Some(Fixture::SwiftUi), Screen::Big),
                // React Native's own accessibility snapshot of this screen costs
                // about 236 ms (spike S4).
                Gate::new(Simulator, WarmSnapshot, 250.0).on(Some(Fixture::Expo), Screen::Big),
                Gate::new(Simulator, TapDispatch, 20.0),
                Gate::new(AndroidEmulator, CliStart, 20.0),
                Gate::new(AndroidEmulator, ColdAttach, 1500.0),
                Gate::new(AndroidEmulator, WarmSnapshot, 100.0).on(None, Screen::Big),
                Gate::new(AndroidEmulator, TapToFrameChange, 50.0),
                Gate::new(AndroidDevice, CliStart, 20.0),
                Gate::new(AndroidDevice, ColdAttach, 1500.0),
            ],
        }
    }

    /// Parses a gate file; every limit must be a positive number of
    /// milliseconds.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let set: GateSet =
            serde_json::from_str(text).map_err(|e| format!("not a gate file: {e}"))?;
        if let Some(gate) = set
            .gates
            .iter()
            .find(|g| !(g.limit_ms.is_finite() && g.limit_ms > 0.0))
        {
            return Err(format!(
                "gate {} has limit {} ms; limits are positive milliseconds",
                gate.name(),
                gate.limit_ms
            ));
        }
        Ok(set)
    }

    /// Pretty-printed JSON, the gate file format.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a gate set always serializes")
    }

    /// Judges the tool under test's measurements in `report`. Gates for target
    /// classes the run has no target of are left out.
    pub fn evaluate(&self, report: &Report) -> GateReport {
        let under_test: Vec<&str> = report
            .provenance
            .tools
            .iter()
            .filter(|t| t.role == Role::UnderTest)
            .map(|t| t.name.as_str())
            .collect();
        let mut rows = Vec::new();
        for gate in &self.gates {
            if !report
                .provenance
                .targets
                .iter()
                .any(|t| t.class == gate.class)
            {
                continue;
            }
            let row = |tool, target, measured_us, status| GateRow {
                gate: gate.name(),
                statistic: gate.statistic,
                limit_us: gate.limit_us(),
                tool,
                target,
                measured_us,
                status,
            };
            let judged: Vec<&Measurement> = report
                .measurements
                .iter()
                .filter(|m| {
                    under_test.contains(&m.tool.as_str())
                        && gate.applies_to(m)
                        && m.not_measured.is_none()
                })
                .collect();
            if judged.is_empty() {
                rows.push(row(None, None, None, GateStatus::NotMeasured));
            }
            for m in judged {
                let measured = m.stats.as_ref().map(|s| gate.statistic.of(s));
                let status = match measured {
                    Some(us) if us < gate.limit_us() => GateStatus::Pass,
                    _ => GateStatus::Fail,
                };
                rows.push(row(
                    Some(m.tool.clone()),
                    Some(m.target.clone()),
                    measured,
                    status,
                ));
            }
        }
        GateReport {
            passed: rows.iter().all(|r| r.status != GateStatus::Fail),
            rows,
        }
    }
}

/// The verdict of one gate on one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateStatus {
    Pass,
    /// Over the limit, or no iteration succeeded.
    Fail,
    /// The run has a target of the gate's class but did not measure what the
    /// gate judges, or the tool could not measure it. Reported, but not a
    /// failure.
    NotMeasured,
}

/// One row of the gate table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRow {
    /// [`Gate::name`].
    pub gate: String,
    pub statistic: Statistic,
    pub limit_us: u64,
    pub tool: Option<String>,
    pub target: Option<String>,
    /// `None` when not measured or when no iteration succeeded.
    pub measured_us: Option<u64>,
    pub status: GateStatus,
}

/// Every gate's verdict for one run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateReport {
    /// No row failed.
    pub passed: bool,
    pub rows: Vec<GateRow>,
}

impl GateReport {
    /// 0 when every gate passed, 1 when any failed.
    pub fn exit_code(&self) -> u8 {
        u8::from(!self.passed)
    }

    /// The gate table in markdown, followed by a one-line verdict.
    pub fn table(&self) -> String {
        if self.rows.is_empty() {
            return "No gate applies to the run's targets.\n".to_owned();
        }
        let mut table = Table::new(&[
            "Gate",
            "Tool",
            "Target",
            "Statistic",
            "Limit ms",
            "Measured ms",
            "Result",
        ]);
        for row in &self.rows {
            table.row(vec![
                row.gate.clone(),
                or_dash(row.tool.as_deref()),
                or_dash(row.target.as_deref()),
                row.statistic.id().to_owned(),
                ms(row.limit_us),
                match (row.measured_us, row.status) {
                    (Some(us), _) => ms(us),
                    (None, GateStatus::NotMeasured) => "—".to_owned(),
                    (None, _) => "no samples".to_owned(),
                },
                match row.status {
                    GateStatus::Pass => "pass",
                    GateStatus::Fail => "FAIL",
                    GateStatus::NotMeasured => "not measured",
                }
                .to_owned(),
            ]);
        }
        let mut out = String::new();
        table.render(&mut out);
        let count = |status| self.rows.iter().filter(|r| r.status == status).count();
        let (failed, unmeasured) = (count(GateStatus::Fail), count(GateStatus::NotMeasured));
        let checked = self.rows.len() - unmeasured;
        if failed > 0 {
            out.push_str(&format!("{failed} of {checked} gate checks failed."));
        } else {
            out.push_str(&format!("All {checked} gate checks passed."));
        }
        if unmeasured > 0 {
            out.push_str(&format!(" {unmeasured} not measured."));
        }
        out.push('\n');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_carry_the_class_primitive_fixture_and_screen() {
        let gate = Gate::new(TargetClass::Simulator, Primitive::WarmSnapshot, 150.0)
            .on(Some(Fixture::SwiftUi), Screen::Big);
        assert_eq!(gate.name(), "simulator warm-snapshot swiftui big");
        assert_eq!(
            Gate::new(
                TargetClass::AndroidEmulator,
                Primitive::TapToFrameChange,
                50.0
            )
            .name(),
            "android-emulator tap-to-frame-change"
        );
    }

    #[test]
    fn limits_convert_to_whole_microseconds() {
        assert_eq!(
            Gate::new(TargetClass::Simulator, Primitive::CliStart, 20.0).limit_us(),
            20_000
        );
        assert_eq!(
            Gate::new(TargetClass::Simulator, Primitive::TapDispatch, 0.0305).limit_us(),
            31
        );
    }

    #[test]
    fn a_gate_file_defaults_to_p95_and_every_fixture() {
        let set = GateSet::from_json(
            r#"{"gates": [{"class": "android-emulator", "primitive": "tap-to-frame-change", "limit_ms": 120}]}"#,
        )
        .unwrap();
        assert_eq!(
            set.gates,
            [Gate::new(
                TargetClass::AndroidEmulator,
                Primitive::TapToFrameChange,
                120.0
            )]
        );
        assert_eq!(GateSet::from_json(&set.to_json()), Ok(set));
    }

    #[test]
    fn a_gate_file_with_a_limit_that_is_not_positive_is_refused() {
        let error = GateSet::from_json(
            r#"{"gates": [{"class": "simulator", "primitive": "cli-start", "limit_ms": 0}]}"#,
        )
        .unwrap_err();
        assert!(error.contains("simulator cli-start"), "{error}");
    }
}
