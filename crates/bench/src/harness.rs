//! Runs series of iterations through adapters and assembles the report.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::adapter::{Adapter, Scenario};
use crate::fixture::{Fixture, Screen};
use crate::primitive::Primitive;
use crate::provenance::{Probe, SystemProbe};
use crate::report::Report;
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
        todo!(
            "run {} series on {} targets with {} tools, {:?}, probing a {} host through {:p}",
            self.series.len(),
            self.targets.len(),
            self.tools.len(),
            self.config,
            self.os,
            self.probe
        )
    }
}
