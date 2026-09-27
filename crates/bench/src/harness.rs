//! Runs series of iterations through adapters and assembles the report.

use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::adapter::{Adapter, AdapterError, Detection, Sample, Scenario};
use crate::fixture::{Fixture, Screen};
use crate::primitive::Primitive;
use crate::provenance::{
    HarnessInfo, Probe, Provenance, SystemProbe, ToolReport, ToolStatus, capture_commit,
    capture_host, capture_xcode, complete_target, utc_timestamp,
};
use crate::report::{Measurement, NodeRange, Outcome, Report, SCHEMA_VERSION, SampleRecord};
use crate::stats::{micros, summarize};
use crate::target::Target;

/// What a series measures. Serialized as `{"primitive": "<id>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Subject {
    Primitive(Primitive),
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subject::Primitive(primitive) => write!(f, "{primitive}"),
        }
    }
}

/// One subject measured on one target in one scenario, by every tool that
/// supports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Series {
    /// The [`Target::id`] of a target in the run.
    pub target: String,
    pub fixture: Option<Fixture>,
    pub screen: Option<Screen>,
    pub subject: Subject,
}

impl Series {
    pub fn scenario(&self) -> Scenario {
        Scenario {
            fixture: self.fixture,
            screen: self.screen,
        }
    }
}

/// Iteration counts shared by every series of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunConfig {
    /// Measured iterations per series and tool.
    pub iterations: u32,
    /// Iterations run first and excluded from every statistic.
    pub warmup: u32,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            iterations: 50,
            warmup: 3,
        }
    }
}

/// A configured run: targets, series, the tools that measure them and the probe
/// that records provenance.
pub struct Harness {
    config: RunConfig,
    targets: Vec<Target>,
    series: Vec<Series>,
    tools: Vec<Box<dyn Adapter>>,
    probe: Box<dyn Probe>,
    os: String,
}

impl Harness {
    /// A run over `series` on `targets`, recording provenance from this host.
    pub fn new(config: RunConfig, targets: Vec<Target>, series: Vec<Series>) -> Self {
        Harness {
            config,
            targets,
            series,
            tools: Vec::new(),
            probe: Box::new(SystemProbe),
            os: std::env::consts::OS.to_owned(),
        }
    }

    /// Adds a tool; tools are detected and run in the order they were added.
    pub fn tool(mut self, tool: Box<dyn Adapter>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Records provenance through `probe` as if on a host running `os`, a value
    /// of `std::env::consts::OS`.
    pub fn probe(mut self, probe: Box<dyn Probe>, os: &str) -> Self {
        self.probe = probe;
        self.os = os.to_owned();
        self
    }

    /// Detects every tool, runs every series through each available tool that
    /// supports it, and returns the report. Fails only on a series that names a
    /// target the run does not have.
    pub fn run(&mut self) -> Result<Report, String> {
        if let Some(series) = self
            .series
            .iter()
            .find(|s| !self.targets.iter().any(|t| t.id == s.target))
        {
            return Err(format!(
                "a {} series names target {}, which the run does not have",
                series.subject, series.target
            ));
        }
        let started = SystemTime::now();
        let probe = self.probe.as_ref();
        let host = capture_host(probe, &self.os);
        let xcode = capture_xcode(probe);
        let commit = capture_commit(probe);
        let mut targets = self.targets.clone();
        for target in &mut targets {
            complete_target(probe, target);
        }
        let tools: Vec<ToolReport> = self
            .tools
            .iter_mut()
            .map(|tool| detect(tool.as_mut()))
            .collect();

        let mut measurements = Vec::new();
        for series in &self.series {
            let Some(target) = targets.iter().find(|t| t.id == series.target) else {
                continue;
            };
            let scenario = series.scenario();
            for (tool, found) in self.tools.iter_mut().zip(&tools) {
                if found.status == ToolStatus::Available
                    && tool.supports(target, &scenario, &series.subject)
                {
                    measurements.push(measure(tool.as_mut(), target, series, self.config));
                }
            }
        }

        Ok(Report {
            schema_version: SCHEMA_VERSION,
            provenance: Provenance {
                started_at: utc_timestamp(started),
                finished_at: utc_timestamp(SystemTime::now()),
                host,
                xcode,
                commit,
                harness: HarnessInfo {
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    iterations: self.config.iterations,
                    warmup: self.config.warmup,
                },
                targets,
                tools,
            },
            measurements,
        })
    }
}

fn detect(tool: &mut dyn Adapter) -> ToolReport {
    let (status, version, note) = match tool.detect() {
        Detection::Available { version } => (ToolStatus::Available, version, None),
        Detection::Absent { note } => (ToolStatus::Skipped, None, Some(note)),
    };
    ToolReport {
        name: tool.name().to_owned(),
        role: tool.role(),
        status,
        version,
        note,
    }
}

/// Runs one series through one tool: the preparation, then the warm-up and
/// measured iterations in order.
fn measure(
    tool: &mut dyn Adapter,
    target: &Target,
    series: &Series,
    config: RunConfig,
) -> Measurement {
    let scenario = series.scenario();
    let mut samples = Vec::new();
    let mut clock = None;
    let error = match tool.prepare(target, &scenario, &series.subject) {
        Err(e) => Some(format!("preparation failed: {e}")),
        Ok(()) => {
            for iteration in 0..config.warmup + config.iterations {
                let warmup = iteration < config.warmup;
                let result = attempt(tool, target, &scenario, &series.subject);
                if let (Ok(sample), false) = (&result, warmup) {
                    clock.get_or_insert(sample.clock);
                }
                samples.push(record(iteration, warmup, result));
            }
            None
        }
    };
    let measured: Vec<&SampleRecord> = samples.iter().filter(|s| !s.warmup).collect();
    let latencies: Vec<u64> = measured.iter().filter_map(|s| s.us).collect();
    let nodes = measured
        .iter()
        .filter_map(|s| s.nodes)
        .fold(None, |range, n| {
            Some(match range {
                None => NodeRange { min: n, max: n },
                Some(NodeRange { min, max }) => NodeRange {
                    min: min.min(n),
                    max: max.max(n),
                },
            })
        });
    Measurement {
        tool: tool.name().to_owned(),
        target: target.id.clone(),
        target_class: target.class,
        fixture: series.fixture,
        screen: series.screen,
        subject: series.subject.clone(),
        clock,
        iterations: config.iterations,
        warmup: config.warmup,
        stats: summarize(&latencies),
        nodes,
        error,
        samples,
    }
}

fn attempt(
    tool: &mut dyn Adapter,
    target: &Target,
    scenario: &Scenario,
    subject: &Subject,
) -> Result<Sample, AdapterError> {
    match subject {
        Subject::Primitive(primitive) => tool.measure(target, scenario, *primitive),
    }
}

fn record(iteration: u32, warmup: bool, result: Result<Sample, AdapterError>) -> SampleRecord {
    match result {
        Ok(sample) => SampleRecord {
            iteration,
            warmup,
            outcome: Outcome::Pass,
            attempts: 1,
            us: Some(micros(sample.elapsed)),
            nodes: sample.nodes,
            detail: sample.detail,
            errors: Vec::new(),
        },
        Err(e) => SampleRecord {
            iteration,
            warmup,
            outcome: Outcome::Fail,
            attempts: 1,
            us: None,
            nodes: None,
            detail: None,
            errors: vec![e.message],
        },
    }
}
