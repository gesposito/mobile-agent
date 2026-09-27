//! `simulator-reader`: the accessibility reader the Simulator driver spawns
//! inside a booted Simulator. See the library documentation.
//!
//! ```text
//! simulator-reader connect <socket> [--token <token>] [--scale <pixels per point>]
//! ```
//!
//! It loads the runtime, connects to the Unix socket the host listens on
//! and serves the device protocol there. Exit status: 0 once the host is
//! gone or shut it down, 2 for a usage error, 3 when the runtime cannot be
//! loaded, 4 when the socket cannot be reached, 70 when a read hung.

use std::process::ExitCode;

#[cfg(target_vendor = "apple")]
fn main() -> ExitCode {
    use std::path::PathBuf;

    use simulator_reader::guest::{self, Stop};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let (Some("connect"), Some(socket)) = (args.first().map(String::as_str), args.get(1)) else {
        eprintln!(
            "usage: simulator-reader connect <socket> [--token <token>] [--scale <pixels per point>]"
        );
        return ExitCode::from(2);
    };
    let scale = match value("--scale").map(|scale| scale.parse::<f64>()) {
        None => 1.0,
        Some(Ok(scale)) if scale > 0.0 => scale,
        Some(_) => {
            eprintln!("simulator-reader: --scale takes a positive number");
            return ExitCode::from(2);
        }
    };
    match guest::run(&PathBuf::from(socket), value("--token"), scale) {
        Ok(_) => ExitCode::SUCCESS,
        Err(Stop::Runtime(reason)) => {
            eprintln!("simulator-reader: the accessibility runtime is unavailable: {reason}");
            ExitCode::from(3)
        }
        Err(Stop::Connect(reason)) => {
            eprintln!("simulator-reader: the host's socket is unreachable: {reason}");
            ExitCode::from(4)
        }
    }
}

#[cfg(not(target_vendor = "apple"))]
fn main() -> ExitCode {
    eprintln!("simulator-reader runs only inside an iOS Simulator");
    ExitCode::from(2)
}
