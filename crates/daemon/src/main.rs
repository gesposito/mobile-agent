//! `mobile-agent-daemon`: the host daemon the `mobile-agent` CLI starts on
//! its first command. See the crate's README for its protocol and files.

use std::process::ExitCode;

const HELP: &str = "\
mobile-agent-daemon: the host daemon behind the mobile-agent CLI

Usage: mobile-agent-daemon [--socket-dir <dir>] [--idle-timeout <time>] [--fake <targets>]
                           [--fake-latency <time>]

Options:
  --socket-dir <dir>      Where the socket and sidecar files live
                          (or MOBILE_AGENT_SOCKET_DIR; default
                          $XDG_RUNTIME_DIR/mobile-agent, else ~/.mobile-agent)
  --idle-timeout <time>   Exit after this long without requests: ms, or a
                          number with s, m or h; 0 never exits
                          (or MOBILE_AGENT_IDLE_TIMEOUT_MS; default 1h)
  --fake <targets>        Serve fake targets, such as android,ios:FAKE-IOS-1
                          (or MOBILE_AGENT_FAKE_TARGETS)
  --fake-latency <time>   Show every change on fake targets this long after
                          its cause (or MOBILE_AGENT_FAKE_LATENCY_MS; default 0)
  -h, --help              Print this help
  -V, --version           Print the version

The mobile-agent CLI starts this daemon on its first command.";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("mobile-agent-daemon {}", daemon::paths::own_version());
        return ExitCode::SUCCESS;
    }
    let options = match daemon::Options::from_args(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("mobile-agent-daemon: {error}");
            return ExitCode::from(2);
        }
    };
    let socket = daemon::paths::Paths::new(&options.socket_dir).socket();
    match daemon::run(options) {
        Ok(daemon::Exit::Stopped) => ExitCode::SUCCESS,
        Ok(daemon::Exit::AlreadyRunning { pid }) => {
            let pid = pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string());
            eprintln!(
                "mobile-agent-daemon: a daemon of this version already serves {} (pid {pid}); leaving it running",
                socket.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("mobile-agent-daemon: {error}");
            ExitCode::FAILURE
        }
    }
}
