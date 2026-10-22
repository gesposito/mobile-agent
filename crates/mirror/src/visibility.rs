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
//!
//! What a tap reaches of an element is clipped by what each of its scroll
//! containers shows, as a stroke through it knows it, before the views
//! drawn over the element are taken away, and an interactive listing marks
//! the share of an element a tap reaches when it is less than all, and
//! leaves out one it reaches none of (rn-app-fixes, design D45).

use protocol::snapshot::{Scroll, ScrollAxis, ScrollFlags, WindowFlags, WindowType};

use crate::geometry::{Point, Rect};
use crate::replica::{Replica, Win};
use crate::snapshot::{Direction, Role};
use crate::view::{self, Cover, CoverName, Facts, Hidden, View};

/// How much of an element is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visibility {
    /// The element's bounds as the replica has them, in points.
    pub bounds: Rect,
    /// The visible bounds: the element's window, clipped by the scroll
    /// containers that hold the element and by what each of them shows,
    /// less the bars drawn over them, and less the side of each view drawn
    /// over part of the element and across it that the element does not fit
    /// in with the least movement.
    pub viewport: Rect,
    /// The share of `bounds` a tap reaches, from 0 to 1: inside the visible
    /// bounds before the views drawn over part of the element, less those
    /// views; 0 for bounds without an area or an element covered whole.
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

/// Whether a keyboard window above the window in front draws nothing on the
/// screen, its bounds being what the drivers say it draws (rn-app-fixes,
/// design D52).
pub(crate) fn keys_hidden(replica: &Replica) -> bool {
    windows_above(replica, in_front(replica))
        .any(|(kind, bounds)| kind == WindowType::INPUT_METHOD && bounds.is_empty())
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

/// The scroll containers among `start` and its ancestors in `window`, from
/// the innermost out, `facts` being the window's: those of a node from its
/// parent, and a scroll container with those around it from itself.
fn scrolling<'v>(
    window: &'v Win,
    facts: &'v [Facts],
    start: Option<usize>,
) -> impl Iterator<Item = usize> + Clone + 'v {
    std::iter::successors(start, move |&a| window.nodes[a].parent)
        .filter(move |&a| view::scrolls(&window.nodes[a], &facts[a]))
}

/// What window `w` shows inside the scroll containers `containers` of node
/// `i`, which they hold or which is one of them: the window clipped by
/// every container, and that clip less the bars drawn over it (see
/// [`bars`]), of the tab bars and toolbars on screen in the window
/// `bar_nodes`.
fn inside(
    replica: &Replica,
    view: &View,
    (w, i): (usize, usize),
    containers: impl Iterator<Item = usize> + Clone,
    bar_nodes: &[(usize, usize)],
) -> (Rect, Rect) {
    let facts = &view.windows[w];
    let clip = containers.clone().fold(
        Rect::from_pixels(replica.windows[w].bounds, replica.scale),
        |clip, c| clip.intersect(&facts[c].rect),
    );
    let pixel = pixel(replica);
    let viewport = bars(replica, view, (w, i), containers, bar_nodes)
        .fold(clip, |viewport, bar| cut_by(viewport, &bar, pixel));
    (clip, viewport)
}

/// How much of node `i` of window `w` is in view, `view` being the
/// replica's view.
pub(crate) fn of(replica: &Replica, view: &View, w: usize, i: usize) -> Visibility {
    reach(replica, view, w, i).0
}

/// How much of node `i` of window `w` is in view and where a tap reaches it,
/// and what covers it as an action on it names the cover (see
/// [`Sight::reach`]).
pub(crate) fn reach(
    replica: &Replica,
    view: &View,
    w: usize,
    i: usize,
) -> (Visibility, Option<CoverName>) {
    Sight::new(replica, view).reach(w, i)
}

/// Where a tap lands as the replica has it (design D1 of action-verdicts):
/// on the keyboard of window `w`, or on node `i` of window `w`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Landing {
    Keyboard(usize),
    Node(usize, usize),
}

/// What a tap at `at` lands on (design D1 of action-verdicts): the top-most
/// element with a ref that a tap reaches there, as the listings judge what
/// a tap reaches of an element ([`Sight::reaches`]): of the windows, top-most
/// first, the first that holds one, and in it the last its tree lists, the
/// innermost or the one drawn over the others, among the elements on screen
/// that no other folds in; or a keyboard window that holds `at` before such a
/// window, which takes the tap where it draws. `None` when no such element
/// lies under `at`.
pub(crate) fn landing(replica: &Replica, view: &View, at: Point) -> Option<Landing> {
    let mut sight = Sight::new(replica, view);
    for (w, window) in replica.windows.iter().enumerate() {
        if !Rect::from_pixels(window.bounds, replica.scale).contains(at) {
            continue;
        }
        if window.kind == WindowType::INPUT_METHOD {
            if window.nodes.is_empty() {
                continue;
            }
            return Some(Landing::Keyboard(w));
        }
        let facts = &view.windows[w];
        let found = (0..window.nodes.len()).rev().find(|&i| {
            let f = &facts[i];
            f.visible && f.bears_ref && !f.absorbed && sight.reaches(w, i, at)
        });
        if let Some(i) = found {
            return Some(Landing::Node(w, i));
        }
    }
    None
}

/// What the views and bars of a replica leave of its elements, keeping what
/// it works out on the way for the next element: the visible bounds of
/// each scroll container met.
pub(crate) struct Sight<'a> {
    replica: &'a Replica,
    view: &'a View,
    /// The tab bars and toolbars on screen and not covered, by window and
    /// node, in window order.
    bars: Vec<(usize, usize)>,
    /// The visible bounds of the scroll container worked out first, by
    /// window and node, and those of the others, which most screens lack.
    shown: Option<((usize, usize), Rect)>,
    more: Vec<((usize, usize), Rect)>,
}

impl<'a> Sight<'a> {
    pub(crate) fn new(replica: &'a Replica, view: &'a View) -> Sight<'a> {
        let bars = view
            .windows
            .iter()
            .enumerate()
            .flat_map(|(w, facts)| {
                facts
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| {
                        matches!(f.role, Role::Tablist | Role::Toolbar)
                            && f.visible
                            && f.hidden == Hidden::No
                    })
                    .map(move |(b, _)| (w, b))
            })
            .collect();
        Sight {
            replica,
            view,
            bars,
            shown: None,
            more: Vec::new(),
        }
    }

    /// The tab bars and toolbars on screen in window `w`, not covered, by
    /// window and node.
    fn bars_of(&self, w: usize) -> &[(usize, usize)] {
        let from = self.bars.partition_point(|&(bw, _)| bw < w);
        let to = self.bars.partition_point(|&(bw, _)| bw <= w);
        &self.bars[from..to]
    }

    /// What scroll container `c` of window `w` shows: its bounds clipped by
    /// its window and the scroll containers around it, less the bars drawn
    /// over them, as an element inside it sees them (see
    /// [`Visibility::viewport`]), each view over part of it and across it
    /// keeping the side it fits in.
    fn shows(&mut self, w: usize, c: usize) -> Rect {
        if let Some(&(_, shown)) = self
            .shown
            .iter()
            .chain(&self.more)
            .find(|(at, _)| *at == (w, c))
        {
            return shown;
        }
        let (replica, view) = (self.replica, self.view);
        let (window, facts) = (&replica.windows[w], &view.windows[w]);
        let containers = scrolling(window, facts, Some(c));
        let shown = inside(replica, view, (w, c), containers, self.bars_of(w)).1;
        let bounds = facts[c].rect;
        let pixel = pixel(replica);
        let shown = view.over_part(w, c).fold(shown, |shown, (_, cover)| {
            across(shown, &bounds, &cover, pixel, Carry::ANY)
        });
        if self.shown.is_none() {
            self.shown = Some(((w, c), shown));
        } else {
            self.more.push(((w, c), shown));
        }
        shown
    }

    /// How much of node `i` of window `w` is in view and where a tap
    /// reaches it (rn-app-fixes, design D37), and what covers it as an
    /// action on it names the cover: the view that covers all of it, or,
    /// when the views drawn over part of it leave no part of it in view that
    /// a tap reaches, what a tap at its centre would land on, one of those
    /// views, one drawn over part of a scroll container that holds it, a bar
    /// or the keyboard ([`KEYBOARD`]). `None` for the cover when something
    /// of it is left to tap, or nothing of it lies in its window and scroll
    /// containers less the bars over them, out of view rather than covered,
    /// or nothing named lies at its centre.
    ///
    /// The visible bounds are the window clipped by the node's scroll
    /// containers less the bars drawn over them, and clipped by what each of
    /// those containers shows ([`Sight::shows`]), as a stroke through it
    /// knows it (design D45): a view drawn over part of a container and
    /// across it keeps of it the side it fits in, whether or not the walk
    /// below the view met the node, which a row whose frame ends at a
    /// footer's top keeps it from. Each view over part of the node and
    /// across it then keeps the side the node fits in with the least
    /// movement ([`across`]), which a stroke that brings it in aims for.
    /// What a tap reaches is the node's bounds inside the visible bounds
    /// before those views, less every one of them.
    pub(crate) fn reach(&mut self, w: usize, i: usize) -> (Visibility, Option<CoverName>) {
        let (replica, view) = (self.replica, self.view);
        let (window, facts) = (&replica.windows[w], &view.windows[w]);
        let bounds = facts[i].rect;
        let pixel = pixel(replica);
        let containers = scrolling(window, facts, window.nodes[i].parent);
        let (clip, before) = inside(replica, view, (w, i), containers.clone(), self.bars_of(w));
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
                    .clone()
                    .any(|c| can_scroll(window.nodes[c].scroll, side))
        });
        let shown = containers
            .clone()
            .fold(before, |shown, c| shown.intersect(&self.shows(w, c)));
        let whole = view.cover(replica, w, i);
        let over: Vec<(Cover, Rect)> = view.over_part(w, i).collect();
        let carry = Carry::of(window, containers.clone());
        let viewport = over.iter().fold(shown, |viewport, (_, cover)| {
            across(viewport, &bounds, cover, pixel, carry)
        });
        let in_view = bounds.intersect(&shown);
        let holes: Vec<Rect> = over.iter().map(|&(_, cover)| cover).collect();
        let (reachable, area) = if whole.is_some() || in_view.is_empty() {
            (None, 0.0)
        } else if holes.is_empty() {
            (
                largest(&bounds, &[in_view]),
                in_view.width() * in_view.height(),
            )
        } else {
            let pieces = maximal(in_view, &holes);
            (largest(&bounds, &pieces), disjoint_area(in_view, &holes))
        };
        let full = bounds.width() * bounds.height();
        let fraction = if full > 0.0 {
            (area / full).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let covered = whole.or_else(|| {
            if reachable.is_some() || bounds.intersect(&before).is_empty() {
                return None;
            }
            let at = bounds.center();
            let under = |node: usize| {
                let over: Vec<(Cover, Rect)> = view.over_part(w, node).collect();
                over.into_iter()
                    .rev()
                    .find(|(_, cover)| cover.contains(at))
                    .map(|(cover, _)| view.cover_name(replica, (w, i), cover, at))
            };
            under(i)
                .or_else(|| containers.clone().find_map(under))
                .or_else(|| {
                    bar_at(replica, view, (w, i), containers, self.bars_of(w), at)
                        .map(|b| view.name_bar(replica, w, b))
                })
                .or_else(|| {
                    windows_above(replica, w)
                        .any(|(kind, window)| {
                            kind == WindowType::INPUT_METHOD && window.contains(at)
                        })
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

    /// Whether a tap at `at` reaches node `i` of window `w`, as
    /// [`Sight::reach`] has what a tap reaches of it (design D1 of
    /// action-verdicts): `at` lies in its bounds inside its visible bounds
    /// before the views drawn over part of it, under none of those views,
    /// and no view covers all of it.
    pub(crate) fn reaches(&mut self, w: usize, i: usize, at: Point) -> bool {
        let (replica, view) = (self.replica, self.view);
        let (window, facts) = (&replica.windows[w], &view.windows[w]);
        if facts[i].hidden != Hidden::No || !facts[i].rect.contains(at) {
            return false;
        }
        let containers = scrolling(window, facts, window.nodes[i].parent);
        let (_, before) = inside(replica, view, (w, i), containers.clone(), self.bars_of(w));
        let shown = containers.fold(before, |shown, c| shown.intersect(&self.shows(w, c)));
        shown.contains(at) && !view.over_part(w, i).any(|(_, cover)| cover.contains(at))
    }
}

/// The share of each element of an interactive listing that a tap reaches
/// (rn-app-fixes, design D45), which the listing marks when it is less than
/// all and leaves out, counting it in a hint, when it is none.
pub(crate) struct Reaches<'a> {
    replica: &'a Replica,
    /// The share of each node of each window in turn.
    shares: Vec<Option<f32>>,
}

impl Reaches<'_> {
    /// The share of node `i` of window `w` a tap reaches, 0 for one a view
    /// drawn over it covers whole; `None` for a node the listing leaves out
    /// for another reason: one off the screen, cut by a modal, folded into
    /// another or without a ref, and one wholly under a keyboard, which the
    /// listing counts under it.
    pub(crate) fn of(&self, w: usize, i: usize) -> Option<f64> {
        let start: usize = self.replica.windows[..w]
            .iter()
            .map(|window| window.nodes.len())
            .sum();
        self.shares[start + i].map(f64::from)
    }
}

/// The share of each element of an interactive listing that a tap reaches
/// (see [`Reaches`]). Most elements lie within what their scroll containers
/// show, or within their window and clear of its bars, with no view over
/// part of them, and a tap reaches all of them; only the rest are measured
/// ([`Sight::reach`]).
pub(crate) fn listed<'a>(replica: &'a Replica, view: &View) -> Reaches<'a> {
    let mut sight = Sight::new(replica, view);
    let mut shares = Vec::with_capacity(replica.windows.iter().map(|w| w.nodes.len()).sum());
    for (w, window) in replica.windows.iter().enumerate() {
        if window.kind == WindowType::INPUT_METHOD {
            shares.extend(std::iter::repeat_n(None, window.nodes.len()));
            continue;
        }
        let facts = &view.windows[w];
        let area = Rect::from_pixels(window.bounds, replica.scale);
        let keyboards = keyboards_above(replica, w);
        // What the scroll containers around the children of the parent met
        // last show, `None` under no container: a list's rows and a grid's
        // cells share a parent.
        let mut last: Option<(Option<usize>, Option<Rect>)> = None;
        for (i, node) in window.nodes.iter().enumerate() {
            let f = &facts[i];
            if !f.visible || !f.bears_ref || f.absorbed {
                shares.push(None);
                continue;
            }
            if f.hidden != Hidden::No {
                // A view drawn over it whole leaves a tap none of it, which
                // the full listing marks (rn-app-fixes, design D57).
                shares.push(view.drawn_over_whole(w, i).then_some(0.0));
                continue;
            }
            if beside(&f.rect, &keyboards).is_some_and(|share| share <= 0.0) {
                shares.push(None);
                continue;
            }
            let clip = match last {
                Some((parent, clip)) if parent == node.parent => clip,
                _ => {
                    let mut containers = scrolling(window, facts, node.parent).peekable();
                    let clip = containers.peek().is_some().then(|| {
                        containers.fold(area, |shown, c| shown.intersect(&sight.shows(w, c)))
                    });
                    last = Some((node.parent, clip));
                    clip
                }
            };
            let whole = view.over_part(w, i).next().is_none()
                && if let Some(shown) = clip {
                    holds_rect(&shown, &f.rect)
                } else {
                    holds_rect(&area, &f.rect)
                        && !windows_above(replica, w).any(|(_, bar)| bar.overlaps(&f.rect))
                        && !sight
                            .bars_of(w)
                            .iter()
                            .any(|&(_, b)| facts[b].rect.overlaps(&f.rect))
                };
            shares.push(Some(if whole {
                1.0
            } else {
                sight.reach(w, i).0.fraction as f32
            }));
        }
    }
    Reaches { replica, shares }
}

/// The bounds of the keyboard windows above window `w`, which are what the
/// drivers say a keyboard draws (tasks 10.21, 10.25), less the place a
/// keyboard's host keeps for an input accessory view, which the app draws
/// (real-app-fixes, task 5.6).
pub(crate) fn keyboards_above(replica: &Replica, w: usize) -> Vec<Rect> {
    replica.windows[..w]
        .iter()
        .filter(|above| above.kind == WindowType::INPUT_METHOD)
        .map(|above| Rect::from_pixels(above.bounds, replica.scale))
        .collect()
}

/// The share of `rect` beside the keyboards at `keyboards`, 0 for one wholly
/// under them; `None` when none of them lies over it.
pub(crate) fn beside(rect: &Rect, keyboards: &[Rect]) -> Option<f64> {
    let area = rect.width() * rect.height();
    if keyboards.is_empty() || area <= 0.0 {
        return None;
    }
    let under: f64 = keyboards
        .iter()
        .map(|keyboard| {
            let shared = rect.intersect(keyboard);
            shared.width() * shared.height()
        })
        .sum();
    (under > 0.0).then(|| (1.0 - under / area).clamp(0.0, 1.0))
}

/// Which ways a stroke can carry an element along each axis, toward its
/// start (up or left) and toward its end (down or right).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Carry {
    vertical: (bool, bool),
    horizontal: (bool, bool),
}

impl Carry {
    /// Every way, for what no stroke is asked to carry.
    const ANY: Carry = Carry {
        vertical: (true, true),
        horizontal: (true, true),
    };

    /// The ways the scroll containers `containers` of `window`, from the
    /// innermost out, carry what they hold (rn-app-fixes, design D46): up
    /// or left when one of them scrolls forward along that axis, its content
    /// moving that way, and down or right when one scrolls back; every way
    /// when one says nothing of the ways it scrolls, or when there is none.
    fn of(window: &Win, containers: impl Iterator<Item = usize> + Clone) -> Carry {
        if containers.clone().next().is_none() {
            return Carry::ANY;
        }
        let way = |side: Direction| {
            containers.clone().any(|c| {
                let scroll = window.nodes[c].scroll;
                scroll.is_none() || can_scroll(scroll, side)
            })
        };
        Carry {
            vertical: (way(Direction::Below), way(Direction::Above)),
            horizontal: (way(Direction::Right), way(Direction::Left)),
        }
    }
}

/// `viewport` less the side of `cover`, a view drawn over part of an element
/// at `bounds`, where the element does not go: one across the element's
/// width keeps what lies above it or below it, one down its height what
/// lies left or right of it, whichever side the element fits in with the
/// least movement, or the one that fits it, or, when neither does, the one
/// with more room, of the sides a stroke can `carry` the element to: a list
/// that stands at its top cannot bring what it holds below a floating button
/// (rn-app-fixes, design D46). Any other cover, or one beside the element,
/// is a hole that leaves the visible bounds as they are.
fn across(viewport: Rect, bounds: &Rect, cover: &Rect, pixel: f64, carry: Carry) -> Rect {
    if !cover.overlaps(bounds) {
        return viewport;
    }
    let wide = cover.left <= bounds.left + pixel && cover.right >= bounds.right - pixel;
    let tall = cover.top <= bounds.top + pixel && cover.bottom >= bounds.bottom - pixel;
    let mut left = viewport;
    match (wide, tall) {
        (true, false) => {
            let range = (viewport.top, viewport.bottom);
            let spans = ((cover.top, cover.bottom), (bounds.top, bounds.bottom));
            if before(range, spans, pixel, carry.vertical) {
                left.bottom = left.bottom.min(cover.top);
            } else {
                left.top = left.top.max(cover.bottom);
            }
        }
        (false, true) => {
            let range = (viewport.left, viewport.right);
            let spans = ((cover.left, cover.right), (bounds.left, bounds.right));
            if before(range, spans, pixel, carry.horizontal) {
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
/// spanning `cover` inside `range`, rather than after it (see [`across`]),
/// a stroke carrying it toward the axis's start and its end as `carry`
/// says.
fn before(
    range: (f64, f64),
    (cover, element): ((f64, f64), (f64, f64)),
    pixel: f64,
    (to_start, to_end): (bool, bool),
) -> bool {
    let length = element.1 - element.0;
    let (room_before, room_after) = (cover.0 - range.0, range.1 - cover.1);
    let fits = |room: f64| room >= length - pixel;
    match (fits(room_before) && to_start, fits(room_after) && to_end) {
        (true, true) => (element.1 - cover.0).max(0.0) <= (cover.1 - element.0).max(0.0),
        (true, false) => true,
        (false, true) => false,
        (false, false) if to_start != to_end => to_start,
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
/// its visible bounds by, of the window's `bar_nodes`.
fn bar_at(
    replica: &Replica,
    view: &View,
    (w, i): (usize, usize),
    containers: impl Iterator<Item = usize> + Clone,
    bar_nodes: &[(usize, usize)],
    at: Point,
) -> Option<usize> {
    let window = &replica.windows[w];
    bar_nodes.iter().map(|&(_, b)| b).find(|&b| {
        drawn_over(window, b, i, containers.clone()) && view.windows[w][b].rect.contains(at)
    })
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
        Some(Sight::new(replica, view).shows(w, c))
    })
}

/// What shows of the scroll container that shows the most of `area`, in
/// the top-most window that has one: among the nodes on screen and not
/// covered that scroll along the vertical axis, or the horizontal one, or
/// along an axis their platform does not name, the one whose visible
/// bounds ([`Sight::shows`]) share the most of `area`. A stroke of the
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
    let mut sight = Sight::new(replica, view);
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
            let shown = sight.shows(w, c);
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
/// `containers`: system and keyboard windows above its window, and of the
/// tab bars and toolbars on screen in its window, `bar_nodes`, those other
/// than the ones holding it and those inside its scroll containers, which
/// scroll with it.
fn bars<'b>(
    replica: &'b Replica,
    view: &'b View,
    (w, i): (usize, usize),
    containers: impl Iterator<Item = usize> + Clone + 'b,
    bar_nodes: &'b [(usize, usize)],
) -> impl Iterator<Item = Rect> + 'b {
    let window = &replica.windows[w];
    windows_above(replica, w).map(|(_, bar)| bar).chain(
        bar_nodes
            .iter()
            .filter(move |&&(_, b)| drawn_over(window, b, i, containers.clone()))
            .map(move |&(_, b)| view.windows[w][b].rect),
    )
}

/// Whether node `b` of `window`, a tab bar or toolbar on screen, is drawn
/// over node `i`, whose scroll containers are `containers`: neither holding
/// it nor inside one of its scroll containers, which scroll it with the
/// node.
fn drawn_over(
    window: &Win,
    b: usize,
    i: usize,
    mut containers: impl Iterator<Item = usize>,
) -> bool {
    !holds(window, b, i) && !containers.any(|c| holds(window, c, b))
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
            across(list, &heading, &footer, PIXEL, Carry::ANY),
            Rect::new(0.0, 62.0, 402.0, 720.0)
        );
        let card = Rect::new(37.0, 41.0, 365.0, 157.0);
        let header = Rect::new(0.0, 0.0, 402.0, 122.0);
        let full = Rect::new(0.0, 0.0, 402.0, 791.0);
        assert_eq!(
            across(full, &card, &header, PIXEL, Carry::ANY),
            Rect::new(0.0, 122.0, 402.0, 791.0)
        );
        let tab = Rect::new(0.0, 122.0, 402.0, 791.0);
        let paste = Rect::new(205.0, 726.0, 382.0, 774.0);
        let disabled = Rect::new(20.0, 708.0, 382.0, 764.0);
        assert_eq!(
            across(tab, &paste, &disabled, PIXEL, Carry::ANY),
            Rect::new(0.0, 122.0, 402.0, 708.0)
        );
        let inside = Rect::new(50.0, 715.0, 70.0, 725.0);
        assert_eq!(across(list, &heading, &inside, PIXEL, Carry::ANY), list);
        let beside = Rect::new(300.0, 700.0, 380.0, 780.0);
        assert_eq!(across(list, &heading, &beside, PIXEL, Carry::ANY), list);
    }
}
