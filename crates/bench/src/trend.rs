//! `mobile-agent-bench trend <previous> <report>`, the comparison of design
//! D5 of `ci-benchmarks`: a scheduled run's report beside the previous
//! scheduled run's, both of the same job.

use crate::micro::ab;
use crate::report::Report;

/// The comparison of two reports' texts, `previous` and `current`, as
/// markdown.
pub fn compare(previous: &str, current: &str) -> Result<String, String> {
    let _ = (previous, current);
    Err("trend is not built yet".to_owned())
}

/// Two A/B reports' head medians and differences, benchmark by benchmark.
pub fn micro(previous: &ab::Report, current: &ab::Report) -> String {
    let _ = (previous, current);
    String::new()
}

/// Two runs' ratios, flow by flow and baseline by baseline.
pub fn harness(previous: &Report, current: &Report) -> String {
    let _ = (previous, current);
    String::new()
}
