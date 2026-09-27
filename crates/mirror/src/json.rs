//! JSON text, written the way agent-browser's `serde_json` writes it: compact,
//! strings escaped for `"`, `\` and control characters only. The mirror
//! writes agent-browser's shapes itself, so a delta's size is known exactly.

use std::fmt::Write as _;

/// Appends `s` as a JSON string literal.
pub(crate) fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `s` as a JSON string literal.
pub(crate) fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    push_str(&mut out, s);
    out
}

#[cfg(test)]
mod tests {
    use super::string;

    #[test]
    fn strings_escape_what_serde_json_escapes() {
        assert_eq!(string("plain"), "\"plain\"");
        assert_eq!(string("a \"b\" \\ c"), "\"a \\\"b\\\" \\\\ c\"");
        assert_eq!(string("l1\nl2\r\t"), "\"l1\\nl2\\r\\t\"");
        assert_eq!(string("\u{1}\u{1f}"), "\"\\u0001\\u001f\"");
        assert_eq!(string("\u{8}\u{c}"), "\"\\b\\f\"");
        assert_eq!(string("/ é “q” \u{7f}"), "\"/ é “q” \u{7f}\"");
    }
}
