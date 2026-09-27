//! `doctor`: checks of the host's toolchains and of the daemon.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// What a command printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// What the checks ask of the host.
pub trait Probe {
    /// `macos` or `linux`.
    fn os(&self) -> &str;
    fn env(&self, key: &str) -> Option<String>;
    /// Runs a program; `None` when it cannot be started.
    fn run(&self, program: &str, args: &[&str]) -> Option<Output>;
    fn is_file(&self, path: &Path) -> bool;
    /// The regular files in `dir` with their text.
    fn files(&self, dir: &Path) -> Vec<(PathBuf, String)>;
    /// The protocol version of the adb server on `port`.
    fn adb_server(&self, port: u16) -> Result<u32, String>;
    /// The daemon's `doctor` data, starting the daemon if needed.
    fn daemon(&self) -> Result<Value, String>;
}

/// A check's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

/// One check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub message: String,
    pub hint: Option<String>,
}

/// One row of the Xcode compatibility table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub subject: String,
    pub feature: String,
    pub status: String,
    pub note: Option<String>,
}

/// The compatibility table the daemon's drivers supply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compatibility {
    pub rows: Vec<Row>,
    pub note: Option<String>,
}

/// Every check and the compatibility table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
    pub compatibility: Compatibility,
}

impl Report {
    /// Whether a check failed.
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Fail)
    }

    /// The report as doctor prints it.
    pub fn to_text(&self) -> String {
        String::new()
    }
}

/// Runs every check.
pub fn run_checks(_probe: &dyn Probe) -> Report {
    Report::default()
}
