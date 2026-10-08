//! The markdown summary of a microbenchmark [`Report`]. Times are
//! microseconds with three decimals, exact renderings of the JSON's whole
//! nanoseconds, and differences the JSON's tenths of a percent, so the two
//! outputs agree on every number and every comparison.

use super::report::Report;

/// Renders `report` as markdown: the provenance, the baseline compared
/// against, the benchmarks grouped by crate, the regressions, improvements
/// and missing benchmarks, and the summary line.
pub fn render(report: &Report) -> String {
    let _ = report;
    todo!()
}

/// The regressions and the improvements as tables, when there are any, then
/// the summary line with the exit status and its reason.
pub fn verdict(report: &Report) -> String {
    let _ = report;
    todo!()
}

/// Microseconds with three decimals: an exact rendering of whole
/// nanoseconds.
pub fn us(ns: u64) -> String {
    let _ = ns;
    todo!()
}
