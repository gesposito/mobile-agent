//! The markdown summaries of a microbenchmark [`Report`] and of an A/B
//! [`ab::Report`]. Times are microseconds with three decimals, exact
//! renderings of the JSON's whole nanoseconds, and differences the JSON's
//! tenths of a percent, so the two outputs agree on every number and every
//! comparison.

use crate::markdown::{Table, or_dash};

use super::ab::{self, SideResult};
use super::compare::{Benchmark, Verdict};
use super::criterion::{Measured, Spread};
use super::report::{Provenance, Report, Summary};

/// Renders `report` as markdown: the summary line, the provenance, the
/// baseline compared against, the benchmarks grouped by crate, then the
/// regressions, the improvements and the missing benchmarks.
pub fn render(report: &Report) -> String {
    let p = &report.provenance;
    let mut out = String::new();
    out.push_str("# Microbenchmark report\n\n");
    out.push_str(&summary_line(report));
    out.push_str("\n\n");
    out.push_str(&format!(
        "Started {}, finished {}. Times are criterion's medians in microseconds, to \
         the nanosecond, with their 95% intervals; allocations and bytes are per \
         iteration, where the bench counts them. A median more than {:.1}% slower \
         than the baseline's is a regression and one more than {:.1}% faster an \
         improvement, unless it lies inside the 95% interval the baseline recorded \
         around its own median; any change in allocations or bytes is one or the \
         other.\n\n",
        p.started_at, p.finished_at, report.threshold_pct, report.threshold_pct
    ));
    provenance(&mut out, p, &[]);
    baseline(&mut out, report);
    benchmarks(&mut out, report);
    moved(&mut out, report, Verdict::Regression);
    moved(&mut out, report, Verdict::Improvement);
    missing(&mut out, &report.missing);
    out
}

/// The regressions and the improvements as tables, when there are any, then
/// the summary line with the exit status and its reason.
pub fn verdict(report: &Report) -> String {
    let mut out = String::new();
    moved(&mut out, report, Verdict::Regression);
    moved(&mut out, report, Verdict::Improvement);
    out.push_str(&summary_line(report));
    out.push('\n');
    out
}

/// Microseconds with three decimals: an exact rendering of whole
/// nanoseconds.
pub fn us(ns: u64) -> String {
    format!("{}.{:03}", ns / 1000, ns % 1000)
}

/// A difference in percent with its sign, such as `+30.5%`.
fn percent(pct: f64) -> String {
    format!("{pct:+.1}%")
}

/// An interval in microseconds, such as `132.178–143.493`.
fn interval(spread: Spread) -> String {
    format!("{}–{}", us(spread.low), us(spread.high))
}

/// `Exit status <n>: <reason>.`, then the counts.
fn summary_line(report: &Report) -> String {
    counts(&report.summary, "in the baseline not measured")
}

/// `Exit status <n>: <reason>.`, then the counts, the missing ones as
/// `missing`.
fn counts(s: &Summary, missing: &str) -> String {
    let benchmarks = match s.benchmarks {
        1 => "1 benchmark".to_owned(),
        n => format!("{n} benchmarks"),
    };
    format!(
        "Exit status {}: {}. Of {benchmarks}: {} regressed, {} improved, {} the same, {} \
         new; {} {missing}.",
        s.exit_status, s.reason, s.regressions, s.improvements, s.same, s.new, s.missing
    )
}

/// The provenance table, with `extra` rows after the run's own.
fn provenance(out: &mut String, p: &Provenance, extra: &[(&str, String)]) {
    out.push_str("## Provenance\n\n");
    let mut table = Table::new(&["Field", "Value"]);
    let mut field = |name: &str, value: String| table.row(vec![name.to_owned(), value]);
    field("Host hardware", or_dash(p.host.hardware.as_deref()));
    field("Host CPU cores", or_dash(p.host.cpu_cores));
    field("Host memory bytes", or_dash(p.host.memory_bytes));
    field("Host OS", or_dash(p.host.os.as_deref()));
    field("Load average at start", loads(p.host.load_average));
    field("Load average at end", loads(p.load_average_end));
    field("Commit under test", or_dash(p.commit.as_ref().map(commit)));
    field("Rust toolchain", or_dash(p.toolchain.as_deref()));
    let h = &p.harness;
    field(
        "Harness",
        match &h.version {
            Some(version) => format!("{} {version}", h.library),
            None => h.library.clone(),
        },
    );
    field(
        "Sample settings",
        format!(
            "{} s warm-up, {} s measurement, {} samples, {} bootstrap resamples, {}% intervals",
            h.warm_up_s, h.measurement_s, h.sample_size, h.resamples, h.confidence_pct
        ),
    );
    field(
        "Guard",
        format!(
            "1-minute load under {}, after {} s of waiting",
            p.guard.max_load, p.guard.waited_s
        ),
    );
    for (name, value) in extra {
        field(name, value.clone());
    }
    table.render(out);
}

/// The 1, 5 and 15 minute load averages with two decimals.
fn loads(load: Option<[f64; 3]>) -> String {
    or_dash(load.map(|[one, five, fifteen]| format!("{one:.2} {five:.2} {fifteen:.2}")))
}

fn commit(commit: &crate::provenance::Commit) -> String {
    format!(
        "{} ({})",
        commit.sha,
        if commit.dirty { "dirty" } else { "clean" }
    )
}

/// What was compared against: the baseline's run, or that there was none.
fn baseline(out: &mut String, report: &Report) {
    let b = &report.baseline;
    out.push_str("## Baseline\n\n");
    match &b.provenance {
        None => out.push_str(&format!(
            "No baseline at `{}`: every benchmark is new, and none is compared.\n\n",
            b.path
        )),
        Some(p) => {
            out.push_str(&format!(
                "Compared with `{}`, measured from {} to {} on commit {}",
                b.path,
                p.started_at,
                p.finished_at,
                or_dash(p.commit.as_ref().map(commit))
            ));
            if let Some([one, ..]) = p.host.load_average {
                out.push_str(&format!(" at a 1-minute load of {one:.2}"));
            }
            if b.environment_differences.is_empty() {
                out.push_str(", in this run's environment.\n\n");
            } else {
                out.push_str(&format!(
                    ". Its environment differs from this run's in {}.\n\n",
                    b.environment_differences.join(", ")
                ));
            }
        }
    }
    if b.saved {
        out.push_str(&format!(
            "This run then wrote its own values and provenance to `{}`.\n\n",
            b.path
        ));
    }
}

/// The crate a benchmark id names first, such as `protocol`.
fn krate(id: &str) -> &str {
    id.split('/').next().unwrap_or(id)
}

/// One table per crate, in id order.
fn benchmarks(out: &mut String, report: &Report) {
    let benchmarks = &report.benchmarks;
    out.push_str("## Benchmarks\n\n");
    let mut crates: Vec<&str> = Vec::new();
    for b in benchmarks {
        let name = krate(&b.measured.id);
        if !crates.contains(&name) {
            crates.push(name);
        }
    }
    for name in crates {
        out.push_str(&format!("### {name}\n\n"));
        let mut table = Table::new(&[
            "Benchmark",
            "Median µs",
            "95% interval µs",
            "Samples",
            "Allocations",
            "Bytes",
            "Baseline µs",
            "Difference",
            "Verdict",
        ]);
        for b in benchmarks.iter().filter(|b| krate(&b.measured.id) == name) {
            let m = &b.measured;
            table.row(vec![
                m.id.clone(),
                us(m.median_ns),
                or_dash(m.spread_ns.map(interval)),
                m.samples.to_string(),
                or_dash(m.allocations),
                or_dash(m.bytes_allocated),
                or_dash(b.baseline.as_ref().map(|saved| us(saved.median_ns))),
                or_dash(b.difference_pct.map(percent)),
                verdict_cell(b, report.threshold_pct),
            ]);
        }
        table.render(out);
    }
}

fn word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Regression => "regression",
        Verdict::Improvement => "improvement",
        Verdict::Same => "same",
        Verdict::New => "new",
    }
}

/// The verdict, and for a regression or an improvement the measures that
/// moved that way, such as `regression (allocations)`; the same verdict on a
/// median past `threshold_pct` says that the baseline's interval held it.
fn verdict_cell(b: &Benchmark, threshold_pct: f64) -> String {
    let measures: Vec<&str> = [
        (b.time_verdict, "median"),
        (b.allocations_verdict, "allocations"),
    ]
    .into_iter()
    .filter(|&(verdict, _)| verdict == Some(b.verdict))
    .map(|(_, measure)| measure)
    .collect();
    match b.verdict {
        Verdict::Regression | Verdict::Improvement => {
            format!("{} ({})", word(b.verdict), measures.join(", "))
        }
        Verdict::Same if held(b, threshold_pct) => {
            "same (median inside the baseline's interval)".to_owned()
        }
        Verdict::Same | Verdict::New => word(b.verdict).to_owned(),
    }
}

/// Whether `b`'s median differs from the baseline's by more than
/// `threshold_pct` and lies inside the interval the baseline recorded
/// around its own median, which keeps its time the same.
fn held(b: &Benchmark, threshold_pct: f64) -> bool {
    let past = b
        .difference_pct
        .is_some_and(|pct| pct.abs() > threshold_pct);
    let inside = b
        .baseline
        .as_ref()
        .and_then(|saved| saved.spread_ns)
        .is_some_and(|i| (i.low..=i.high).contains(&b.measured.median_ns));
    past && inside
}

/// How a count moved from `saved` to `now`.
fn direction(saved: u64, now: u64) -> Verdict {
    match now.cmp(&saved) {
        std::cmp::Ordering::Greater => Verdict::Regression,
        std::cmp::Ordering::Less => Verdict::Improvement,
        std::cmp::Ordering::Equal => Verdict::Same,
    }
}

/// Every measure that moved as `way`, a regression or an improvement, with
/// the baseline's and the measured value, under its heading; nothing when
/// none did. A median's row has the two conditions it met: the interval the
/// baseline recorded around its own median, and the threshold.
fn moved(out: &mut String, report: &Report, way: Verdict) {
    let mut table = Table::new(&[
        "Benchmark",
        "Measure",
        "Baseline",
        "Baseline interval",
        "Measured",
        "Difference",
        "Threshold",
    ]);
    let mut rows = 0;
    for b in &report.benchmarks {
        let (m, Some(saved)) = (&b.measured, &b.baseline) else {
            continue;
        };
        if b.time_verdict == Some(way) {
            table.row(vec![
                m.id.clone(),
                "median µs".to_owned(),
                us(saved.median_ns),
                or_dash(saved.spread_ns.map(interval)),
                us(m.median_ns),
                or_dash(b.difference_pct.map(percent)),
                format!("{:.1}%", report.threshold_pct),
            ]);
            rows += 1;
        }
        for (measure, before, now) in [
            ("allocations", saved.allocations, m.allocations),
            ("bytes", saved.bytes_allocated, m.bytes_allocated),
        ] {
            if let (Some(before), Some(now)) = (before, now)
                && direction(before, now) == way
            {
                table.row(vec![
                    m.id.clone(),
                    measure.to_owned(),
                    before.to_string(),
                    "—".to_owned(),
                    now.to_string(),
                    if now > before { "more" } else { "fewer" }.to_owned(),
                    "exact".to_owned(),
                ]);
                rows += 1;
            }
        }
    }
    if rows == 0 {
        return;
    }
    out.push_str(match way {
        Verdict::Regression => "## Regressions\n\n",
        _ => "## Improvements\n\n",
    });
    table.render(out);
}

/// The baseline's benchmarks the run did not measure, when there are any.
fn missing(out: &mut String, missing: &[Measured]) {
    if missing.is_empty() {
        return;
    }
    out.push_str("## Missing\n\nIn the baseline, and not measured by this run:\n\n");
    let mut table = Table::new(&[
        "Benchmark",
        "Baseline µs",
        "Baseline allocations",
        "Baseline bytes",
    ]);
    for m in missing {
        table.row(vec![
            m.id.clone(),
            us(m.median_ns),
            or_dash(m.allocations),
            or_dash(m.bytes_allocated),
        ]);
    }
    table.render(out);
}

/// Renders an A/B `report` as markdown: the summary line, the provenance
/// with the base, the rounds and the toolchain, the benchmarks grouped by
/// crate with both sides' values, then the regressions, the improvements and
/// the benchmarks head did not measure.
pub fn render_ab(report: &ab::Report) -> String {
    let p = &report.provenance;
    let mut out = String::new();
    out.push_str("# Microbenchmark A/B report\n\n");
    out.push_str(&ab_summary_line(report));
    out.push_str("\n\n");
    out.push_str(&format!(
        "Started {}, finished {}. Head is the checkout under test and base `{}`, checked out \
         into a worktree of its own and built in a target directory of its own; each side ran \
         the suites {} times, the side that went first alternating from round to round. A \
         side's time for a \
         benchmark is the median of its rounds' medians, criterion's medians in microseconds \
         to the nanosecond; allocations and bytes are per iteration, where the bench counts \
         them. A head median more than {:.1}% slower than base's is a regression and one more \
         than {:.1}% faster an improvement; any change in allocations or bytes is one or the \
         other.\n\n",
        p.started_at,
        p.finished_at,
        report.base.name,
        report.rounds,
        report.threshold_pct,
        report.threshold_pct
    ));
    let rounds: Vec<String> = report
        .order
        .iter()
        .map(|[first, second]| format!("{first} then {second}"))
        .collect();
    provenance(
        &mut out,
        p,
        &[
            (
                "Base",
                format!("{} at {}", report.base.name, report.base.sha),
            ),
            (
                "Rounds",
                format!("{}: {}", report.rounds, rounds.join(", ")),
            ),
            (
                "Toolchain for both sides",
                report
                    .rustup_toolchain
                    .clone()
                    .unwrap_or_else(|| "each tree's own".to_owned()),
            ),
        ],
    );
    ab_benchmarks(&mut out, report);
    ab_moved(&mut out, report, Verdict::Regression);
    ab_moved(&mut out, report, Verdict::Improvement);
    ab_missing(&mut out, &report.missing);
    out
}

/// The regressions and the improvements of an A/B `report` as tables, when
/// there are any, then the summary line.
pub fn verdict_ab(report: &ab::Report) -> String {
    let mut out = String::new();
    ab_moved(&mut out, report, Verdict::Regression);
    ab_moved(&mut out, report, Verdict::Improvement);
    out.push_str(&ab_summary_line(report));
    out.push('\n');
    out
}

fn ab_summary_line(report: &ab::Report) -> String {
    counts(&report.summary, "in base not measured in head")
}

/// A side's round medians in round order, such as `100.000, 104.000`.
fn rounds_cell(side: &SideResult) -> String {
    let rounds: Vec<String> = side.rounds_ns.iter().map(|&ns| us(ns)).collect();
    rounds.join(", ")
}

/// The verdict of an A/B benchmark, with the measures that moved:
/// `regressed (median, allocations)`, `faster`, `fewer allocations`, `same`
/// or `new`.
fn ab_verdict_cell(b: &ab::Benchmark) -> String {
    let moved = |way| {
        [
            (b.time_verdict, "median"),
            (b.allocations_verdict, "allocations"),
        ]
        .into_iter()
        .filter(move |&(verdict, _)| verdict == Some(way))
        .map(|(_, measure)| measure)
    };
    match b.verdict {
        Verdict::Regression => {
            let measures: Vec<&str> = moved(Verdict::Regression).collect();
            format!("regressed ({})", measures.join(", "))
        }
        Verdict::Improvement => {
            let words: Vec<&str> = moved(Verdict::Improvement)
                .map(|measure| match measure {
                    "median" => "faster",
                    _ => "fewer allocations",
                })
                .collect();
            words.join(", ")
        }
        Verdict::Same => "same".to_owned(),
        Verdict::New => "new".to_owned(),
    }
}

/// One table per crate, in id order, with both sides' values.
fn ab_benchmarks(out: &mut String, report: &ab::Report) {
    out.push_str("## Benchmarks\n\n");
    let mut crates: Vec<&str> = Vec::new();
    for b in &report.benchmarks {
        let name = krate(&b.id);
        if !crates.contains(&name) {
            crates.push(name);
        }
    }
    for name in crates {
        out.push_str(&format!("### {name}\n\n"));
        let mut table = Table::new(&[
            "Benchmark",
            "Base µs",
            "Head µs",
            "Difference",
            "Base rounds µs",
            "Head rounds µs",
            "Base allocations",
            "Head allocations",
            "Base bytes",
            "Head bytes",
            "Verdict",
        ]);
        for b in report.benchmarks.iter().filter(|b| krate(&b.id) == name) {
            let base = b.base.as_ref();
            table.row(vec![
                b.id.clone(),
                or_dash(base.map(|s| us(s.median_ns))),
                us(b.head.median_ns),
                or_dash(b.difference_pct.map(percent)),
                or_dash(base.map(rounds_cell)),
                rounds_cell(&b.head),
                or_dash(base.and_then(|s| s.allocations)),
                or_dash(b.head.allocations),
                or_dash(base.and_then(|s| s.bytes_allocated)),
                or_dash(b.head.bytes_allocated),
                ab_verdict_cell(b),
            ]);
        }
        table.render(out);
    }
}

/// Every measure of an A/B report that moved as `way`, with base's and
/// head's values, under its heading; nothing when none did.
fn ab_moved(out: &mut String, report: &ab::Report, way: Verdict) {
    let mut table = Table::new(&[
        "Benchmark",
        "Measure",
        "Base",
        "Head",
        "Difference",
        "Threshold",
    ]);
    let mut rows = 0;
    for b in &report.benchmarks {
        let Some(base) = &b.base else {
            continue;
        };
        if b.time_verdict == Some(way) {
            table.row(vec![
                b.id.clone(),
                "median µs".to_owned(),
                us(base.median_ns),
                us(b.head.median_ns),
                or_dash(b.difference_pct.map(percent)),
                format!("{:.1}%", report.threshold_pct),
            ]);
            rows += 1;
        }
        for (measure, before, now) in [
            ("allocations", base.allocations, b.head.allocations),
            ("bytes", base.bytes_allocated, b.head.bytes_allocated),
        ] {
            if let (Some(before), Some(now)) = (before, now)
                && direction(before, now) == way
            {
                table.row(vec![
                    b.id.clone(),
                    measure.to_owned(),
                    before.to_string(),
                    now.to_string(),
                    if now > before { "more" } else { "fewer" }.to_owned(),
                    "exact".to_owned(),
                ]);
                rows += 1;
            }
        }
    }
    if rows == 0 {
        return;
    }
    out.push_str(match way {
        Verdict::Regression => "## Regressions\n\n",
        _ => "## Improvements\n\n",
    });
    table.render(out);
}

/// Base's benchmarks head did not measure, when there are any.
fn ab_missing(out: &mut String, missing: &[ab::Missing]) {
    if missing.is_empty() {
        return;
    }
    out.push_str("## Missing\n\nMeasured by base, and not by head:\n\n");
    let mut table = Table::new(&["Benchmark", "Base µs", "Base allocations", "Base bytes"]);
    for m in missing {
        table.row(vec![
            m.id.clone(),
            us(m.base.median_ns),
            or_dash(m.base.allocations),
            or_dash(m.base.bytes_allocated),
        ]);
    }
    table.render(out);
}
