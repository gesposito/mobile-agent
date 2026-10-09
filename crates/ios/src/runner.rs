//! The runner's identity (design D20): what its hello must name for the
//! driver to use it, and how the test manager's failures to start it read.
//! The session itself is the macOS driver's (`macos/runner.rs`).

use crate::model::RunnerSpec;

/// The version a hello's agent names after `name`, such as `0.1.0` for
/// `mobile-agent-ios-runner 0.1.0`; `None` when it names another agent.
pub fn reported_version<'a>(name: &str, agent: &'a str) -> Option<&'a str> {
    agent
        .strip_prefix(name)?
        .strip_prefix(' ')
        .map(str::trim)
        .filter(|version| !version.is_empty())
}

/// Whether a hello naming `agent` is the runner `spec` asks for.
pub fn is_expected(spec: &RunnerSpec, agent: &str) -> bool {
    reported_version(&spec.name, agent) == Some(spec.version.as_str())
}

/// Why the test manager's session ended before the runner's hello, read
/// from what `idevice` reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartFailure {
    /// RSD does not list the test manager: the developer disk image is not
    /// mounted ("service not found").
    NoTestManager,
    /// The test manager did not answer its handshake, which S10 saw for a
    /// minute or two after runners were ended mid-start; a later start can
    /// succeed.
    HandshakeUnanswered(String),
    /// XCTest could not load the test bundle (`XCTestErrorDomain` 103),
    /// which `idevice` 0.1.68 reports as an untrusted developer certificate
    /// though S10's cause was an executable of the wrong architecture.
    BundleNotLoaded(String),
    /// Anything else, as reported.
    Other(String),
}

impl StartFailure {
    /// Reads `reason`, the session's error as `idevice` describes it.
    pub fn classify(reason: &str) -> StartFailure {
        let lower = reason.to_ascii_lowercase();
        if lower.contains("service not found") || lower.contains("servicenotfound") {
            StartFailure::NoTestManager
        } else if lower.contains("timed out") && !lower.contains("runner") {
            StartFailure::HandshakeUnanswered(reason.to_owned())
        } else if lower.contains("untrusted developer certificate")
            || lower.contains("nserror code 103")
        {
            StartFailure::BundleNotLoaded(reason.to_owned())
        } else {
            StartFailure::Other(reason.to_owned())
        }
    }

    /// What a person reads of it.
    pub fn describe(&self) -> String {
        match self {
            StartFailure::NoTestManager => "RSD lists no com.apple.dt.testmanagerd.remote, which \
                 the developer disk image carries"
                .to_owned(),
            StartFailure::HandshakeUnanswered(reason) => format!(
                "the test manager did not answer its handshake ({reason}); it stops answering for \
                 a minute or two after a runner is ended mid-start"
            ),
            StartFailure::BundleNotLoaded(reason) => format!(
                "XCTest could not load the runner's test bundle ({reason}); idevice reports this \
                 as an untrusted certificate, but an executable not thinned to arm64 gives it too"
            ),
            StartFailure::Other(reason) => reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hello_names_the_runner_and_its_version() {
        let spec = RunnerSpec::new("0.1.0");
        assert_eq!(spec.agent(), "mobile-agent-ios-runner 0.1.0");
        assert!(is_expected(&spec, "mobile-agent-ios-runner 0.1.0"));
        assert!(!is_expected(&spec, "mobile-agent-ios-runner 0.0.9"));
        assert!(!is_expected(&spec, "mobile-agent-android 0.1.0"));
        assert!(!is_expected(&spec, "mobile-agent-ios-runner"));
        assert_eq!(
            reported_version("mobile-agent-ios-runner", "mobile-agent-ios-runner 0.0.9"),
            Some("0.0.9")
        );
        assert_eq!(
            reported_version("mobile-agent-ios-runner", "mobile-agent-ios-runnerx 1"),
            None
        );
    }

    #[test]
    fn the_test_managers_failures_say_what_stands_in_the_way() {
        assert_eq!(
            StartFailure::classify("service not found"),
            StartFailure::NoTestManager
        );
        assert!(matches!(
            StartFailure::classify("xctest session timed out after 30.0s"),
            StartFailure::HandshakeUnanswered(_)
        ));
        assert!(matches!(
            StartFailure::classify(
                "untrusted developer certificate — go to Settings → General → VPN & Device Management"
            ),
            StartFailure::BundleNotLoaded(_)
        ));
        assert!(matches!(
            StartFailure::classify("NSError code 103"),
            StartFailure::BundleNotLoaded(_)
        ));
        assert!(matches!(
            StartFailure::classify("test runner did not connect within the timeout"),
            StartFailure::Other(_)
        ));
        assert!(
            StartFailure::BundleNotLoaded("103".into())
                .describe()
                .contains("arm64")
        );
    }
}
