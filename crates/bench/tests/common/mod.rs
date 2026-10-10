//! Helpers shared by the integration tests: a scripted host, scratch
//! directories, a reader for the markdown summary's tables, and `mobile-agent`
//! with a daemon serving fake targets.

#![allow(dead_code)]

pub mod appium;
pub mod micro;
pub mod tools;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bench::cli::CliConfig;
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
                if key.ends_with("_ns") {
                    out.insert(micro::us(u));
                }
            }
            // The summary prints a signed number's sign apart from its
            // digits, as in `-4.9%`.
            if let Some(f) = n.as_f64() {
                for f in [f, f.abs()] {
                    out.insert(f.to_string());
                    out.insert(format!("{f:.1}"));
                    out.insert(format!("{f:.2}"));
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, key, out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                // An interval's bounds keep its unit: `spread_ns` holds `low`
                // and `high`.
                let k = if key.ends_with("_ns") || key.ends_with("_us") {
                    key
                } else {
                    k
                };
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

/// The `mobile-agent` CLI, built with its daemon beside this crate's binary on
/// first use: `cargo test --workspace` has built both already.
pub fn mobile_agent() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = Path::new(env!("CARGO_BIN_EXE_mobile-agent-bench"))
            .parent()
            .expect("the harness binary lives in a directory");
        let mut cargo = Command::new(env!("CARGO"));
        cargo
            .args(["build", "--quiet", "-p", "cli", "-p", "daemon", "--bins"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if dir.file_name().is_some_and(|name| name == "release") {
            cargo.arg("--release");
        }
        let status = cargo.status().expect("cargo runs");
        assert!(status.success(), "cargo build -p cli -p daemon failed");
        dir.join("mobile-agent")
    })
    .clone()
}

/// The daemon beside [`mobile_agent`].
pub fn mobile_agent_daemon() -> PathBuf {
    mobile_agent().with_file_name("mobile-agent-daemon")
}

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A private socket directory whose daemon serves the fake targets `fake`
/// (`MOBILE_AGENT_FAKE_TARGETS`), with a home and working directory inside it
/// so no user or project settings apply. Dropping it stops the daemon and
/// removes the directory.
pub struct FakeDevices {
    pub dir: PathBuf,
    pub fake: String,
    /// How long the fake targets take to show a change
    /// (`MOBILE_AGENT_FAKE_LATENCY_MS`), so tap spans are not near zero.
    pub latency_ms: u32,
    /// How long detaching a fake target takes (`MOBILE_AGENT_FAKE_STOP_MS`),
    /// as stopping the Android agent takes its time.
    pub stop_ms: u32,
}

impl FakeDevices {
    /// Fake targets that show a change at once.
    pub fn new(fake: &str) -> Self {
        FakeDevices::slow(fake, 0)
    }

    /// Fake targets that show every change `latency_ms` late.
    pub fn slow(fake: &str, latency_ms: u32) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // The socket path must fit in 103 bytes.
        let base = std::env::temp_dir();
        let base = if base.as_os_str().len() > 60 {
            PathBuf::from("/tmp")
        } else {
            base
        };
        let dir = base.join(format!("mab-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("home")).unwrap();
        FakeDevices {
            dir,
            fake: fake.to_owned(),
            latency_ms,
            stop_ms: 0,
        }
    }

    /// These fake targets, each taking `stop_ms` to detach.
    pub fn stopping(mut self, stop_ms: u32) -> Self {
        self.stop_ms = stop_ms;
        self
    }

    /// The environment every command gets: the developer's `MOBILE_AGENT_*`
    /// variables and runtime directory removed, then this directory's own.
    pub fn env(&self) -> Vec<(OsString, Option<OsString>)> {
        let mut env: Vec<(OsString, Option<OsString>)> = std::env::vars_os()
            .map(|(key, _)| key)
            .filter(|key| {
                let key = key.to_string_lossy();
                key.starts_with("MOBILE_AGENT_") || key == "XDG_RUNTIME_DIR"
            })
            .map(|key| (key, None))
            .collect();
        let mut set = |key: &str, value: OsString| env.push((key.into(), Some(value)));
        set("HOME", self.dir.join("home").into());
        set("MOBILE_AGENT_SOCKET_DIR", self.dir.clone().into());
        set("MOBILE_AGENT_DAEMON_BIN", mobile_agent_daemon().into());
        set("MOBILE_AGENT_FAKE_TARGETS", self.fake.clone().into());
        set("MOBILE_AGENT_IDLE_TIMEOUT_MS", "60000".into());
        set(
            "MOBILE_AGENT_FAKE_LATENCY_MS",
            self.latency_ms.to_string().into(),
        );
        set("MOBILE_AGENT_FAKE_STOP_MS", self.stop_ms.to_string().into());
        env
    }

    /// The CLI adapter's configuration for this directory, writing its files
    /// under `work_dir`.
    pub fn config(&self, work_dir: &Path) -> CliConfig {
        let mut config = CliConfig::new(work_dir);
        config.program = mobile_agent();
        config.env = self.env();
        config.cwd = Some(self.dir.clone());
        config.command_timeout = Duration::from_secs(60);
        config
    }

    /// `mobile-agent --json <args>` in this directory's environment: its
    /// reply's `data`, asserting the command succeeded.
    pub fn cli(&self, args: &[&str]) -> Value {
        let mut command = Command::new(mobile_agent());
        for (key, value) in self.env() {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        let output = command
            .arg("--json")
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reply: Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("mobile-agent {args:?} printed no JSON ({e}): {stdout}"));
        assert_eq!(reply["success"], true, "mobile-agent {args:?}: {reply}");
        reply["data"].clone()
    }

    /// The pid of the daemon serving the directory, starting one if needed.
    pub fn daemon_pid(&self) -> u64 {
        self.cli(&["status"])["daemon"]["pid"].as_u64().unwrap()
    }
}

impl Drop for FakeDevices {
    fn drop(&mut self) {
        let mut command = Command::new(mobile_agent());
        for (key, value) in self.env() {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        let _ = command
            .arg("shutdown")
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A stand-in for `mobile-agent` that logs its arguments and answers from
/// canned replies: `--version` with 0.2.0, the first of `replies` whose
/// pattern the command line contains with its exit status, a settle with a
/// settled screen, and anything else with an empty success.
pub fn stand_in(dir: &Path, replies: &[(&str, &str, i32)]) -> PathBuf {
    let mut cases = String::new();
    for (n, (pattern, reply, status)) in replies.iter().enumerate() {
        fs::write(dir.join(format!("reply-{n}.json")), reply).unwrap();
        cases.push_str(&format!(
            "*\" {pattern} \"*) cat \"$dir/reply-{n}.json\"; exit {status} ;;\n"
        ));
    }
    fs::write(
        dir.join("settle.json"),
        r#"{"success":true,"data":{"elapsedMs":212.5,"frameChanges":1,"settled":true,"stillChanging":[],"treeChanges":1,"waited":"settle"},"error":null}"#,
    )
    .unwrap();
    fs::write(
        dir.join("empty.json"),
        r#"{"success":true,"data":{},"error":null}"#,
    )
    .unwrap();
    let path = dir.join("mobile-agent");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             dir=\"${{0%/*}}\"\n\
             echo \"$@\" >> \"$dir/mobile-agent.log\"\n\
             case \" $* \" in\n\
             *\" --version \"*) echo 'mobile-agent 0.2.0' ;;\n\
             {cases}\
             *\" --settle \"*) cat \"$dir/settle.json\" ;;\n\
             *) cat \"$dir/empty.json\" ;;\n\
             esac\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
