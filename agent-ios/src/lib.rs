//! The runner's agent (design D20, spec `ios-device-driver`): the device
//! protocol of `crates/protocol` served from inside an XCTest bundle on a
//! physical iPhone. `build.sh` links this crate, built as a static library
//! for `aarch64-apple-ios`, with the Objective-C shim (`shim/Runner.m`) into
//! the runner's test bundle; `README.md` describes the bundle.
//!
//! - [`xctree`] turns XCTest's snapshot of the foreground application, which
//!   the shim hands over as JSON, into the Simulator reader's
//!   [`simulator_reader::elements::Element`]s, which the reader's conversion
//!   turns into the protocol's snapshot.

pub mod xctree;

/// The name the runner's hello puts before its version, which the driver
/// checks (`ios::RUNNER_NAME`).
pub const NAME: &str = "mobile-agent-ios-runner";

/// The runner's version, from `VERSION` (`build.rs`).
pub const VERSION: &str = env!("AGENT_IOS_VERSION");

/// The runner's name and version, as its hello announces them.
pub const AGENT: &str = concat!("mobile-agent-ios-runner ", env!("AGENT_IOS_VERSION"));

/// The loopback port the runner listens on, which the host reaches through
/// usbmuxd's `Connect` (`ios::RUNNER_PORT`).
pub const PORT: u16 = 27183;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hello_names_the_runner_and_the_version_file() {
        let file = include_str!("../VERSION").trim();
        assert_eq!(VERSION, file);
        assert_eq!(AGENT, format!("{NAME} {file}"));
    }
}
