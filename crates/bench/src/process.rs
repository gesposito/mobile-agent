//! Running a tool's process with a deadline and its output captured.

use std::process::{Command, ExitStatus};
use std::time::Duration;

/// A process that exited before its deadline.
#[derive(Debug)]
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
    /// From the spawn call to the exit being observed. The exit is polled at an
    /// interval of 1 % of the time elapsed so far, bounded to 50 µs..=5 ms.
    pub elapsed: Duration,
}

/// Runs `command` with stdin closed and stdout and stderr captured, killing it
/// once `timeout` has passed. Errors name the program and say whether it could
/// not start or timed out.
pub fn run(command: &mut Command, timeout: Duration) -> Result<Finished, String> {
    todo!("run {:?} within {timeout:?}", command.get_program())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn captures_both_streams_and_the_exit_status() {
        let finished = run(
            Command::new("sh").args(["-c", "echo out; echo err >&2; exit 3"]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(finished.status.code(), Some(3));
        assert_eq!(finished.stdout, "out\n");
        assert_eq!(finished.stderr, "err\n");
    }

    #[test]
    fn a_process_past_its_deadline_is_killed_and_reported() {
        let start = Instant::now();
        let error = run(Command::new("sleep").arg("5"), Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("sleep"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn a_missing_program_is_an_error_naming_it() {
        let error = run(
            &mut Command::new("mobile-agent-bench-no-such-program"),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(
            error.contains("mobile-agent-bench-no-such-program"),
            "{error}"
        );
    }

    #[test]
    fn large_output_does_not_block_the_process() {
        // More than a pipe buffer on both streams, so the child blocks unless
        // both are drained while it runs.
        let finished = run(
            Command::new("sh").args([
                "-c",
                "i=0; while [ $i -lt 20000 ]; do echo 0123456789; echo 0123456789 >&2; i=$((i+1)); done",
            ]),
            Duration::from_secs(20),
        )
        .unwrap();
        assert!(finished.status.success());
        assert_eq!(finished.stdout.len(), 220_000);
        assert_eq!(finished.stderr.len(), 220_000);
    }

    #[test]
    fn elapsed_covers_the_run() {
        let finished = run(Command::new("sleep").arg("0.2"), Duration::from_secs(10)).unwrap();
        assert!(finished.elapsed >= Duration::from_millis(200));
        assert!(finished.elapsed < Duration::from_secs(5));
    }
}
