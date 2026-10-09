//! The developer services the driver needs over the CoreDevice tunnel, by
//! their RSD names (design D18; spike S9 listed each on iOS 26.6.2). Without
//! the developer disk image the handshake lists fewer services: 61 instead
//! of 82 in spike S10, the test manager and the HID services among those
//! gone.

/// A service the driver needs, and what for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needed {
    pub name: &'static str,
    pub for_what: &'static str,
}

/// The test manager, through which the runner starts (task 3.4).
pub const TEST_MANAGER: &str = "com.apple.dt.testmanagerd.remote";
/// DVT's service hub: process control, the process list, screenshots.
pub const DVT: &str = "com.apple.instruments.dtservicehub";
/// CoreDevice's app service: the second lifecycle path.
pub const APP_SERVICE: &str = "com.apple.coredevice.appservice";
/// CoreDevice's screen capture: the second screenshot connection (task 3.5).
pub const SCREEN_CAPTURE: &str = "com.apple.coredevice.screencaptureservice";
/// `dtuhidd`'s Indigo events: hardware buttons and keys (task 3.8).
pub const HID_INDIGO: &str = "com.apple.coredevice.hid.indigo";
/// `dtuhidd`'s HID reports: touches (task 3.8).
pub const HID_UNIVERSAL: &str = "com.apple.coredevice.hid.universalhidservice";

/// Every service the driver needs, in the order the tasks that use them
/// come.
pub const NEEDED: [Needed; 6] = [
    Needed {
        name: TEST_MANAGER,
        for_what: "the runner's session through the test manager",
    },
    Needed {
        name: DVT,
        for_what: "DVT's process control, process list and screenshots",
    },
    Needed {
        name: APP_SERVICE,
        for_what: "CoreDevice's app service, the second lifecycle path",
    },
    Needed {
        name: SCREEN_CAPTURE,
        for_what: "CoreDevice's screenshots, the second frame connection",
    },
    Needed {
        name: HID_INDIGO,
        for_what: "dtuhidd's Indigo events: hardware buttons and keys",
    },
    Needed {
        name: HID_UNIVERSAL,
        for_what: "dtuhidd's HID reports: touches",
    },
];

/// The needed services that `listed` lacks, in [`NEEDED`]'s order.
pub fn missing<'a>(listed: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let _ = listed;
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_needed_service_listed_leaves_none_missing() {
        let listed: Vec<&str> = NEEDED.iter().map(|needed| needed.name).collect();
        assert!(missing(listed).is_empty());
    }

    #[test]
    fn the_needed_services_not_listed_are_named_in_order() {
        let listed = [
            DVT,
            APP_SERVICE,
            SCREEN_CAPTURE,
            "com.apple.afc.shim.remote",
        ];
        assert_eq!(
            missing(listed),
            vec![TEST_MANAGER, HID_INDIGO, HID_UNIVERSAL]
        );
        assert_eq!(missing([]), NEEDED.map(|needed| needed.name).to_vec());
    }
}
