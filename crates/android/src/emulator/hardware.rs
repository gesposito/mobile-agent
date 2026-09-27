//! What the running emulator reports about its virtual hardware, and the frame
//! size to request from it.

use std::collections::BTreeMap;

use super::proto;

/// Size of the emulator's display in pixels, in its natural orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySize {
    pub width: u32,
    pub height: u32,
}

/// The virtual hardware the driver needs to know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    /// From `hw.lcd.width` and `hw.lcd.height`.
    pub display: DisplaySize,
    /// Whether the AVD declares a keyboard (`hw.keyboard=yes`). Without one the
    /// controller's `sendKey` returns OK and the guest never sees the key (S7),
    /// so keys and text go through the agent unless this is true.
    pub keyboard: bool,
}

impl Hardware {
    /// Reads the hardware keys from `(key, value)` entries.
    pub fn from_entries<'a>(
        entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let entries: BTreeMap<&str, &str> = entries.into_iter().collect();
        let pixels = |key: &str| -> Result<u32, String> {
            let value = entries
                .get(key)
                .ok_or_else(|| format!("the emulator reports no {key}"))?;
            match value.trim().parse() {
                Ok(n) if n > 0 => Ok(n),
                _ => Err(format!("{key} is {value:?}, not a positive pixel count")),
            }
        };
        Ok(Self {
            display: DisplaySize {
                width: pixels("hw.lcd.width")?,
                height: pixels("hw.lcd.height")?,
            },
            keyboard: entries
                .get("hw.keyboard")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("yes")),
        })
    }

    /// Reads the hardware from `getStatus`. Emulator 37.1.11 leaves
    /// `platformConfig` empty and reports the display size only in the
    /// deprecated `hardwareConfig` (S7), so a key missing from the former is
    /// taken from the latter.
    #[allow(deprecated)]
    pub fn from_status(status: &proto::EmulatorStatus) -> Result<Self, String> {
        let hardware = status
            .hardware_config
            .iter()
            .flat_map(|list| &list.entry)
            .map(|e| (e.key.as_str(), e.value.as_str()));
        let platform = status
            .platform_config
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.as_str(), v.as_str()));
        // Later entries win when collected, so platformConfig goes last.
        Self::from_entries(hardware.chain(platform))
    }
}

/// The frame size to request for frames `working_width` pixels wide.
///
/// The controller treats a zero in either dimension as "display size", so a
/// width alone does not scale (S7); both are set, the height following the
/// display's aspect ratio and rounded up. A working width of 0, or one at least
/// as wide as the display, requests the display size as `(0, 0)`.
pub fn scaled_size(display: DisplaySize, working_width: u32) -> (u32, u32) {
    if working_width == 0 || working_width >= display.width {
        return (0, 0);
    }
    let height =
        (u64::from(working_width) * u64::from(display.height)).div_ceil(u64::from(display.width));
    (working_width, u32::try_from(height).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXEL_8: DisplaySize = DisplaySize {
        width: 1080,
        height: 2400,
    };

    #[test]
    fn scaling_sets_both_width_and_height() {
        assert_eq!(scaled_size(PIXEL_8, 540), (540, 1200));
        assert_eq!(scaled_size(PIXEL_8, 270), (270, 600));
    }

    #[test]
    fn the_height_keeps_the_aspect_ratio_rounded_up() {
        assert_eq!(scaled_size(PIXEL_8, 500), (500, 1112));
        let tall = DisplaySize {
            width: 1080,
            height: 2340,
        };
        assert_eq!(scaled_size(tall, 540), (540, 1170));
    }

    #[test]
    fn no_scaling_requests_the_display_size_as_zeros() {
        assert_eq!(scaled_size(PIXEL_8, 0), (0, 0));
        assert_eq!(scaled_size(PIXEL_8, 1080), (0, 0));
        assert_eq!(scaled_size(PIXEL_8, 4000), (0, 0));
    }

    #[test]
    fn the_display_size_comes_from_the_lcd_keys() {
        let hw = Hardware::from_entries([
            ("hw.lcd.density", "420"),
            ("hw.lcd.width", "1080"),
            ("hw.lcd.height", "2400"),
            ("hw.keyboard", "no"),
        ])
        .unwrap();
        assert_eq!(hw.display, PIXEL_8);
        assert!(!hw.keyboard);
    }

    #[test]
    fn a_missing_or_zero_display_size_is_an_error() {
        let missing = Hardware::from_entries([("hw.lcd.width", "1080")]).unwrap_err();
        assert!(missing.contains("hw.lcd.height"), "{missing}");
        let zero =
            Hardware::from_entries([("hw.lcd.width", "0"), ("hw.lcd.height", "2400")]).unwrap_err();
        assert!(zero.contains("hw.lcd.width"), "{zero}");
        let garbage = Hardware::from_entries([("hw.lcd.width", "wide"), ("hw.lcd.height", "2400")])
            .unwrap_err();
        assert!(garbage.contains("wide"), "{garbage}");
    }

    #[test]
    fn keys_go_through_the_controller_only_with_a_declared_keyboard() {
        let with = |value: &'static str| {
            Hardware::from_entries([
                ("hw.lcd.width", "1080"),
                ("hw.lcd.height", "2400"),
                ("hw.keyboard", value),
            ])
            .unwrap()
            .keyboard
        };
        assert!(with("yes"));
        assert!(with("YES"));
        assert!(!with("no"));
        assert!(!with(""));
        let absent =
            Hardware::from_entries([("hw.lcd.width", "1080"), ("hw.lcd.height", "2400")]).unwrap();
        assert!(!absent.keyboard);
    }

    #[test]
    #[allow(deprecated)]
    fn the_status_falls_back_to_the_deprecated_hardware_config() {
        let entry = |key: &str, value: &str| proto::Entry {
            key: key.into(),
            value: value.into(),
        };
        let mut status = proto::EmulatorStatus {
            hardware_config: Some(proto::EntryList {
                entry: vec![
                    entry("hw.lcd.width", "1080"),
                    entry("hw.lcd.height", "2400"),
                    entry("hw.keyboard", "no"),
                ],
            }),
            ..Default::default()
        };
        let hw = Hardware::from_status(&status).unwrap();
        assert_eq!(hw.display, PIXEL_8);
        assert!(!hw.keyboard);

        // platformConfig, where a newer emulator fills it, takes precedence.
        status
            .platform_config
            .insert("hw.keyboard".into(), "yes".into());
        assert!(Hardware::from_status(&status).unwrap().keyboard);

        let empty = proto::EmulatorStatus::default();
        assert!(Hardware::from_status(&empty).is_err());
    }
}
