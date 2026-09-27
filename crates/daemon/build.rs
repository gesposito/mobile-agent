//! Builds the Android agent with `agent-android/build.sh` and embeds its jar
//! in the daemon, with its digest, so an installed daemon deploys the agent
//! without the source tree or `MOBILE_AGENT_JAR` (task 8.11).
//!
//! The script runs on a copy of `agent-android` in `OUT_DIR`, so builds of
//! other profiles never share its `out/` and `dist/`, and the source tree is
//! left alone. `MOBILE_AGENT_JAR` names a prebuilt jar to embed instead;
//! `SKIP_AGENT_BUILD=1` embeds none, for hosts without the Android SDK and a
//! Kotlin compiler, and the daemon then refuses to deploy an agent unless
//! `MOBILE_AGENT_JAR` names one at run time. Any other failure to produce
//! the jar fails the build, with the script's output.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `agent-android/build.sh` reads.
const INPUTS: [&str; 4] = ["src", "VERSION", "build.sh", "agent.pro"];

/// Where `build.sh` writes the jar, relative to its directory.
const JAR: &str = "dist/mobile-agent-android.jar";

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let agent = manifest.join("../../agent-android");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets it"));
    for input in INPUTS {
        println!("cargo:rerun-if-changed={}", agent.join(input).display());
    }
    for variable in [
        "MOBILE_AGENT_JAR",
        "SKIP_AGENT_BUILD",
        "ANDROID_HOME",
        "KOTLIN_HOME",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    let prebuilt = env::var_os("MOBILE_AGENT_JAR").filter(|path| !path.is_empty());
    let skipped = env::var("SKIP_AGENT_BUILD").is_ok_and(|value| value == "1");
    let jar = match prebuilt {
        Some(path) => fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "MOBILE_AGENT_JAR names {}, which cannot be read ({error}); build the agent with \
                 agent-android/build.sh or unset MOBILE_AGENT_JAR",
                Path::new(&path).display()
            )
        }),
        None if skipped => {
            println!(
                "cargo:warning=SKIP_AGENT_BUILD=1: this daemon embeds no Android agent and \
                 deploys one only from the jar MOBILE_AGENT_JAR names at run time"
            );
            Vec::new()
        }
        None => build(&agent, &out),
    };
    let embedded = out.join("mobile-agent-android.jar");
    fs::write(&embedded, &jar).expect("OUT_DIR is writable");
    println!(
        "cargo:rustc-env=MOBILE_AGENT_EMBEDDED_JAR={}",
        embedded.display()
    );
    println!(
        "cargo:rustc-env=MOBILE_AGENT_EMBEDDED_JAR_BUILD={}",
        digest(&jar)
    );
}

/// The agent's build as `android::agent::AgentJar::build` names it: the
/// first 12 hex digits of the jar's xxh3 digest; empty for no jar.
fn digest(jar: &[u8]) -> String {
    if jar.is_empty() {
        return String::new();
    }
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(jar))[..12].to_owned()
}

/// Runs `build.sh` on a copy of `agent` in `out` and reads the jar it
/// wrote; fails the build, with the script's output, when it cannot.
fn build(agent: &Path, out: &Path) -> Vec<u8> {
    let copy = out.join("agent-android");
    let _ = fs::remove_dir_all(&copy);
    for input in INPUTS {
        copy_all(&agent.join(input), &copy.join(input)).unwrap_or_else(|error| {
            panic!(
                "cannot copy {} to build the Android agent: {error}",
                agent.join(input).display()
            )
        });
    }
    let ran = Command::new("bash").arg(copy.join("build.sh")).output();
    let jar = copy.join(JAR);
    match ran {
        Ok(output) if output.status.success() && jar.is_file() => {
            fs::read(&jar).expect("build.sh wrote the jar")
        }
        Ok(output) => panic!(
            "agent-android/build.sh did not build the Android agent the daemon embeds \
             ({}):\n{}{}\nInstall the Android SDK (platforms;android-36 and build-tools) and a \
             Kotlin compiler, or set MOBILE_AGENT_JAR to a jar build.sh wrote, or set \
             SKIP_AGENT_BUILD=1 to build a daemon without the agent",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => panic!(
            "cannot run agent-android/build.sh ({error}); set MOBILE_AGENT_JAR to a jar it wrote, \
             or SKIP_AGENT_BUILD=1 to build a daemon without the agent"
        ),
    }
}

/// Copies a file, or a directory with everything in it.
fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from, to).map(|_| ())
    }
}
