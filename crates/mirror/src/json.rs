//! JSON text, written the way agent-browser's `serde_json` writes it: compact,
//! strings escaped for `"`, `\` and control characters only. The mirror
//! writes agent-browser's shapes itself, so a delta's size is known exactly;
//! a host writes a reply around a listing's strings with [`push_str`], as
//! the daemon writes the listing's reply.

use std::fmt::Write as _;

/// What each byte becomes in a JSON string, as `serde_json` escapes it:
/// itself (0), a backslash and this letter, or for `u` a backslash and
/// `u00` with the byte in two hexadecimal digits.
static ESCAPE: [u8; 256] = {
    let mut escape = [0; 256];
    let mut byte = 0;
    while byte < 0x20 {
        escape[byte] = b'u';
        byte += 1;
    }
    escape[0x08] = b'b';
    escape[0x09] = b't';
    escape[0x0a] = b'n';
    escape[0x0c] = b'f';
    escape[0x0d] = b'r';
    escape[b'"' as usize] = b'"';
    escape[b'\\' as usize] = b'\\';
    escape
};

/// Appends `s` as a JSON string literal, escaped as `serde_json` escapes
/// it.
pub fn push_str(out: &mut String, s: &str) {
    out.push('"');
    // The text between escapes goes in a run at a time, looked up a byte at
    // a time as `serde_json` looks it up: every byte that takes an escape
    // is ASCII, so each run ends on a character's boundary.
    let mut run = 0;
    for (at, &byte) in s.as_bytes().iter().enumerate() {
        let escape = ESCAPE[usize::from(byte)];
        if escape == 0 {
            continue;
        }
        out.push_str(&s[run..at]);
        out.push('\\');
        if escape == b'u' {
            let _ = write!(out, "u{byte:04x}");
        } else {
            out.push(char::from(escape));
        }
        run = at + 1;
    }
    out.push_str(&s[run..]);
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
