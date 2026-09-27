//! The markdown summary of a [`Report`]: provenance, per-series statistics and
//! a comparison table across tools and targets. Durations are milliseconds
//! with three decimals, exact renderings of the JSON's whole microseconds.

use crate::report::Report;

/// Renders `report` as markdown.
pub fn render(report: &Report) -> String {
    todo!("render {} measurements", report.measurements.len())
}
