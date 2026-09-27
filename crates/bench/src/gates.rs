//! Latency gates: per target class, a statistic of one primitive's latency that
//! must stay under a limit. The defaults are the benchmark-harness spec's; a
//! gate file replaces them. Gates judge only the tool under test.

use serde::{Deserialize, Serialize};

use crate::fixture::{Fixture, Screen};
use crate::primitive::Primitive;
use crate::report::{Measurement, Report};
use crate::stats::Summary;
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
        todo!("convert {} ms", self.limit_ms)
    }

    /// Whether the gate judges `measurement`, ignoring which tool made it.
    pub fn applies_to(&self, measurement: &Measurement) -> bool {
        todo!("match {} against {}", self.name(), measurement.subject)
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
        todo!("the spec's default gates")
    }

    /// Parses a gate file.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("not a gate file: {e}"))
    }

    /// Pretty-printed JSON, the gate file format.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a gate set always serializes")
    }

    /// Judges the tool under test's measurements in `report`. Gates for target
    /// classes the run has no target of are left out.
    pub fn evaluate(&self, report: &Report) -> GateReport {
        todo!(
            "evaluate {} gates against {} measurements",
            self.gates.len(),
            report.measurements.len()
        )
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
    /// gate judges. Reported, but not a failure.
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
        todo!("exit code for {} rows", self.rows.len())
    }

    /// The gate table in markdown, followed by a one-line verdict.
    pub fn table(&self) -> String {
        todo!("table of {} rows", self.rows.len())
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
}
