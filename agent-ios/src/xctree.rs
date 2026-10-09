//! XCTest's snapshot of the foreground application as the Simulator reader's
//! [`Element`]s (design D21), so that the reader's conversion,
//! [`simulator_reader::elements::snapshot`], builds the same snapshot payload
//! from a device as from a Simulator.
//!
//! The shim serializes `-[XCUIElement snapshotWithError:]` as JSON, one
//! object a node: `t` the element type, `tr` the traits, `f` the frame
//! `[x, y, width, height]` in points, `i` the identifier, `l` the label, `v`
//! the value, `p` the placeholder, `kf` true on the node with keyboard focus,
//! and `c` the children; strings only when present.
//!
//! XCTest's snapshot names no platform class (`XC_kAXXCAttributeElementType`)
//! and no scroll area, so every element's `class` and `scroll_area` stay
//! empty, and the conversion's rules that need a class see none (spike S10).
//! Keyboard focus becomes the editing trait the conversion reads for it.

use std::collections::HashMap;

use serde_json::Value;
use simulator_reader::codes;
use simulator_reader::elements::{self, Element, Limits, Owner, Read, traits};

use crate::shim::Shim;

fn error(code: &str, message: impl Into<String>, retryable: bool) -> protocol::Error {
    protocol::Error {
        code: code.to_owned(),
        message: message.into(),
        retryable,
    }
}

/// The pid of the process in front: the first of XCTest's active
/// applications, else its system application, SpringBoard, as on the home
/// screen.
pub fn foreground_pid<S: Shim>(shim: &S) -> Result<i32, protocol::Error> {
    let foreground = shim.foreground();
    foreground
        .active
        .first()
        .copied()
        .or(foreground.system)
        .ok_or_else(|| {
            error(
                codes::NO_FOREGROUND,
                "XCTest lists no active application and no system application",
                true,
            )
        })
}

/// The bundle identifier of `pid`'s application, asked of the daemon once
/// and kept in `bundles`.
pub fn bundle<S: Shim>(
    shim: &S,
    bundles: &mut HashMap<i32, String>,
    pid: i32,
) -> Result<String, protocol::Error> {
    if let Some(bundle) = bundles.get(&pid) {
        return Ok(bundle.clone());
    }
    let bundle = shim.bundle(pid).ok_or_else(|| {
        error(
            codes::SNAPSHOT_FAILED,
            format!("the test manager names no application for pid {pid}"),
            true,
        )
    })?;
    bundles.insert(pid, bundle.clone());
    Ok(bundle)
}

/// Reads the tree of the process in front through XCTest's snapshot of its
/// application, at `scale` pixels a point, and checks the process in front
/// again afterwards: a change during the read is `window_changed`, which
/// the host repeats.
pub fn read<S: Shim>(
    shim: &S,
    bundles: &mut HashMap<i32, String>,
    limits: Limits,
    scale: f64,
) -> Result<Read, protocol::Error> {
    let pid = foreground_pid(shim)?;
    let bundle = bundle(shim, bundles, pid)?;
    let tree = shim
        .tree(&bundle)
        .map_err(|reason| error(codes::SNAPSHOT_FAILED, format!("{bundle}: {reason}"), true))?;
    let parsed: Value = serde_json::from_str(&tree).map_err(|reason| {
        error(
            protocol::code::INTERNAL,
            format!("the shim's tree is not JSON: {reason}"),
            false,
        )
    })?;
    let root = element(&parsed);
    let after = foreground_pid(shim).ok();
    if after != Some(pid) {
        return Err(error(
            protocol::code::WINDOW_CHANGED,
            format!(
                "the foreground changed from pid {pid} to {} during the read",
                after.map_or("none".to_owned(), |pid| pid.to_string())
            ),
            true,
        ));
    }
    Ok(Read {
        owner: owner(pid, &bundle, &root),
        root,
        scale,
        limits,
        depth_cut: false,
    })
}

/// One node of the shim's JSON, with its children, as an [`Element`].
pub fn element(node: &Value) -> Element {
    let text = |key: &str| node.get(key).and_then(Value::as_str).map(str::to_owned);
    let frame = node.get("f").and_then(Value::as_array).and_then(|values| {
        let numbers: Vec<f64> = values.iter().filter_map(Value::as_f64).collect();
        match numbers[..] {
            [x, y, width, height] if numbers.iter().all(|n| n.is_finite()) => {
                Some([x, y, width, height])
            }
            _ => None,
        }
    });
    let mut traits = node.get("tr").and_then(Value::as_u64).unwrap_or(0);
    if node.get("kf").and_then(Value::as_bool) == Some(true) {
        traits |= traits::IS_EDITING;
    }
    let children: Vec<Element> = node
        .get("c")
        .and_then(Value::as_array)
        .map(|children| children.iter().map(element).collect())
        .unwrap_or_default();
    Element {
        automation_type: node.get("t").and_then(Value::as_u64),
        class: None,
        label: text("l"),
        identifier: text("i"),
        value: text("v"),
        placeholder: text("p"),
        traits,
        frame,
        children_count: Some(children.len() as u64),
        children,
        scroll_area: None,
        user_interaction: None,
        modal: false,
    }
}

/// The process whose tree was read, from its pid, its bundle identifier and
/// the application element: named by the element's label, else its bundle,
/// and system UI when it is SpringBoard, which owns the home screen.
pub fn owner(pid: i32, bundle: &str, root: &Element) -> Owner {
    let name = root
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or(bundle)
        .to_owned();
    Owner {
        pid,
        bundle_id: Some(bundle.to_owned()),
        name,
        system: bundle == elements::SPRINGBOARD,
    }
}

#[cfg(test)]
mod tests {
    use protocol::snapshot::{Bounds, Node, NodeFlags, WindowType};
    use simulator_reader::elements::{self, Limits, Read, SPRINGBOARD, traits};

    use super::*;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).expect("JSON")
    }

    /// The SwiftUI fixture's Big tab as the shim serializes it, cut to two
    /// cells: the application, its window, the grid with its header and the
    /// tab bar.
    const BIG: &str = r#"{"t":2,"tr":0,"f":[0,0,375,667],"l":"FixtureSwiftUI","c":[
        {"t":4,"tr":0,"f":[0,0,375,667],"c":[
            {"t":1,"tr":0,"f":[0,20,375,598],"c":[
                {"t":48,"tr":65536,"f":[16,26,343,20],"i":"big-count","l":"500 elements"},
                {"t":9,"tr":1,"f":[8,56.5,18,21],"i":"cell-1","l":"Cell 1"},
                {"t":9,"tr":1,"f":[26,56.5,18,21],"i":"cell-2","l":"Cell 2"}]},
            {"t":22,"tr":0,"f":[0,618,375,49],"c":[
                {"t":9,"tr":9,"f":[0,618,75,49],"i":"tab-big","l":"Big"}]}]}]}"#;

    fn find<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
        if node.res_id.as_deref() == Some(id) {
            return Some(node);
        }
        node.children.iter().find_map(|child| find(child, id))
    }

    #[test]
    fn a_node_and_its_children_become_elements() {
        let root = element(&json(
            r#"{"t":2,"tr":0,"f":[0,0,375,667],"l":"FixtureSwiftUI","c":[
                {"t":9,"tr":1,"f":[10,20,30,40],"i":"cell-1","l":"Cell 1"},
                {"t":49,"tr":0,"f":[0,0,1,1],"p":"Username","v":"ada","kf":true}]}"#,
        ));
        assert_eq!(root.automation_type, Some(2));
        assert_eq!(root.label.as_deref(), Some("FixtureSwiftUI"));
        assert_eq!(root.frame, Some([0.0, 0.0, 375.0, 667.0]));
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children_count, Some(2));
        assert_eq!(
            root.class, None,
            "XCTest's snapshot names no platform class"
        );
        let cell = &root.children[0];
        assert_eq!(cell.identifier.as_deref(), Some("cell-1"));
        assert_eq!(cell.label.as_deref(), Some("Cell 1"));
        assert_eq!(cell.traits, traits::BUTTON);
        assert_eq!(cell.frame, Some([10.0, 20.0, 30.0, 40.0]));
        assert_eq!(cell.children_count, Some(0));
        let field = &root.children[1];
        assert_eq!(field.placeholder.as_deref(), Some("Username"));
        assert_eq!(field.value.as_deref(), Some("ada"));
        assert_ne!(
            field.traits & traits::IS_EDITING,
            0,
            "keyboard focus is the editing trait the conversion reads"
        );
    }

    #[test]
    fn a_number_value_is_its_text_and_a_frame_that_is_not_four_numbers_is_none() {
        let root = element(&json(
            r#"{"t":2,"f":[0,0,375,667],"c":[
                {"t":40,"tr":0,"f":[0,0,51,31],"v":"1"},
                {"t":9,"tr":1,"f":[0,0,1]},
                {"t":9,"tr":1}]}"#,
        ));
        assert_eq!(root.children[0].value.as_deref(), Some("1"));
        assert_eq!(root.children[1].frame, None);
        assert_eq!(root.children[2].frame, None);
        assert_eq!(root.children[2].traits, traits::BUTTON);
    }

    #[test]
    fn an_application_is_owned_by_its_bundle_and_springboard_is_system_ui() {
        let app = element(&json(BIG));
        let owner = owner(812, "dev.mobileagent.fixture.swiftui", &app);
        assert_eq!(owner.pid, 812);
        assert_eq!(
            owner.bundle_id.as_deref(),
            Some("dev.mobileagent.fixture.swiftui")
        );
        assert_eq!(owner.name, "FixtureSwiftUI", "the application's label");
        assert!(!owner.system);

        let springboard = element(&json(r#"{"t":2,"f":[0,0,375,667]}"#));
        let owner = owner_of(34, &springboard);
        assert!(owner.system);
        assert_eq!(owner.name, SPRINGBOARD, "no label: the bundle names it");
    }

    fn owner_of(pid: i32, root: &Element) -> Owner {
        owner(pid, SPRINGBOARD, root)
    }

    /// The device's tree goes through the reader's own conversion: one
    /// application window that sends no change events, XCTest's element
    /// types as classes, identifiers, labels, traits as flags and frames as
    /// pixels at the screen's scale.
    #[test]
    fn the_device_tree_goes_through_the_readers_conversion() {
        let root = element(&json(BIG));
        let read = Read {
            owner: owner(812, "dev.mobileagent.fixture.swiftui", &root),
            root,
            scale: 2.0,
            limits: Limits::default(),
            depth_cut: false,
        };
        let snapshot = elements::snapshot(&read);
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.node_count, 8);
        let window = &snapshot.windows[0];
        assert_eq!(window.id, 812);
        assert_eq!(window.kind, WindowType::APPLICATION);
        assert_eq!(window.emits_events, Some(false));
        assert_eq!(window.title.as_deref(), Some("FixtureSwiftUI"));
        assert_eq!(window.bounds, Bounds::new(0, 0, 750, 1334));
        let app = window.root.as_ref().expect("the application's node");
        let class = |node: &Node| snapshot.strings[node.class as usize].clone();
        assert_eq!(class(app), "XCUIElementTypeApplication");
        assert_eq!(class(&app.children[0]), "XCUIElementTypeWindow");

        let cell = find(app, "cell-1").expect("cell-1");
        assert_eq!(class(cell), "XCUIElementTypeButton");
        assert_eq!(cell.desc.as_deref(), Some("Cell 1"));
        assert_eq!(cell.bounds, Bounds::new(16, 113, 52, 155));
        assert!(
            cell.flags
                .contains(NodeFlags::CLICKABLE | NodeFlags::VISIBLE | NodeFlags::ENABLED)
        );

        let header = find(app, "big-count").expect("the header");
        assert_eq!(class(header), "XCUIElementTypeHeading");
        let tab = find(app, "tab-big").expect("the tab");
        assert!(tab.flags.contains(NodeFlags::SELECTED));
    }

    #[test]
    fn the_focused_field_is_focused_and_editable_with_its_placeholder_as_description() {
        let root = element(&json(
            r#"{"t":2,"f":[0,0,375,667],"l":"FixtureSwiftUI","c":[
                {"t":49,"tr":0,"f":[16,100,343,34],"i":"login-username","p":"Username","kf":true}]}"#,
        ));
        let read = Read {
            owner: owner(812, "dev.mobileagent.fixture.swiftui", &root),
            root,
            scale: 2.0,
            limits: Limits::default(),
            depth_cut: false,
        };
        let snapshot = elements::snapshot(&read);
        let app = snapshot.windows[0].root.as_ref().expect("the application");
        let field = find(app, "login-username").expect("the field");
        assert!(
            field
                .flags
                .contains(NodeFlags::FOCUSED | NodeFlags::EDITABLE)
        );
        assert_eq!(field.desc.as_deref(), Some("Username"));
        assert_eq!(
            field.text, None,
            "an empty field's text is not its placeholder"
        );
    }
}
