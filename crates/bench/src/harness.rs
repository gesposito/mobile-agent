//! Runs series of iterations through adapters and assembles the report.

use std::collections::HashMap;
use std::fmt;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::adapter::{Adapter, AdapterError, Detection, Sample, Scenario};
use crate::end_state::EndState;
use crate::fixture::{Fixture, Screen};
use crate::flow::Flow;
use crate::primitive::Primitive;
use crate::provenance::{
    HarnessInfo, Probe, Provenance, SystemProbe, ToolReport, ToolStatus, capture_commit,
    capture_host, capture_xcode, complete_target, utc_timestamp,
};
use crate::report::{
    Measurement, NodeRange, Outcome, Reliability, Report, SCHEMA_VERSION, SampleRecord,
};
use crate::stats::{micros, summarize};
use crate::target::Target;

/// What a series measures. Serialized as `{"primitive": "<id>"}` or
/// `{"flow": "<name>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Subject {
    Primitive(Primitive),
    /// The [`Flow::name`] of one of the run's flows.
    Flow(String),
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subject::Primitive(primitive) => write!(f, "{primitive}"),
            Subject::Flow(name) => write!(f, "flow {name}"),
        }
    }
}

/// One subject measured on one target in one scenario, by every tool that
/// supports it. A flow's series names the flow's fixture and no screen.
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
    /// Further attempts after a failed one, within one iteration. An iteration
    /// that succeeds on a retry is a flake; one that never succeeds, a failure.
    pub retries: u32,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            iterations: 50,
            warmup: 3,
            retries: 1,
        }
    }
}

/// A configured run: targets, series, the tools that measure them and the probe
/// that records provenance.
pub struct Harness {
    config: RunConfig,
    targets: Vec<Target>,
    series: Vec<Series>,
    flows: Vec<Flow>,
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
            flows: Vec::new(),
            tools: Vec::new(),
            probe: Box::new(SystemProbe),
            os: std::env::consts::OS.to_owned(),
        }
    }

    /// The flows the run's flow series name.
    pub fn flows(mut self, flows: Vec<Flow>) -> Self {
        self.flows = flows;
        self
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
    /// supports it, and returns the report. Fails, before anything runs, on two
    /// flows with one name on one fixture, on a series naming a target the run
    /// lacks or a fixture its target's platform lacks, and on a flow series
    /// naming a flow the run lacks on the series' fixture, or a screen.
    pub fn run(&mut self) -> Result<Report, String> {
        for (i, flow) in self.flows.iter().enumerate() {
            if find_flow(&self.flows[..i], &flow.name, Some(flow.fixture)).is_some() {
                return Err(format!(
                    "the run has two flows named {} on the {} fixture",
                    flow.name, flow.fixture
                ));
            }
        }
        self.series.iter().try_for_each(|s| self.check(s))?;
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

        // Every flow with a golden runs once through each tool that times it,
        // and its end state is read, before anything is timed.
        let mut checks: HashMap<(usize, usize), Check> = HashMap::new();
        for (s, series) in self.series.iter().enumerate() {
            let Subject::Flow(name) = &series.subject else {
                continue;
            };
            let (Some(target), Some(flow)) = (
                targets.iter().find(|t| t.id == series.target),
                find_flow(&self.flows, name, series.fixture),
            ) else {
                continue;
            };
            if flow.golden.is_empty() {
                continue;
            }
            for (t, (tool, found)) in self.tools.iter_mut().zip(&tools).enumerate() {
                if found.status == ToolStatus::Available
                    && tool.supports(target, &series.scenario(), &series.subject)
                {
                    let result = check(tool.as_mut(), target, flow, self.config.retries);
                    checks.insert((s, t), result);
                }
            }
        }

        let mut measurements = Vec::new();
        for (s, series) in self.series.iter().enumerate() {
            let Some(target) = targets.iter().find(|t| t.id == series.target) else {
                continue;
            };
            let scenario = series.scenario();
            for (t, (tool, found)) in self.tools.iter_mut().zip(&tools).enumerate() {
                if found.status == ToolStatus::Available
                    && tool.supports(target, &scenario, &series.subject)
                {
                    measurements.push(measure(
                        tool.as_mut(),
                        target,
                        series,
                        &self.flows,
                        self.config,
                        checks.remove(&(s, t)),
                    ));
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
                    retries: self.config.retries,
                },
                targets,
                tools,
            },
            measurements,
            gates: None,
        })
    }

    /// Refuses a series the run cannot carry out; see [`Harness::run`].
    fn check(&self, series: &Series) -> Result<(), String> {
        let Some(target) = self.targets.iter().find(|t| t.id == series.target) else {
            return Err(format!(
                "a {} series names target {}, which the run does not have",
                series.subject, series.target
            ));
        };
        if let Some(fixture) = series.fixture
            && fixture.app_id(target.class.platform()).is_none()
        {
            return Err(format!(
                "a {} series runs the {fixture} fixture on {}, a {} where it does not exist",
                series.subject, target.id, target.class
            ));
        }
        if let Subject::Flow(name) = &series.subject {
            let fixtures: Vec<String> = self
                .flows
                .iter()
                .filter(|f| f.name == *name)
                .map(|f| f.fixture.to_string())
                .collect();
            if fixtures.is_empty() {
                return Err(format!(
                    "a series names flow {name}, which the run does not have"
                ));
            }
            if find_flow(&self.flows, name, series.fixture).is_none() || series.screen.is_some() {
                return Err(format!(
                    "the series of flow {name} must name a fixture the run has the flow for, {}, \
                     and no screen",
                    fixtures.join(" or ")
                ));
            }
        }
        Ok(())
    }
}

/// The flow `name` for `fixture` among `flows`.
fn find_flow<'a>(flows: &'a [Flow], name: &str, fixture: Option<Fixture>) -> Option<&'a Flow> {
    flows
        .iter()
        .find(|f| f.name == name && Some(f.fixture) == fixture)
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

/// A flow's golden check through one tool: the end state it read last, and
/// why it did not match the golden, when it did not.
struct Check {
    end_state: Option<EndState>,
    error: Option<String>,
}

/// Runs `flow` untimed through `tool` and compares the end state with the
/// flow's golden, up to `1 + retries` times until one matches.
fn check(tool: &mut dyn Adapter, target: &Target, flow: &Flow, retries: u32) -> Check {
    let mut errors = Vec::new();
    let mut end_state = None;
    for _ in 0..=retries {
        match tool.check_flow(target, flow) {
            Ok(state) if state == flow.golden => {
                return Check {
                    end_state: Some(state),
                    error: None,
                };
            }
            Ok(state) => {
                errors.push(format!(
                    "the end state differs from the golden: {}",
                    flow.golden.differences(&state).join("; ")
                ));
                end_state = Some(state);
            }
            Err(e) => errors.push(format!("the check run failed: {e}")),
        }
    }
    errors.dedup();
    Check {
        end_state,
        error: Some(errors.join(" / ")),
    }
}

/// Runs one series through one tool: the preparation, then the warm-up and
/// measured iterations in order. A flow whose golden check failed is not run
/// again, and every measured iteration counts as failed. A tool that reports
/// it cannot measure the subject, while preparing or in any attempt, ends the
/// series there: it is not measured, with no samples and no failures.
fn measure(
    tool: &mut dyn Adapter,
    target: &Target,
    series: &Series,
    flows: &[Flow],
    config: RunConfig,
    check: Option<Check>,
) -> Measurement {
    let scenario = series.scenario();
    let mut samples = Vec::new();
    let mut clock = None;
    let mut not_measured = None;
    let (end_state, check_error) = check.map_or((None, None), |c| (c.end_state, c.error));
    let prepared = match check_error {
        Some(error) => Err(AdapterError::new(error)),
        None => tool
            .prepare(target, &scenario, &series.subject)
            .map_err(|e| {
                if e.not_measured {
                    e
                } else {
                    AdapterError::new(format!("preparation failed: {e}"))
                }
            }),
    };
    let error = match prepared {
        Err(e) if e.not_measured => {
            not_measured = Some(e.message);
            None
        }
        Err(e) => Some(e.message),
        Ok(()) => {
            'iterations: for iteration in 0..config.warmup + config.iterations {
                let warmup = iteration < config.warmup;
                let mut errors = Vec::new();
                let mut success = None;
                for _ in 0..=config.retries {
                    match attempt(tool, target, &scenario, &series.subject, flows) {
                        Ok(sample) => {
                            success = Some(sample);
                            break;
                        }
                        Err(e) if e.not_measured => {
                            not_measured = Some(e.message);
                            break 'iterations;
                        }
                        Err(e) => errors.push(e.message),
                    }
                }
                if let (Some(sample), false) = (&success, warmup) {
                    clock.get_or_insert(sample.clock);
                }
                samples.push(record(iteration, warmup, success, errors));
            }
            None
        }
    };
    if not_measured.is_some() {
        samples.clear();
        clock = None;
    }
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
        reliability: match not_measured {
            Some(_) => Reliability::default(),
            None => Reliability::of(&samples, config.iterations),
        },
        nodes,
        error,
        not_measured,
        end_state,
        samples,
    }
}

fn attempt(
    tool: &mut dyn Adapter,
    target: &Target,
    scenario: &Scenario,
    subject: &Subject,
    flows: &[Flow],
) -> Result<Sample, AdapterError> {
    match subject {
        Subject::Primitive(primitive) => tool.measure(target, scenario, *primitive),
        Subject::Flow(name) => match find_flow(flows, name, scenario.fixture) {
            Some(flow) => tool.run_flow(target, flow),
            None => Err(AdapterError::new(format!("the run has no flow {name}"))),
        },
    }
}

/// One iteration: its successful attempt, if any, after the failed ones whose
/// messages are in `errors`.
fn record(
    iteration: u32,
    warmup: bool,
    success: Option<Sample>,
    errors: Vec<String>,
) -> SampleRecord {
    let failed = u32::try_from(errors.len()).unwrap_or(u32::MAX);
    match success {
        Some(sample) => SampleRecord {
            iteration,
            warmup,
            outcome: if errors.is_empty() {
                Outcome::Pass
            } else {
                Outcome::Flake
            },
            attempts: failed + 1,
            us: Some(micros(sample.elapsed)),
            nodes: sample.nodes,
            detail: sample.detail,
            errors,
        },
        None => SampleRecord {
            iteration,
            warmup,
            outcome: Outcome::Fail,
            attempts: failed,
            us: None,
            nodes: None,
            detail: None,
            errors,
        },
    }
}
