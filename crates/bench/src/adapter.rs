//! The interface between the harness and a tool it drives: this project's
//! CLI, a baseline tool, or the scripted fake.
//!
//! The harness never times a call itself. An adapter measures the span a
//! primitive defines, leaving its own setup and teardown outside it, and names
//! the clock it read in the [`Sample`].

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::end_state::EndState;
use crate::fixture::{Fixture, Screen};
use crate::flow::Flow;
use crate::harness::Subject;
use crate::primitive::Primitive;
use crate::target::Target;

/// Whether gates apply to a tool's measurements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// The tool the gates judge: `mobile-agent`, or the fake standing in for it.
    UnderTest,
    /// A tool measured for comparison only.
    Baseline,
}

/// Whether a tool can run on this host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detection {
    /// Found; the version is `None` when the tool does not report one.
    Available { version: Option<String> },
    /// Not found; the harness skips the tool and reports `note`.
    Absent { note: String },
}

/// Which clock a sample's latency was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Clock {
    /// Wall clock read by the adapter around the tool's process or request.
    Wall,
    /// Reported by the tool itself, such as a dispatch time in its JSON output.
    Tool,
}

/// One successful attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The latency of the measured span.
    pub elapsed: Duration,
    pub clock: Clock,
    /// Accessibility nodes the tool returned, for snapshots.
    pub nodes: Option<u32>,
    /// Anything else worth keeping with the sample, such as a text entry method.
    pub detail: Option<String>,
}

/// A failed attempt: the tool ran and did not do what the primitive or flow
/// asked, or could not be run. Or, when `not_measured` is set, no failure:
/// the tool cannot measure the span at all, and `message` says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterError {
    pub message: String,
    pub not_measured: bool,
}

impl AdapterError {
    pub fn new(message: impl Into<String>) -> Self {
        AdapterError {
            message: message.into(),
            not_measured: false,
        }
    }

    /// The tool cannot measure the span, such as one its replies do not carry
    /// yet: the harness reports the series as not measured, with `reason`,
    /// instead of failing it.
    pub fn not_measured(reason: impl Into<String>) -> Self {
        AdapterError {
            message: reason.into(),
            not_measured: true,
        }
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AdapterError {}

/// Where a series runs: which fixture app and which of its screens. Primitives
/// that do not involve an app, such as CLI process start, leave both empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scenario {
    pub fixture: Option<Fixture>,
    pub screen: Option<Screen>,
}

/// A tool the harness drives. One adapter instance serves a whole run.
pub trait Adapter {
    /// The tool's name in reports, such as `mobile-agent` or `maestro`.
    fn name(&self) -> &str;

    fn role(&self) -> Role;

    /// Looks for the tool on this host. Called once per run, before any series.
    fn detect(&mut self) -> Detection;

    /// Whether the tool can measure `subject` on `target` in `scenario`. The
    /// harness skips unsupported combinations instead of recording failures.
    fn supports(&self, target: &Target, scenario: &Scenario, subject: &Subject) -> bool;

    /// Once per series, before its first iteration: launch the fixture on the
    /// scenario's screen, warm or reset whatever the subject needs. A failure
    /// fails every iteration of the series.
    fn prepare(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        subject: &Subject,
    ) -> Result<(), AdapterError> {
        let _ = (target, scenario, subject);
        Ok(())
    }

    /// One attempt at `primitive`. Setup the span excludes, such as stopping the
    /// daemon before a cold attach, happens inside this call but outside the
    /// measured span. Tools that only run flows keep the default.
    fn measure(
        &mut self,
        target: &Target,
        scenario: &Scenario,
        primitive: Primitive,
    ) -> Result<Sample, AdapterError> {
        let _ = (target, scenario);
        Err(AdapterError::new(format!(
            "{} does not measure {primitive}",
            self.name()
        )))
    }

    /// One run of `flow`, all of its steps, on `target`. The sample spans the
    /// whole flow.
    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        let _ = target;
        Err(AdapterError::new(format!(
            "{} does not run flows such as {}",
            self.name(),
            flow.name
        )))
    }

    /// The screen `target` shows after a run of `flow`, read through the tool
    /// and summarized in the terms of the flow's golden.
    fn end_state(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        let _ = target;
        Err(AdapterError::new(format!(
            "{} does not read the end state of flows such as {}",
            self.name(),
            flow.name
        )))
    }

    /// One untimed run of `flow` and the end state it left. The harness calls
    /// it before timing a flow that has a golden.
    fn check_flow(&mut self, target: &Target, flow: &Flow) -> Result<EndState, AdapterError> {
        self.run_flow(target, flow)?;
        self.end_state(target, flow)
    }

    /// Lets go of `target` so that another tool can drive it: stops what the
    /// tool keeps on the target between its commands, such as a daemon whose
    /// agent holds Android's one UI automation role or a helper that does.
    /// The harness calls it outside every series, when another tool is about
    /// to drive the target and once the run is over. Returns what the release
    /// did, for the report; `None` when the tool held nothing.
    fn release(&mut self, target: &Target) -> Result<Option<String>, AdapterError> {
        let _ = target;
        Ok(None)
    }
}
