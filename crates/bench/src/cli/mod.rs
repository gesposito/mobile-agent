//! The adapter for `mobile-agent`, the tool under test, driven through its
//! command line in `--json` mode.

pub mod listing;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::adapter::{Adapter, Detection, Role, Scenario};
use crate::baseline::search_path;
use crate::harness::Subject;
use crate::target::Target;

/// The tool's name in reports.
pub const TOOL: &str = "mobile-agent";

/// The session every command runs in.
pub const SESSION: &str = "mobile-agent-bench";

/// Where the CLI adapter finds `mobile-agent`, what environment its commands
/// get, and where it writes files.
#[derive(Clone, Debug)]
pub struct CliConfig {
    /// The executable: a path, or a name looked up in `search_path`.
    pub program: PathBuf,
    /// Directories searched for a program given by name: `PATH` by default.
    pub search_path: Vec<PathBuf>,
    /// Changes to the harness's environment for every command: `Some` sets a
    /// variable, `None` removes it.
    pub env: Vec<(OsString, Option<OsString>)>,
    /// The working directory of every command; `None` keeps the harness's.
    pub cwd: Option<PathBuf>,
    /// Where screenshots are written.
    pub work_dir: PathBuf,
    /// Deadline for one command's process.
    pub command_timeout: Duration,
}

impl CliConfig {
    /// `mobile-agent` from `PATH` with the harness's environment, writing
    /// under `work_dir`, 120 s per command.
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        CliConfig {
            program: PathBuf::from(TOOL),
            search_path: search_path(),
            env: Vec::new(),
            cwd: None,
            work_dir: work_dir.into(),
            command_timeout: Duration::from_secs(120),
        }
    }
}

/// `mobile-agent` driven through its CLI.
#[derive(Clone, Debug)]
pub struct CliAdapter {
    config: CliConfig,
}

impl CliAdapter {
    pub fn new(config: CliConfig) -> Self {
        CliAdapter { config }
    }
}

impl Adapter for CliAdapter {
    fn name(&self) -> &str {
        TOOL
    }

    fn role(&self) -> Role {
        Role::UnderTest
    }

    fn detect(&mut self) -> Detection {
        let _ = &self.config;
        Detection::Absent {
            note: "the CLI adapter is not written yet".to_owned(),
        }
    }

    fn supports(&self, _target: &Target, _scenario: &Scenario, _subject: &Subject) -> bool {
        false
    }
}
