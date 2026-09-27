//! CLI process overhead (spec `agent-cli`: Daemon lifecycle): a command
//! against a running daemon costs under 20 ms, measured with hyperfine when
//! it is installed and by timing the runs here otherwise. The whole run is
//! measured, the daemon's reply included, so it bounds the overhead from
//! process start to the daemon request from above.

mod support;

use std::process::Command;
use std::time::Duration;

use support::{CLI, TestEnv, clean_env};

const GATE: Duration = Duration::from_millis(20);
const WARMUP: usize = 3;
const RUNS: usize = 30;

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// The median of `mobile-agent status` through hyperfine.
fn hyperfine(env: &TestEnv) -> Duration {
    let export = env.path().join("hyperfine.json");
    let mut command = Command::new("hyperfine");
    clean_env(&mut command);
    for (key, value) in env.vars() {
        command.env(key, value);
    }
    let status = command
        .args([
            "--warmup",
            &WARMUP.to_string(),
            "--runs",
            &RUNS.to_string(),
            "-N",
            "--style",
            "none",
            "--export-json",
        ])
        .arg(&export)
        .arg(format!("{CLI} status"))
        .status()
        .expect("hyperfine runs");
    assert!(status.success(), "hyperfine failed");
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&export).expect("hyperfine's report"))
            .expect("hyperfine writes JSON");
    Duration::from_secs_f64(report["results"][0]["median"].as_f64().expect("a median"))
}

/// The median of `mobile-agent status`, timed here.
fn timed(env: &TestEnv) -> Duration {
    for _ in 0..WARMUP {
        env.cli(&["status"]).ok();
    }
    let mut samples: Vec<Duration> = (0..RUNS)
        .map(|_| env.cli(&["status"]).ok().elapsed)
        .collect();
    samples.sort();
    samples[RUNS / 2]
}

#[test]
fn status_overhead_against_a_running_daemon_is_under_20_ms() {
    let env = TestEnv::new("android");
    // The first command starts the daemon; the gate measures the ones after.
    env.cli(&["status"]).ok();
    let (median, how) = if on_path("hyperfine") {
        (hyperfine(&env), "hyperfine")
    } else {
        (timed(&env), "timed runs")
    };
    assert!(
        median < GATE,
        "status took {median:?} at the median over {RUNS} runs ({how}), over the {GATE:?} gate"
    );
}
