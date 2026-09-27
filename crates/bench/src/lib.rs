//! Benchmark harness for `mobile-agent` (design D10, benchmark-harness spec).
//!
//! The harness runs series of iterations through [`adapter::Adapter`]s, one per
//! tool, and assembles a [`report::Report`]: per-primitive statistics with
//! warm-up iterations excluded, plus the provenance needed to tell whether two
//! runs are comparable. The same report renders as JSON and as a markdown
//! summary, so the two outputs cannot disagree.

pub mod adapter;
pub mod baseline;
pub mod fake;
pub mod fixture;
pub mod flow;
pub mod gates;
pub mod harness;
pub mod markdown;
pub mod primitive;
pub mod process;
pub mod provenance;
pub mod report;
pub mod stats;
pub mod target;
