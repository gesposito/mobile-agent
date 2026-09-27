//! Key names, as agent-browser's `press` takes them, to platform key codes.
//!
//! On Android a code is `KeyEvent.KEYCODE_*` and the meta holds
//! `META_*_ON` bits. On iOS a code is a USB HID usage as
//! `(page << 16) | usage`, keyboard keys on page 0x07 and buttons such as
//! Home on the consumer page 0x0C, and the meta holds the HID modifier byte
//! (left control 0x01, shift 0x02, alt 0x04, GUI 0x08). A number of two or
//! more digits passes through as a code; a single digit is that digit's key.

use crate::driver::Platform;
use crate::wire::Failure;

/// The code of keys the daemon cannot name.
pub(crate) const UNKNOWN_KEY: &str = "unknown_key";

/// A key and its modifiers, in a platform's codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyCode {
    pub code: u32,
    pub meta: Option<u32>,
}

const KEYBOARD: u32 = 0x07 << 16;
const CONSUMER: u32 = 0x0C << 16;

/// The codes of `name` (case-insensitive) on each platform.
fn named(name: &str) -> Option<(Option<u32>, Option<u32>)> {
    let codes = match name {
        "enter" | "return" => (Some(66), Some(KEYBOARD | 0x28)),
        "tab" => (Some(61), Some(KEYBOARD | 0x2B)),
        "escape" | "esc" => (Some(111), Some(KEYBOARD | 0x29)),
        "backspace" => (Some(67), Some(KEYBOARD | 0x2A)),
        "delete" | "del" => (Some(112), Some(KEYBOARD | 0x4C)),
        "space" => (Some(62), Some(KEYBOARD | 0x2C)),
        "arrowup" | "up" => (Some(19), Some(KEYBOARD | 0x52)),
        "arrowdown" | "down" => (Some(20), Some(KEYBOARD | 0x51)),
        "arrowleft" | "left" => (Some(21), Some(KEYBOARD | 0x50)),
        "arrowright" | "right" => (Some(22), Some(KEYBOARD | 0x4F)),
        "home" => (Some(3), Some(CONSUMER | 0x40)),
        "end" => (Some(123), Some(KEYBOARD | 0x4D)),
        "pageup" => (Some(92), Some(KEYBOARD | 0x4B)),
        "pagedown" => (Some(93), Some(KEYBOARD | 0x4E)),
        "back" => (Some(4), None),
        "appswitch" | "recents" => (Some(187), None),
        "menu" => (Some(82), None),
        "volumeup" => (Some(24), Some(CONSUMER | 0xE9)),
        "volumedown" => (Some(25), Some(CONSUMER | 0xEA)),
        "power" => (Some(26), Some(CONSUMER | 0x30)),
        _ => return function_key(name),
    };
    Some(codes)
}

fn function_key(name: &str) -> Option<(Option<u32>, Option<u32>)> {
    let n: u32 = name.strip_prefix('f')?.parse().ok()?;
    (1..=12)
        .contains(&n)
        .then(|| (Some(130 + n), Some(KEYBOARD | (0x39 + n))))
}

/// A single letter or digit: its codes and whether it needs shift.
fn character(c: char) -> Option<((u32, u32), bool)> {
    if c.is_ascii_alphabetic() {
        let index = u32::from(c.to_ascii_lowercase() as u8 - b'a');
        return Some((
            (29 + index, KEYBOARD | (0x04 + index)),
            c.is_ascii_uppercase(),
        ));
    }
    let digit = c.to_digit(10)?;
    let ios = if digit == 0 { 0x27 } else { 0x1D + digit };
    Some(((7 + digit, KEYBOARD | ios), false))
}

fn modifier(platform: Platform, name: &str) -> Option<u32> {
    let (android, ios) = match name {
        "shift" => (0x1, 0x02),
        "alt" | "option" => (0x2, 0x04),
        "control" | "ctrl" => (0x1000, 0x01),
        "meta" | "command" | "cmd" => (0x10000, 0x08),
        _ => return None,
    };
    Some(match platform {
        Platform::Android => android,
        Platform::Ios => ios,
    })
}

fn unknown(name: &str, platform: Platform) -> Failure {
    Failure::new(
        UNKNOWN_KEY,
        format!(
            "{name:?} is not a key on {}; press takes Enter, Tab, Escape, Backspace, Delete, Space, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, Back, AppSwitch and Menu (Android), VolumeUp, VolumeDown, Power, F1 to F12, a letter or digit, modifiers joined with + (Shift, Control, Alt, Meta), or a platform key code",
            platform.id()
        ),
    )
}

/// The platform codes of a key as `press` takes it: `Enter`, `a`, `7`,
/// `Control+a`, `Shift+Tab` or a code such as `66`.
pub(crate) fn resolve(platform: Platform, text: &str) -> Result<KeyCode, Failure> {
    let text = text.trim();
    if text.len() > 1
        && let Ok(code) = text.parse::<u32>()
    {
        return Ok(KeyCode { code, meta: None });
    }
    let mut parts: Vec<&str> = text.split('+').collect();
    let key = parts
        .pop()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| unknown(text, platform))?;
    let mut meta = 0;
    for part in parts {
        meta |= modifier(platform, &part.to_ascii_lowercase())
            .ok_or_else(|| unknown(text, platform))?;
    }
    let mut chars = key.chars();
    let code = match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let ((android, ios), shifted) = character(c).ok_or_else(|| unknown(text, platform))?;
            if shifted {
                meta |= modifier(platform, "shift").unwrap_or(0);
            }
            match platform {
                Platform::Android => android,
                Platform::Ios => ios,
            }
        }
        _ => {
            let (android, ios) =
                named(&key.to_ascii_lowercase()).ok_or_else(|| unknown(text, platform))?;
            match platform {
                Platform::Android => android,
                Platform::Ios => ios,
            }
            .ok_or_else(|| unknown(text, platform))?
        }
    };
    Ok(KeyCode {
        code,
        meta: (meta != 0).then_some(meta),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(platform: Platform, text: &str) -> (u32, Option<u32>) {
        let code = resolve(platform, text).unwrap();
        (code.code, code.meta)
    }

    #[test]
    fn android_keys_are_keycodes_with_meta_bits() {
        assert_eq!(key(Platform::Android, "Enter"), (66, None));
        assert_eq!(key(Platform::Android, "back"), (4, None));
        assert_eq!(key(Platform::Android, "a"), (29, None));
        assert_eq!(key(Platform::Android, "A"), (29, Some(0x1)));
        assert_eq!(key(Platform::Android, "0"), (7, None));
        assert_eq!(key(Platform::Android, "Control+a"), (29, Some(0x1000)));
        assert_eq!(key(Platform::Android, "F12"), (142, None));
        assert_eq!(key(Platform::Android, "66"), (66, None));
    }

    #[test]
    fn ios_keys_are_hid_usages_with_their_page() {
        assert_eq!(key(Platform::Ios, "Enter"), (0x70028, None));
        assert_eq!(key(Platform::Ios, "Home"), (0xC0040, None));
        assert_eq!(key(Platform::Ios, "z"), (0x7001D, None));
        assert_eq!(key(Platform::Ios, "1"), (0x7001E, None));
        assert_eq!(key(Platform::Ios, "0"), (0x70027, None));
        assert_eq!(key(Platform::Ios, "Shift+Tab"), (0x7002B, Some(0x02)));
        assert_eq!(key(Platform::Ios, "F1"), (0x7003A, None));
    }

    #[test]
    fn unknown_keys_and_android_only_keys_on_ios_are_refused_with_a_code() {
        for (platform, text) in [
            (Platform::Android, "NoSuchKey"),
            (Platform::Ios, "Back"),
            (Platform::Android, "Hyper+a"),
            (Platform::Android, ""),
        ] {
            let failure = resolve(platform, text).expect_err(text);
            assert_eq!(failure.code, UNKNOWN_KEY);
        }
    }
}
