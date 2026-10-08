//! The markdown summary of a microbenchmark [`Report`]. Times are
//! microseconds with three decimals, exact renderings of the JSON's whole
//! nanoseconds, and differences the JSON's tenths of a percent, so the two
//! outputs agree on every number and every comparison.

use crate::markdown::{Table, or_dash};

use super::compare::{Benchmark, Verdict};
use super::criterion::Measured;
use super::report::{Provenance, Report};

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
         improvement; any change in allocations or bytes is one or the other.\n\n",
        p.started_at, p.finished_at, report.threshold_pct, report.threshold_pct
    ));
    provenance(&mut out, p);
    baseline(&mut out, report);
    benchmarks(&mut out, &report.benchmarks);
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

/// `Exit status <n>: <reason>.`, then the counts.
fn summary_line(report: &Report) -> String {
    let s = &report.summary;
    let benchmarks = match s.benchmarks {
        1 => "1 benchmark".to_owned(),
        n => format!("{n} benchmarks"),
    };
    format!(
        "Exit status {}: {}. Of {benchmarks}: {} regressed, {} improved, {} the same, {} \
         new; {} in the baseline not measured.",
        s.exit_status, s.reason, s.regressions, s.improvements, s.same, s.new, s.missing
    )
}

fn provenance(out: &mut String, p: &Provenance) {
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
fn benchmarks(out: &mut String, benchmarks: &[Benchmark]) {
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
                format!("{}–{}", us(m.spread_ns.low), us(m.spread_ns.high)),
                m.samples.to_string(),
                or_dash(m.allocations),
                or_dash(m.bytes_allocated),
                or_dash(b.baseline.as_ref().map(|saved| us(saved.median_ns))),
                or_dash(b.difference_pct.map(percent)),
                verdict_cell(b),
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
/// moved that way, such as `regression (allocations)`.
fn verdict_cell(b: &Benchmark) -> String {
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
        Verdict::Same | Verdict::New => word(b.verdict).to_owned(),
    }
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
/// none did.
fn moved(out: &mut String, report: &Report, way: Verdict) {
    let mut table = Table::new(&[
        "Benchmark",
        "Measure",
        "Baseline",
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
