//! Baseline tools the harness compares `mobile-agent` with on the same flows:
//! Maestro, maestro-runner and agent-device. Each adapter runs a flow through
//! the tool's own CLI in the shape the tool's documentation gives, and a tool
//! that is not on `PATH` is skipped with a note. Only flows run through them;
//! none of the three exposes the spans the primitives measure.

pub mod agent_device;
pub mod maestro;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::adapter::{Adapter, AdapterError, Clock, Detection, Role, Sample, Scenario};
use crate::flow::Flow;
use crate::harness::Subject;
use crate::process::{self, Finished};
use crate::target::Target;

/// The agent-device session every flow runs in.
pub const SESSION: &str = "mobile-agent-bench";

/// A baseline tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineTool {
    Maestro,
    MaestroRunner,
    AgentDevice,
}

impl BaselineTool {
    /// Every baseline, in report order.
    pub const ALL: [BaselineTool; 3] = [
        BaselineTool::Maestro,
        BaselineTool::MaestroRunner,
        BaselineTool::AgentDevice,
    ];

    /// The executable's name, which is also the tool's name in reports.
    pub fn name(self) -> &'static str {
        match self {
            BaselineTool::Maestro => "maestro",
            BaselineTool::MaestroRunner => "maestro-runner",
            BaselineTool::AgentDevice => "agent-device",
        }
    }
}

/// Where a baseline adapter looks for its tool, where it writes flow files and
/// how long it waits.
#[derive(Clone, Debug)]
pub struct BaselineConfig {
    /// Directories searched for the executable, in order: `PATH` by default.
    pub search_path: Vec<PathBuf>,
    /// Where flow files are written for the tools to read.
    pub work_dir: PathBuf,
    /// Deadline for one run of a flow.
    pub flow_timeout: Duration,
    /// Deadline for `--version`; Maestro starts a JVM to answer it.
    pub version_timeout: Duration,
}

impl BaselineConfig {
    /// Searches `PATH`, writes flow files under `work_dir`, and gives a flow
    /// 120 s and `--version` 30 s.
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        BaselineConfig {
            search_path: search_path(),
            work_dir: work_dir.into(),
            flow_timeout: Duration::from_secs(120),
            version_timeout: Duration::from_secs(30),
        }
    }
}

/// The directories of the `PATH` environment variable.
pub fn search_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

/// The first executable file named `name` in `dirs`, following symbolic links.
pub fn find_executable(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

/// Whether `path` is a file anyone may execute.
pub(crate) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The version in a tool's `--version` output: the first word of its first
/// non-empty line that starts with a digit, or with `v` and a digit, else the
/// whole line.
pub fn version_from(output: &str) -> Option<String> {
    let line = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    let version = line.split_whitespace().find_map(|word| {
        let bare = word.strip_prefix('v').unwrap_or(word);
        bare.starts_with(|c: char| c.is_ascii_digit())
            .then_some(bare)
    });
    Some(version.unwrap_or(line).to_owned())
}

/// One adapter per baseline tool, in report order.
pub fn baselines(config: &BaselineConfig) -> Vec<Box<dyn Adapter>> {
    BaselineTool::ALL
        .into_iter()
        .map(|tool| Box::new(Baseline::new(tool, config.clone())) as Box<dyn Adapter>)
        .collect()
}

/// A baseline tool driven through its CLI.
#[derive(Clone, Debug)]
pub struct Baseline {
    tool: BaselineTool,
    config: BaselineConfig,
    /// The executable, once detected.
    path: Option<PathBuf>,
}

impl Baseline {
    pub fn new(tool: BaselineTool, config: BaselineConfig) -> Self {
        Baseline {
            tool,
            config,
            path: None,
        }
    }

    /// The flow file for `flow` on `target`, rewritten before every run.
    fn flow_file(&self, flow: &Flow, target: &Target, extension: &str) -> PathBuf {
        self.config.work_dir.join(format!(
            "{}-{}-{}.{extension}",
            self.tool.name(),
            flow.name,
            target.id
        ))
    }
}

impl Adapter for Baseline {
    fn name(&self) -> &str {
        self.tool.name()
    }

    fn role(&self) -> Role {
        Role::Baseline
    }

    fn detect(&mut self) -> Detection {
        let name = self.tool.name();
        let Some(path) = find_executable(name, &self.config.search_path) else {
            return Detection::Absent {
                note: format!("{name} not found on PATH"),
            };
        };
        let version = process::run(
            Command::new(&path).arg("--version"),
            self.config.version_timeout,
        )
        .ok()
        .filter(|finished| finished.status.success())
        .and_then(|finished| {
            version_from(&finished.stdout).or_else(|| version_from(&finished.stderr))
        });
        self.path = Some(path);
        Detection::Available { version }
    }

    fn supports(&self, _target: &Target, _scenario: &Scenario, subject: &Subject) -> bool {
        matches!(subject, Subject::Flow(_))
    }

    fn run_flow(&mut self, target: &Target, flow: &Flow) -> Result<Sample, AdapterError> {
        let name = self.tool.name();
        let path = self
            .path
            .clone()
            .ok_or_else(|| AdapterError::new(format!("{name} was not detected")))?;
        let platform = target.class.platform();
        let app = flow.fixture.app_id(platform).ok_or_else(|| {
            AdapterError::new(format!(
                "the {} fixture does not run on {}",
                flow.fixture, target.class
            ))
        })?;
        let (file, contents, args) = match self.tool {
            BaselineTool::Maestro | BaselineTool::MaestroRunner => {
                let file = self.flow_file(flow, target, "yaml");
                let args = if self.tool == BaselineTool::Maestro {
                    maestro::maestro_args(target, &file)
                } else {
                    maestro::runner_args(target, &file)
                };
                (file, maestro::flow_yaml(flow, app), args)
            }
            BaselineTool::AgentDevice => {
                let file = self.flow_file(flow, target, "json");
                let args = agent_device::args(target, SESSION, &file);
                (file, agent_device::steps(flow, app, platform), args)
            }
        };
        fs::create_dir_all(&self.config.work_dir)
            .and_then(|()| fs::write(&file, contents))
            .map_err(|e| AdapterError::new(format!("cannot write {}: {e}", file.display())))?;
        let finished = process::run(Command::new(&path).args(&args), self.config.flow_timeout)
            .map_err(AdapterError::new)?;
        let detail = match self.tool {
            BaselineTool::AgentDevice => {
                let detail = agent_device::outcome(&finished.stdout).map_err(AdapterError::new)?;
                if !finished.status.success() {
                    return Err(AdapterError::new(exit_failure(&finished)));
                }
                detail
            }
            BaselineTool::Maestro | BaselineTool::MaestroRunner => {
                if !finished.status.success() {
                    return Err(AdapterError::new(exit_failure(&finished)));
                }
                None
            }
        };
        Ok(Sample {
            elapsed: finished.elapsed,
            clock: Clock::Wall,
            nodes: None,
            detail,
        })
    }
}

/// The exit status and the last three lines the tool printed, at most 500
/// characters of them.
pub(crate) fn exit_failure(finished: &Finished) -> String {
    let status = finished.status.code().map_or_else(
        || "killed by a signal".to_owned(),
        |code| format!("exit status {code}"),
    );
    let lines: Vec<&str> = finished
        .stdout
        .lines()
        .chain(finished.stderr.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail: String = lines[lines.len().saturating_sub(3)..]
        .join(" / ")
        .chars()
        .take(500)
        .collect();
    if tail.is_empty() {
        status
    } else {
        format!("{status}: {tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mobile-agent-bench-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(path: &Path, mode: u32) {
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn an_executable_is_found_in_the_first_directory_holding_it() {
        let dir = scratch("find-first");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        file(&a.join("maestro"), 0o755);
        file(&b.join("maestro"), 0o755);
        assert_eq!(
            find_executable("maestro", &[b.clone(), a.clone()]),
            Some(b.join("maestro"))
        );
        assert_eq!(find_executable("maestro-runner", &[a, b]), None);
    }

    #[test]
    fn files_nobody_may_execute_and_directories_are_not_tools() {
        let dir = scratch("find-skip");
        let (plain, holder, real) = (dir.join("plain"), dir.join("holder"), dir.join("real"));
        for d in [&plain, &holder, &real] {
            fs::create_dir_all(d).unwrap();
        }
        file(&plain.join("agent-device"), 0o644);
        fs::create_dir_all(holder.join("agent-device")).unwrap();
        file(&real.join("agent-device"), 0o755);
        assert_eq!(
            find_executable("agent-device", &[plain, holder, real.clone()]),
            Some(real.join("agent-device"))
        );
    }

    #[test]
    fn a_symbolic_link_to_an_executable_is_a_tool() {
        let dir = scratch("find-link");
        file(&dir.join("agent-device.mjs"), 0o755);
        std::os::unix::fs::symlink(dir.join("agent-device.mjs"), dir.join("agent-device")).unwrap();
        assert_eq!(
            find_executable("agent-device", std::slice::from_ref(&dir)),
            Some(dir.join("agent-device"))
        );
    }

    #[test]
    fn versions_are_read_from_the_first_line() {
        assert_eq!(version_from("2.10.0\n").as_deref(), Some("2.10.0"));
        assert_eq!(
            version_from("\nmaestro-runner version v1.1.0\nbuilt 2026-09-01\n").as_deref(),
            Some("1.1.0")
        );
        assert_eq!(version_from("0.21.14").as_deref(), Some("0.21.14"));
        assert_eq!(
            version_from("development build").as_deref(),
            Some("development build")
        );
        assert_eq!(version_from(" \n"), None);
    }

    #[test]
    fn baselines_are_flow_only_and_in_report_order() {
        let adapters = baselines(&BaselineConfig::new("/nonexistent"));
        let names: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
        assert_eq!(names, ["maestro", "maestro-runner", "agent-device"]);
        let target = Target::new("emulator-5554", crate::target::TargetClass::AndroidEmulator);
        let adapter = &adapters[0];
        assert_eq!(adapter.role(), Role::Baseline);
        assert!(adapter.supports(
            &target,
            &Scenario::default(),
            &Subject::Flow("login".into())
        ));
        assert!(!adapter.supports(
            &target,
            &Scenario::default(),
            &Subject::Primitive(crate::primitive::Primitive::WarmSnapshot)
        ));
    }
}
