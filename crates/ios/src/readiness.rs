//! What stands between a device and a session (spec `ios-device-driver`,
//! "Device discovery and readiness"): each state names the step the person
//! at the device or the Mac takes, and a command on a device that is not
//! ready fails with the state's code rather than waiting.
//!
//! The states come in the order a person makes a device ready: trust the
//! Mac, turn on Developer Mode, have the developer disk image mounted,
//! unlock the device, have a signing identity for the runner. A device is
//! reported in the first state it has not passed. An operation needs a
//! prefix of that order ([`Need`]), so it fails only on the states it needs:
//! a launch needs the device unlocked but not the signing identity.

use std::fmt;

use crate::compat;
use crate::identity::IdentityCheck;
use crate::model::{DiskImage, Probe, TEAM_VARIABLE, Trust};

/// What an operation needs of a device, in the order a person makes a device
/// ready; each level needs the ones before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Need {
    /// A lockdown session with the host's pairing record, on a supported iOS.
    Trust,
    /// Developer Mode on, which a development-signed app needs to install
    /// and run.
    DeveloperMode,
    /// The developer disk image mounted: the developer services over the
    /// tunnel, such as DVT's process control and the test manager.
    DiskImage,
    /// The device unlocked: launching an app (design D19).
    Unlocked,
    /// A signing identity for the runner on this Mac: the runner's session.
    Session,
}

/// What stands between a device and a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// The device runs an iOS outside the range the driver supports.
    UnsupportedIos {
        version: String,
    },
    /// No lockdown session: the Mac is not paired with the device, or the
    /// device refused its pairing record; the reason.
    NotTrusted {
        reason: String,
    },
    DeveloperModeOff,
    DiskImageUnmounted,
    Locked,
    /// No Apple Development identity for the runner's team on this Mac; the
    /// reason.
    NoSigningIdentity {
        reason: String,
    },
}

impl Readiness {
    /// A stable name for the state, which a command on a device in it fails
    /// with.
    pub fn code(&self) -> &'static str {
        match self {
            Readiness::Ready => "ready",
            Readiness::UnsupportedIos { .. } => "unsupported_ios",
            Readiness::NotTrusted { .. } => "not_trusted",
            Readiness::DeveloperModeOff => "developer_mode_off",
            Readiness::DiskImageUnmounted => "disk_image_unmounted",
            Readiness::Locked => "device_locked",
            Readiness::NoSigningIdentity { .. } => "no_signing_identity",
        }
    }

    /// Whether nothing stands between the device and a session.
    pub fn is_ready(&self) -> bool {
        *self == Readiness::Ready
    }

    /// What the state is and the step that leaves it.
    pub fn message(&self) -> String {
        match self {
            Readiness::Ready => "ready".to_owned(),
            Readiness::UnsupportedIos { version } => format!(
                "it runs iOS {version}, outside the versions this driver supports, {}; see {}",
                compat::supported_range(),
                compat::NOTES
            ),
            Readiness::NotTrusted { reason } => format!(
                "this Mac is not trusted by it ({reason}): unlock it, connect it over USB and tap \
                 Trust when it asks whether to trust this computer, then enter its passcode; if it \
                 does not ask, opening Finder with the device connected starts the pairing"
            ),
            Readiness::DeveloperModeOff => "Developer Mode is off: on the device open Settings > \
                 Privacy & Security > Developer Mode, turn it on, let it restart, then confirm \
                 with the passcode"
                .to_owned(),
            Readiness::DiskImageUnmounted => "the developer disk image is not mounted, as after \
                 every restart: connecting mounts the selected Xcode's, which needs this Mac \
                 online for Apple's signing server; `xcrun devicectl device info ddiServices \
                 --device <identifier>` mounts it by hand"
                .to_owned(),
            Readiness::Locked => "it is locked: unlock it, and set Settings > Display & \
                 Brightness > Auto-Lock to Never for long runs"
                .to_owned(),
            Readiness::NoSigningIdentity { reason } => format!(
                "{reason}; the runner is signed with an Apple Development identity of the team \
                 {TEAM_VARIABLE} names: create the development certificate in Xcode (Settings > \
                 Accounts, Manage Certificates) with the team's account, and export \
                 {TEAM_VARIABLE} with the team's identifier"
            ),
        }
    }
}

impl fmt::Display for Readiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// The first state among those `need` covers that stands between the device
/// and the operation, given what the probe read, the device's iOS version and
/// the host's signing identity.
pub fn assess(
    ios_version: Option<&str>,
    probe: &Probe,
    identity: &IdentityCheck,
    need: Need,
) -> Result<(), Readiness> {
    // Lockdown gives the version without a session, so an unsupported iOS is
    // told before anything else; a version it did not give leaves the
    // trust check to speak.
    if let Some(version) = ios_version
        && !compat::supported_ios(version)
    {
        return Err(Readiness::UnsupportedIos {
            version: version.to_owned(),
        });
    }
    match &probe.trust {
        Trust::Trusted => {}
        Trust::NotPaired => {
            return Err(Readiness::NotTrusted {
                reason: "usbmuxd holds no pairing record for it".to_owned(),
            });
        }
        Trust::Refused => {
            return Err(Readiness::NotTrusted {
                reason: "it refused this Mac's pairing record".to_owned(),
            });
        }
        Trust::Unreachable(reason) => {
            return Err(Readiness::NotTrusted {
                reason: format!("lockdown did not start a session: {reason}"),
            });
        }
    }
    if need >= Need::DeveloperMode && probe.developer_mode != Some(true) {
        return Err(Readiness::DeveloperModeOff);
    }
    if need >= Need::DiskImage && !matches!(probe.disk_image, Some(DiskImage::Mounted { .. })) {
        return Err(Readiness::DiskImageUnmounted);
    }
    if need >= Need::Unlocked && probe.password_protected != Some(false) {
        return Err(Readiness::Locked);
    }
    if need >= Need::Session && !identity.ok() {
        return Err(Readiness::NoSigningIdentity {
            reason: identity.reason(),
        });
    }
    Ok(())
}

/// The device's state for a session: the first state that stands between it
/// and the runner, or `Ready`.
pub fn readiness(ios_version: Option<&str>, probe: &Probe, identity: &IdentityCheck) -> Readiness {
    match assess(ios_version, probe, identity, Need::Session) {
        Ok(()) => Readiness::Ready,
        Err(state) => state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiskImage, Trust};

    fn ready_probe() -> Probe {
        Probe {
            trust: Trust::Trusted,
            developer_mode: Some(true),
            disk_image: Some(DiskImage::Mounted {
                image_type: "Personalized".into(),
                mount_path: Some("/System/Developer".into()),
            }),
            password_protected: Some(false),
        }
    }

    fn identity() -> IdentityCheck {
        IdentityCheck::Found {
            name: "Apple Development: placeholder-developer (PLACEHOLD0)".into(),
            team: "TEAMPLACE1".into(),
        }
    }

    const IOS: Option<&str> = Some("26.6.2");

    #[test]
    fn a_trusted_unlocked_device_with_developer_mode_and_the_image_is_ready() {
        assert_eq!(
            readiness(IOS, &ready_probe(), &identity()),
            Readiness::Ready
        );
        assert!(Readiness::Ready.is_ready());
        assert_eq!(Readiness::Ready.code(), "ready");
    }

    #[test]
    fn each_unready_state_is_the_first_check_the_device_has_not_passed() {
        let untrusted = Probe::untrusted(Trust::NotPaired);
        assert!(matches!(
            readiness(IOS, &untrusted, &identity()),
            Readiness::NotTrusted { .. }
        ));
        let refused = Probe::untrusted(Trust::Refused);
        assert!(matches!(
            readiness(IOS, &refused, &identity()),
            Readiness::NotTrusted { .. }
        ));
        let developer_mode_off = Probe {
            developer_mode: Some(false),
            disk_image: None,
            ..ready_probe()
        };
        assert_eq!(
            readiness(IOS, &developer_mode_off, &identity()),
            Readiness::DeveloperModeOff
        );
        let unmounted = Probe {
            disk_image: Some(DiskImage::NotMounted),
            ..ready_probe()
        };
        assert_eq!(
            readiness(IOS, &unmounted, &identity()),
            Readiness::DiskImageUnmounted
        );
        let unreadable = Probe {
            disk_image: Some(DiskImage::Unreadable(
                "the image mounter did not answer".into(),
            )),
            ..ready_probe()
        };
        assert_eq!(
            readiness(IOS, &unreadable, &identity()),
            Readiness::DiskImageUnmounted
        );
        let locked = Probe {
            password_protected: Some(true),
            ..ready_probe()
        };
        assert_eq!(readiness(IOS, &locked, &identity()), Readiness::Locked);
        assert!(matches!(
            readiness(IOS, &ready_probe(), &IdentityCheck::NoTeam),
            Readiness::NoSigningIdentity { .. }
        ));
    }

    #[test]
    fn an_earlier_state_wins_over_a_later_one() {
        let everything_wrong = Probe {
            developer_mode: Some(false),
            disk_image: Some(DiskImage::NotMounted),
            password_protected: Some(true),
            ..ready_probe()
        };
        assert_eq!(
            readiness(IOS, &everything_wrong, &IdentityCheck::NoTeam),
            Readiness::DeveloperModeOff
        );
        let locked_without_identity = Probe {
            password_protected: Some(true),
            ..ready_probe()
        };
        assert_eq!(
            readiness(IOS, &locked_without_identity, &IdentityCheck::NoTeam),
            Readiness::Locked
        );
    }

    #[test]
    fn a_value_that_could_not_be_read_does_not_pass_its_check() {
        let unread = Probe {
            developer_mode: None,
            ..ready_probe()
        };
        assert_eq!(
            readiness(IOS, &unread, &identity()),
            Readiness::DeveloperModeOff
        );
        let lock_unread = Probe {
            password_protected: None,
            ..ready_probe()
        };
        assert_eq!(readiness(IOS, &lock_unread, &identity()), Readiness::Locked);
    }

    #[test]
    fn an_ios_outside_the_supported_range_is_refused_naming_the_version() {
        for version in ["16.7.10", "17.3.1", "27.0", "28.1"] {
            let state = readiness(Some(version), &ready_probe(), &identity());
            assert_eq!(
                state,
                Readiness::UnsupportedIos {
                    version: version.into()
                },
                "{version}"
            );
            assert_eq!(state.code(), "unsupported_ios");
            assert!(state.message().contains(version), "{}", state.message());
        }
        for version in ["17.4", "18.2", "26.6.2", "26.7"] {
            assert_eq!(
                readiness(Some(version), &ready_probe(), &identity()),
                Readiness::Ready,
                "{version}"
            );
        }
        // Lockdown gives the version without a session; one it did not
        // give does not refuse the device, the trust check speaks for it.
        assert!(matches!(
            readiness(None, &Probe::untrusted(Trust::NotPaired), &identity()),
            Readiness::NotTrusted { .. }
        ));
    }

    #[test]
    fn an_operation_fails_only_on_the_states_it_needs() {
        let locked = Probe {
            password_protected: Some(true),
            ..ready_probe()
        };
        assert_eq!(
            assess(IOS, &locked, &IdentityCheck::NoTeam, Need::DiskImage),
            Ok(())
        );
        assert_eq!(
            assess(IOS, &locked, &IdentityCheck::NoTeam, Need::Unlocked),
            Err(Readiness::Locked)
        );
        assert_eq!(
            assess(IOS, &ready_probe(), &IdentityCheck::NoTeam, Need::Unlocked),
            Ok(())
        );
        assert!(matches!(
            assess(IOS, &ready_probe(), &IdentityCheck::NoTeam, Need::Session),
            Err(Readiness::NoSigningIdentity { .. })
        ));
        let unmounted = Probe {
            disk_image: Some(DiskImage::NotMounted),
            ..ready_probe()
        };
        assert_eq!(
            assess(IOS, &unmounted, &identity(), Need::DeveloperMode),
            Ok(())
        );
        assert_eq!(
            assess(IOS, &unmounted, &identity(), Need::DiskImage),
            Err(Readiness::DiskImageUnmounted)
        );
        let off = Probe {
            developer_mode: Some(false),
            disk_image: None,
            ..ready_probe()
        };
        assert_eq!(assess(IOS, &off, &identity(), Need::Trust), Ok(()));
        assert_eq!(
            assess(IOS, &off, &identity(), Need::DeveloperMode),
            Err(Readiness::DeveloperModeOff)
        );
        assert!(matches!(
            assess(
                IOS,
                &Probe::untrusted(Trust::Refused),
                &identity(),
                Need::Trust
            ),
            Err(Readiness::NotTrusted { .. })
        ));
    }

    #[test]
    fn every_unready_state_has_a_code_and_a_message_naming_its_step() {
        let cases = [
            (
                Readiness::NotTrusted {
                    reason: "usbmuxd holds no pairing record for it".into(),
                },
                "not_trusted",
                &["Trust", "passcode", "no pairing record"][..],
            ),
            (
                Readiness::DeveloperModeOff,
                "developer_mode_off",
                &["Settings > Privacy & Security > Developer Mode", "restart"][..],
            ),
            (
                Readiness::DiskImageUnmounted,
                "disk_image_unmounted",
                &["developer disk image", "signing server", "devicectl"][..],
            ),
            (
                Readiness::Locked,
                "device_locked",
                &["unlock it", "Auto-Lock"][..],
            ),
            (
                Readiness::NoSigningIdentity {
                    reason: "MOBILE_AGENT_IOS_TEAM is not set".into(),
                },
                "no_signing_identity",
                &["Apple Development", "MOBILE_AGENT_IOS_TEAM", "certificate"][..],
            ),
            (
                Readiness::UnsupportedIos {
                    version: "16.7.10".into(),
                },
                "unsupported_ios",
                &["16.7.10", "17.4", "26.6.2", "README"][..],
            ),
        ];
        for (state, code, parts) in cases {
            assert_eq!(state.code(), code);
            assert!(!state.is_ready(), "{code}");
            let message = state.message();
            for part in parts {
                assert!(
                    message.contains(part),
                    "{part:?} is missing from: {message}"
                );
            }
        }
    }
}
