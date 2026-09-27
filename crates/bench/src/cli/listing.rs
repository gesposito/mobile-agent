//! Reads the element lines of a `mobile-agent snapshot` listing, as the
//! mirror writes them: `- role "name" [checked=…, selected, disabled,
//! focused, id=…, ref=eN]: value`, indented two spaces per level, with the
//! name, the bracket and the value each left out when there is none. The name
//! is a JSON string; the identifier is bare unless it holds whitespace, a
//! comma, a bracket, a quote or a backslash, when it is a JSON string too.
//! Lines that do not start with `- ` after their indent, such as scroll hints,
//! are not elements.

/// One element line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub role: String,
    /// The name; empty when the line has none.
    pub name: String,
    pub id: Option<String>,
    /// The ref without its `@`, such as `e3`.
    pub reference: Option<String>,
    /// The value after `: `, which the listing prints when it differs from
    /// the name.
    pub value: Option<String>,
}

impl Line {
    /// The text the element shows, as `get text` reads it: the value, else
    /// the name.
    pub fn text(&self) -> &str {
        self.value
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.name)
    }
}

/// The element lines of `listing`, in order.
pub fn parse(listing: &str) -> Vec<Line> {
    listing.lines().filter_map(line).collect()
}

/// One element line, or `None` for any other line.
fn line(text: &str) -> Option<Line> {
    let rest = text.trim_start_matches(' ').strip_prefix("- ")?;
    let end = rest.find([' ', ':']).unwrap_or(rest.len());
    let (role, mut rest) = rest.split_at(end);
    if role.is_empty() {
        return None;
    }
    let mut line = Line {
        role: role.to_owned(),
        ..Line::default()
    };
    if let Some(quoted) = rest.strip_prefix(' ').filter(|r| r.starts_with('"')) {
        let (name, after) = json_string(quoted)?;
        line.name = name;
        rest = after;
    }
    if let Some(attributes) = rest.strip_prefix(" [") {
        rest = read_attributes(attributes, &mut line)?;
    }
    if let Some(value) = rest.strip_prefix(": ") {
        line.value = Some(value.to_owned());
    }
    Some(line)
}

/// Reads `key=value, key, …]` into `line`, keeping the identifier and the ref,
/// and returns the text after the closing bracket.
fn read_attributes<'a>(mut rest: &'a str, line: &mut Line) -> Option<&'a str> {
    loop {
        let end = rest.find(['=', ',', ']'])?;
        let key = &rest[..end];
        rest = &rest[end..];
        if let Some(after) = rest.strip_prefix('=') {
            let (value, tail) = if after.starts_with('"') {
                json_string(after)?
            } else {
                let end = after.find([',', ']'])?;
                (after[..end].to_owned(), &after[end..])
            };
            rest = tail;
            match key {
                "id" => line.id = Some(value),
                "ref" => line.reference = Some(value),
                _ => {}
            }
        }
        match rest.strip_prefix(", ") {
            Some(tail) => rest = tail,
            None => return rest.strip_prefix(']'),
        }
    }
}

/// The JSON string `text` starts with, decoded, and the text after it.
fn json_string(text: &str) -> Option<(String, &str)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    // Escapes and the closing quote are ASCII, which never occurs inside a
    // multi-byte character, so scanning bytes finds them.
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => {
                let value = serde_json::from_str(&text[..=i]).ok()?;
                return Some((value, &text[i + 1..]));
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(role: &str, name: &str, id: Option<&str>, reference: Option<&str>) -> Line {
        Line {
            role: role.into(),
            name: name.into(),
            id: id.map(Into::into),
            reference: reference.map(Into::into),
            value: None,
        }
    }

    #[test]
    fn element_lines_give_their_role_name_identifier_ref_and_value() {
        let listing = "\
- heading \"Login\" [id=login-title, ref=e198]
- textbox \"Username\" [id=login-username, ref=e199]: agent
- button \"Big\" [selected, id=tab-big, ref=e503]
- switch \"Wi-Fi\" [checked=true, disabled, focused, id=settings-wifi, ref=e3]
- text \"Welcome, agent\" [id=login-status]
- grid [id=big-grid]
  - button \"Cell 1\" [id=cell-1, ref=e1]
- text \"12:30\"
- textbox: typed";
        let mut username = element("textbox", "Username", Some("login-username"), Some("e199"));
        username.value = Some("agent".into());
        let mut typed = element("textbox", "", None, None);
        typed.value = Some("typed".into());
        assert_eq!(
            parse(listing),
            [
                element("heading", "Login", Some("login-title"), Some("e198")),
                username,
                element("button", "Big", Some("tab-big"), Some("e503")),
                element("switch", "Wi-Fi", Some("settings-wifi"), Some("e3")),
                element("text", "Welcome, agent", Some("login-status"), None),
                element("grid", "", Some("big-grid"), None),
                element("button", "Cell 1", Some("cell-1"), Some("e1")),
                element("text", "12:30", None, None),
                typed,
            ]
        );
    }

    #[test]
    fn quoted_names_and_identifiers_are_read_as_json_strings() {
        let lines = parse(
            r#"- button "Say \"hi\" [now], \\ok" [id="odd id, [x]", ref=e7]: v: w
- text "Tab\tstop" [id="a\"b"]"#,
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].name, r#"Say "hi" [now], \ok"#);
        assert_eq!(lines[0].id.as_deref(), Some("odd id, [x]"));
        assert_eq!(lines[0].reference.as_deref(), Some("e7"));
        assert_eq!(lines[0].value.as_deref(), Some("v: w"));
        assert_eq!(lines[1].name, "Tab\tstop");
        assert_eq!(lines[1].id.as_deref(), Some("a\"b"));
    }

    #[test]
    fn hints_and_summaries_are_not_elements() {
        let listing = "\
- button \"Row 1\" [id=row-1, ref=e506]
(183 more actionable items below, scroll down to reveal)
  (3 more lines)
(empty screen)";
        let lines = parse(listing);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].id.as_deref(), Some("row-1"));
    }

    #[test]
    fn an_element_shows_its_value_else_its_name() {
        let lines = parse(
            "- textbox \"Username\" [id=login-username]: agent\n- text \"Detail 200\" [id=detail-200]",
        );
        assert_eq!(lines[0].text(), "agent");
        assert_eq!(lines[1].text(), "Detail 200");
    }
}
