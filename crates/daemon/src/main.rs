//! `mobile-agent-daemon`: the host daemon the `mobile-agent` CLI starts on
//! its first command. See the crate's README for its protocol and files.

use std::process::ExitCode;

const HELP: &str = "\
mobile-agent-daemon: the host daemon behind the mobile-agent CLI

Usage: mobile-agent-daemon [--socket-dir <dir>] [--idle-timeout <time>] [--drivers <names>]
                           [--emulator-controller <auto|off>]
                           [--fake <targets>] [--fake-latency <time>] [--fake-frame-delay <time>]
                           [--fake-splash <time>] [--fake-activation <time>] [--fake-stop <time>]
                           [--fake-fling]

Options:
  --socket-dir <dir>      Where the socket and sidecar files live
                          (or MOBILE_AGENT_SOCKET_DIR; default
                          $XDG_RUNTIME_DIR/mobile-agent, else ~/.mobile-agent)
  --idle-timeout <time>   Exit after this long without requests: ms, or a
                          number with s, m or h; 0 never exits
                          (or MOBILE_AGENT_IDLE_TIMEOUT_MS; default 1h)
  --drivers <names>       Device drivers to register: simulator (macOS only),
                          android, or none (or MOBILE_AGENT_DRIVERS; default
                          every driver this host runs, none with --fake)
  --emulator-controller <auto|off>
                          Whether taps, swipes and frames on Android
                          emulators go through the emulator's gRPC
                          controller when it answers (auto), or through the
                          agent, as on a device (off)
                          (or MOBILE_AGENT_EMULATOR_CONTROLLER; default auto)
  --fake <targets>        Serve fake targets, such as android,ios:FAKE-IOS-1
                          (or MOBILE_AGENT_FAKE_TARGETS)
  --fake-latency <time>   Show every change on fake targets this long after
                          its cause (or MOBILE_AGENT_FAKE_LATENCY_MS; default 0)
  --fake-frame-delay <time>
                          Start fake targets' frame streams this long after
                          the attach (or MOBILE_AGENT_FAKE_FRAME_DELAY_MS;
                          default 0)
  --fake-splash <time>    Show fake targets' fixture apps' launch splash this
                          long when their process starts
                          (or MOBILE_AGENT_FAKE_SPLASH_MS; default 0)
  --fake-activation <time>
                          Hold fake targets' input until their input session
                          has been started this long
                          (or MOBILE_AGENT_FAKE_ACTIVATION_MS; default 0)
  --fake-stop <time>      Take this long to detach each fake target, as
                          stopping the Android agent takes
                          (or MOBILE_AGENT_FAKE_STOP_MS; default 0)
  --fake-fling            Let fake targets' lists glide on after a stroke
                          whose finger lifts moving, as far as the
                          platform's fling carries them
                          (or MOBILE_AGENT_FAKE_FLING=1; default off)
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
