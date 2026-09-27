//! Running a tool's process with a deadline and its output captured.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long output is still collected after the process exits. A tool that
/// starts a daemon can leave it holding the pipes; its output is cut off here
/// instead of blocking the run.
const OUTPUT_GRACE: Duration = Duration::from_millis(250);

/// A process that exited before its deadline.
#[derive(Debug)]
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
    /// Just before the spawn call.
    pub started: Instant,
    /// From the spawn call to the exit being observed. The exit is polled at an
    /// interval of 1 % of the time elapsed so far, bounded to 50 µs..=5 ms.
    pub elapsed: Duration,
}

/// Runs `command` with stdin closed and stdout and stderr captured, killing it
/// once `timeout` has passed. Errors name the program and say whether it could
/// not start or timed out.
pub fn run(command: &mut Command, timeout: Duration) -> Result<Finished, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let start = Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    // Both pipes drain on their own threads, so a chatty process never blocks
    // on a full pipe while it is being waited for.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let status = wait(&mut child, start, timeout, &program)?;
    let elapsed = start.elapsed();
    let deadline = Instant::now() + OUTPUT_GRACE;
    Ok(Finished {
        status,
        stdout: collect(stdout, deadline),
        stderr: collect(stderr, deadline),
        started: start,
        elapsed,
    })
}

/// Runs `command` with its streams closed, as hyperfine runs a command whose
/// output it discards, and returns its exit status and the time from the
/// spawn call to the exit being observed, polled as [`run`] polls it. Errors
/// name the program and say whether it could not start or timed out.
pub fn time(command: &mut Command, timeout: Duration) -> Result<(ExitStatus, Duration), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let start = Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    let status = wait(&mut child, start, timeout, &program)?;
    Ok((status, start.elapsed()))
}

/// Polls `child`, spawned at `start`, until it exits, killing it once
/// `timeout` has passed.
fn wait(
    child: &mut Child,
    start: Instant,
    timeout: Duration,
    program: &str,
) -> Result<ExitStatus, String> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => return Err(format!("cannot wait for {program}: {e}")),
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{program} timed out after {} ms",
                timeout.as_millis()
            ));
        }
        thread::sleep((elapsed / 100).clamp(Duration::from_micros(50), Duration::from_millis(5)));
    }
}

/// A pipe being read to its end on its own thread.
struct Drain {
    reader: JoinHandle<()>,
    bytes: Arc<Mutex<Vec<u8>>>,
}

fn drain(mut pipe: impl Read + Send + 'static) -> Drain {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bytes);
    let reader = thread::spawn(move || {
        let mut chunk = [0; 8192];
        while let Ok(n @ 1..) = pipe.read(&mut chunk) {
            sink.lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(&chunk[..n]);
        }
    });
    Drain { reader, bytes }
}

/// What the pipe delivered by its end or by `deadline`, whichever is first.
fn collect(drain: Option<Drain>, deadline: Instant) -> String {
    let Some(drain) = drain else {
        return String::new();
    };
    while !drain.reader.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    let bytes = drain.bytes.lock().unwrap_or_else(|e| e.into_inner());
    String::from_utf8_lossy(&bytes).into_owned()
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
    fn a_background_process_holding_the_pipes_does_not_hold_the_run() {
        let start = Instant::now();
        let finished = run(
            Command::new("sh").args(["-c", "sleep 3 & echo started"]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(finished.stdout, "started\n");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_timed_run_gives_the_exit_status_and_covers_the_run() {
        let (status, elapsed) =
            time(Command::new("sleep").arg("0.2"), Duration::from_secs(10)).unwrap();
        assert!(status.success());
        assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        let (status, _) = time(
            Command::new("sh").args(["-c", "echo discarded; exit 4"]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(status.code(), Some(4));
        let error = time(Command::new("sleep").arg("5"), Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn elapsed_covers_the_run() {
        let finished = run(Command::new("sleep").arg("0.2"), Duration::from_secs(10)).unwrap();
        assert!(finished.elapsed >= Duration::from_millis(200));
        assert!(finished.elapsed < Duration::from_secs(5));
    }
}
