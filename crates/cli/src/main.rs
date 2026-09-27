//! `mobile-agent`: drive devices, emulators and Simulators with
//! agent-browser's grammar. See the crate's README.

use std::process::ExitCode;

fn main() -> ExitCode {
    let status = cli::run(std::env::args_os().collect());
    ExitCode::from(u8::try_from(status).unwrap_or(1))
}
