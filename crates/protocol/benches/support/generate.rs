//! The inputs of the codec benchmarks (design D14). Not generated yet.

// Not every target that includes this file uses all of it.
#![allow(dead_code)]

use protocol::Message;
use protocol::message::{Method, Reply, Response};
use protocol::snapshot::{Snapshot, Truncation};

/// A 500-node snapshot response in spike S5's shape.
pub fn snapshot_500() -> Message {
    empty_snapshot()
}

/// A 1,049-node snapshot response in the React Native shape.
pub fn snapshot_1049() -> Message {
    empty_snapshot()
}

/// 10,000 events on the tree and window streams.
pub fn events_10k() -> Vec<Message> {
    Vec::new()
}

/// 1,000 requests in S5's mix.
pub fn requests_1k() -> Vec<Message> {
    Vec::new()
}

/// Every dataset, by the name its benchmarks carry.
pub fn datasets() -> Vec<(&'static str, Vec<Message>)> {
    vec![
        ("snapshot-500", vec![snapshot_500()]),
        ("snapshot-1049", vec![snapshot_1049()]),
        ("events-10k", events_10k()),
        ("requests-1k", requests_1k()),
    ]
}

fn empty_snapshot() -> Message {
    Message::Response(Response {
        id: 0,
        method: Method::SNAPSHOT,
        result: Ok(Reply::Snapshot(Snapshot {
            strings: Vec::new(),
            windows: Vec::new(),
            node_count: 0,
            truncated: Truncation::EMPTY,
            seq: 0,
        })),
    })
}
