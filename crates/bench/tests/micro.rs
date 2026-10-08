//! Task 3.1: `mobile-agent-bench micro`, the runner of the host
//! microbenchmarks (design D15): the quiet-host guard, the reader of
//! criterion's files and of the suites' allocation lines, the comparison
//! against the committed baseline (design D16) and the two reports. No test
//! runs `cargo bench`: in-process, recorded suites write criterion's files
//! through `micro::Suites`; through the binary, a fake cargo does.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bench::micro::compare::{self, Baseline, Benchmark, Verdict};
use bench::micro::criterion::{self, Measured, Spread};
use bench::micro::guard::{self, Reading};
use bench::micro::report::Report;
use bench::micro::{self, BASELINE, CargoBench, Finished, Settings};
use bench::provenance::ScriptedProbe;
use common::micro::{
    FakeTools, Recorded, RecordedSuites, SHA, ScriptedHost, TOOLCHAIN, checkout, copy_dir, fixture,
    fixtures, process, provenance, quiet, reading, us,
};
use serde_json::{Value, json};

/// Settings for a run in the checkout `root` on a macOS host, the reports
/// going to `root/out`.
fn settings(root: &Path) -> Settings {
    Settings {
        out: Some(root.join("out")),
        os: "macos".to_owned(),
        ..Settings::new(root)
    }
}

/// A run of `suites` on a host quiet at a 1-minute load of 2.5.
fn run(settings: &Settings, suites: &mut RecordedSuites) -> Result<Finished, String> {
    let mut host = ScriptedHost::new(vec![quiet(2.5)]);
    micro::run(settings, &mut host, &common::micro::probe(), suites)
}

/// Writes `benchmarks` as the baseline of `root`, with the provenance of an
/// earlier run on the same host.
fn save_baseline(root: &Path, benchmarks: Vec<Measured>) {
    let baseline = Baseline {
        schema_version: micro::SCHEMA_VERSION,
        provenance: provenance(),
        benchmarks,
    };
    baseline.save(&root.join(BASELINE)).unwrap();
}

/// The benchmark `id` of `report`.
fn find<'a>(report: &'a Report, id: &str) -> &'a Benchmark {
    report
        .benchmarks
        .iter()
        .find(|b| b.measured.id == id)
        .unwrap_or_else(|| panic!("no benchmark {id} in {:?}", report.benchmarks))
}

/// The table of `id`'s crate in the markdown `md` and `id`'s row in it.
fn row(md: &str, id: &str) -> (common::Table, Vec<String>) {
    let krate = id.split('/').next().unwrap();
    let table = common::table(md, krate);
    let row = table
        .rows
        .iter()
        .find(|r| table.cell(r, "Benchmark") == id)
        .unwrap_or_else(|| panic!("no row for {id} under {krate} in:\n{md}"))
        .clone();
    (table, row)
}

/// The value of the provenance row `name`.
fn field(table: &common::Table, name: &str) -> String {
    table
        .rows
        .iter()
        .find(|r| r[0] == name)
        .map(|r| r[1].clone())
        .unwrap_or_else(|| panic!("no provenance row {name:?} in {table:?}"))
}

/// Runs `command` to its exit: the status, stdout and stderr.
fn output(mut command: Command) -> (Option<i32>, String, String) {
    let output = command.output().unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// Quiet-host guard.

/// `ps -A -o pid=,ppid=,pcpu=,comm=` on a host where the runner, pid 800,
/// started under `just` and runs cargo, which runs rustc, while another
/// session's emulator, a Gradle daemon's `java` and a Simulator are busy.
const LISTING: &str = "    1     0   0.1 /sbin/launchd
  500     1  13.3 /Applications/Warp.app/Contents/MacOS/stable
  600   500   0.0 -zsh
  700   600   0.4 just
  800   700  25.0 /opt/rust/target/release/mobile-agent-bench
  801   800  96.0 /opt/rust/toolchains/1.98.1/bin/cargo
  802   801 310.0 /opt/rust/toolchains/1.98.1/bin/rustc
  900     1  17.2 /opt/android-sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64-headless
  901     1   6.3 /usr/bin/java
  902     1   4.9 /usr/bin/java
  903     1  12.0 /Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator
  904     1   6.1 /Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/XPCServices/com.apple.CoreSimulator.CoreSimulatorService.xpc/Contents/MacOS/com.apple.CoreSimulator.CoreSimulatorService
  905     1  11.0 placeholder scanner
  906     1  50.0 /opt/rust/bin/cargo-clippy
  907     1   5.0 /opt/rust/toolchains/1.98.1/bin/rustc
";

#[test]
fn a_busy_host_is_refused_naming_the_load_and_the_busy_processes_and_nothing_is_measured() {
    let root = checkout("micro-busy");
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 125_000)]);
    let mut host = ScriptedHost::new(vec![reading(
        6.5,
        vec![process(4242, "qemu-system-aarch64-headless", 17.2)],
    )]);
    let error = micro::run(
        &settings(&root),
        &mut host,
        &common::micro::probe(),
        &mut suites,
    )
    .unwrap_err();
    for named in [
        "6.50, not under 4",
        "qemu-system-aarch64-headless (pid 4242, 17.2% of a core)",
        "nothing was measured",
    ] {
        assert!(error.contains(named), "{named:?} in {error}");
    }
    assert_eq!((host.checks, host.waits.len()), (1, 0));
    assert!(suites.runs.is_empty());
    assert!(!root.join("out").exists());

    // A load under the bound does not make up for a busy process.
    let mut host = ScriptedHost::new(vec![reading(2.0, vec![process(977, "java", 6.3)])]);
    let error = micro::run(
        &settings(&root),
        &mut host,
        &common::micro::probe(),
        &mut suites,
    )
    .unwrap_err();
    for named in ["load average is 2.00", "java (pid 977, 6.3% of a core)"] {
        assert!(error.contains(named), "{named:?} in {error}");
    }
    assert!(suites.runs.is_empty());
}

#[test]
fn a_run_asked_to_wait_measures_once_the_host_is_quiet_and_records_the_load_at_that_start() {
    let root = checkout("micro-wait");
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 125_000)]);
    let mut host = ScriptedHost::new(vec![
        reading(6.5, Vec::new()),
        reading(3.1, vec![process(5554, "emulator", 40.0)]),
        Reading {
            load_average: Some([3.2, 3.6, 4.1]),
            busy: Some(Vec::new()),
        },
        // Once the suites have run.
        Reading {
            load_average: Some([3.4, 3.5, 4.0]),
            busy: Some(Vec::new()),
        },
    ]);
    let mut settings = settings(&root);
    settings.wait = Duration::from_secs(5 * 60);
    let finished = micro::run(&settings, &mut host, &common::micro::probe(), &mut suites).unwrap();

    // Two checks found the host busy, each followed by a wait of 30 s that
    // says why; the third found it quiet, and the suites ran once.
    let waits: Vec<Duration> = host.waits.iter().map(|(d, _)| *d).collect();
    assert_eq!(waits, [Duration::from_secs(30); 2]);
    assert!(host.waits[0].1.contains("6.50"), "{:?}", host.waits);
    assert!(
        host.waits[1].1.contains("emulator (pid 5554"),
        "{:?}",
        host.waits
    );
    assert_eq!(host.checks, 4);
    assert_eq!(suites.runs.len(), 1);

    let p = &finished.report.provenance;
    assert_eq!(p.host.load_average, Some([3.2, 3.6, 4.1]));
    assert_eq!(p.load_average_end, Some([3.4, 3.5, 4.0]));
    assert_eq!((p.guard.max_load, p.guard.waited_s), (4.0, 60));
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let table = common::table(&md, "Provenance");
    assert_eq!(field(&table, "Load average at start"), "3.20 3.60 4.10");
    assert_eq!(field(&table, "Load average at end"), "3.40 3.50 4.00");
    assert_eq!(
        field(&table, "Guard"),
        "1-minute load under 4, after 60 s of waiting"
    );
}

#[test]
fn a_host_still_busy_at_the_wait_limit_is_refused_and_nothing_is_measured() {
    let root = checkout("micro-limit");
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 125_000)]);
    let mut host = ScriptedHost::new(vec![reading(
        6.5,
        vec![process(4242, "qemu-system-aarch64-headless", 17.2)],
    )]);
    let mut settings = settings(&root);
    settings.wait = Duration::from_secs(60);
    let error = micro::run(&settings, &mut host, &common::micro::probe(), &mut suites).unwrap_err();
    // Checked at the start, then 30 s and 60 s in, and refused at the limit.
    assert_eq!(host.checks, 3);
    assert_eq!(host.waits.len(), 2);
    for named in [
        "after waiting 1 minute",
        "6.50",
        "qemu-system-aarch64-headless",
        "nothing was measured",
    ] {
        assert!(error.contains(named), "{named:?} in {error}");
    }
    assert!(suites.runs.is_empty());
    assert!(!root.join("out").exists());
}

#[test]
fn the_guard_counts_watched_processes_above_five_percent_outside_its_own_process_tree() {
    let processes = guard::processes(LISTING);
    assert_eq!(processes.len(), 15);
    assert_eq!(
        processes[0],
        guard::Process {
            pid: 1,
            ppid: 0,
            cpu_pct: 0.1,
            name: "launchd".to_owned(),
        }
    );
    let names: Vec<&str> = processes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names[11], "com.apple.CoreSimulator.CoreSimulatorService");
    assert_eq!(names[12], "placeholder scanner");
    // The runner's cargo and rustc are its own; the java under 5% and the
    // rustc at exactly 5% are not busy; cargo-clippy, Warp and the scanner are
    // not watched.
    let found = guard::busy(&processes, 800);
    let busy: Vec<(u32, &str)> = found.iter().map(|p| (p.pid, p.name.as_str())).collect();
    assert_eq!(
        busy,
        [
            (900, "qemu-system-aarch64-headless"),
            (901, "java"),
            (903, "Simulator"),
            (904, "com.apple.CoreSimulator.CoreSimulatorService"),
        ]
    );
}

#[test]
fn the_guard_watches_the_builders_and_devices_the_reports_waited_for() {
    for name in [
        "cargo",
        "rustc",
        "xcodebuild",
        "java",
        "Simulator",
        "gradle",
        "kotlinc-jvm",
        "qemu-system-aarch64-headless",
        "emulator",
        "emulator64-crash-service",
        "com.apple.CoreSimulator.CoreSimulatorService",
    ] {
        assert!(guard::watched(name), "{name} is watched");
    }
    for name in [
        "cargo-clippy",
        "rustdoc",
        "javac",
        "SimulatorTrampoline",
        "placeholder scanner",
        "mobile-agent-bench",
    ] {
        assert!(!guard::watched(name), "{name} is not watched");
    }
}

#[test]
fn the_guard_reads_the_load_from_sysctl_or_proc_and_the_processes_from_ps() {
    let ps = "ps -A -o pid=,ppid=,pcpu=,comm=";
    let mac = ScriptedProbe::new()
        .with("sysctl -n vm.loadavg", "{ 6.50 5.00 4.00 }")
        .with(ps, LISTING);
    let found = guard::read(&mac, "macos", 800);
    assert_eq!(found.load_average, Some([6.5, 5.0, 4.0]));
    let pids: Vec<u32> = found.busy.unwrap().iter().map(|p| p.pid).collect();
    assert_eq!(pids, [900, 901, 903, 904]);

    let linux = ScriptedProbe::new()
        .with("cat /proc/loadavg", "0.52 0.58 0.59 1/1234 5678")
        .with(ps, "    1     0   0.0 systemd\n");
    assert_eq!(
        guard::read(&linux, "linux", 800),
        Reading {
            load_average: Some([0.52, 0.58, 0.59]),
            busy: Some(Vec::new()),
        }
    );

    // A host whose load or processes cannot be read is not quiet.
    let unknown = guard::read(&ScriptedProbe::new(), "macos", 800);
    assert_eq!(
        unknown,
        Reading {
            load_average: None,
            busy: None,
        }
    );
    let why = guard::noise(&unknown, 4.0).unwrap();
    assert!(why.contains("the load average cannot be read"), "{why}");
    assert!(why.contains("the process list cannot be read"), "{why}");
    // The 1-minute load must be under the bound, strictly.
    assert_eq!(guard::noise(&quiet(3.99), 4.0), None);
    let why = guard::noise(&quiet(4.0), 4.0).unwrap();
    assert!(why.contains("4.00, not under 4"), "{why}");
}

#[test]
fn save_baseline_is_refused_with_a_raised_load_bound_or_a_filter_before_the_host_is_checked() {
    let root = checkout("micro-save-refused");
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 125_000)]);
    let mut host = ScriptedHost::new(vec![quiet(2.5)]);

    let mut raised = settings(&root);
    raised.save_baseline = true;
    raised.max_load = 6.0;
    let error = micro::run(&raised, &mut host, &common::micro::probe(), &mut suites).unwrap_err();
    assert!(error.contains("--max-load 6"), "{error}");
    assert!(error.contains("under 4"), "{error}");

    let mut filtered = settings(&root);
    filtered.save_baseline = true;
    filtered.filter = Some("protocol/decode".to_owned());
    let error = micro::run(&filtered, &mut host, &common::micro::probe(), &mut suites).unwrap_err();
    assert!(error.contains("--filter"), "{error}");

    assert_eq!(host.checks, 0);
    assert!(suites.runs.is_empty());
    assert!(!root.join(BASELINE).exists());

    // A bound under the default is stricter, and a baseline may come from it.
    let mut stricter = settings(&root);
    stricter.save_baseline = true;
    stricter.max_load = 3.0;
    let finished = run(&stricter, &mut suites).unwrap();
    assert_eq!(finished.baseline, Some(root.join(BASELINE)));
    assert_eq!(finished.report.provenance.guard.max_load, 3.0);
}

// The reader of criterion's files and of the allocation lines.

#[test]
fn criterion_s_files_as_spike_s8_recorded_them_give_the_median_its_interval_and_the_samples() {
    let read = criterion::read(&fixtures().join("s8-home"), "").unwrap();
    assert_eq!(
        read,
        [
            Measured {
                id: "decode".to_owned(),
                median_ns: 124_952,
                spread_ns: Spread {
                    low: 124_596,
                    high: 125_234,
                },
                samples: 100,
                allocations: None,
                bytes_allocated: None,
            },
            Measured {
                id: "decode_slow".to_owned(),
                median_ns: 131_275,
                spread_ns: Spread {
                    low: 130_868,
                    high: 131_548,
                },
                samples: 100,
                allocations: None,
                bytes_allocated: None,
            },
        ]
    );
}

#[test]
fn only_criterion_s_new_files_count_and_an_id_is_the_benchmark_s_path_under_criterion_home() {
    let home = common::scratch("micro-criterion-home");
    let s8 = fixtures().join("s8-home");
    let dir = home.join("protocol/decode/snapshot-500");
    copy_dir(&s8.join("decode/new"), &dir.join("new"));
    // The copy criterion keeps of an earlier run and the change it computed
    // against it, both of another value.
    copy_dir(&s8.join("decode_slow/new"), &dir.join("base"));
    copy_dir(&s8.join("decode_slow/new"), &dir.join("change"));
    let read = criterion::read(&home, "").unwrap();
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0].id, "protocol/decode/snapshot-500");
    assert_eq!(read[0].median_ns, 124_952);

    // A benchmark whose sample count is not there is an error naming it.
    fs::remove_file(dir.join("new/sample.json")).unwrap();
    let error = criterion::read(&home, "").unwrap_err();
    assert!(error.contains("protocol/decode/snapshot-500"), "{error}");
    assert!(error.contains("sample.json"), "{error}");
}

#[test]
fn allocation_lines_go_to_the_benchmarks_that_ran_and_the_lines_of_skipped_ones_are_dropped() {
    let listed = fixture("list.stdout");
    let lines = criterion::allocations(&listed);
    assert_eq!(lines.len(), 23);
    assert_eq!(lines["protocol/decode/snapshot-500"], (765, 95_687));
    assert_eq!(lines["daemon/wire/reply-snapshot-big"], (21, 295_346));
    assert_eq!(lines["protocol/frame-decoder/events-10k"], (2, 196_608));

    // Under --list no benchmark runs, so none is read, whatever the lines.
    let empty = common::scratch("micro-criterion-listed");
    assert_eq!(criterion::read(&empty, &listed).unwrap(), []);

    // A filtered run prints every suite's lines and measures two
    // benchmarks; S8's round stands in for criterion's own output.
    let home = common::scratch("micro-criterion-filtered");
    let s8 = fixtures().join("s8-home");
    copy_dir(
        &s8.join("decode/new"),
        &home.join("protocol/decode/snapshot-500/new"),
    );
    copy_dir(
        &s8.join("decode_slow/new"),
        &home.join("daemon/socket/snapshot-interactive-big/new"),
    );
    let stdout = format!("{listed}{}", fixture("s8-round-01.txt"));
    let read = criterion::read(&home, &stdout).unwrap();
    let counted: Vec<(&str, Option<u64>, Option<u64>)> = read
        .iter()
        .map(|m| (m.id.as_str(), m.allocations, m.bytes_allocated))
        .collect();
    assert_eq!(
        counted,
        [
            ("daemon/socket/snapshot-interactive-big", None, None),
            ("protocol/decode/snapshot-500", Some(765), Some(95_687)),
        ]
    );
}

#[test]
fn an_allocation_line_names_an_id_and_two_counts_and_a_later_one_replaces_an_earlier_one() {
    let lines = criterion::allocations(
        "allocations a/b/c 1 2\n\
         allocations a/b/c 3 4\n\
         allocations a/b/d 5\n\
         allocations a/b/e five 6\n\
         Benchmarking allocations a/b/f 7 8\n",
    );
    let lines: Vec<(String, (u64, u64))> = lines.into_iter().collect();
    assert_eq!(lines, [("a/b/c".to_owned(), (3, 4))]);
}

#[test]
fn criterion_s_version_comes_from_the_lock_file() {
    let lock = fs::read_to_string(checkout("micro-lock").join("Cargo.lock")).unwrap();
    assert_eq!(criterion::version(&lock).as_deref(), Some("0.8.2"));
    assert_eq!(
        criterion::version("[[package]]\nname = \"serde\"\nversion = \"1.0.229\"\n"),
        None
    );
    // The workspace pins the version whose files the reader is tested on
    // (design D13).
    let workspace =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")).unwrap();
    assert_eq!(criterion::version(&workspace).as_deref(), Some("0.8.2"));
}

#[test]
fn the_runner_names_every_suite_s_bench_target_and_passes_noplot_and_the_filter() {
    assert_eq!(
        CargoBench::args(None).join(" "),
        "bench --workspace --bench codec --bench mirror --bench daemon --bench frames -- --noplot"
    );
    assert_eq!(
        CargoBench::args(Some("protocol/decode")).join(" "),
        "bench --workspace --bench codec --bench mirror --bench daemon --bench frames -- --noplot \
         protocol/decode"
    );
}

// The comparison against the committed baseline.

#[test]
fn a_snapshot_decode_from_59_to_77_us_past_a_10_percent_threshold_is_a_regression_and_exits_1() {
    let root = checkout("micro-regression");
    save_baseline(
        &root,
        vec![
            Recorded::new("protocol/decode/snapshot-500", 59_000)
                .allocating(765, 95_687)
                .measured(),
            Recorded::new("protocol/encode/snapshot-500", 20_000)
                .allocating(0, 0)
                .measured(),
        ],
    );
    let mut suites = RecordedSuites::new(vec![
        Recorded::new("protocol/decode/snapshot-500", 77_000).allocating(765, 95_687),
        Recorded::new("protocol/encode/snapshot-500", 20_100).allocating(0, 0),
    ]);
    let mut settings = settings(&root);
    settings.threshold_pct = 10.0;
    let report = run(&settings, &mut suites).unwrap().report;

    assert_eq!(report.summary.exit_status, 1);
    assert_eq!((report.summary.regressions, report.summary.same), (1, 1));
    assert_eq!(
        report.summary.reason,
        "1 regression against the baseline: protocol/decode/snapshot-500"
    );
    let decode = find(&report, "protocol/decode/snapshot-500");
    assert_eq!(decode.baseline.as_ref().map(|b| b.median_ns), Some(59_000));
    assert_eq!(decode.difference_pct, Some(30.5));
    assert_eq!(
        (
            decode.time_verdict,
            decode.allocations_verdict,
            decode.verdict
        ),
        (
            Some(Verdict::Regression),
            Some(Verdict::Same),
            Verdict::Regression
        )
    );

    // What the run prints at its end names the benchmark, both medians and
    // the threshold, as the markdown does.
    let printed = report.verdict();
    let md = report.to_markdown();
    for text in [printed.as_str(), md.as_str()] {
        let table = common::table(text, "Regressions");
        assert_eq!(table.rows.len(), 1, "{text}");
        let row = &table.rows[0];
        assert_eq!(table.cell(row, "Benchmark"), "protocol/decode/snapshot-500");
        assert_eq!(table.cell(row, "Measure"), "median µs");
        assert_eq!(table.cell(row, "Baseline"), "59.000");
        assert_eq!(table.cell(row, "Measured"), "77.000");
        assert_eq!(table.cell(row, "Difference"), "+30.5%");
        assert_eq!(table.cell(row, "Threshold"), "10.0%");
        assert!(
            text.contains(
                "Exit status 1: 1 regression against the baseline: protocol/decode/snapshot-500"
            ),
            "{text}"
        );
    }
}

#[test]
fn medians_within_the_threshold_with_unchanged_allocations_exit_0_and_show_each_difference() {
    let root = checkout("micro-within");
    save_baseline(
        &root,
        vec![
            Recorded::new("protocol/decode/response-snapshot", 1_000)
                .allocating(17, 2_412)
                .measured(),
            Recorded::new("protocol/decode/snapshot-500", 100_000)
                .allocating(765, 95_687)
                .measured(),
            Recorded::new("simulator/pixels_hash/460x1000", 50_000).measured(),
            Recorded::new("simulator/scale_bgra/1206x2622-to-460x1000", 1_100_000).measured(),
        ],
    );
    let mut suites = RecordedSuites::new(vec![
        Recorded::new("protocol/decode/response-snapshot", 1_000).allocating(17, 2_412),
        Recorded::new("protocol/decode/snapshot-500", 104_900).allocating(765, 95_687),
        Recorded::new("simulator/pixels_hash/460x1000", 47_550),
        Recorded::new("simulator/scale_bgra/1206x2622-to-460x1000", 1_155_000),
    ]);
    let finished = run(&settings(&root), &mut suites).unwrap();
    let report = &finished.report;
    assert_eq!(report.summary.exit_status, 0);
    assert_eq!(report.summary.reason, "no regression against the baseline");
    assert_eq!(report.summary.same, 4);

    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    // 5.0% is not more than the 5% threshold.
    for (id, difference, shown) in [
        ("protocol/decode/response-snapshot", 0.0, "+0.0%"),
        ("protocol/decode/snapshot-500", 4.9, "+4.9%"),
        ("simulator/pixels_hash/460x1000", -4.9, "-4.9%"),
        ("simulator/scale_bgra/1206x2622-to-460x1000", 5.0, "+5.0%"),
    ] {
        let b = find(report, id);
        assert_eq!(b.difference_pct, Some(difference), "{id}");
        assert_eq!(b.verdict, Verdict::Same, "{id}");
        let (table, row) = row(&md, id);
        assert_eq!(table.cell(&row, "Difference"), shown, "{id}");
        assert_eq!(table.cell(&row, "Verdict"), "same", "{id}");
    }
    assert!(
        md.contains("Exit status 0: no regression against the baseline"),
        "{md}"
    );
    assert!(!md.contains("## Regressions"), "{md}");
}

#[test]
fn a_median_faster_by_more_than_the_threshold_is_an_improvement_and_exits_0() {
    let root = checkout("micro-improvement");
    let id = "daemon/wire/reply-snapshot-big";
    save_baseline(
        &root,
        vec![
            Recorded::new(id, 400_000)
                .allocating(21, 295_346)
                .measured(),
        ],
    );
    let mut suites = RecordedSuites::new(vec![Recorded::new(id, 320_000).allocating(21, 295_346)]);
    let report = run(&settings(&root), &mut suites).unwrap().report;
    assert_eq!(report.summary.exit_status, 0);
    assert_eq!(report.summary.improvements, 1);
    let b = find(&report, id);
    assert_eq!(
        (b.difference_pct, b.time_verdict, b.verdict),
        (
            Some(-20.0),
            Some(Verdict::Improvement),
            Verdict::Improvement
        )
    );
    let table = common::table(&report.verdict(), "Improvements");
    assert_eq!(table.rows.len(), 1);
    let row = &table.rows[0];
    assert_eq!(table.cell(row, "Benchmark"), id);
    assert_eq!(table.cell(row, "Baseline"), "400.000");
    assert_eq!(table.cell(row, "Measured"), "320.000");
    assert_eq!(table.cell(row, "Difference"), "-20.0%");
}

#[test]
fn a_change_in_allocations_is_a_regression_or_an_improvement_at_any_size_whatever_the_time() {
    let root = checkout("micro-allocations");
    let id = "mirror/snapshot/interactive-grid";
    save_baseline(
        &root,
        vec![
            Recorded::new(id, 300_000)
                .allocating(412, 40_000)
                .measured(),
        ],
    );
    // 30% faster, which alone would be an improvement, with 6 more
    // allocations.
    let mut suites = RecordedSuites::new(vec![Recorded::new(id, 210_000).allocating(418, 40_000)]);
    let report = run(&settings(&root), &mut suites).unwrap().report;
    assert_eq!(report.summary.exit_status, 1);
    let b = find(&report, id);
    assert_eq!(
        (b.time_verdict, b.allocations_verdict, b.verdict),
        (
            Some(Verdict::Improvement),
            Some(Verdict::Regression),
            Verdict::Regression
        )
    );
    let printed = report.verdict();
    let regressions = common::table(&printed, "Regressions");
    assert_eq!(regressions.rows.len(), 1, "{printed}");
    let row = &regressions.rows[0];
    assert_eq!(regressions.cell(row, "Measure"), "allocations");
    assert_eq!(
        (
            regressions.cell(row, "Baseline"),
            regressions.cell(row, "Measured")
        ),
        ("412", "418")
    );
    assert_eq!(regressions.cell(row, "Threshold"), "exact");
    let improvements = common::table(&printed, "Improvements");
    assert_eq!(improvements.rows.len(), 1, "{printed}");
    assert_eq!(
        improvements.cell(&improvements.rows[0], "Difference"),
        "-30.0%"
    );

    // Fewer allocations improve; bytes count alone; unchanged is the same;
    // a benchmark counted on one side only is compared on its time.
    let counted = |n, bytes| Recorded::new(id, 300_000).allocating(n, bytes).measured();
    let judge = |baseline: Measured, measured: Measured| {
        let baseline = Baseline {
            schema_version: micro::SCHEMA_VERSION,
            provenance: provenance(),
            benchmarks: vec![baseline],
        };
        let mut compared = compare::compare(&[measured], Some(&baseline), 5.0);
        let b = compared.benchmarks.remove(0);
        (b.allocations_verdict, b.verdict)
    };
    assert_eq!(
        judge(counted(418, 40_000), counted(412, 40_000)),
        (Some(Verdict::Improvement), Verdict::Improvement)
    );
    assert_eq!(
        judge(counted(412, 40_000), counted(412, 40_008)),
        (Some(Verdict::Regression), Verdict::Regression)
    );
    assert_eq!(
        judge(counted(412, 40_000), counted(411, 40_100)),
        (Some(Verdict::Regression), Verdict::Regression)
    );
    assert_eq!(
        judge(counted(412, 40_000), counted(412, 40_000)),
        (Some(Verdict::Same), Verdict::Same)
    );
    assert_eq!(
        judge(Recorded::new(id, 300_000).measured(), counted(412, 40_000)),
        (None, Verdict::Same)
    );
}

#[test]
fn a_new_benchmark_and_a_missing_one_are_listed_without_changing_the_exit_status() {
    let root = checkout("micro-new-missing");
    save_baseline(
        &root,
        vec![
            Recorded::new("android/frame-hasher/key-540x1200", 53_000).measured(),
            Recorded::new("protocol/decode/snapshot-500", 125_000)
                .allocating(765, 95_687)
                .measured(),
        ],
    );
    let new = Recorded::new("mirror/pump/first-read-grid", 410_000).allocating(2_048, 180_000);
    let mut suites = RecordedSuites::new(vec![
        Recorded::new("android/frame-hasher/key-540x1200", 53_500),
        new.clone(),
    ]);
    let finished = run(&settings(&root), &mut suites).unwrap();
    let report = &finished.report;
    assert_eq!(report.summary.exit_status, 0);
    assert_eq!(
        (
            report.summary.same,
            report.summary.new,
            report.summary.missing
        ),
        (1, 1, 1)
    );
    let b = find(report, "mirror/pump/first-read-grid");
    assert_eq!(b.measured, new.measured());
    assert_eq!(
        (
            b.verdict,
            b.baseline.as_ref(),
            b.difference_pct,
            b.time_verdict
        ),
        (Verdict::New, None, None, None)
    );
    let missing: Vec<&str> = report.missing.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(missing, ["protocol/decode/snapshot-500"]);

    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let (table, row) = row(&md, "mirror/pump/first-read-grid");
    assert_eq!(table.cell(&row, "Verdict"), "new");
    assert_eq!(table.cell(&row, "Median µs"), "410.000");
    assert_eq!(table.cell(&row, "Allocations"), "2048");
    assert_eq!(table.cell(&row, "Bytes"), "180000");
    assert_eq!(table.cell(&row, "Baseline µs"), "—");
    assert_eq!(table.cell(&row, "Difference"), "—");
    let missing = common::table(&md, "Missing");
    assert_eq!(missing.rows.len(), 1);
    let row = &missing.rows[0];
    assert_eq!(
        missing.cell(row, "Benchmark"),
        "protocol/decode/snapshot-500"
    );
    assert_eq!(missing.cell(row, "Baseline µs"), "125.000");
    assert_eq!(missing.cell(row, "Baseline allocations"), "765");
    assert_eq!(missing.cell(row, "Baseline bytes"), "95687");
}

#[test]
fn without_a_baseline_every_benchmark_is_new_and_the_report_says_there_was_none() {
    let root = checkout("micro-no-baseline");
    let mut suites = RecordedSuites::new(vec![
        Recorded::new("protocol/decode/snapshot-500", 125_000),
        Recorded::new("simulator/encode/460x1000-q80", 950_000),
    ]);
    let finished = run(&settings(&root), &mut suites).unwrap();
    let report = &finished.report;
    assert_eq!(report.summary.exit_status, 0);
    assert_eq!(report.summary.new, 2);
    assert_eq!(
        report.summary.reason,
        "no baseline at reports/micro/baseline.json, so every benchmark is new"
    );
    assert!(report.benchmarks.iter().all(|b| b.verdict == Verdict::New));
    assert_eq!(report.baseline.path, "reports/micro/baseline.json");
    assert_eq!(report.baseline.provenance, None);
    assert!(report.missing.is_empty());
    assert_eq!(finished.baseline, None);
    assert!(!root.join(BASELINE).exists());
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    assert!(
        md.contains("No baseline at `reports/micro/baseline.json`"),
        "{md}"
    );
    assert!(
        report
            .verdict()
            .contains("Exit status 0: no baseline at reports/micro/baseline.json"),
        "{}",
        report.verdict()
    );
}

#[test]
fn differences_are_percentages_of_the_baseline_to_a_tenth() {
    assert_eq!(compare::difference_pct(59_000, 77_000), Some(30.5));
    assert_eq!(compare::difference_pct(100_000, 95_000), Some(-5.0));
    assert_eq!(compare::difference_pct(100_000, 104_940), Some(4.9));
    // A difference that rounds to nothing has no sign.
    let nothing = compare::difference_pct(100_000, 99_996).unwrap();
    assert_eq!(nothing, 0.0);
    assert!(nothing.is_sign_positive());
    assert_eq!(compare::difference_pct(0, 10), None);
}

#[test]
fn a_baseline_from_another_environment_is_compared_and_said_to_be_from_it() {
    let root = checkout("micro-elsewhere");
    let mut elsewhere = provenance();
    elsewhere.host.os = Some("macOS 26.5 (25F71)".to_owned());
    elsewhere.toolchain = Some("rustc 1.97.0 (7a1b2c3d4 2026-07-21)".to_owned());
    Baseline {
        schema_version: micro::SCHEMA_VERSION,
        provenance: elsewhere,
        benchmarks: vec![Recorded::new("protocol/decode/snapshot-500", 125_000).measured()],
    }
    .save(&root.join(BASELINE))
    .unwrap();
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 126_000)]);
    let report = run(&settings(&root), &mut suites).unwrap().report;
    assert_eq!(
        report.baseline.environment_differences,
        ["host.os", "toolchain"]
    );
    assert_eq!(
        find(&report, "protocol/decode/snapshot-500").verdict,
        Verdict::Same
    );
    let md = report.to_markdown();
    assert!(md.contains("host.os, toolchain"), "{md}");
}

// The reports.

#[test]
fn the_json_report_and_the_markdown_summary_agree_on_every_number_and_every_comparison() {
    let root = checkout("micro-two-outputs");
    save_baseline(
        &root,
        vec![
            Recorded::new("daemon/wire/parse-snapshot-request", 950)
                .allocating(8, 680)
                .measured(),
            Recorded::new("mirror/snapshot/interactive-grid", 300_000)
                .allocating(412, 40_000)
                .measured(),
            Recorded::new("protocol/decode/snapshot-500", 59_000)
                .allocating(765, 95_687)
                .measured(),
            Recorded::new("protocol/encode/snapshot-500", 26_900)
                .allocating(0, 0)
                .measured(),
            Recorded::new("simulator/content_hash/460x1000", 110_000).measured(),
        ],
    );
    let mut suites = RecordedSuites::new(vec![
        // New.
        Recorded::new("android/frame-hasher/key-540x1200", 53_123),
        // +1.2%, the same.
        Recorded::new("daemon/wire/parse-snapshot-request", 961).allocating(8, 680),
        // +3.3% with 6 more allocations.
        Recorded::new("mirror/snapshot/interactive-grid", 310_000).allocating(418, 40_000),
        // +30.5%.
        Recorded::new("protocol/decode/snapshot-500", 77_000).allocating(765, 95_687),
        // -21.9%, in 87 samples.
        Recorded::new("protocol/encode/snapshot-500", 21_000)
            .allocating(0, 0)
            .sampled(87),
    ]);
    let finished = run(&settings(&root), &mut suites).unwrap();
    assert_eq!(finished.files.json, root.join("out/report.json"));
    assert_eq!(finished.files.markdown, root.join("out/report.md"));
    let text = fs::read_to_string(&finished.files.json).unwrap();
    let json: Value = serde_json::from_str(&text).unwrap();
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    assert_eq!(Report::from_json(&text).unwrap(), finished.report);

    // The benchmarks, grouped by crate.
    let headings: Vec<String> = common::tables(&md)
        .into_iter()
        .map(|(heading, _)| heading)
        .collect();
    for krate in ["android", "daemon", "mirror", "protocol"] {
        assert!(
            headings.iter().any(|h| h == krate),
            "{krate} in {headings:?}"
        );
    }
    let benchmarks = json["benchmarks"].as_array().unwrap();
    assert_eq!(benchmarks.len(), 5);
    let ns = |v: &Value| us(v.as_u64().unwrap());
    let count = |v: &Value| match v {
        Value::Null => "—".to_owned(),
        n => n.to_string(),
    };
    for b in benchmarks {
        let id = b["id"].as_str().unwrap();
        let (table, row) = row(&md, id);
        assert_eq!(table.cell(&row, "Median µs"), ns(&b["median_ns"]), "{id}");
        assert_eq!(
            table.cell(&row, "95% interval µs"),
            format!(
                "{}–{}",
                ns(&b["spread_ns"]["low"]),
                ns(&b["spread_ns"]["high"])
            ),
            "{id}"
        );
        assert_eq!(
            table.cell(&row, "Samples"),
            b["samples"].to_string(),
            "{id}"
        );
        assert_eq!(
            table.cell(&row, "Allocations"),
            count(&b["allocations"]),
            "{id}"
        );
        assert_eq!(
            table.cell(&row, "Bytes"),
            count(&b["bytes_allocated"]),
            "{id}"
        );
        let baseline = match &b["baseline"] {
            Value::Null => "—".to_owned(),
            baseline => ns(&baseline["median_ns"]),
        };
        assert_eq!(table.cell(&row, "Baseline µs"), baseline, "{id}");
        let difference = match b["difference_pct"].as_f64() {
            None => "—".to_owned(),
            Some(d) => format!("{d:+.1}%"),
        };
        assert_eq!(table.cell(&row, "Difference"), difference, "{id}");
        let verdict = b["verdict"].as_str().unwrap();
        assert!(
            table.cell(&row, "Verdict").starts_with(verdict),
            "{id}: {} against {verdict}",
            table.cell(&row, "Verdict")
        );
    }
    assert_eq!(
        find(&finished.report, "protocol/encode/snapshot-500").difference_pct,
        Some(-21.9)
    );

    let summary = &json["summary"];
    for (key, expected) in [
        ("benchmarks", 5),
        ("regressions", 2),
        ("improvements", 1),
        ("same", 1),
        ("new", 1),
        ("missing", 1),
        ("exit_status", 1),
    ] {
        assert_eq!(summary[key], expected, "{key}");
    }
    let line = format!("Exit status 1: {}", summary["reason"].as_str().unwrap());
    assert!(md.contains(&line), "{line:?} in {md}");

    // Each measure that moved, in both outputs.
    let measures = |table: &common::Table| -> Vec<(String, String)> {
        table
            .rows
            .iter()
            .map(|r| {
                (
                    table.cell(r, "Benchmark").to_owned(),
                    table.cell(r, "Measure").to_owned(),
                )
            })
            .collect()
    };
    let regressions = common::table(&md, "Regressions");
    assert_eq!(
        measures(&regressions),
        [
            (
                "mirror/snapshot/interactive-grid".to_owned(),
                "allocations".to_owned()
            ),
            (
                "protocol/decode/snapshot-500".to_owned(),
                "median µs".to_owned()
            ),
        ]
    );
    let improvements = common::table(&md, "Improvements");
    assert_eq!(
        measures(&improvements),
        [(
            "protocol/encode/snapshot-500".to_owned(),
            "median µs".to_owned()
        )]
    );
    let missing = common::table(&md, "Missing");
    assert_eq!(missing.rows.len(), 1);
    assert_eq!(
        missing.cell(&missing.rows[0], "Benchmark"),
        "simulator/content_hash/460x1000"
    );
    common::assert_every_markdown_number_is_in_the_json(&md, &json);
}

#[test]
fn both_reports_carry_the_provenance_of_the_device_reports_with_the_harness_and_the_loads() {
    let root = checkout("micro-provenance");
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 125_000)]);
    let mut host = ScriptedHost::new(vec![
        Reading {
            load_average: Some([2.89, 3.41, 4.12]),
            busy: Some(Vec::new()),
        },
        Reading {
            load_average: Some([3.27, 3.93, 5.49]),
            busy: Some(Vec::new()),
        },
    ]);
    let mut settings = settings(&root);
    settings.threshold_pct = 7.5;
    let finished = micro::run(&settings, &mut host, &common::micro::probe(), &mut suites).unwrap();
    let json: Value =
        serde_json::from_str(&fs::read_to_string(&finished.files.json).unwrap()).unwrap();
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["threshold_pct"], 7.5);
    let p = &json["provenance"];
    assert_eq!(p["host"]["hardware"], "Apple M1 Max");
    assert_eq!(p["host"]["cpu_cores"], 10);
    assert_eq!(p["host"]["memory_bytes"], 68_719_476_736_u64);
    assert_eq!(p["host"]["os"], "macOS 26.6.2 (25G83)");
    assert_eq!(p["host"]["load_average"], json!([2.89, 3.41, 4.12]));
    assert_eq!(p["load_average_end"], json!([3.27, 3.93, 5.49]));
    assert_eq!(p["commit"], json!({"sha": SHA, "dirty": false}));
    assert_eq!(p["toolchain"], TOOLCHAIN);
    assert_eq!(
        p["harness"],
        json!({"library": "criterion", "version": "0.8.2", "warm_up_s": 3, "measurement_s": 5,
               "sample_size": 100, "resamples": 100_000, "confidence_pct": 95})
    );
    assert_eq!(p["guard"], json!({"max_load": 4.0, "waited_s": 0}));
    let started = p["started_at"].as_str().unwrap();
    let ended = p["finished_at"].as_str().unwrap();
    assert!(
        started.ends_with('Z') && ended.ends_with('Z'),
        "{started} {ended}"
    );

    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let table = common::table(&md, "Provenance");
    let commit = format!("{SHA} (clean)");
    for (name, value) in [
        ("Host hardware", "Apple M1 Max"),
        ("Host CPU cores", "10"),
        ("Host memory bytes", "68719476736"),
        ("Host OS", "macOS 26.6.2 (25G83)"),
        ("Load average at start", "2.89 3.41 4.12"),
        ("Load average at end", "3.27 3.93 5.49"),
        ("Commit under test", commit.as_str()),
        ("Rust toolchain", TOOLCHAIN),
        ("Harness", "criterion 0.8.2"),
        (
            "Sample settings",
            "3 s warm-up, 5 s measurement, 100 samples, 100000 bootstrap resamples, 95% intervals",
        ),
        ("Guard", "1-minute load under 4, after 0 s of waiting"),
    ] {
        assert_eq!(field(&table, name), value, "{name}");
    }
    assert!(
        md.contains(&format!("Started {started}, finished {ended}.")),
        "{md}"
    );
    assert!(md.contains("7.5%"), "{md}");
    common::assert_every_markdown_number_is_in_the_json(&md, &json);
}

#[test]
fn a_saved_baseline_holds_the_run_s_values_and_provenance_and_the_next_run_compares_against_them() {
    let root = checkout("micro-save");
    let first = vec![
        Recorded::new("protocol/decode/snapshot-500", 125_000).allocating(765, 95_687),
        Recorded::new("android/frame-hasher/key-540x1200", 53_000),
    ];
    let mut saving = settings(&root);
    saving.out = Some(root.join("out-1"));
    saving.save_baseline = true;
    let saved = run(&saving, &mut RecordedSuites::new(first.clone())).unwrap();
    let path = root.join(BASELINE);
    assert_eq!(saved.baseline.as_deref(), Some(path.as_path()));
    assert!(saved.report.baseline.saved);
    // It compared against no baseline before it wrote one.
    assert_eq!(saved.report.summary.new, 2);

    let text = fs::read_to_string(&path).unwrap();
    let file = Baseline::from_json(&text).unwrap();
    assert_eq!(file.schema_version, 1);
    assert_eq!(file.provenance, saved.report.provenance);
    let mut values: Vec<Measured> = first.iter().map(Recorded::measured).collect();
    values.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(file.benchmarks, values);
    // The fields of design D16.
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        json["benchmarks"][1],
        json!({"id": "protocol/decode/snapshot-500", "median_ns": 125_000,
               "spread_ns": {"low": 123_750, "high": 126_250}, "samples": 100,
               "allocations": 765, "bytes_allocated": 95_687})
    );

    // The next run compares against it, and leaves it as it was.
    let mut next = settings(&root);
    next.out = Some(root.join("out-2"));
    let compared = run(
        &next,
        &mut RecordedSuites::new(vec![
            Recorded::new("protocol/decode/snapshot-500", 162_500).allocating(765, 95_687),
            Recorded::new("android/frame-hasher/key-540x1200", 53_100),
        ]),
    )
    .unwrap();
    let report = &compared.report;
    assert_eq!(
        report.baseline.provenance.as_ref(),
        Some(&saved.report.provenance)
    );
    assert_eq!(
        report.baseline.environment_differences,
        Vec::<String>::new()
    );
    assert!(!report.baseline.saved);
    assert_eq!(compared.baseline, None);
    let decode = find(report, "protocol/decode/snapshot-500");
    assert_eq!(
        (decode.difference_pct, decode.verdict),
        (Some(30.0), Verdict::Regression)
    );
    assert_eq!(
        find(report, "android/frame-hasher/key-540x1200").verdict,
        Verdict::Same
    );
    assert_eq!(report.summary.exit_status, 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn a_run_that_cannot_complete_is_an_error_the_runner_exits_2_on() {
    let root = checkout("micro-cannot");
    // The suites fail.
    let mut failing = RecordedSuites::new(Vec::new());
    failing.fail = Some("cargo bench failed: exit status: 101".to_owned());
    let error = run(&settings(&root), &mut failing).unwrap_err();
    assert!(error.contains("exit status: 101"), "{error}");

    // They measure nothing, as a filter that matches no benchmark leaves
    // them.
    let mut nothing = settings(&root);
    nothing.out = Some(root.join("out-nothing"));
    nothing.filter = Some("no-such-benchmark".to_owned());
    let error = run(&nothing, &mut RecordedSuites::new(Vec::new())).unwrap_err();
    assert!(error.contains("no benchmark"), "{error}");
    assert!(error.contains("no-such-benchmark"), "{error}");

    // criterion's directory of an earlier run in the same output directory
    // is not read again.
    let mut suites =
        RecordedSuites::new(vec![Recorded::new("protocol/decode/snapshot-500", 125_000)]);
    let error = run(&settings(&root), &mut suites).unwrap_err();
    assert!(error.contains("criterion"), "{error}");
    assert!(suites.runs.is_empty());

    // A baseline that does not read is refused before the host is checked.
    let corrupt = checkout("micro-corrupt");
    fs::create_dir_all(corrupt.join("reports/micro")).unwrap();
    fs::write(corrupt.join(BASELINE), "{\"schema_version\": 1").unwrap();
    let mut host = ScriptedHost::new(vec![quiet(2.5)]);
    let error = micro::run(
        &settings(&corrupt),
        &mut host,
        &common::micro::probe(),
        &mut RecordedSuites::new(Vec::new()),
    )
    .unwrap_err();
    assert!(error.contains("reports/micro/baseline.json"), "{error}");
    assert_eq!(host.checks, 0);
}

// Through the binary.

#[test]
fn micro_runs_the_suites_through_cargo_bench_with_criterion_s_flags_and_exits_0_without_a_baseline()
{
    let tools = FakeTools::new(
        "micro-binary",
        &[
            Recorded::new("protocol/decode/snapshot-500", 124_952).allocating(765, 95_687),
            Recorded::new("protocol/encode/snapshot-500", 26_900).allocating(0, 0),
        ],
    );
    let out = tools.dir.join("out");
    let (status, stdout, stderr) = output(tools.micro(&[
        "--max-load",
        "1000",
        "--filter",
        "protocol/.*/snapshot-500",
        "--out",
        out.to_str().unwrap(),
    ]));
    assert_eq!(status, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    let (args, home, cwd) = tools.cargo_run().expect("the fake cargo ran");
    assert_eq!(
        args,
        "bench --workspace --bench codec --bench mirror --bench daemon --bench frames -- --noplot \
         protocol/.*/snapshot-500"
    );
    assert_eq!(PathBuf::from(home), out.join("criterion"));
    assert_eq!(
        fs::canonicalize(cwd).unwrap(),
        fs::canonicalize(&tools.root).unwrap()
    );

    // The suites' output, echoed as it came, then where the reports went and
    // the summary line.
    for line in [
        "allocations protocol/decode/snapshot-500 765 95687",
        "Benchmarking protocol/decode/snapshot-500: Analyzing",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
    assert!(
        stdout.contains(&format!("wrote {}", out.join("report.json").display())),
        "{stdout}"
    );
    let last = stdout.trim_end().lines().last().unwrap();
    assert!(
        last.starts_with("Exit status 0: no baseline at reports/micro/baseline.json"),
        "{stdout}"
    );
    let report = Report::from_json(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report.benchmarks.len(), 2);
    assert!(out.join("report.md").is_file());
    assert_eq!(
        report.provenance.commit.as_ref().map(|c| c.sha.as_str()),
        Some(SHA)
    );
    assert_eq!(report.provenance.toolchain.as_deref(), Some(TOOLCHAIN));
    assert_eq!(report.provenance.guard.max_load, 1000.0);
    assert_eq!(
        find(&report, "protocol/decode/snapshot-500")
            .measured
            .allocations,
        Some(765)
    );
}

#[test]
fn micro_exits_1_on_a_regression_and_prints_it_with_both_medians() {
    let tools = FakeTools::new(
        "micro-binary-regression",
        &[Recorded::new("protocol/decode/snapshot-500", 124_952).allocating(765, 95_687)],
    );
    save_baseline(
        &tools.root,
        vec![
            Recorded::new("protocol/decode/snapshot-500", 96_117)
                .allocating(765, 95_687)
                .measured(),
        ],
    );
    let out = tools.dir.join("out");
    let (status, stdout, stderr) =
        output(tools.micro(&["--max-load", "1000", "--out", out.to_str().unwrap()]));
    assert_eq!(status, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    let table = common::table(&stdout, "Regressions");
    assert_eq!(table.rows.len(), 1, "{stdout}");
    let row = &table.rows[0];
    assert_eq!(table.cell(row, "Benchmark"), "protocol/decode/snapshot-500");
    assert_eq!(table.cell(row, "Baseline"), "96.117");
    assert_eq!(table.cell(row, "Measured"), "124.952");
    assert_eq!(table.cell(row, "Difference"), "+30.0%");
    assert_eq!(table.cell(row, "Threshold"), "5.0%");
    assert!(
        stdout.contains(
            "Exit status 1: 1 regression against the baseline: protocol/decode/snapshot-500"
        ),
        "{stdout}"
    );
}

#[test]
fn micro_exits_2_on_a_busy_host_a_baseline_it_may_not_save_or_a_failed_suite() {
    let tools = FakeTools::new(
        "micro-binary-refusals",
        &[Recorded::new("protocol/decode/snapshot-500", 124_952)],
    );
    let out = |name: &str| tools.dir.join(name).to_str().unwrap().to_owned();

    // A busy emulator, whatever the load.
    tools.processes(
        "    1     0   0.1 /sbin/launchd\n \
         4242     1  17.2 /opt/android-sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64-headless\n",
    );
    let (status, _, stderr) =
        output(tools.micro(&["--max-load", "1000", "--out", &out("out-busy")]));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(
        stderr.contains("qemu-system-aarch64-headless (pid 4242, 17.2% of a core)"),
        "{stderr}"
    );
    assert_eq!(tools.cargo_run(), None, "nothing was measured");

    // A baseline under a raised bound.
    tools.processes("    1     0   0.1 /sbin/launchd\n");
    let (status, _, stderr) = output(tools.micro(&["--save-baseline", "--max-load", "6"]));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(stderr.contains("--max-load 6"), "{stderr}");
    assert_eq!(tools.cargo_run(), None, "nothing was measured");

    // A bound that is not a positive number.
    let (status, _, stderr) = output(tools.micro(&["--max-load", "0"]));
    assert_eq!(status, Some(2), "{stderr}");

    // A suite that fails.
    tools.fail_cargo(101);
    let (status, _, stderr) =
        output(tools.micro(&["--max-load", "1000", "--out", &out("out-failed")]));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(stderr.contains("101"), "{stderr}");
    assert!(tools.cargo_run().is_some());
}
