//! Helpers shared by the integration tests: a scripted host, scratch
//! directories and a reader for the markdown summary's tables.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bench::provenance::ScriptedProbe;
use serde_json::Value;

/// An empty directory under Cargo's per-target scratch space.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The host of spike S7's setup: an M1 Max on macOS 26.6.2 with Xcode 27.0, a
/// clean checkout, a Simulator `SIM-1` and an emulator `emulator-5554`.
pub fn probe() -> ScriptedProbe {
    ScriptedProbe::new()
        .with("sysctl -n machdep.cpu.brand_string", "Apple M1 Max")
        .with("sysctl -n hw.model", "placeholder-model")
        .with("sysctl -n hw.ncpu", "10")
        .with("sysctl -n hw.memsize", "68719476736")
        .with("sysctl -n vm.loadavg", "{ 3.85 4.10 5.00 }")
        .with("sw_vers -productName", "macOS")
        .with("sw_vers -productVersion", "26.6.2")
        .with("sw_vers -buildVersion", "25G83")
        .with("xcodebuild -version", "Xcode 27.0\nBuild version 27A266a")
        .with("git rev-parse HEAD", "d7d0f41c0ffee0000000000000000000000000ab")
        .with("git status --porcelain --untracked-files=no", "")
        .with(
            "adb -s emulator-5554 shell getprop",
            "[ro.boot.qemu.avd_name]: [pixel_api36]\n\
             [ro.build.version.release]: [16]\n\
             [ro.build.version.sdk]: [36]\n\
             [ro.product.model]: [sdk_gphone64_arm64]",
        )
        .with(
            "xcrun simctl list devices --json",
            r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-5":[
                {"udid":"SIM-1","name":"bench","deviceTypeIdentifier":"com.apple.CoreSimulator.SimDeviceType.iPhone-17"}]}}"#,
        )
}

/// A markdown table: its header cells and body rows.
#[derive(Debug, Clone)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// The body cell in `column` (a header name) of `row`.
    pub fn cell<'a>(&self, row: &'a [String], column: &str) -> &'a str {
        let index = self
            .header
            .iter()
            .position(|h| h == column)
            .unwrap_or_else(|| panic!("no column {column:?} in {:?}", self.header));
        &row[index]
    }
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|c| c.trim().to_owned())
        .collect()
}

/// Every table in `md` with the heading it appears under.
pub fn tables(md: &str) -> Vec<(String, Table)> {
    let lines: Vec<&str> = md.lines().collect();
    let mut heading = String::new();
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(title) = line.strip_prefix('#') {
            heading = title.trim_start_matches('#').trim().to_owned();
        }
        let is_separator = |l: &str| l.starts_with("|---") || l.starts_with("| ---");
        if line.starts_with('|') && i + 1 < lines.len() && is_separator(lines[i + 1]) {
            let header = cells(line);
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].starts_with('|') {
                rows.push(cells(lines[i]));
                i += 1;
            }
            found.push((heading.clone(), Table { header, rows }));
            continue;
        }
        i += 1;
    }
    found
}

/// The first table under the heading `title`.
pub fn table(md: &str, title: &str) -> Table {
    tables(md)
        .into_iter()
        .find(|(heading, _)| heading == title)
        .map(|(_, table)| table)
        .unwrap_or_else(|| panic!("no table under heading {title:?} in:\n{md}"))
}

/// Maximal runs of digits and dots, without a trailing dot.
pub fn numeric_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || (c == '.' && !current.is_empty()) {
            current.push(c);
        } else if !current.is_empty() {
            tokens.push(current.trim_end_matches('.').to_owned());
            current.clear();
        }
    }
    tokens
}

/// Milliseconds with three decimals, as the summary prints microseconds.
pub fn ms(us: u64) -> String {
    format!("{}.{:03}", us / 1000, us % 1000)
}

fn collect(value: &Value, key: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => out.extend(numeric_tokens(s)),
        Value::Number(n) => {
            out.insert(n.to_string());
            if let Some(u) = n.as_u64() {
                out.insert(u.to_string());
                if key.ends_with("_us") {
                    out.insert(ms(u));
                }
            }
            if let Some(f) = n.as_f64() {
                out.insert(format!("{f:.1}"));
                out.insert(format!("{f:.2}"));
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, key, out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                collect(v, k, out);
            }
        }
        Value::Bool(_) | Value::Null => {}
    }
}

/// Asserts that every number in every table body cell of `md` is one of the
/// JSON report's values, in one of the renderings the summary uses.
pub fn assert_every_markdown_number_is_in_the_json(md: &str, json: &Value) {
    let mut known = BTreeSet::new();
    collect(json, "", &mut known);
    for (heading, table) in tables(md) {
        for row in &table.rows {
            for cell in row {
                for token in numeric_tokens(cell) {
                    assert!(
                        known.contains(&token),
                        "{token:?} in cell {cell:?} under {heading:?} is not a JSON value"
                    );
                }
            }
        }
    }
}

/// The markdown Subject cell for a JSON measurement.
pub fn subject_cell(measurement: &Value) -> String {
    if let Some(primitive) = measurement["primitive"].as_str() {
        primitive.to_owned()
    } else {
        format!("flow {}", measurement["flow"].as_str().unwrap())
    }
}

/// `—` for a JSON null, else the string.
pub fn or_dash(value: &Value) -> String {
    value.as_str().unwrap_or("—").to_owned()
}
