//! Tree reads: the agent's `snapshot` as the mirror's `TreeRead`. The agent
//! carries each window's metadata (whether its toolkit sends change events,
//! the system process that owns it, whether it is modal) and each scrolling
//! node's semantics in the snapshot itself, so the payload passes through as
//! the agent sent it and the mirror reads them there.

use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Instant;

use mirror::backend::{BackendError, BackendInfo, ReadReason, ReadRequest, TreeRead, TreeSource};
use protocol::event::Stream;
use protocol::message::{Call, Reply, ResyncParams, SnapshotParams};
use protocol::snapshot::{Node, Snapshot};
use protocol::{Capabilities, code};

use super::{INPUT_QUIET, Inner, ReadOptions, ReadStats};

/// The backend name the mirror reports for reads from the agent.
pub const BACKEND: &str = "android-agent";

/// Physical pixels per device point from `wm density` output: the override
/// density when one is set, else the physical one, over 160.
pub fn scale_from_density(wm_density: &str) -> Option<f64> {
    let density = |label: &str| {
        wm_density
            .lines()
            .find_map(|line| line.trim().strip_prefix(label))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|dpi| *dpi > 0.0)
    };
    density("Override density:")
        .or_else(|| density("Physical density:"))
        .map(|dpi| dpi / 160.0)
}

/// Whether a window of `snapshot` sends no change events by the agent's
/// account, as a window of Jetpack Compose content does: its changes are
/// seen only by reading it again.
pub fn has_event_less_window(snapshot: &Snapshot) -> bool {
    snapshot
        .windows
        .iter()
        .any(|window| window.emits_events == Some(false))
}

/// The parameters of a read for `reason`: reads the target's own changes
/// trigger do not wait for idle, reads after an attach, an action or a
/// resync take the settle bound.
pub fn snapshot_params(reason: ReadReason, options: &ReadOptions) -> SnapshotParams {
    let idle_ms = match reason {
        ReadReason::Attach | ReadReason::Action | ReadReason::Resync => options.settle_idle_ms,
        ReadReason::Events | ReadReason::Poll | ReadReason::Frames => Some(options.idle_ms),
    };
    SnapshotParams {
        max_depth: options.max_depth,
        max_nodes: options.max_nodes,
        idle_ms,
    }
}

/// Nodes a snapshot carries.
pub(crate) fn serialized_nodes(snapshot: &Snapshot) -> u32 {
    fn count(node: &Node) -> u32 {
        1 + node.children.iter().map(count).sum::<u32>()
    }
    snapshot
        .windows
        .iter()
        .filter_map(|w| w.root.as_ref())
        .map(count)
        .sum()
}

/// The session's tree source.
pub(crate) struct AgentTree {
    pub(crate) inner: Arc<Inner>,
}

impl TreeSource for AgentTree {
    fn read(&mut self, request: &ReadRequest) -> Result<TreeRead, BackendError> {
        let inner = &*self.inner;
        let started = Instant::now();
        let deadline = inner.options.read.deadline;
        // A resync the host session could not complete leaves the stream's
        // events dropped until another one succeeds.
        let failed: Vec<Stream> = inner
            .failed_resyncs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for stream in failed {
            let _ = inner.connection.resync(stream);
        }
        let mut cleared_cache = false;
        let foreign = inner
            .foreign_input
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(input) = foreign {
            // The agent's idle wait knows nothing of this input, and its cache
            // of windows without events is stale after it.
            let quiet = input + INPUT_QUIET;
            if let Some(wait) = quiet.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            // Resync is what empties the cache, and needs the events
            // capability; without it the read goes on with the cache.
            let can_resync = inner.agent.capabilities.contains(Capabilities::EVENTS);
            if can_resync && inner.event_less.load(Ordering::SeqCst) {
                inner.connection.call(
                    Call::Resync(ResyncParams {
                        stream: Stream::TREE,
                    }),
                    deadline,
                )?;
                cleared_cache = true;
            }
        }
        let params = snapshot_params(request.reason, &inner.options.read);
        let Reply::Snapshot(snapshot) = inner.connection.call(Call::Snapshot(params), deadline)?
        else {
            return Err(BackendError::new(
                code::PROTOCOL_ERROR,
                "the agent answered a snapshot request with another result",
            ));
        };
        inner
            .event_less
            .store(has_event_less_window(&snapshot), Ordering::SeqCst);
        inner.reads.fetch_add(1, Ordering::SeqCst);
        *inner
            .last_read
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(ReadStats {
            node_count: snapshot.node_count,
            serialized: serialized_nodes(&snapshot),
            windows: u32::try_from(snapshot.windows.len()).unwrap_or(u32::MAX),
            took: started.elapsed(),
            reason: request.reason,
            cleared_cache,
        });
        Ok(TreeRead {
            snapshot,
            backend: BackendInfo::new(BACKEND, false),
            scale: inner.scale,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use protocol::snapshot::{Bounds, NodeFlags, Truncation, Window, WindowFlags, WindowType};

    pub(crate) fn node(id: u32, class: u32, children: Vec<Node>) -> Node {
        Node {
            id,
            class,
            res_pkg: None,
            res_id: None,
            text: None,
            desc: None,
            bounds: Bounds::new(0, 0, 100, 100),
            flags: NodeFlags::ENABLED | NodeFlags::VISIBLE,
            children,
            scroll: None,
        }
    }

    pub(crate) fn window(
        id: u32,
        kind: WindowType,
        layer: i32,
        package: u32,
        bounds: Bounds,
        root: Node,
    ) -> Window {
        Window {
            id,
            kind,
            layer,
            package: Some(package),
            title: None,
            bounds,
            flags: WindowFlags::EMPTY,
            root: Some(root),
            emits_events: None,
            owner: None,
            modal: false,
        }
    }

    /// `window` with the metadata the agent reports: whether it sends change
    /// events, and its system owner as a string-table index.
    pub(crate) fn reported(window: Window, emits_events: bool, owner: Option<u32>) -> Window {
        Window {
            emits_events: Some(emits_events),
            owner,
            ..window
        }
    }

    /// Strings: 0 FrameLayout, 1 ComposeView, 2 com.android.systemui,
    /// 3 the fixture, 4 Button, 5 the keyboard's package.
    pub(crate) fn strings() -> Vec<String> {
        [
            "android.widget.FrameLayout",
            "androidx.compose.ui.platform.ComposeView",
            "com.android.systemui",
            "dev.mobileagent.fixture.compose",
            "android.widget.Button",
            "com.google.android.inputmethod.latin",
        ]
        .map(String::from)
        .to_vec()
    }

    /// A keyboard, a status bar and a Compose app window, top-most first,
    /// with the metadata the agent reports for them.
    pub(crate) fn compose_screen(cells: u32) -> Snapshot {
        let full = Bounds::new(0, 0, 1080, 2400);
        let buttons = (0..cells).map(|i| node(10 + i, 4, Vec::new())).collect();
        Snapshot {
            strings: strings(),
            windows: vec![
                reported(
                    window(
                        3,
                        WindowType::INPUT_METHOD,
                        3,
                        5,
                        Bounds::new(0, 1500, 1080, 2400),
                        node(1, 0, Vec::new()),
                    ),
                    true,
                    Some(5),
                ),
                reported(
                    window(
                        2,
                        WindowType::SYSTEM,
                        2,
                        2,
                        Bounds::new(0, 0, 1080, 132),
                        node(2, 0, Vec::new()),
                    ),
                    true,
                    Some(2),
                ),
                reported(
                    window(
                        1,
                        WindowType::APPLICATION,
                        1,
                        3,
                        full,
                        node(3, 0, vec![node(4, 1, buttons)]),
                    ),
                    false,
                    None,
                ),
            ],
            node_count: 4 + cells,
            truncated: Truncation::EMPTY,
            seq: 0,
        }
    }

    #[test]
    fn the_scale_is_the_density_over_160_with_the_override_first() {
        assert_eq!(scale_from_density("Physical density: 420\n"), Some(2.625));
        assert_eq!(
            scale_from_density("Physical density: 420\nOverride density: 480\n"),
            Some(3.0)
        );
        assert_eq!(scale_from_density("Physical density: 160"), Some(1.0));
        assert_eq!(scale_from_density("Error: no display"), None);
    }

    #[test]
    fn event_less_windows_come_from_the_agents_metadata() {
        let mut snapshot = compose_screen(3);
        assert!(has_event_less_window(&snapshot));
        snapshot.windows[2].emits_events = Some(true);
        assert!(!has_event_less_window(&snapshot));
        // A window the agent could not judge, such as one without a root.
        snapshot.windows[2].emits_events = None;
        assert!(!has_event_less_window(&snapshot));
    }

    #[test]
    fn reads_after_an_attach_action_or_resync_wait_for_idle_and_others_do_not() {
        let options = ReadOptions {
            max_nodes: Some(1000),
            max_depth: Some(40),
            idle_ms: 0,
            settle_idle_ms: Some(300),
            ..ReadOptions::default()
        };
        for reason in [ReadReason::Attach, ReadReason::Action, ReadReason::Resync] {
            let params = snapshot_params(reason, &options);
            assert_eq!(params.idle_ms, Some(300), "{reason:?}");
            assert_eq!(params.max_nodes, Some(1000));
            assert_eq!(params.max_depth, Some(40));
        }
        for reason in [ReadReason::Events, ReadReason::Poll, ReadReason::Frames] {
            assert_eq!(
                snapshot_params(reason, &options).idle_ms,
                Some(0),
                "{reason:?}"
            );
        }
        let defaults = snapshot_params(ReadReason::Attach, &ReadOptions::default());
        assert_eq!(defaults, SnapshotParams::default(), "the agent's defaults");
    }

    #[test]
    fn serialized_nodes_count_every_window() {
        assert_eq!(serialized_nodes(&compose_screen(5)), 9);
    }
}
