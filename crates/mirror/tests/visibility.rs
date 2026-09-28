//! Task 10.15: how much of an element is in view. A scroll that looks for
//! an element stops once the element is mostly in view, so the mirror tells
//! the share of an element's bounds inside the visible bounds of what holds
//! it (its window, clipped by its scroll containers, less the bars drawn
//! over them) and whether its bounds end at an edge where a container can
//! still scroll: Android reports bounds clipped to their ancestors, so a
//! row a list cuts off reads as the sliver that shows.
//!
//! Task 10.16: how far a stroke may carry the content before an element
//! passes through the view unseen. The mirror tells the visible bounds of
//! the scroll container a stroke through a point moves, as an element
//! inside it sees them.

mod support;

use std::time::Duration;

use mirror::testing::FakeTarget;
use mirror::{
    Condition, Direction, ElementId, Mirror, MirrorConfig, Point, Rect, Selector, Visibility,
};
use serde_json::{Value, json};
use support::Screen;

/// The rows of a list at `[left, top, right, bottom]`, as wide as the list
/// and 48 points tall, the first at `first_top`, cut to the list as Android
/// reports them when `clipped`, and whole as iOS reports them otherwise;
/// rows wholly outside it are left out, as a virtualized list leaves them.
fn rows(class: &str, first_top: f64, count: u32, list: [f64; 4], clipped: bool) -> Vec<Value> {
    let [left, top, right, bottom] = list;
    (1..=count)
        .filter_map(|n| {
            let row_top = first_top + 48.0 * f64::from(n - 1);
            let row_bottom = row_top + 48.0;
            if row_bottom <= top || row_top >= bottom {
                return None;
            }
            let (t, b) = if clipped {
                (row_top.max(top), row_bottom.min(bottom))
            } else {
                (row_top, row_bottom)
            };
            Some(json!({
                "class": class, "id": format!("row-{n}"), "desc": format!("Row {n}"),
                "flags": ["clickable", "focusable"], "bounds": [left, t, right, b]
            }))
        })
        .collect()
}

/// The status bar window of an Android screen 408 points wide.
fn status_bar() -> Value {
    json!({
        "id": 2, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Status bar", "bounds": [0, 0, 408, 24], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 24]}
    })
}

/// An Android app window holding `content` below its root.
fn app_window(content: Vec<Value>) -> Value {
    json!({
        "id": 1, "kind": "application", "layer": 1, "package": "dev.mobileagent.fixture.expo",
        "title": "FixtureExpo", "bounds": [0, 0, 408, 912], "flags": ["active", "focused"],
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 0, 408, 912], "children": content}
    })
}

fn android(windows: Vec<Value>) -> Screen {
    Screen::from_json(json!({
        "backend": {"name": "android-agent", "fallback": false},
        "scale": 2.625,
        "windows": windows
    }))
}

/// The Expo fixture's list on Android as its agent reads it: a scroll view
/// at [0, 72, 408, 840] whose rows start at `first_top`, clipped to it, and
/// whose scroll semantics say it can scroll back and forward as `back` and
/// `forward` say; `None` for a scroll view that reports no semantics.
fn android_list(first_top: f64, semantics: Option<(bool, bool)>) -> Screen {
    let mut list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 72, 408, 840],
        "flags": ["scrollable", "focusable"],
        "children": [{
            "class": "android.view.ViewGroup", "bounds": [0, 72, 408, 840],
            "children": rows("android.view.ViewGroup", first_top, 200, [0.0, 72.0, 408.0, 840.0], true)
        }]
    });
    if let Some((back, forward)) = semantics {
        list["scroll"] = json!({"axis": "vertical", "back": back, "forward": forward});
    }
    android(vec![status_bar(), app_window(vec![list])])
}

/// A mirror that has read `screen`.
fn mirror_showing(screen: &Screen) -> Mirror {
    let target = FakeTarget::new(screen.read());
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().expect("the first read");
    mirror
}

/// The element with `identifier`, on screen.
fn element(mirror: &mut Mirror, identifier: &str) -> ElementId {
    mirror
        .wait(
            &Selector::identifier(identifier),
            Condition::Appear,
            Duration::ZERO,
        )
        .unwrap_or_else(|e| panic!("{identifier} is on screen: {e}"))
        .element
        .expect("the element")
        .id
}

fn visibility(mirror: &mut Mirror, identifier: &str) -> Visibility {
    let id = element(mirror, identifier);
    mirror
        .visibility(id)
        .expect("the replica holds the element")
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.01
}

#[test]
fn a_row_inside_its_list_is_wholly_in_view_within_the_list_s_bounds() {
    let mut mirror = mirror_showing(&android_list(72.0, Some((false, true))));
    let row = visibility(&mut mirror, "row-3");
    assert_eq!(row.bounds, Rect::new(0.0, 168.0, 408.0, 216.0), "{row:?}");
    assert_eq!(row.viewport, Rect::new(0.0, 72.0, 408.0, 840.0), "{row:?}");
    assert_eq!(row.fraction, 1.0, "{row:?}");
    assert_eq!(row.cut, None, "{row:?}");
}

/// The frame-path run of the harness's list-scroll flow stopped with
/// `row-200` 1.2 points tall at the Expo list's bottom edge: Android clips
/// a row's bounds to the list, so its bounds are all in view, and only the
/// edge they end at, where the list can still scroll, tells that more of
/// the row may lie beyond.
#[test]
fn a_row_android_clips_at_the_bottom_edge_of_a_list_that_scrolls_on_is_cut_there() {
    // Row 17 starts at 838.8, 1.2 points above the list's bottom edge.
    let mut mirror = mirror_showing(&android_list(838.8 - 768.0, Some((true, true))));
    let row = visibility(&mut mirror, "row-17");
    assert!(row.bounds.height() < 1.5, "a sliver: {row:?}");
    assert_eq!(row.bounds.bottom, 840.0, "{row:?}");
    assert_eq!(
        row.fraction, 1.0,
        "its bounds, as reported, are in view: {row:?}"
    );
    assert_eq!(row.cut, Some(Direction::Below), "{row:?}");

    let whole = visibility(&mut mirror, "row-10");
    assert_eq!(whole.cut, None, "a row short of the edges: {whole:?}");
}

#[test]
fn a_row_android_clips_at_the_top_edge_of_a_list_that_scrolls_back_is_cut_there() {
    let mut mirror = mirror_showing(&android_list(72.0 - 40.0, Some((true, true))));
    let row = visibility(&mut mirror, "row-1");
    assert_eq!(row.bounds, Rect::new(0.0, 72.0, 408.0, 80.0), "{row:?}");
    assert_eq!(row.cut, Some(Direction::Above), "{row:?}");
}

/// At the end of its range a list shows no more below its last row, so a
/// row that ends at the bottom edge there is whole.
#[test]
fn a_row_at_the_edge_of_a_list_that_cannot_scroll_that_way_is_whole() {
    // Rows 185 to 200 fill the list exactly; it cannot scroll forward.
    let mut mirror = mirror_showing(&android_list(72.0 - 48.0 * 184.0, Some((true, false))));
    let row = visibility(&mut mirror, "row-200");
    assert_eq!(row.bounds, Rect::new(0.0, 792.0, 408.0, 840.0), "{row:?}");
    assert_eq!(row.fraction, 1.0, "{row:?}");
    assert_eq!(row.cut, None, "{row:?}");
}

/// A container that reports no scroll semantics cannot tell whether it
/// scrolls further, and an edge its rows end at is no cut.
#[test]
fn without_scroll_semantics_an_edge_is_no_cut() {
    let mut mirror = mirror_showing(&android_list(838.8 - 768.0, None));
    let row = visibility(&mut mirror, "row-17");
    assert_eq!(row.cut, None, "{row:?}");
}

/// iOS reports whole frames, and a UIKit list runs under the navigation bar
/// and the tab bar: the bars cut its visible bounds, and a row under the
/// tab bar is in view only as far as it shows above it.
#[test]
fn bars_over_a_list_cut_its_visible_bounds_and_a_whole_frame_shows_its_share() {
    let list_rows = rows(
        "XCUIElementTypeCell",
        62.0 + 16.0,
        20,
        [0.0, 0.0, 402.0, 874.0],
        false,
    );
    let screen = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [{
            "id": 7, "kind": "application", "package": "dev.mobileagent.fixture.swiftui",
            "title": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "flags": ["active", "focused"], "events": false,
            "root": {"class": "XCUIElementTypeApplication", "desc": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "children": [
                {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                    {"class": "XCUIElementTypeNavigationBar", "id": "nav", "desc": "List", "bounds": [0, 0, 402, 62]},
                    {"class": "XCUIElementTypeCollectionView", "id": "list", "bounds": [0, 0, 402, 874],
                     "scroll": {"axis": "vertical", "back": false, "forward": true},
                     "children": list_rows},
                    {"class": "XCUIElementTypeTabBar", "id": "tabs", "desc": "Tab Bar", "bounds": [0, 791, 402, 874], "children": [
                        {"class": "XCUIElementTypeButton", "id": "tab-list", "desc": "List", "bounds": [0, 791, 134, 874]}
                    ]}
                ]}
            ]}
        }]
    }));
    let mut mirror = mirror_showing(&screen);
    // Row 15 spans [750, 798]: 41 of its 48 points show above the tab bar.
    let row = visibility(&mut mirror, "row-15");
    assert_eq!(row.viewport, Rect::new(0.0, 62.0, 402.0, 791.0), "{row:?}");
    assert!(close(row.fraction, 41.0 / 48.0), "{row:?}");
    assert_eq!(row.cut, None, "a whole frame is no cut: {row:?}");
    // Row 16 spans [798, 846], under the tab bar.
    let hidden = visibility(&mut mirror, "row-16");
    assert_eq!(hidden.fraction, 0.0, "{hidden:?}");

    // A tab of the tab bar is not under its own bar.
    let tab = visibility(&mut mirror, "tab-list");
    assert_eq!(tab.fraction, 1.0, "{tab:?}");
}

/// System bars and the keyboard are windows above the app's, and an app
/// drawn edge to edge runs under them.
#[test]
fn system_bars_and_the_keyboard_above_the_app_cut_its_visible_bounds() {
    let list = |first_top: f64| {
        json!({
            "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 0, 408, 912],
            "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": true, "forward": true},
            "children": rows("android.view.ViewGroup", first_top, 30, [0.0, 0.0, 408.0, 912.0], true)
        })
    };
    let navigation_bar = json!({
        "id": 3, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Navigation bar", "bounds": [0, 864, 408, 912], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 864, 408, 912]}
    });
    let screen = android(vec![
        status_bar(),
        navigation_bar.clone(),
        app_window(vec![list(0.0)]),
    ]);
    let mut mirror = mirror_showing(&screen);
    // Row 18 spans [816, 864], just above the navigation bar; row 19, [864,
    // 912], is under it.
    let above = visibility(&mut mirror, "row-18");
    assert_eq!(
        above.viewport,
        Rect::new(0.0, 24.0, 408.0, 864.0),
        "{above:?}"
    );
    assert_eq!(above.fraction, 1.0, "{above:?}");
    let under = visibility(&mut mirror, "row-19");
    assert_eq!(under.fraction, 0.0, "{under:?}");
    // Row 1 spans [0, 48]: half of it is under the status bar.
    let first = visibility(&mut mirror, "row-1");
    assert_eq!(first.fraction, 0.5, "{first:?}");

    let keyboard = json!({
        "id": 5, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    });
    let typing = android(vec![
        keyboard,
        status_bar(),
        navigation_bar,
        app_window(vec![list(0.0)]),
    ]);
    let mut mirror = mirror_showing(&typing);
    // Row 13 spans [576, 624], half under the keyboard; row 14 is under it.
    let row = visibility(&mut mirror, "row-13");
    assert_eq!(row.viewport, Rect::new(0.0, 24.0, 408.0, 600.0), "{row:?}");
    assert_eq!(row.fraction, 0.5, "{row:?}");
    let under = visibility(&mut mirror, "row-14");
    assert_eq!(under.fraction, 0.0, "{under:?}");
}

/// An element no scroll container holds is in view as far as its window
/// shows it.
#[test]
fn an_element_outside_any_scroll_container_is_measured_against_its_window() {
    let screen = android(vec![
        status_bar(),
        app_window(vec![json!({
            "class": "android.widget.Button", "id": "submit", "text": "Sign in",
            "flags": ["clickable", "focusable"], "bounds": [16, 200, 392, 248]
        })]),
    ]);
    let mut mirror = mirror_showing(&screen);
    let button = visibility(&mut mirror, "submit");
    assert_eq!(
        button.viewport,
        Rect::new(0.0, 24.0, 408.0, 912.0),
        "{button:?}"
    );
    assert_eq!(button.fraction, 1.0, "{button:?}");
    assert_eq!(button.cut, None, "{button:?}");
    assert_eq!(mirror.visibility(ElementId(u64::MAX)), None);
}

/// Task 10.16: a stroke through the middle of the Expo list moves the list,
/// whose visible bounds are its own inside its window.
#[test]
fn a_stroke_through_a_list_moves_the_list_and_its_visible_bounds_are_its_own() {
    let mirror = mirror_showing(&android_list(72.0, Some((false, true))));
    assert_eq!(
        mirror.scroll_viewport(Point::new(204.0, 456.0), true),
        Some(Rect::new(0.0, 72.0, 408.0, 840.0))
    );
    // Without scroll semantics the scroll view still scrolls.
    let bare = mirror_showing(&android_list(72.0, None));
    assert_eq!(
        bare.scroll_viewport(Point::new(204.0, 456.0), true),
        Some(Rect::new(0.0, 72.0, 408.0, 840.0))
    );
}

/// Task 10.16: the bars over a list are left out of what a stroke through
/// it can bring into view, whether they are the list's own window's
/// navigation and tab bars, as a UIKit list runs under them, or windows
/// above it, as an edge-to-edge Android list runs under the system bars and
/// the keyboard.
#[test]
fn the_bars_over_a_list_are_left_out_of_its_visible_bounds() {
    let screen = Screen::from_json(json!({
        "backend": {"name": "simulator-reader", "fallback": false},
        "scale": 3.0,
        "windows": [{
            "id": 7, "kind": "application", "package": "dev.mobileagent.fixture.swiftui",
            "title": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "flags": ["active", "focused"], "events": false,
            "root": {"class": "XCUIElementTypeApplication", "desc": "FixtureSwiftUI", "bounds": [0, 0, 402, 874], "children": [
                {"class": "XCUIElementTypeWindow", "bounds": [0, 0, 402, 874], "children": [
                    {"class": "XCUIElementTypeNavigationBar", "id": "nav", "desc": "List", "bounds": [0, 0, 402, 62]},
                    {"class": "XCUIElementTypeCollectionView", "id": "list", "bounds": [0, 0, 402, 874],
                     "children": rows("XCUIElementTypeCell", 78.0, 20, [0.0, 0.0, 402.0, 874.0], false)},
                    {"class": "XCUIElementTypeTabBar", "id": "tabs", "desc": "Tab Bar", "bounds": [0, 791, 402, 874], "children": [
                        {"class": "XCUIElementTypeButton", "id": "tab-list", "desc": "List", "bounds": [0, 791, 134, 874]}
                    ]}
                ]}
            ]}
        }]
    }));
    let uikit = mirror_showing(&screen);
    assert_eq!(
        uikit.scroll_viewport(Point::new(201.0, 437.0), true),
        Some(Rect::new(0.0, 62.0, 402.0, 791.0))
    );

    let list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 0, 408, 912],
        "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": true, "forward": true},
        "children": rows("android.view.ViewGroup", 0.0, 30, [0.0, 0.0, 408.0, 912.0], true)
    });
    let navigation_bar = json!({
        "id": 3, "kind": "system", "layer": 3, "package": "com.android.systemui",
        "title": "Navigation bar", "bounds": [0, 864, 408, 912], "owner": "com.android.systemui",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 864, 408, 912]}
    });
    let keyboard = json!({
        "id": 5, "kind": "input_method", "layer": 4, "package": "com.google.android.inputmethod.latin",
        "title": "Keyboard", "bounds": [0, 600, 408, 912], "owner": "com.google.android.inputmethod.latin",
        "root": {"class": "android.widget.FrameLayout", "bounds": [0, 600, 408, 912]}
    });
    let edge_to_edge = mirror_showing(&android(vec![
        status_bar(),
        navigation_bar.clone(),
        app_window(vec![list.clone()]),
    ]));
    assert_eq!(
        edge_to_edge.scroll_viewport(Point::new(204.0, 456.0), true),
        Some(Rect::new(0.0, 24.0, 408.0, 864.0))
    );
    let typing = mirror_showing(&android(vec![
        keyboard,
        status_bar(),
        navigation_bar,
        app_window(vec![list]),
    ]));
    assert_eq!(
        typing.scroll_viewport(Point::new(204.0, 300.0), true),
        Some(Rect::new(0.0, 24.0, 408.0, 600.0))
    );
}

/// Task 10.16: of the containers under a point, a stroke moves the
/// innermost one that scrolls along its axis: a carousel for a stroke
/// across it, the list that holds the carousel for one along the list.
#[test]
fn a_stroke_moves_the_innermost_container_that_scrolls_along_its_axis() {
    let carousel = json!({
        "class": "androidx.recyclerview.widget.RecyclerView", "id": "carousel",
        "bounds": [0, 200, 408, 400], "flags": ["scrollable"],
        "scroll": {"axis": "horizontal", "back": false, "forward": true},
        "children": [{
            "class": "android.view.ViewGroup", "id": "card-1", "desc": "Card 1",
            "flags": ["clickable"], "bounds": [16, 216, 216, 384]
        }]
    });
    let list = json!({
        "class": "android.widget.ScrollView", "id": "list", "bounds": [0, 72, 408, 840],
        "flags": ["scrollable"], "scroll": {"axis": "vertical", "back": false, "forward": true},
        "children": [{"class": "android.view.ViewGroup", "bounds": [0, 72, 408, 840], "children": [carousel]}]
    });
    let mirror = mirror_showing(&android(vec![status_bar(), app_window(vec![list])]));
    let inside = Point::new(204.0, 300.0);
    assert_eq!(
        mirror.scroll_viewport(inside, false),
        Some(Rect::new(0.0, 200.0, 408.0, 400.0)),
        "across the carousel"
    );
    assert_eq!(
        mirror.scroll_viewport(inside, true),
        Some(Rect::new(0.0, 72.0, 408.0, 840.0)),
        "along the list"
    );
}

/// Task 10.16: a point that no scroll container holds has no container a
/// stroke through it moves.
#[test]
fn a_point_outside_every_scroll_container_has_none() {
    let mirror = mirror_showing(&android_list(72.0, Some((false, true))));
    assert_eq!(
        mirror.scroll_viewport(Point::new(204.0, 880.0), true),
        None,
        "below the list"
    );
    let form = mirror_showing(&android(vec![
        status_bar(),
        app_window(vec![json!({
            "class": "android.widget.Button", "id": "submit", "text": "Sign in",
            "flags": ["clickable", "focusable"], "bounds": [16, 200, 392, 248]
        })]),
    ]));
    assert_eq!(form.scroll_viewport(Point::new(204.0, 224.0), true), None);
}
