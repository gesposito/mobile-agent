//! The markdown summary of a [`Report`]: provenance, per-series statistics, a
//! comparison table across tools and targets, and the hand-offs between
//! tools. Durations are milliseconds with three decimals, exact renderings of
//! the JSON's whole microseconds, so the two outputs agree on every number.

use std::fmt::Display;

use crate::adapter::{Clock, Role};
use crate::fixture::{Fixture, Screen};
use crate::handoff::HandOff;
use crate::harness::Subject;
use crate::provenance::{Provenance, ToolStatus};
use crate::report::{Measurement, NodeRange, Report};
use crate::stats::{Summary, ms};
use crate::target::{Battery, TargetClass};

/// Renders `report` as markdown.
pub fn render(report: &Report) -> String {
    let p = &report.provenance;
    let mut out = String::new();
    out.push_str("# Benchmark report\n\n");
    out.push_str(&format!(
        "Started {}, finished {}. Durations are milliseconds; percentiles are \
         nearest-rank over the measured iterations, with warm-up iterations \
         excluded. Fail % is the share of measured iterations in which no \
         attempt succeeded, Flake % the share that succeeded only on a retry.\n\n",
        p.started_at, p.finished_at
    ));
    provenance(&mut out, p);
    measurements(&mut out, &report.measurements);
    comparison(&mut out, report);
    ratios(&mut out, report);
    handoffs(&mut out, &report.handoffs);
    if let Some(gates) = &report.gates {
        out.push_str("## Gates\n\n");
        out.push_str(&gates.table());
        out.push('\n');
    }
    out
}

/// A markdown table being built.
pub(crate) struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub(crate) fn new(header: &[&str]) -> Self {
        Table {
            header: header.iter().map(|h| (*h).to_owned()).collect(),
            rows: Vec::new(),
        }
    }

    pub(crate) fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub(crate) fn render(&self, out: &mut String) {
        let line = |cells: &[String]| {
            let escaped: Vec<String> = cells.iter().map(|c| escape(c)).collect();
            format!("| {} |\n", escaped.join(" | "))
        };
        out.push_str(&line(&self.header));
        out.push_str(&format!("|{}\n", "---|".repeat(self.header.len())));
        for row in &self.rows {
            out.push_str(&line(row));
        }
        out.push('\n');
    }
}

/// A cell's text with the characters that would break a table row replaced.
pub(crate) fn escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// The value, or a dash when there is none.
pub(crate) fn or_dash<T: Display>(value: Option<T>) -> String {
    value.map_or_else(|| "—".to_owned(), |v| v.to_string())
}

fn fixture_cell(fixture: Option<Fixture>) -> String {
    or_dash(fixture)
}

fn screen_cell(screen: Option<Screen>) -> String {
    or_dash(screen)
}

/// The Subject column: a primitive's id.
fn subject_cell(subject: &Subject) -> String {
    subject.to_string()
}

fn clock_cell(clock: Option<Clock>) -> String {
    or_dash(clock.map(|c| match c {
        Clock::Wall => "wall",
        Clock::Tool => "tool",
    }))
}

fn nodes_cell(nodes: Option<NodeRange>) -> String {
    match nodes {
        None => "—".to_owned(),
        Some(NodeRange { min, max }) if min == max => max.to_string(),
        Some(NodeRange { min, max }) => format!("{min}–{max}"),
    }
}

fn provenance(out: &mut String, p: &Provenance) {
    out.push_str("## Provenance\n\n");
    let mut table = Table::new(&["Field", "Value"]);
    let mut field = |name: &str, value: String| table.row(vec![name.to_owned(), value]);
    field("Host hardware", or_dash(p.host.hardware.as_deref()));
    field("Host CPU cores", or_dash(p.host.cpu_cores));
    field("Host memory bytes", or_dash(p.host.memory_bytes));
    field("Host OS", or_dash(p.host.os.as_deref()));
    field(
        "Load average at start",
        or_dash(
            p.host
                .load_average
                .map(|[one, five, fifteen]| format!("{one:.2} {five:.2} {fifteen:.2}")),
        ),
    );
    field("Xcode", or_dash(p.xcode.as_deref()));
    field(
        "Commit under test",
        or_dash(
            p.commit
                .as_ref()
                .map(|c| format!("{} ({})", c.sha, if c.dirty { "dirty" } else { "clean" })),
        ),
    );
    let retries = match p.harness.retries {
        1 => "1 retry".to_owned(),
        n => format!("{n} retries"),
    };
    field(
        "Harness",
        format!(
            "bench {}, {} iterations after {} warm-up, {retries}",
            p.harness.version, p.harness.iterations, p.harness.warmup
        ),
    );
    table.render(out);

    out.push_str("### Targets\n\n");
    let mut targets = Table::new(&[
        "Target",
        "Class",
        "Model",
        "OS",
        "Android API level",
        "Memory bytes",
        "Emulator controller",
        "Frame source",
    ]);
    for t in &p.targets {
        targets.row(vec![
            t.id.clone(),
            t.class.to_string(),
            or_dash(t.model.as_deref()),
            or_dash(t.os.as_deref()),
            or_dash(t.api_level),
            or_dash(t.memory_bytes),
            or_dash(t.controller),
            or_dash(
                t.frame_source
                    .map(|frames| if frames { "yes" } else { "no" }),
            ),
        ]);
    }
    targets.render(out);
    iphones(out, p);

    out.push_str("### Tools\n\n");
    let mut tools = Table::new(&["Tool", "Role", "Status", "Version", "Note"]);
    for t in &p.tools {
        tools.row(vec![
            t.name.clone(),
            match t.role {
                Role::UnderTest => "under test",
                Role::Baseline => "baseline",
            }
            .to_owned(),
            match t.status {
                ToolStatus::Available => "available",
                ToolStatus::Skipped => "skipped",
            }
            .to_owned(),
            or_dash(t.version.as_deref()),
            or_dash(t.note.as_deref()),
        ]);
    }
    tools.render(out);
}

/// The physical iPhones of the run, when it has any: the runner the tool
/// under test started on each, its input and frame paths, the battery it
/// read, the Auto-Lock the person running stated and the fixtures' builds
/// the harness installed.
fn iphones(out: &mut String, p: &Provenance) {
    let phones: Vec<_> = p
        .targets
        .iter()
        .filter(|t| t.class == TargetClass::IosDevice)
        .collect();
    if phones.is_empty() {
        return;
    }
    out.push_str("### Physical iPhones\n\n");
    let mut table = Table::new(&[
        "Target",
        "Runner",
        "Input",
        "Frames",
        "Battery",
        "Auto-Lock",
        "Installed before the run",
    ]);
    for t in phones {
        let installed: Vec<String> = t
            .installs
            .iter()
            .map(|install| format!("{} in {} ms", install.fixture, ms(install.elapsed_us)))
            .collect();
        table.row(vec![
            t.id.clone(),
            or_dash(t.runner.as_deref()),
            or_dash(t.input_path.as_deref()),
            or_dash(t.frame_path.as_deref()),
            or_dash(t.battery.and_then(battery_cell)),
            match t.auto_lock_off {
                Some(true) => "off, as stated",
                _ => "not stated",
            }
            .to_owned(),
            if installed.is_empty() {
                "none".to_owned()
            } else {
                installed.join("; ")
            },
        ]);
    }
    table.render(out);
}

/// A battery as `76%, charging, on external power`, from what was read of
/// it; `None` when nothing was.
fn battery_cell(battery: Battery) -> Option<String> {
    let parts: Vec<String> = [
        battery.level_pct.map(|level| format!("{level}%")),
        battery
            .charging
            .map(|charging| if charging { "charging" } else { "not charging" }.to_owned()),
        battery.external_power.map(|external| {
            if external {
                "on external power"
            } else {
                "on its battery"
            }
            .to_owned()
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn measurements(out: &mut String, measurements: &[Measurement]) {
    out.push_str("## Measurements\n\n");
    let mut table = Table::new(&[
        "Tool", "Target", "Fixture", "Screen", "Subject", "Clock", "n", "p50 ms", "p95 ms",
        "max ms", "Fail %", "Flake %", "Nodes",
    ]);
    for m in measurements {
        let stat = |pick: fn(&Summary) -> u64| or_dash(m.stats.as_ref().map(|s| ms(pick(s))));
        // A series the tool could not measure has no rates to show.
        let rate = |pct: f64| match m.not_measured {
            Some(_) => "—".to_owned(),
            None => format!("{pct:.1}"),
        };
        table.row(vec![
            m.tool.clone(),
            m.target.clone(),
            fixture_cell(m.fixture),
            screen_cell(m.screen),
            subject_cell(&m.subject),
            clock_cell(m.clock),
            m.stats.map_or(0, |s| s.n).to_string(),
            stat(|s| s.p50_us),
            stat(|s| s.p95_us),
            stat(|s| s.max_us),
            rate(m.reliability.failure_rate_pct),
            rate(m.reliability.flake_rate_pct),
            nodes_cell(m.nodes),
        ]);
    }
    table.render(out);
    let listed = |title: &str, reason: fn(&Measurement) -> Option<&str>, out: &mut String| {
        let series: Vec<(&Measurement, &str)> = measurements
            .iter()
            .filter_map(|m| reason(m).map(|r| (m, r)))
            .collect();
        if series.is_empty() {
            return;
        }
        out.push_str(&format!("{title}:\n\n"));
        for (m, reason) in series {
            out.push_str(&format!(
                "- {} on {}, {}: {}\n",
                m.tool,
                m.target,
                subject_cell(&m.subject),
                escape(reason)
            ));
        }
        out.push('\n');
    };
    listed("Series that could not run", |m| m.error.as_deref(), out);
    listed("Series not measured", |m| m.not_measured.as_deref(), out);
}

/// The key of a comparison row.
type RowKey<'a> = (&'a str, Option<Fixture>, Option<Screen>, &'a Subject);

fn comparison(out: &mut String, report: &Report) {
    out.push_str("## Comparison\n\n");
    out.push_str(
        "p50 / p95 in milliseconds per tool and target; — where a tool did not \
         measure the subject.\n\n",
    );
    let tools: Vec<&str> = report
        .provenance
        .tools
        .iter()
        .map(|t| t.name.as_str())
        .filter(|name| report.measurements.iter().any(|m| m.tool == *name))
        .collect();
    let mut keys: Vec<RowKey> = Vec::new();
    for m in &report.measurements {
        let key = (m.target.as_str(), m.fixture, m.screen, &m.subject);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let mut header = vec!["Target", "Fixture", "Screen", "Subject"];
    header.extend(&tools);
    let mut table = Table::new(&header);
    for (target, fixture, screen, subject) in keys {
        let mut row = vec![
            target.to_owned(),
            fixture_cell(fixture),
            screen_cell(screen),
            subject_cell(subject),
        ];
        for tool in &tools {
            let found = report.measurements.iter().find(|m| {
                m.tool == *tool
                    && m.target == target
                    && m.fixture == fixture
                    && m.screen == screen
                    && m.subject == *subject
            });
            row.push(match found {
                None => "—".to_owned(),
                Some(Measurement {
                    not_measured: Some(_),
                    ..
                }) => "not measured".to_owned(),
                Some(Measurement { stats: None, .. }) => "no samples".to_owned(),
                Some(Measurement { stats: Some(s), .. }) => {
                    format!("{} / {}", ms(s.p50_us), ms(s.p95_us))
                }
            });
        }
        table.row(row);
    }
    table.render(out);
    let skipped: Vec<String> = report
        .provenance
        .tools
        .iter()
        .filter(|t| t.status == ToolStatus::Skipped)
        .map(|t| format!("{} ({})", t.name, t.note.as_deref().unwrap_or("absent")))
        .collect();
    if !skipped.is_empty() {
        out.push_str(&format!("Skipped: {}.\n\n", skipped.join("; ")));
    }
}

/// The ratios of the tool under test against each baseline, with the
/// baselines skipped, when the report has any.
fn ratios(out: &mut String, report: &Report) {
    let Some(ratios) = report.ratios.as_ref().filter(|r| !r.is_empty()) else {
        return;
    };
    let tool = ratios.tool.as_deref().unwrap_or("the tool under test");
    out.push_str("## Ratios\n\n");
    out.push_str(&format!(
        "How many times faster {tool}'s p50 was than each baseline's on the same flow, \
         target and fixture, in this run: the baseline's p50 over {tool}'s, under 1 when the \
         baseline was faster. A CI runner's run is judged by these rather than by the gates, \
         whose limits belong to the measured hosts.\n\n"
    ));
    out.push_str(ratios.table().trim_end());
    out.push_str("\n\n");
}

/// The hand-offs of targets between tools, when the run had any.
fn handoffs(out: &mut String, handoffs: &[HandOff]) {
    if handoffs.is_empty() {
        return;
    }
    out.push_str("## Hand-offs\n\n");
    out.push_str(
        "Between series, before another tool drives a target and once the run is \
         over, the tool that drove it lets go of it, the run waits until nothing that \
         tool keeps for the target is left running, ending itself what the release \
         did not reach, and an Android target gets back the input method the run \
         found.\n\n",
    );
    let mut table = Table::new(&["Target", "From", "To", "ms", "What happened"]);
    for handoff in handoffs {
        let mut happened: Vec<String> = [&handoff.released, &handoff.input_method]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        if let Some(error) = &handoff.error {
            happened.push(format!("failed: {error}"));
        }
        table.row(vec![
            handoff.target.clone(),
            handoff.from.clone(),
            handoff
                .to
                .clone()
                .unwrap_or_else(|| "end of the run".to_owned()),
            ms(handoff.elapsed_us),
            if happened.is_empty() {
                "nothing held".to_owned()
            } else {
                happened.join("; ")
            },
        ]);
    }
    table.render(out);
}
