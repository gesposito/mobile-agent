//! How much of an element is in view: the part of its bounds inside the
//! visible bounds of what holds it (task 10.15).
//!
//! The visible bounds start from the element's window, clipped by every
//! scroll container that holds the element (a node that says it scrolls, a
//! list or grid, or one with scroll semantics, as scroll hints find them),
//! and lose the bars drawn over them that the replica shows: a system or
//! keyboard window above the element's window, and a tab bar or toolbar of
//! the element's own window that does not scroll with it, such as the tab
//! bar a UIKit list runs under. A bar cuts the bounds only where it spans
//! them from one side to the other and reaches one of their edges, so the
//! visible bounds stay a rectangle.
//!
//! Android reports a node's bounds clipped to every ancestor
//! (`View.getBoundsOnScreen` with `clipToParent`, and Compose's bounds in
//! root are clipped the same way), so a row a list cuts off reads as the
//! part of it that shows: on the API 36 emulator a row of 48 points read as
//! 1.2 points tall at the list's bottom edge. iOS reports whole frames. An
//! element whose bounds end at an edge of its scroll containers' bounds,
//! where one of them can still scroll toward that edge, may therefore go on
//! beyond it by an amount the replica does not show; that edge is its
//! `cut`.
//!
//! The same visible bounds, seen from inside a scroll container, say how
//! far a stroke through the container may carry its content before an
//! element passes through them between two looks (task 10.16).
//!
//! The same windows, cutting the area a stroke goes through, say where the
//! stroke may come down without landing on a system bar or the keyboard
//! (task 10.32).

use protocol::snapshot::{Scroll, ScrollAxis, ScrollFlags, WindowFlags, WindowType};

use crate::geometry::{Point, Rect};
use crate::replica::{Replica, Win};
use crate::snapshot::{Direction, Role};
use crate::view::{self, Cover, CoverName, Hidden, View};

/// How much of an element is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visibility {
    /// The element's bounds as the replica has them, in points.
    pub bounds: Rect,
    /// The visible bounds: the element's window, clipped by the scroll
    /// containers that hold the element, less the bars drawn over them.
    pub viewport: Rect,
    /// The share of `bounds` inside `viewport`, from 0 to 1; 0 for bounds
    /// without an area.
    pub fraction: f64,
    /// The edge of the scroll containers' bounds that `bounds` ends at while
    /// a container holding the element can still scroll toward it, as its
    /// scroll semantics say: bounds clipped there, as Android clips them,
    /// leave out whatever lies beyond. `None` when the bounds end at no such
    /// edge.
    pub cut: Option<Direction>,
    /// Where a tap reaches the element: the largest rectangle of `bounds`
    /// inside `viewport`, at least [`REACH_FLOOR`] points on each side, or
    /// as long as the element is on a side where it is shorter; a tap goes
    /// to its centre. `None` when no such rectangle is left.
    pub reachable: Option<Rect>,
}

/// The least a tap needs of an element on each side, in points: less than
/// this left of it between the covers and the edges of what shows, and a
/// tap aimed at it may land beside it.
pub const REACH_FLOOR: f64 = 8.0;

/// How an action on an element names the keyboard when a tap at the
/// element's centre would land on it (rn-app-fixes, design D38).
pub const KEYBOARD: &str = "keyboard";

/// What of an area a stroke may go through: the area less the system and
/// input-method windows above the window it lies in (task 10.32), since a
/// stroke that comes down on one of them goes to that window, and a
/// keyboard takes a stroke across its keys for typing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uncovered {
    /// The area less those windows, each cutting it as a bar cuts visible
    /// bounds (see [`Visibility::viewport`]): where it spans the area from
    /// one side to the other and reaches one of its edges, so what is left
    /// stays a rectangle, empty when one window covers all of it.
    pub area: Rect,
    /// Whether an input-method window cut the area.
    pub keyboard: bool,
}

/// What of `area` a stroke may go through, when it lies in window `w`, or
/// in the window in front when `w` is `None`: `area` less the system and
/// input-method windows above that window, as [`bars`] has them.
pub(crate) fn uncovered(replica: &Replica, w: Option<usize>, area: Rect) -> Uncovered {
    let pixel = pixel(replica);
    let w = w.unwrap_or_else(|| in_front(replica));
    windows_above(replica, w).fold(
        Uncovered {
            area,
            keyboard: false,
        },
        |left, (kind, bar)| {
            let cut = cut_by(left.area, &bar, pixel);
            Uncovered {
                area: cut,
                keyboard: left.keyboard || (kind == WindowType::INPUT_METHOD && cut != left.area),
            }
        },
    )
}

/// The window in front: the active window, unless it is a keyboard's,
/// which Android flags active while a finger touches it; else the top-most
/// application window; else the bottom-most window.
fn in_front(replica: &Replica) -> usize {
    let windows = &replica.windows;
    windows
        .iter()
        .position(|w| w.flags.contains(WindowFlags::ACTIVE) && w.kind != WindowType::INPUT_METHOD)
        .or_else(|| {
            windows
                .iter()
                .position(|w| w.kind == WindowType::APPLICATION)
        })
        .unwrap_or_else(|| windows.len().saturating_sub(1))
}

/// The system and input-method windows above window `w`, top-most first,
/// with their kinds and bounds: the bars other windows draw over it.
fn windows_above(replica: &Replica, w: usize) -> impl Iterator<Item = (WindowType, Rect)> + '_ {
    replica.windows[..w]
        .iter()
        .filter(|above| matches!(above.kind, WindowType::SYSTEM | WindowType::INPUT_METHOD))
        .map(|above| (above.kind, Rect::from_pixels(above.bounds, replica.scale)))
}

/// One physical pixel in points: bounds that end within it of an edge end
/// there.
fn pixel(replica: &Replica) -> f64 {
    if replica.scale > 0.0 {
        1.0 / replica.scale
    } else {
        1.0
    }
}

/// The scroll containers among node `i` of window `w`'s ancestors, from the
/// innermost out.
fn containers_around(window: &Win, view: &View, w: usize, i: usize) -> Vec<usize> {
    let facts = &view.windows[w];
    std::iter::successors(window.nodes[i].parent, |&a| window.nodes[a].parent)
        .filter(|&a| view::scrolls(&window.nodes[a], &facts[a]))
        .collect()
}

/// What window `w` shows inside the scroll containers `containers` of node
/// `i`, which they hold or which is one of them: the window clipped by
/// every container, and that clip less the bars drawn over it.
fn inside(
    replica: &Replica,
    view: &View,
    w: usize,
    i: usize,
    containers: &[usize],
) -> (Rect, Rect) {
    let facts = &view.windows[w];
    let clip = containers.iter().fold(
        Rect::from_pixels(replica.windows[w].bounds, replica.scale),
        |clip, &c| clip.intersect(&facts[c].rect),
    );
    let pixel = pixel(replica);
    let viewport = bars(replica, view, w, i, containers)
        .iter()
        .fold(clip, |viewport, bar| cut_by(viewport, bar, pixel));
    (clip, viewport)
}

/// How much of node `i` of window `w` is in view, `view` being the
/// replica's view.
pub(crate) fn of(replica: &Replica, view: &View, w: usize, i: usize) -> Visibility {
    reach(replica, view, w, i).0
}

/// How much of node `i` of window `w` is in view and where a tap reaches it
/// (rn-app-fixes, design D37), and what covers it as an action on it names
/// the cover: the view that covers all of it, or, when the views drawn over
/// part of it leave no part of it in view that a tap reaches, what a tap at
/// its centre would land on, one of those views or a bar. `None` for the
/// cover when something of it is left to tap, or nothing of it lies in its
/// visible bounds before the covers, out of view rather than covered, or
/// nothing named lies at its centre. The keyboard is named [`KEYBOARD`].
///
/// The visible bounds are the window clipped by the node's scroll
/// containers less the bars drawn over them, each view over part of the
/// node and across it then keeping the side the node fits in with the
/// least movement ([`across`]), which a stroke that brings it in aims for.
/// What a tap reaches is the node's bounds inside the visible bounds
/// before those views, less every one of them.
pub(crate) fn reach(
    replica: &Replica,
    view: &View,
    w: usize,
    i: usize,
) -> (Visibility, Option<CoverName>) {
    let window = &replica.windows[w];
    let bounds = view.windows[w][i].rect;
    let pixel = pixel(replica);
    let containers = containers_around(window, view, w, i);
    let (clip, shown) = inside(replica, view, w, i, &containers);
    let cut = [
        Direction::Below,
        Direction::Above,
        Direction::Right,
        Direction::Left,
    ]
    .into_iter()
    .find(|&side| {
        ends_at(&bounds, &clip, side, pixel)
            && containers
                .iter()
                .any(|&c| can_scroll(window.nodes[c].scroll, side))
    });
    let whole = view.cover(replica, w, i);
    let over: Vec<(Cover, Rect)> = view.over_part(w, i).collect();
    let viewport = over.iter().fold(shown, |viewport, (_, cover)| {
        across(viewport, &bounds, cover, pixel)
    });
    let in_view = bounds.intersect(&shown);
    let holes: Vec<Rect> = over.iter().map(|&(_, cover)| cover).collect();
    let (pieces, area) = if whole.is_some() || in_view.is_empty() {
        (Vec::new(), 0.0)
    } else {
        (maximal(in_view, &holes), disjoint_area(in_view, &holes))
    };
    let reachable = largest(&bounds, &pieces);
    let full = bounds.width() * bounds.height();
    let fraction = if full > 0.0 {
        (area / full).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let covered = whole.or_else(|| {
        if reachable.is_some() || in_view.is_empty() {
            return None;
        }
        let at = bounds.center();
        over.iter()
            .rev()
            .find(|(_, cover)| cover.contains(at))
            .map(|&(cover, _)| view.name(replica, cover, at))
            .or_else(|| {
                bar_at(replica, view, w, i, &containers, at).map(|b| view.name_bar(replica, w, b))
            })
            .or_else(|| {
                windows_above(replica, w)
                    .any(|(kind, window)| kind == WindowType::INPUT_METHOD && window.contains(at))
                    .then(|| CoverName {
                        by: KEYBOARD.to_owned(),
                        drawn_over: true,
                    })
            })
    });
    let seen = Visibility {
        bounds,
        viewport,
        fraction,
        cut,
        reachable,
    };
    (seen, covered)
}

/// `viewport` less the side of `cover`, a view drawn over part of an element
/// at `bounds`, where the element does not go: one across the element's
/// width keeps what lies above it or below it, one down its height what
/// lies left or right of it, whichever side the element fits in with the
/// least movement, or the one that fits it, or, when neither does, the one
/// with more room. Any other cover, or one beside the element, is a hole
/// that leaves the visible bounds as they are.
fn across(viewport: Rect, bounds: &Rect, cover: &Rect, pixel: f64) -> Rect {
    if !cover.overlaps(bounds) {
        return viewport;
    }
    let wide = cover.left <= bounds.left + pixel && cover.right >= bounds.right - pixel;
    let tall = cover.top <= bounds.top + pixel && cover.bottom >= bounds.bottom - pixel;
    let mut left = viewport;
    match (wide, tall) {
        (true, false) => {
            let range = (viewport.top, viewport.bottom);
            if before(
                range,
                (cover.top, cover.bottom),
                (bounds.top, bounds.bottom),
                pixel,
            ) {
                left.bottom = left.bottom.min(cover.top);
            } else {
                left.top = left.top.max(cover.bottom);
            }
        }
        (false, true) => {
            let range = (viewport.left, viewport.right);
            if before(
                range,
                (cover.left, cover.right),
                (bounds.left, bounds.right),
                pixel,
            ) {
                left.right = left.right.min(cover.left);
            } else {
                left.left = left.left.max(cover.right);
            }
        }
        _ => {}
    }
    left
}

/// Whether an element spanning `element` along an axis goes before a cover
/// spanning `cover` inside `range`, rather than after it (see [`across`]).
fn before(range: (f64, f64), cover: (f64, f64), element: (f64, f64), pixel: f64) -> bool {
    let length = element.1 - element.0;
    let (room_before, room_after) = (cover.0 - range.0, range.1 - cover.1);
    let fits = |room: f64| room >= length - pixel;
    match (fits(room_before), fits(room_after)) {
        (true, true) => (element.1 - cover.0).max(0.0) <= (cover.1 - element.0).max(0.0),
        (true, false) => true,
        (false, true) => false,
        (false, false) => room_before >= room_after,
    }
}

/// The maximal rectangles of `area` that none of `holes` overlaps: each
/// hole leaves of a rectangle it overlaps the parts above, below, left and
/// right of it, and a rectangle another holds goes.
fn maximal(area: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut free = vec![area];
    for hole in holes {
        let mut next: Vec<Rect> = Vec::with_capacity(free.len() + 3);
        for r in free {
            if !r.overlaps(hole) {
                next.push(r);
                continue;
            }
            next.extend(
                [
                    Rect::new(r.left, r.top, r.right, hole.top),
                    Rect::new(r.left, hole.bottom, r.right, r.bottom),
                    Rect::new(r.left, r.top, hole.left, r.bottom),
                    Rect::new(hole.right, r.top, r.right, r.bottom),
                ]
                .into_iter()
                .filter(|piece| !piece.is_empty()),
            );
        }
        free = next
            .iter()
            .enumerate()
            .filter(|&(k, r)| {
                !next
                    .iter()
                    .enumerate()
                    .any(|(j, other)| j != k && holds_rect(other, r) && (other != r || j < k))
            })
            .map(|(_, r)| *r)
            .collect();
    }
    free
}

/// Whether `outer` holds all of `inner`.
fn holds_rect(outer: &Rect, inner: &Rect) -> bool {
    inner.left >= outer.left
        && inner.top >= outer.top
        && inner.right <= outer.right
        && inner.bottom <= outer.bottom
}

/// The area of `area` that none of `holes` overlaps, in square points:
/// each hole leaves of a rectangle it overlaps the bands above and below
/// it and, between them, the parts left and right of it, which do not
/// overlap.
fn disjoint_area(area: Rect, holes: &[Rect]) -> f64 {
    let mut free = vec![area];
    for hole in holes {
        let mut next: Vec<Rect> = Vec::with_capacity(free.len() + 3);
        for r in free {
            if !r.overlaps(hole) {
                next.push(r);
                continue;
            }
            let (top, bottom) = (r.top.max(hole.top), r.bottom.min(hole.bottom));
            next.extend(
                [
                    Rect::new(r.left, r.top, r.right, hole.top),
                    Rect::new(r.left, hole.bottom, r.right, r.bottom),
                    Rect::new(r.left, top, hole.left, bottom),
                    Rect::new(hole.right, top, r.right, bottom),
                ]
                .into_iter()
                .filter(|piece| !piece.is_empty()),
            );
        }
        free = next;
    }
    free.iter().map(|r| r.width() * r.height()).sum()
}

/// The tab bar or toolbar of window `w` drawn over node `i`, whose scroll
/// containers are `containers`, that holds `at`, among those [`bars`] cuts
/// its visible bounds by.
fn bar_at(
    replica: &Replica,
    view: &View,
    w: usize,
    i: usize,
    containers: &[usize],
    at: Point,
) -> Option<usize> {
    let window = &replica.windows[w];
    view.windows[w]
        .iter()
        .enumerate()
        .position(|(b, f)| node_bar(window, view, w, b, i, containers) && f.rect.contains(at))
}

/// The largest of `pieces`, parts of an element at `bounds`, that is at
/// least [`REACH_FLOOR`] points on each side, or as long as the element is
/// on a side where it is shorter.
fn largest(bounds: &Rect, pieces: &[Rect]) -> Option<Rect> {
    let (wide, tall) = (
        REACH_FLOOR.min(bounds.width()),
        REACH_FLOOR.min(bounds.height()),
    );
    let area = |piece: &Rect| piece.width() * piece.height();
    pieces
        .iter()
        .filter(|piece| !piece.is_empty() && piece.width() >= wide && piece.height() >= tall)
        .copied()
        .reduce(|best, piece| {
            if area(&piece) > area(&best) {
                piece
            } else {
                best
            }
        })
}

/// The visible bounds of the scroll container that a stroke along the
/// vertical axis, or the horizontal one, through `at` moves: the innermost
/// node on screen and not covered that scrolls ([`view::scrolls`]) along
/// that axis, or along an axis its platform does not name, and whose bounds
/// hold `at`, in the top-most window that has one. They are what an element
/// inside it sees of it (see [`Visibility::viewport`]): its bounds clipped
/// by its window and the scroll containers around it, less the bars drawn
/// over them. `None` when no such node holds `at`.
pub(crate) fn scroller_at(
    replica: &Replica,
    view: &View,
    at: Point,
    vertical: bool,
) -> Option<Rect> {
    let axis = if vertical {
        ScrollAxis::VERTICAL
    } else {
        ScrollAxis::HORIZONTAL
    };
    replica.windows.iter().enumerate().find_map(|(w, window)| {
        let facts = &view.windows[w];
        // Nodes come in pre-order, so the last one that holds the point is
        // the innermost, or one drawn over it.
        let c = (0..window.nodes.len()).rev().find(|&c| {
            let (node, f) = (&window.nodes[c], &facts[c]);
            f.visible
                && f.hidden == Hidden::No
                && view::scrolls(node, f)
                && node
                    .scroll
                    .is_none_or(|scroll| scroll.axis == axis || scroll.axis == ScrollAxis::UNKNOWN)
                && f.rect.contains(at)
        })?;
        Some(container_shows(replica, view, w, c))
    })
}

/// What scroll container `c` of window `w` shows: its bounds clipped by
/// its window and the scroll containers around it, less the bars drawn
/// over them, as an element inside it sees them (see
/// [`Visibility::viewport`]), each view over part of it and across it
/// keeping the side it fits in.
fn container_shows(replica: &Replica, view: &View, w: usize, c: usize) -> Rect {
    let window = &replica.windows[w];
    let mut containers = vec![c];
    containers.extend(containers_around(window, view, w, c));
    let shown = inside(replica, view, w, c, &containers).1;
    let bounds = view.windows[w][c].rect;
    let pixel = pixel(replica);
    view.over_part(w, c).fold(shown, |shown, (_, cover)| {
        across(shown, &bounds, &cover, pixel)
    })
}

/// What shows of the scroll container that shows the most of `area`, in
/// the top-most window that has one: among the nodes on screen and not
/// covered that scroll along the vertical axis, or the horizontal one, or
/// along an axis their platform does not name, the one whose visible
/// bounds ([`container_shows`]) share the most of `area`. A stroke of the
/// screen with the keyboard up goes through it, rather than through what
/// the keyboard leaves of the screen, where a header may lie (rn-app-fixes,
/// design D41). `None` when no such node shares any of `area`.
pub(crate) fn scroller_in(
    replica: &Replica,
    view: &View,
    area: Rect,
    vertical: bool,
) -> Option<Rect> {
    let axis = if vertical {
        ScrollAxis::VERTICAL
    } else {
        ScrollAxis::HORIZONTAL
    };
    replica.windows.iter().enumerate().find_map(|(w, window)| {
        let facts = &view.windows[w];
        let mut best: Option<(f64, Rect)> = None;
        for (c, node) in window.nodes.iter().enumerate() {
            let f = &facts[c];
            let along = node
                .scroll
                .is_none_or(|scroll| scroll.axis == axis || scroll.axis == ScrollAxis::UNKNOWN);
            if !f.visible || f.hidden != Hidden::No || !view::scrolls(node, f) || !along {
                continue;
            }
            let shown = container_shows(replica, view, w, c);
            let shared = shown.intersect(&area);
            let size = shared.width() * shared.height();
            if !shared.is_empty() && best.is_none_or(|(most, _)| size > most) {
                best = Some((size, shown));
            }
        }
        best.map(|(_, shown)| shown)
    })
}

/// Whether `bounds` ends at `clip`'s edge on `side`, within `pixel`, rather
/// than short of it or beyond it.
fn ends_at(bounds: &Rect, clip: &Rect, side: Direction, pixel: f64) -> bool {
    let (edge, limit) = match side {
        Direction::Above => (bounds.top, clip.top),
        Direction::Below => (bounds.bottom, clip.bottom),
        Direction::Left => (bounds.left, clip.left),
        Direction::Right => (bounds.right, clip.right),
    };
    (edge - limit).abs() <= pixel
}

/// Whether a container with scroll semantics `scroll` can scroll toward
/// `side`: along that side's axis, or an axis its platform does not name,
/// back toward the start above or left and forward toward the end below or
/// right, as scroll hints read them. Without semantics it cannot tell, and
/// the answer is no.
fn can_scroll(scroll: Option<Scroll>, side: Direction) -> bool {
    let Some(scroll) = scroll else {
        return false;
    };
    let (axis, way) = match side {
        Direction::Above => (ScrollAxis::VERTICAL, ScrollFlags::BACK),
        Direction::Below => (ScrollAxis::VERTICAL, ScrollFlags::FORWARD),
        Direction::Left => (ScrollAxis::HORIZONTAL, ScrollFlags::BACK),
        Direction::Right => (ScrollAxis::HORIZONTAL, ScrollFlags::FORWARD),
    };
    (scroll.axis == axis || scroll.axis == ScrollAxis::UNKNOWN) && scroll.flags.contains(way)
}

/// The bars drawn over node `i` of window `w`, whose scroll containers are
/// `containers`: system and keyboard windows above its window, and the tab
/// bars and toolbars on screen in its window other than those holding it
/// and those inside its scroll containers, which scroll with it.
fn bars(replica: &Replica, view: &View, w: usize, i: usize, containers: &[usize]) -> Vec<Rect> {
    let mut out: Vec<Rect> = windows_above(replica, w).map(|(_, bar)| bar).collect();
    let window = &replica.windows[w];
    for (b, f) in view.windows[w].iter().enumerate() {
        if node_bar(window, view, w, b, i, containers) {
            out.push(f.rect);
        }
    }
    out
}

/// Whether node `b` of window `w` is a tab bar or toolbar on screen drawn
/// over node `i`, whose scroll containers are `containers`: neither
/// holding it nor inside one of its scroll containers, which scroll it
/// with the node.
fn node_bar(window: &Win, view: &View, w: usize, b: usize, i: usize, containers: &[usize]) -> bool {
    let f = &view.windows[w][b];
    matches!(f.role, Role::Tablist | Role::Toolbar)
        && f.visible
        && f.hidden == Hidden::No
        && !holds(window, b, i)
        && !containers.iter().any(|&c| holds(window, c, b))
}

/// Whether node `outer` of `window` is node `inner` or one of its
/// ancestors.
fn holds(window: &Win, outer: usize, inner: usize) -> bool {
    std::iter::successors(Some(inner), |&n| window.nodes[n].parent).any(|n| n == outer)
}

/// `viewport` less `bar` where the bar spans it from one side to the other
/// and reaches one of its edges; a bar over all of it leaves nothing, and
/// any other bar leaves it as it is.
fn cut_by(viewport: Rect, bar: &Rect, pixel: f64) -> Rect {
    if !viewport.overlaps(bar) {
        return viewport;
    }
    let across = bar.left <= viewport.left + pixel && bar.right >= viewport.right - pixel;
    let down = bar.top <= viewport.top + pixel && bar.bottom >= viewport.bottom - pixel;
    let mut left = viewport;
    match (across, down) {
        (true, true) => {
            return Rect::new(viewport.left, viewport.top, viewport.left, viewport.top);
        }
        (true, false) => {
            if bar.top <= viewport.top + pixel {
                left.top = bar.bottom;
            } else if bar.bottom >= viewport.bottom - pixel {
                left.bottom = bar.top;
            }
        }
        (false, true) => {
            if bar.left <= viewport.left + pixel {
                left.left = bar.right;
            } else if bar.right >= viewport.right - pixel {
                left.right = bar.left;
            }
        }
        (false, false) => {}
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXEL: f64 = 1.0 / 2.625;

    #[test]
    fn a_bar_across_the_bottom_or_top_cuts_that_edge() {
        let list = Rect::new(0.0, 0.0, 400.0, 800.0);
        let tab_bar = Rect::new(0.0, 717.0, 400.0, 874.0);
        assert_eq!(
            cut_by(list, &tab_bar, PIXEL),
            Rect::new(0.0, 0.0, 400.0, 717.0)
        );
        let navigation_bar = Rect::new(0.0, 0.0, 400.0, 62.0);
        assert_eq!(
            cut_by(list, &navigation_bar, PIXEL),
            Rect::new(0.0, 62.0, 400.0, 800.0)
        );
    }

    #[test]
    fn a_bar_that_does_not_span_the_bounds_or_reach_an_edge_cuts_nothing() {
        let list = Rect::new(0.0, 100.0, 400.0, 800.0);
        let button = Rect::new(300.0, 700.0, 380.0, 780.0);
        assert_eq!(cut_by(list, &button, PIXEL), list);
        let middle = Rect::new(0.0, 400.0, 400.0, 450.0);
        assert_eq!(cut_by(list, &middle, PIXEL), list);
        let elsewhere = Rect::new(0.0, 820.0, 400.0, 900.0);
        assert_eq!(cut_by(list, &elsewhere, PIXEL), list);
        let side = Rect::new(0.0, 0.0, 50.0, 900.0);
        assert_eq!(
            cut_by(list, &side, PIXEL),
            Rect::new(50.0, 100.0, 400.0, 800.0)
        );
    }

    #[test]
    fn a_bar_over_all_of_the_bounds_leaves_nothing() {
        let list = Rect::new(0.0, 600.0, 400.0, 800.0);
        let keyboard = Rect::new(0.0, 500.0, 400.0, 900.0);
        assert!(cut_by(list, &keyboard, PIXEL).is_empty());
    }

    /// What covers leave of an area: its largest rectangles, which may
    /// overlap, and its area, counted once.
    #[test]
    fn what_holes_leave_of_an_area_is_its_maximal_rectangles_and_its_area_once() {
        let card = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(maximal(card, &[]), [card]);
        // A corner cut out leaves two rectangles that overlap in the other
        // corner, and three quarters of the area.
        let corner = Rect::new(50.0, 50.0, 150.0, 150.0);
        assert_eq!(
            maximal(card, &[corner]),
            [
                Rect::new(0.0, 0.0, 100.0, 50.0),
                Rect::new(0.0, 0.0, 50.0, 100.0)
            ]
        );
        assert_eq!(disjoint_area(card, &[corner]), 7500.0);
        // A band across the middle leaves what lies above and below it.
        let band = Rect::new(-10.0, 40.0, 110.0, 60.0);
        assert_eq!(
            maximal(card, &[band]),
            [
                Rect::new(0.0, 0.0, 100.0, 40.0),
                Rect::new(0.0, 60.0, 100.0, 100.0)
            ]
        );
        assert_eq!(disjoint_area(card, &[band]), 8000.0);
        // A hole that holds the area leaves nothing.
        let over = Rect::new(-1.0, -1.0, 101.0, 101.0);
        assert!(maximal(card, &[over]).is_empty());
        assert_eq!(disjoint_area(card, &[over, corner]), 0.0);
    }

    /// A tap needs 8 points of an element on each side, or all of it on a
    /// side where it is shorter.
    #[test]
    fn a_tap_goes_to_the_largest_part_at_least_8_points_on_each_side() {
        let row = Rect::new(0.0, 0.0, 300.0, 48.0);
        let thin = Rect::new(0.0, 0.0, 300.0, 7.0);
        let low = Rect::new(0.0, 38.0, 300.0, 48.0);
        assert_eq!(largest(&row, &[thin, low]), Some(low));
        assert_eq!(largest(&row, &[thin]), None);
        let dot = Rect::new(0.0, 0.0, 6.0, 6.0);
        assert_eq!(largest(&dot, &[dot]), Some(dot));
        assert_eq!(largest(&row, &[]), None);
    }

    /// A cover across an element keeps the side of it the element fits in
    /// with the least movement: above a footer button the heading half
    /// under it, below a header the card most of whose top it covers, and
    /// above a disabled button the control 38 of whose 48 points it covers,
    /// where 27 points below it would not hold the control. A cover that
    /// spans neither way is a hole that keeps the visible bounds.
    #[test]
    fn a_cover_across_an_element_keeps_the_side_it_fits_in_with_the_least_movement() {
        let list = Rect::new(0.0, 62.0, 402.0, 874.0);
        let heading = Rect::new(33.0, 709.0, 90.33, 731.0);
        let footer = Rect::new(20.0, 720.0, 382.0, 764.0);
        assert_eq!(
            across(list, &heading, &footer, PIXEL),
            Rect::new(0.0, 62.0, 402.0, 720.0)
        );
        let card = Rect::new(37.0, 41.0, 365.0, 157.0);
        let header = Rect::new(0.0, 0.0, 402.0, 122.0);
        let full = Rect::new(0.0, 0.0, 402.0, 791.0);
        assert_eq!(
            across(full, &card, &header, PIXEL),
            Rect::new(0.0, 122.0, 402.0, 791.0)
        );
        let tab = Rect::new(0.0, 122.0, 402.0, 791.0);
        let paste = Rect::new(205.0, 726.0, 382.0, 774.0);
        let disabled = Rect::new(20.0, 708.0, 382.0, 764.0);
        assert_eq!(
            across(tab, &paste, &disabled, PIXEL),
            Rect::new(0.0, 122.0, 402.0, 708.0)
        );
        let inside = Rect::new(50.0, 715.0, 70.0, 725.0);
        assert_eq!(across(list, &heading, &inside, PIXEL), list);
        let beside = Rect::new(300.0, 700.0, 380.0, 780.0);
        assert_eq!(across(list, &heading, &beside, PIXEL), list);
    }
}
