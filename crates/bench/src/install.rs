//! Fixtures installed on a physical iPhone before a run.

use std::path::{Path, PathBuf};

use crate::fixture::Fixture;
use crate::target::{Install, Target};

/// `<fixture>=<path>`, as `run --ios-app` takes it.
pub fn parse_app(text: &str) -> Result<(Fixture, PathBuf), String> {
    Err(format!("{text}: not parsed yet"))
}

/// Installs `bundle`, `fixture`'s build, on the physical iPhone `target`.
pub fn install(target: &Target, fixture: Fixture, bundle: &Path) -> Result<Install, String> {
    let _ = (target, fixture, bundle);
    Err("not installed yet".to_owned())
}
