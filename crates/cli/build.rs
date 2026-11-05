//! The version `mobile-agent --version` prints, as `MOBILE_AGENT_VERSION`:
//! the release the build was made for, then the commit it was built from,
//! the first 12 digits of its hash in parentheses, when it is known. The
//! release workflow names the release in `MOBILE_AGENT_RELEASE`. A build of
//! the crate's package, as `cargo install` makes from crates.io, is of the
//! package's version and of the commit its `.cargo_vcs_info.json` records,
//! which cargo writes into a package it makes in a git checkout. Any other
//! build is `local`, of the checkout's commit when the source is one.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=MOBILE_AGENT_RELEASE");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let packaged = fs::read_to_string(manifest.join(".cargo_vcs_info.json")).ok();
    let release = env::var("MOBILE_AGENT_RELEASE")
        .ok()
        .filter(|release| !release.is_empty())
        .or_else(|| {
            packaged
                .as_ref()
                .map(|_| env::var("CARGO_PKG_VERSION").expect("cargo sets it"))
        })
        .unwrap_or_else(|| "local".to_owned());
    let commit = match &packaged {
        Some(info) => packaged_commit(info),
        None => {
            // HEAD's reflog grows with every commit, checkout and reset of
            // the checkout, so the script runs again when the commit changes.
            for path in ["HEAD", "logs/HEAD"] {
                if let Some(file) = git(&["rev-parse", "--git-path", path]) {
                    println!("cargo:rerun-if-changed={file}");
                }
            }
            git(&["rev-parse", "HEAD"])
        }
    };
    let version = match commit {
        Some(sha) if sha.len() >= 12 => format!("{release} ({})", &sha[..12]),
        _ => release,
    };
    println!("cargo:rustc-env=MOBILE_AGENT_VERSION={version}");
}

/// The commit in a package's `.cargo_vcs_info.json`, its `"sha1"`, as in
/// `{"git": {"sha1": "<40 hex digits>", ...}, "path_in_vcs": "crates/cli"}`.
fn packaged_commit(info: &str) -> Option<String> {
    let sha = info.split("\"sha1\"").nth(1)?.split('"').nth(1)?;
    sha.bytes()
        .all(|b| b.is_ascii_hexdigit())
        .then(|| sha.to_owned())
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
