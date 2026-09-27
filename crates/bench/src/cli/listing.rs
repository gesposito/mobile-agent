//! Reads the element lines of a `mobile-agent snapshot` listing.

/// One element line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub role: String,
    /// The name; empty when the line has none.
    pub name: String,
    pub id: Option<String>,
    /// The ref without its `@`, such as `e3`.
    pub reference: Option<String>,
    /// The value after `: `.
    pub value: Option<String>,
}

impl Line {
    /// The text the element shows, as `get text` reads it: the value, else
    /// the name.
    pub fn text(&self) -> &str {
        &self.name
    }
}

/// The element lines of `listing`, in order.
pub fn parse(listing: &str) -> Vec<Line> {
    let _ = listing;
    Vec::new()
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
- text "Tab	stop" [id="a\"b"]"#,
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
