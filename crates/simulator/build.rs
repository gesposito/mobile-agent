//! Builds the in-guest reader (`reader/`, package `simulator-reader`) for
//! `aarch64-apple-ios-sim`, so the crate can embed it: the driver writes it
//! to the host's cache and spawns it into each booted Simulator (design D6).
//! Nothing is built on the machine that runs the driver.
//!
//! The reader is built by a nested cargo in release mode, in a target
//! directory of its own under the outer one, so an unchanged reader costs a
//! fingerprint check. `MOBILE_AGENT_READER` names a prebuilt reader to embed
//! instead. When the host cannot build it (not macOS, or the target's
//! standard library is missing) the crate embeds nothing, prints a warning,
//! and tree reads use the host translator, the degraded fallback; a reader
//! that fails to compile fails the build.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "aarch64-apple-ios-sim";
const PACKAGE: &str = "simulator-reader";

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("set by cargo"));
    for path in [
        "build.rs",
        "reader/Cargo.toml",
        "reader/src",
        "../protocol/Cargo.toml",
        "../protocol/src",
        "../../Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=MOBILE_AGENT_READER");

    let macos = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let outcome = if !macos {
        Err("the Simulator driver runs only on macOS".to_owned())
    } else if let Some(prebuilt) = env::var_os("MOBILE_AGENT_READER") {
        Ok(PathBuf::from(prebuilt))
    } else {
        build(&manifest_dir, &out_dir)
    };

    let binary = out_dir.join(PACKAGE);
    let missing = out_dir.join(format!("{PACKAGE}-missing.txt"));
    match outcome {
        Ok(built) => {
            fs::copy(&built, &binary)
                .unwrap_or_else(|e| panic!("copying the reader from {}: {e}", built.display()));
            fs::write(&missing, "").expect("OUT_DIR is writable");
        }
        Err(why) => {
            if macos {
                println!(
                    "cargo:warning=the Simulator reader is not built into this binary, so tree reads will use the host translator: {why}"
                );
            }
            fs::write(&binary, []).expect("OUT_DIR is writable");
            fs::write(&missing, &why).expect("OUT_DIR is writable");
        }
    }
    println!(
        "cargo:rustc-env=SIMULATOR_READER_BINARY={}",
        binary.display()
    );
    println!(
        "cargo:rustc-env=SIMULATOR_READER_MISSING={}",
        missing.display()
    );
}

/// Builds the reader and returns the path of the binary.
fn build(manifest_dir: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let sysroot = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| format!("running rustc: {e}"))?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    if !sysroot.join("lib/rustlib").join(TARGET).is_dir() {
        return Err(format!(
            "the {TARGET} standard library is not installed; run `rustup target add {TARGET}`"
        ));
    }

    // OUT_DIR is <target>/<profile>/build/<package>-<hash>/out; the reader
    // gets <target>/<profile>/simulator-reader-target, shared by every build
    // of this profile. (<profile>/simulator-reader is its host binary.)
    let target_dir = out_dir
        .ancestors()
        .nth(3)
        .map(|profile| profile.join(format!("{PACKAGE}-target")))
        .unwrap_or_else(|| out_dir.join("reader-target"));
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["build", "--release", "--offline", "--locked"])
        .args(["--package", PACKAGE, "--bin", PACKAGE, "--target", TARGET])
        .arg("--manifest-path")
        .arg(manifest_dir.join("reader/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        // The outer build's wrappers and flags are for the host: clippy's
        // driver under `cargo clippy`, and host rustflags.
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .output()
        .map_err(|e| format!("running cargo: {e}"))?;
    if !output.status.success() {
        panic!(
            "building the Simulator reader for {TARGET} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(target_dir.join(TARGET).join("release").join(PACKAGE))
}
