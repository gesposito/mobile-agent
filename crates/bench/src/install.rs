//! Fixtures installed on a physical iPhone before a run. Each device build
//! `run --ios-app` names goes in through CoreDevice, `xcrun devicectl device
//! install app --device <id> --json-output - <bundle>`, timed from the
//! process's start to its exit, and the app CoreDevice reports installed must
//! be the fixture's. The harness installs through CoreDevice rather than
//! through the tool under test, so it stays independent of that tool and
//! opens no second tunnel to the phone. It installs nothing on other targets,
//! whose fixtures are installed beforehand.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::fixture::Fixture;
use crate::plan;
use crate::process;
use crate::stats::micros;
use crate::target::{Install, Platform, Target};

/// How long one install may take; the fixtures took 1.5 to 3.4 s on an
/// iPhone SE (3rd generation).
const TIMEOUT: Duration = Duration::from_secs(120);

/// `<fixture>=<path>`, as `run --ios-app` takes it: a fixture with an iPhone
/// build, `swiftui` or `expo`, and its device build, an app's bundle.
pub fn parse_app(text: &str) -> Result<(Fixture, PathBuf), String> {
    let (name, path) = text
        .split_once('=')
        .filter(|(name, path)| !name.is_empty() && !path.is_empty())
        .ok_or_else(|| {
            format!(
                "--ios-app takes <fixture>=<path>, such as swiftui=FixtureSwiftUI.app, not {text}"
            )
        })?;
    let iphone = "the fixtures with an iPhone build are swiftui and expo";
    let fixture = plan::FIXTURES
        .into_iter()
        .find(|fixture| fixture.id() == name)
        .ok_or_else(|| format!("--ios-app {text}: {name} is no fixture; {iphone}"))?;
    if fixture.app_id(Platform::Ios).is_none() {
        return Err(format!(
            "--ios-app {text}: the {fixture} fixture has no iPhone build; {iphone}"
        ));
    }
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err(format!(
            "--ios-app {text}: {} is not a directory, as an app's bundle is",
            path.display()
        ));
    }
    Ok((fixture, path))
}

/// Installs `bundle`, `fixture`'s device build, on the physical iPhone
/// `target` through CoreDevice, and checks that the app it installed is the
/// fixture's. Fails with CoreDevice's reason when the install fails.
pub fn install(target: &Target, fixture: Fixture, bundle: &Path) -> Result<Install, String> {
    let shown = bundle.display().to_string();
    let failed = |reason: String| {
        format!(
            "cannot install the {fixture} fixture's build {shown} on {}: {reason}",
            target.id
        )
    };
    let finished = process::run(
        Command::new("xcrun")
            .args(["devicectl", "device", "install", "app", "--device"])
            .arg(&target.id)
            .args(["--json-output", "-"])
            .arg(bundle),
        TIMEOUT,
    )
    .map_err(failed)?;
    let reply: Option<Value> = serde_json::from_str(finished.stdout.trim()).ok();
    if !finished.status.success() {
        let reason = reply
            .as_ref()
            .and_then(|reply| errors(&reply["error"]))
            .or_else(|| {
                finished
                    .stderr
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("devicectl exited with {}", finished.status));
        return Err(failed(reason));
    }
    let Some(expected) = fixture.app_id(Platform::Ios) else {
        return Err(failed(format!("the {fixture} fixture has no iPhone build")));
    };
    let installed: Vec<&str> = reply
        .as_ref()
        .and_then(|reply| reply["result"]["installedApplications"].as_array())
        .map(|apps| {
            apps.iter()
                .filter_map(|app| app["bundleID"].as_str())
                .collect()
        })
        .unwrap_or_default();
    if !installed.contains(&expected) {
        let found = if installed.is_empty() {
            "no app CoreDevice named".to_owned()
        } else {
            installed.join(", ")
        };
        return Err(format!(
            "the {fixture} fixture's build {shown} on {} installed {found}, not the {fixture} \
             fixture's {expected}",
            target.id
        ));
    }
    Ok(Install {
        fixture,
        bundle: shown,
        elapsed_us: micros(finished.elapsed),
    })
}

/// CoreDevice's error descriptions in `error`, the outermost first and each
/// underlying one after it, joined by `: `; `None` when it gives none.
fn errors(error: &Value) -> Option<String> {
    let mut found = Vec::new();
    let mut current = error;
    while current.is_object() {
        if let Some(text) = current["userInfo"]["NSLocalizedDescription"]["string"].as_str() {
            found.push(text.to_owned());
        }
        current = &current["userInfo"]["NSUnderlyingError"]["error"];
    }
    (!found.is_empty()).then(|| found.join(": "))
}
