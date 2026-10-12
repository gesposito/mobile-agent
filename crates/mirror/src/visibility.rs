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
use crate::view::{self, Hidden, View};

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
    let window = &replica.windows[w];
    let bounds = view.windows[w][i].rect;
    let pixel = pixel(replica);
    let containers = containers_around(window, view, w, i);
    let (clip, viewport) = inside(replica, view, w, i, &containers);
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
    Visibility {
        bounds,
        viewport,
        fraction: share(&bounds, &viewport),
        cut,
        reachable: largest(&bounds, &[bounds.intersect(&viewport)]),
    }
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
        .reduce(|best, piece| if area(&piece) > area(&best) { piece } else { best })
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
        let mut containers = vec![c];
        containers.extend(containers_around(window, view, w, c));
        Some(inside(replica, view, w, c, &containers).1)
    })
}

/// The share of `bounds` inside `viewport`.
fn share(bounds: &Rect, viewport: &Rect) -> f64 {
    let area = bounds.width() * bounds.height();
    if area <= 0.0 {
        return 0.0;
    }
    let shown = bounds.intersect(viewport);
    (shown.width() * shown.height() / area).clamp(0.0, 1.0)
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
    let facts = &view.windows[w];
    for (b, f) in facts.iter().enumerate() {
        if !matches!(f.role, Role::Tablist | Role::Toolbar) || !f.visible || f.hidden != Hidden::No
        {
            continue;
        }
        if holds(window, b, i) || containers.iter().any(|&c| holds(window, c, b)) {
            continue;
        }
        out.push(f.rect);
    }
    out
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

    #[test]
    fn the_share_in_view_is_by_area_and_zero_without_one() {
        let viewport = Rect::new(0.0, 0.0, 400.0, 800.0);
        assert_eq!(share(&Rect::new(0.0, 776.0, 400.0, 824.0), &viewport), 0.5);
        assert_eq!(share(&Rect::new(0.0, 100.0, 400.0, 148.0), &viewport), 1.0);
        assert_eq!(share(&Rect::new(0.0, 900.0, 400.0, 948.0), &viewport), 0.0);
        assert_eq!(share(&Rect::new(0.0, 100.0, 400.0, 100.0), &viewport), 0.0);
    }
}
