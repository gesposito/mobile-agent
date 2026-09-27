//! USB HID Keyboard page (0x07) usages, from spike S3: which usage and shift
//! state type a character, and the named keys that type nothing.

/// The usage and whether Shift is held for `character`.
pub fn usage_for(character: char) -> Option<(u32, bool)> {
    let offset = |base: char| character as u32 - base as u32;
    Some(match character {
        'a'..='z' => (0x04 + offset('a'), false),
        'A'..='Z' => (0x04 + offset('A'), true),
        '1'..='9' => (0x1E + offset('1'), false),
        '0' => (0x27, false),
        // The shifted number row shares the digits' usages.
        '!' => (0x1E, true),
        '@' => (0x1F, true),
        '#' => (0x20, true),
        '$' => (0x21, true),
        '%' => (0x22, true),
        '^' => (0x23, true),
        '&' => (0x24, true),
        '*' => (0x25, true),
        '(' => (0x26, true),
        ')' => (0x27, true),
        '\n' => (0x28, false),
        '\t' => (0x2B, false),
        ' ' => (0x2C, false),
        '-' => (0x2D, false),
        '_' => (0x2D, true),
        '=' => (0x2E, false),
        '+' => (0x2E, true),
        '[' => (0x2F, false),
        '{' => (0x2F, true),
        ']' => (0x30, false),
        '}' => (0x30, true),
        '\\' => (0x31, false),
        '|' => (0x31, true),
        ';' => (0x33, false),
        ':' => (0x33, true),
        '\'' => (0x34, false),
        '"' => (0x34, true),
        '`' => (0x35, false),
        '~' => (0x35, true),
        ',' => (0x36, false),
        '<' => (0x36, true),
        '.' => (0x37, false),
        '>' => (0x37, true),
        '/' => (0x38, false),
        '?' => (0x38, true),
        _ => return None,
    })
}

/// A named key: return, escape, delete, tab, space, the arrows and home.
pub fn usage_for_name(name: &str) -> Option<u32> {
    Some(match name {
        "return" | "enter" => 0x28,
        "escape" => 0x29,
        "delete" | "backspace" => 0x2A,
        "tab" => 0x2B,
        "space" => 0x2C,
        "home" => 0x4A,
        "right" => 0x4F,
        "left" => 0x50,
        "down" => 0x51,
        "up" => 0x52,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_start_at_the_keyboard_a_usage() {
        assert_eq!(usage_for('a'), Some((0x04, false)));
        assert_eq!(usage_for('z'), Some((0x1D, false)));
        assert_eq!(usage_for('A'), Some((0x04, true)));
        assert_eq!(usage_for('Z'), Some((0x1D, true)));
    }

    #[test]
    fn digits_run_one_to_nine_then_zero() {
        assert_eq!(usage_for('1'), Some((0x1E, false)));
        assert_eq!(usage_for('9'), Some((0x26, false)));
        assert_eq!(usage_for('0'), Some((0x27, false)));
    }

    #[test]
    fn the_fixture_password_maps_to_usages() {
        let usages: Option<Vec<(u32, bool)>> = "hunter2".chars().map(usage_for).collect();
        assert_eq!(
            usages,
            Some(vec![
                (0x0B, false),
                (0x18, false),
                (0x11, false),
                (0x17, false),
                (0x08, false),
                (0x15, false),
                (0x1F, false),
            ])
        );
    }

    #[test]
    fn shifted_symbols_reuse_their_unshifted_usage() {
        assert_eq!(usage_for('!'), Some((0x1E, true)));
        assert_eq!(usage_for('@'), Some((0x1F, true)));
        assert_eq!(usage_for('?'), Some((0x38, true)));
        assert_eq!(usage_for('/'), Some((0x38, false)));
        assert_eq!(usage_for('_'), Some((0x2D, true)));
        assert_eq!(usage_for(' '), Some((0x2C, false)));
        assert_eq!(usage_for('\n'), Some((0x28, false)));
    }

    #[test]
    fn a_character_without_a_usage_is_refused() {
        assert_eq!(usage_for('é'), None);
        assert_eq!(usage_for('€'), None);
    }

    #[test]
    fn the_named_keys_resolve() {
        assert_eq!(usage_for_name("return"), Some(0x28));
        assert_eq!(usage_for_name("escape"), Some(0x29));
        assert_eq!(usage_for_name("delete"), Some(0x2A));
        assert_eq!(usage_for_name("tab"), Some(0x2B));
        assert_eq!(usage_for_name("space"), Some(0x2C));
        assert_eq!(usage_for_name("right"), Some(0x4F));
        assert_eq!(usage_for_name("left"), Some(0x50));
        assert_eq!(usage_for_name("down"), Some(0x51));
        assert_eq!(usage_for_name("up"), Some(0x52));
        assert_eq!(usage_for_name("home"), Some(0x4A));
        assert_eq!(usage_for_name("lock"), None, "a button, not a key");
    }
}
