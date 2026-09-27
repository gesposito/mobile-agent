//! Choosing a device's input transport from the host's versions (design D6,
//! spike S3). The host injects the same input service into every Simulator
//! it boots, so the choice depends on the CoreSimulator that vends
//! `SimDevice`, not on the runtime; nothing is probed, because opening one
//! transport to see whether it works can disable the other for the rest of
//! the boot (research note). The fallback is opened only when the chosen
//! transport cannot be reached.

use std::fmt;

use crate::error::{Error, Result};
use crate::model::{TransportChoice, TransportKind};
use crate::version::Version;

/// The first CoreSimulator whose guests run dtuhidd.
pub const DTUHIDD_SINCE: &str = "1155.4";

/// The host's versions that input depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostVersions {
    /// `CFBundleVersion` of the CoreSimulator that vends `SimDevice`.
    pub core_simulator: Option<String>,
    /// The selected Xcode's version and build, such as "27.0 (27A266a)".
    pub xcode: Option<String>,
}

impl fmt::Display for HostVersions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unknown = "unknown".to_string();
        write!(
            f,
            "CoreSimulator {}, Xcode {}",
            self.core_simulator.as_ref().unwrap_or(&unknown),
            self.xcode.as_ref().unwrap_or(&unknown)
        )
    }
}

/// The transports to try, in order, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub transports: Vec<TransportKind>,
    pub reason: String,
}

/// The plan for a host: dtuhidd from CoreSimulator 1155.4, with Indigo
/// behind it only for a dtuhidd that cannot be reached; Indigo alone before.
/// An unreadable version is taken as current.
pub fn plan(host: &HostVersions, choice: TransportChoice) -> Plan {
    let _ = (host, choice);
    unimplemented!("task 6.5: transport selection")
}

/// Opens the first transport of `plan` that `open` can open, skipping the
/// ones in `failed`. When none opens, the error names the host's versions,
/// the runtime and every transport tried with why it failed.
pub fn open_first<T>(
    plan: &Plan,
    failed: &[(TransportKind, String)],
    host: &HostVersions,
    runtime: &str,
    open: impl FnMut(TransportKind) -> Result<T>,
) -> Result<T> {
    let _ = (plan, failed, host, runtime, open);
    let _ = Version::parse(DTUHIDD_SINCE);
    let _ = Error::PlatformUnsupported;
    unimplemented!("task 6.5: transport selection")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn host(core_simulator: &str) -> HostVersions {
        HostVersions {
            core_simulator: Some(core_simulator.into()),
            xcode: Some("27.0 (27A266a)".into()),
        }
    }

    #[test]
    fn the_host_core_simulator_selects_dtuhidd_from_1155_4() {
        let both = vec![TransportKind::Dtuhid, TransportKind::Indigo];
        assert_eq!(
            plan(&host("1171.7"), TransportChoice::Auto).transports,
            both
        );
        assert_eq!(
            plan(&host("1155.4"), TransportChoice::Auto).transports,
            both
        );
        assert_eq!(
            plan(&host("1155.10"), TransportChoice::Auto).transports,
            both
        );
        assert_eq!(
            plan(&host("1155.3"), TransportChoice::Auto).transports,
            vec![TransportKind::Indigo]
        );
        let reason = plan(&host("1171.7"), TransportChoice::Auto).reason;
        assert!(
            reason.contains("1171.7") && reason.contains("1155.4"),
            "{reason}"
        );
    }

    #[test]
    fn an_unreadable_version_is_taken_as_current() {
        let unknown = HostVersions::default();
        assert_eq!(
            plan(&unknown, TransportChoice::Auto).transports,
            vec![TransportKind::Dtuhid, TransportKind::Indigo]
        );
    }

    #[test]
    fn a_forced_transport_is_the_only_one_tried() {
        for kind in [TransportKind::Dtuhid, TransportKind::Indigo] {
            let forced = plan(&host("1171.7"), TransportChoice::Only(kind));
            assert_eq!(forced.transports, vec![kind]);
            assert!(forced.reason.contains("forced"), "{}", forced.reason);
        }
    }

    #[test]
    fn the_fallback_is_opened_only_when_the_first_fails() {
        let plan = plan(&host("1171.7"), TransportChoice::Auto);
        let opened = RefCell::new(Vec::new());
        let transport = open_first(&plan, &[], &host("1171.7"), "iOS 26.5 (23F77)", |kind| {
            opened.borrow_mut().push(kind);
            Ok(kind)
        })
        .expect("dtuhidd opens");
        assert_eq!(transport, TransportKind::Dtuhid);
        assert_eq!(
            *opened.borrow(),
            vec![TransportKind::Dtuhid],
            "nothing probed"
        );

        opened.borrow_mut().clear();
        let transport = open_first(&plan, &[], &host("1171.7"), "iOS 26.5 (23F77)", |kind| {
            opened.borrow_mut().push(kind);
            match kind {
                TransportKind::Dtuhid => Err(Error::Unsupported {
                    what: "the digitizer service".into(),
                }),
                TransportKind::Indigo => Ok(kind),
            }
        })
        .expect("Indigo stands in");
        assert_eq!(transport, TransportKind::Indigo);
        assert_eq!(
            *opened.borrow(),
            vec![TransportKind::Dtuhid, TransportKind::Indigo]
        );
    }

    #[test]
    fn a_transport_that_failed_on_the_target_is_skipped() {
        let plan = plan(&host("1171.7"), TransportChoice::Auto);
        let failed = [(TransportKind::Dtuhid, "no activation reply".to_string())];
        let transport =
            open_first(&plan, &failed, &host("1171.7"), "iOS 26.5", Ok).expect("Indigo opens");
        assert_eq!(transport, TransportKind::Indigo);
    }

    #[test]
    fn the_unsupported_combination_names_every_version_and_transport_tried() {
        let plan = plan(&host("1171.7"), TransportChoice::Auto);
        let failed = [(
            TransportKind::Dtuhid,
            "dtuhidd did not answer the activation probe in 5 attempts".to_string(),
        )];
        let error = open_first::<()>(&plan, &failed, &host("1171.7"), "iOS 26.5 (23F77)", |_| {
            Err(Error::Load {
                what: "SimDeviceLegacyHIDClient".into(),
                reason: "SimulatorKit registers no such class".into(),
            })
        })
        .expect_err("nothing works");
        assert_eq!(error.code(), "no_input_transport");
        let message = error.to_string();
        for part in [
            "CoreSimulator 1171.7",
            "Xcode 27.0 (27A266a)",
            "iOS 26.5 (23F77)",
            "dtuhidd",
            "did not answer the activation probe",
            "Indigo",
            "SimDeviceLegacyHIDClient",
        ] {
            assert!(
                message.contains(part),
                "{part:?} is missing from: {message}"
            );
        }
    }
}
