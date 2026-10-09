//! Reads `VERSION` into the crate as `AGENT_IOS_VERSION`, the version the
//! runner's hello names, as `agent-android/build.sh` turns its `VERSION` into
//! `BuildConfig.VERSION`. `build.sh` writes the same file's version into the
//! bundle's `CFBundleShortVersionString`, so the three cannot disagree.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let path = manifest.join("VERSION");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", path.display()));
    let version = text.trim();
    assert!(
        is_version(version),
        "{} must hold one semantic version such as 0.2.0, found {version:?}",
        path.display()
    );
    println!("cargo:rustc-env=AGENT_IOS_VERSION={version}");
}

/// Whether `text` is one semantic version, as `agent-android/build.sh` checks
/// its own: three dot-separated numbers, then optionally a hyphen and a
/// pre-release of letters, digits, dots and hyphens.
fn is_version(text: &str) -> bool {
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        && pre.is_none_or(|pre| {
            !pre.is_empty()
                && pre
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
}
