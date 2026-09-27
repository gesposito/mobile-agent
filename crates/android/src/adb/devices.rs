//! Target listing: the adb server's answer to `host:devices-l`, the text that
//! `adb devices -l` prints, with every connection state kept, and the targets
//! built from it.

use std::collections::BTreeMap;

/// Connection state of a listed target, as the adb server names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Online and authorized.
    Device,
    Offline,
    /// USB debugging has not been accepted on the device.
    Unauthorized,
    Authorizing,
    Connecting,
    /// `no permissions (...)`, with the server's explanation kept verbatim.
    NoPermissions(String),
    /// Any other state (bootloader, recovery, rescue, sideload, host,
    /// detached), verbatim.
    Other(String),
}

impl State {
    /// What the user can do about this state; `None` for [`State::Device`].
    pub fn advice(&self) -> Option<String> {
        let text = match self {
            State::Device => return None,
            State::Unauthorized => {
                "USB debugging is not authorized for this computer: unlock the device and \
                 accept the \"Allow USB debugging?\" prompt on the device, ticking \"Always \
                 allow from this computer\". If no prompt appears, choose \"Revoke USB \
                 debugging authorizations\" in Developer options and reconnect the cable."
                    .to_owned()
            }
            State::Offline => "adb has lost contact with the device: reconnect the cable, or \
                               restart the emulator; `adb reconnect offline` retries without \
                               unplugging."
                .to_owned(),
            State::Authorizing => {
                "the device is still authorizing this computer; list targets again in a moment."
                    .to_owned()
            }
            State::Connecting => {
                "adb is still connecting to the device; list targets again in a moment.".to_owned()
            }
            State::NoPermissions(detail) => format!(
                "the adb server may not open this USB device ({detail}): install the udev \
                 rules for Android devices, add your user to the plugdev group, then reconnect \
                 the device and restart the adb server."
            ),
            State::Other(mode) => format!(
                "the device is in {mode} mode, not running Android: reboot it into Android to \
                 automate it."
            ),
        };
        Some(text)
    }
}

/// One line of the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub serial: String,
    pub state: State,
    /// The long form's trailing `key:value` fields: usb, product, model, device
    /// and transport_id.
    pub props: BTreeMap<String, String>,
}

impl DeviceEntry {
    /// The server's transport id, present in the long form only.
    pub fn transport_id(&self) -> Option<u32> {
        self.props.get("transport_id")?.parse().ok()
    }

    /// The model the server reports, with spaces replaced by underscores.
    pub fn model(&self) -> Option<&str> {
        self.props.get("model").map(String::as_str)
    }
}

/// Parses a `host:devices` or `host:devices-l` answer. Header, blank and
/// `* daemon ...` lines are skipped, as is any line without a state, so one odd
/// line never hides the other targets.
pub fn parse_devices(listing: &str) -> Vec<DeviceEntry> {
    listing.lines().filter_map(parse_line).collect()
}

/// Keys of the long form's trailing fields. The server appends them after the
/// state, ending with transport_id, so they are taken from the end of the line.
const PROP_KEYS: [&str; 5] = ["usb", "product", "model", "device", "transport_id"];

/// What the server prints for a target whose serial is empty.
const NO_SERIAL: &str = "(no serial number)";

fn parse_line(line: &str) -> Option<DeviceEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
        return None;
    }
    let (serial, rest) = match line.strip_prefix(NO_SERIAL) {
        Some(rest) => (NO_SERIAL, rest),
        None => line.split_once(char::is_whitespace)?,
    };
    let mut tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut props = BTreeMap::new();
    while let Some((key, value)) = tokens.last().and_then(|t| t.split_once(':')) {
        if !PROP_KEYS.contains(&key) {
            break;
        }
        props.insert(key.to_owned(), value.to_owned());
        tokens.pop();
    }
    if tokens.is_empty() {
        return None;
    }
    // A state may contain spaces: "no permissions (...); see [...]".
    let state = match tokens.join(" ").as_str() {
        "device" => State::Device,
        "offline" => State::Offline,
        "unauthorized" => State::Unauthorized,
        "authorizing" => State::Authorizing,
        "connecting" => State::Connecting,
        text if text.starts_with("no permissions") => State::NoPermissions(text.to_owned()),
        text => State::Other(text.to_owned()),
    };
    Some(DeviceEntry {
        serial: serial.to_owned(),
        state,
        props,
    })
}

/// The shell command [`crate::adb::AdbServer::targets`] runs on each online
/// target, one property per output line in this order.
pub const PROP_QUERY: &str = "getprop ro.build.version.sdk; getprop ro.boot.qemu; \
                              getprop ro.kernel.qemu; getprop ro.product.model";

/// A listed target with what the driver reports about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub serial: String,
    pub state: State,
    /// `ro.product.model` for an online target, else the listing's model.
    pub model: Option<String>,
    /// `ro.build.version.sdk`; read from online targets only.
    pub api_level: Option<u32>,
    /// From `ro.boot.qemu` or `ro.kernel.qemu` when the target is online, else
    /// from an `emulator-` serial.
    pub emulator: bool,
    pub transport_id: Option<u32>,
}

impl Target {
    /// A target from its listing entry and, when it could be read, the output
    /// of [`PROP_QUERY`] on it.
    pub fn new(entry: DeviceEntry, props: Option<&str>) -> Self {
        let lines: Vec<&str> = props
            .map(|p| p.lines().map(str::trim).collect())
            .unwrap_or_default();
        let prop = |i: usize| lines.get(i).copied().filter(|v| !v.is_empty());
        let api_level = prop(0).and_then(|v| v.parse().ok());
        let qemu = prop(1) == Some("1") || prop(2) == Some("1");
        let model = prop(3)
            .map(str::to_owned)
            .or_else(|| entry.model().map(str::to_owned));
        Target {
            emulator: qemu || entry.serial.starts_with("emulator-"),
            transport_id: entry.transport_id(),
            serial: entry.serial,
            state: entry.state,
            model,
            api_level,
        }
    }

    /// What the user can do to make the target usable; `None` when it is.
    pub fn advice(&self) -> Option<String> {
        self.state.advice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMULATOR: &str = "emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1";

    fn one(listing: &str) -> DeviceEntry {
        let mut all = parse_devices(listing);
        assert_eq!(
            all.len(),
            1,
            "expected one entry in {listing:?}, got {all:?}"
        );
        all.remove(0)
    }

    #[test]
    fn parses_the_booted_emulator_line() {
        let e = one(EMULATOR);
        assert_eq!(e.serial, "emulator-5554");
        assert_eq!(e.state, State::Device);
        assert_eq!(e.props["product"], "sdk_gphone64_arm64");
        assert_eq!(e.model(), Some("sdk_gphone64_arm64"));
        assert_eq!(e.props["device"], "emu64a");
        assert_eq!(e.transport_id(), Some(1));
    }

    #[test]
    fn skips_header_blank_and_daemon_status_lines() {
        let listing = format!(
            "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\n{EMULATOR}\n\n"
        );
        assert_eq!(one(&listing).serial, "emulator-5554");
    }

    #[test]
    fn keeps_unauthorized_offline_and_authorizing_targets() {
        let listing = "List of devices attached\n\
                       R58M123ABC             unauthorized usb:1-1 transport_id:2\n\
                       emulator-5556          offline transport_id:3\n\
                       emulator-5558          authorizing transport_id:7\n\
                       emulator-5560          connecting transport_id:8\n";
        let all = parse_devices(listing);
        let states: Vec<_> = all
            .iter()
            .map(|e| (e.serial.as_str(), e.state.clone()))
            .collect();
        assert_eq!(
            states,
            [
                ("R58M123ABC", State::Unauthorized),
                ("emulator-5556", State::Offline),
                ("emulator-5558", State::Authorizing),
                ("emulator-5560", State::Connecting),
            ]
        );
        assert_eq!(all[0].props["usb"], "1-1");
        assert_eq!(all[0].model(), None);
        let ids: Vec<_> = all.iter().map(DeviceEntry::transport_id).collect();
        assert_eq!(ids, [Some(2), Some(3), Some(7), Some(8)]);
    }

    #[test]
    fn keeps_the_no_permissions_explanation() {
        let e = one(
            "0123456789ABCDEF       no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:1-2 transport_id:4",
        );
        assert_eq!(e.serial, "0123456789ABCDEF");
        assert_eq!(
            e.state,
            State::NoPermissions(
                "no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html]".into()
            )
        );
        assert_eq!(e.props["usb"], "1-2");
        assert_eq!(e.transport_id(), Some(4));
    }

    #[test]
    fn parses_network_and_mdns_serials() {
        let listing = "192.168.1.20:5555      device product:husky model:Pixel_8_Pro device:husky transport_id:5\n\
                       adb-35091FDH2000J2-UfqGmB._adb-tls-connect._tcp device product:shiba model:Pixel_8 device:shiba transport_id:6\n";
        let all = parse_devices(listing);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].serial, "192.168.1.20:5555");
        assert_eq!(all[0].model(), Some("Pixel_8_Pro"));
        assert_eq!(
            all[1].serial,
            "adb-35091FDH2000J2-UfqGmB._adb-tls-connect._tcp"
        );
        assert_eq!(all[1].state, State::Device);
    }

    #[test]
    fn parses_the_short_tab_separated_form() {
        let all = parse_devices(
            "List of devices attached\nemulator-5554\tdevice\nR58M123ABC\tunauthorized\n\n",
        );
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].serial, "emulator-5554");
        assert_eq!(all[0].state, State::Device);
        assert!(all[0].props.is_empty());
        assert_eq!(all[0].transport_id(), None);
        assert_eq!(all[1].state, State::Unauthorized);
    }

    #[test]
    fn keeps_the_no_serial_placeholder() {
        let e = one("(no serial number)     unauthorized usb:1-3 transport_id:8");
        assert_eq!(e.serial, "(no serial number)");
        assert_eq!(e.state, State::Unauthorized);
        assert_eq!(e.transport_id(), Some(8));
    }

    #[test]
    fn other_states_are_kept_verbatim() {
        assert_eq!(
            one("ZY22ABCDEF             recovery usb:1-4 transport_id:9").state,
            State::Other("recovery".into())
        );
    }

    #[test]
    fn no_targets_is_an_empty_list() {
        assert!(parse_devices("List of devices attached\n\n").is_empty());
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn a_line_without_a_state_is_skipped_and_the_rest_survive() {
        let listing = format!("garbage\n{EMULATOR}\n");
        assert_eq!(one(&listing).serial, "emulator-5554");
    }

    #[test]
    fn crlf_line_endings_are_accepted() {
        let e = one(&format!("List of devices attached\r\n{EMULATOR}\r\n\r\n"));
        assert_eq!(e.transport_id(), Some(1));
        assert_eq!(e.props["device"], "emu64a");
    }

    #[test]
    fn unauthorized_advice_says_to_accept_the_prompt_on_the_device() {
        let advice = State::Unauthorized.advice().unwrap();
        assert!(advice.contains("Allow USB debugging"), "{advice}");
        assert!(advice.contains("accept"), "{advice}");
        assert!(advice.contains("on the device"), "{advice}");
    }

    #[test]
    fn every_unusable_state_has_advice_and_an_online_device_has_none() {
        assert_eq!(State::Device.advice(), None);
        for state in [
            State::Offline,
            State::Authorizing,
            State::Connecting,
            State::NoPermissions("no permissions (x)".into()),
            State::Other("bootloader".into()),
        ] {
            assert!(state.advice().is_some(), "{state:?} has no advice");
        }
        assert!(State::Offline.advice().unwrap().contains("reconnect"));
        assert!(
            State::NoPermissions("no permissions (missing udev rules?)".into())
                .advice()
                .unwrap()
                .contains("missing udev rules?")
        );
        assert!(
            State::Other("recovery".into())
                .advice()
                .unwrap()
                .contains("recovery")
        );
    }

    #[test]
    fn an_online_target_takes_its_level_model_and_emulator_flag_from_getprop() {
        let t = Target::new(one(EMULATOR), Some("36\n1\n\nsdk_gphone64_arm64\n"));
        assert_eq!(t.api_level, Some(36));
        assert!(t.emulator);
        assert_eq!(t.model.as_deref(), Some("sdk_gphone64_arm64"));
        assert_eq!(t.transport_id, Some(1));
        assert_eq!(t.advice(), None);

        let phone =
            one("R58M123ABC device product:husky model:Pixel_8_Pro device:husky transport_id:5");
        let t = Target::new(phone, Some("35\n\n\nPixel 8 Pro\n"));
        assert_eq!(t.api_level, Some(35));
        assert!(!t.emulator);
        assert_eq!(t.model.as_deref(), Some("Pixel 8 Pro"));
    }

    #[test]
    fn older_emulators_are_recognised_by_ro_kernel_qemu() {
        let t = Target::new(
            one("emulator-5554 device transport_id:1"),
            Some("29\n\n1\nAndroid SDK built for x86\n"),
        );
        assert!(t.emulator);
        assert_eq!(t.api_level, Some(29));
    }

    #[test]
    fn a_target_without_properties_keeps_the_listing_facts() {
        let t = Target::new(one("emulator-5556 offline transport_id:3"), None);
        assert_eq!(t.state, State::Offline);
        assert_eq!(t.api_level, None);
        assert!(t.emulator, "an emulator- serial marks an emulator");
        assert_eq!(t.transport_id, Some(3));
        let t = Target::new(one(EMULATOR), None);
        assert_eq!(t.model.as_deref(), Some("sdk_gphone64_arm64"));
        let t = Target::new(one(EMULATOR), Some("garbage"));
        assert_eq!(t.api_level, None);
    }
}
