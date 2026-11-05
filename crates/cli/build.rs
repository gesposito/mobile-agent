//! The version `mobile-agent --version` prints, as `MOBILE_AGENT_VERSION`:
//! the release the build was made for, which the release workflow names in
//! `MOBILE_AGENT_RELEASE`, else `local`, then the commit it was built from,
//! the first 12 digits of its hash in parentheses, when the source is a git
//! checkout.

use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=MOBILE_AGENT_RELEASE");
    let release = env::var("MOBILE_AGENT_RELEASE")
        .ok()
        .filter(|release| !release.is_empty())
        .unwrap_or_else(|| "local".to_owned());
    // HEAD's reflog grows with every commit, checkout and reset of the
    // checkout, so the script runs again when the commit changes.
    for path in ["HEAD", "logs/HEAD"] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    let version = match git(&["rev-parse", "HEAD"]) {
        Some(sha) if sha.len() >= 12 => format!("{release} ({})", &sha[..12]),
        _ => release,
    };
    println!("cargo:rustc-env=MOBILE_AGENT_VERSION={version}");
}

/// What git prints for `args` in the package's directory, or `None` when
/// git is missing or the directory is not in a checkout.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}
