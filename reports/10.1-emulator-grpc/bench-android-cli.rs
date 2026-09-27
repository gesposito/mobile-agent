//! Stands in for `mobile-agent` as `mobile-agent-bench run --cli` on an Android target, where
//! the bench's CLI adapter and the device disagree in two ways (reports/10.1-emulator.md,
//! Findings):
//!
//! - The adapter opens a fixture's Big or List screen with `--env FIXTURE_INITIAL_TAB=<tab>` on
//!   every platform. The Android fixtures read the launch intent's string extra `initial_tab`
//!   instead (fixtures/CONTRACT.md, Launch options), and the daemon passes each `--env` entry
//!   as an extra of the same name, so the app stays on its Login tab. The shim renames the entry
//!   that follows `--env`.
//! - Android has one UI automation owner, and the bench hands the device from one tool to the
//!   next without releasing it. The daemon keeps its agent, and with it UI automation, until it
//!   exits, so Maestro's and agent-device's drivers fail with "UiAutomationService ... already
//!   registered!"; the daemon that the shim's commands start exits after
//!   `MOBILE_AGENT_IDLE_TIMEOUT_MS` without a request, 1000 ms unless set, and stops its agent
//!   as it exits, before a baseline's driver starts. agent-device in turn leaves its snapshot
//!   helper's instrumentation holding UI automation after each batch, so mobile-agent's next
//!   attach fails with `automation_conflict`; `MOBILE_AGENT_FORCE`, 1 unless set, has the agent
//!   stop that holder and take the role over.
//!
//! Every other argument passes through unchanged, and the shim replaces its process with
//! mobile-agent, so the output and exit status are mobile-agent's and the cost is one exec.
//! `MOBILE_AGENT_BIN` names the executable; the default is the `mobile-agent` beside the shim.
//!
//! ```sh
//! cargo build --release -p cli -p daemon -p bench
//! rustc -O -o target/release/bench-android-cli scripts/bench-android-cli.rs
//! target/release/mobile-agent-bench run --target emulator-5554 \
//!   --cli target/release/bench-android-cli --out reports/<run>
//! ```

use std::env;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

/// Settings the shim gives the commands it runs, unless the environment sets them.
const DEFAULTS: [(&str, &str); 2] = [
    ("MOBILE_AGENT_IDLE_TIMEOUT_MS", "1000"),
    ("MOBILE_AGENT_FORCE", "1"),
];

fn main() -> ExitCode {
    let program = env::var_os("MOBILE_AGENT_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            let shim = env::current_exe().ok()?;
            Some(shim.parent()?.join("mobile-agent"))
        })
        .unwrap_or_else(|| PathBuf::from("mobile-agent"));

    let mut args: Vec<OsString> = Vec::new();
    let mut after_env = false;
    for arg in env::args_os().skip(1) {
        let is_env = arg == "--env";
        let renamed = after_env
            .then(|| arg.to_str()?.strip_prefix("FIXTURE_INITIAL_TAB="))
            .flatten()
            .map(|tab| OsString::from(format!("initial_tab={tab}")));
        args.push(renamed.unwrap_or(arg));
        after_env = is_env;
    }

    let mut command = Command::new(&program);
    command.args(&args);
    for (key, value) in DEFAULTS {
        if env::var_os(key).is_none() {
            command.env(key, value);
        }
    }
    let error = command.exec();
    eprintln!(
        "bench-android-cli: cannot run {}: {error}",
        program.display()
    );
    ExitCode::from(127)
}
