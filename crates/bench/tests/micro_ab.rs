//! Task 3.1 of `ci-benchmarks`: `mobile-agent-bench micro --ab <base>`, two
//! refs compared on one host (design D2). The base is checked out beside
//! the checkout and built in a target directory of its own, the suites run
//! in rounds with the sides alternating, and each benchmark is judged by the
//! relative difference of the two sides' medians against the threshold
//! alone, its allocations exactly. No test runs `cargo bench` or checks a
//! ref out: in-process, recorded trees write each round's criterion files;
//! through the binary, a fake git and a fake cargo do.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bench::micro::ab::{self, Report, Settings, Side};
use bench::micro::compare::Verdict;
use common::micro::{
    BASE_REF, BASE_SHA, FakeTools, RUSTUP_TOOLCHAIN, Recorded, RecordedTrees, SHA, ScriptedHost,
    checkout, process, quiet, reading, us,
};
use serde_json::Value;

/// The probe of the micro tests, which also resolves `BASE_REF`.
fn probe() -> bench::provenance::ScriptedProbe {
    common::micro::probe().with(
        &format!("git rev-parse --verify {BASE_REF}^{{commit}}"),
        BASE_SHA,
    )
}

/// Settings for an A/B of the checkout `root` against `BASE_REF` on a macOS
/// host of 10 cores, the reports going to `root/out`.
fn settings(root: &Path) -> Settings {
    Settings {
        root: root.to_owned(),
        base: BASE_REF.to_owned(),
        out: Some(root.join("out")),
        rounds: ab::ROUNDS,
        max_load: 10.0,
        wait: Duration::ZERO,
        threshold_pct: ab::THRESHOLD_PCT,
        filter: None,
        toolchain: Some(RUSTUP_TOOLCHAIN.to_owned()),
        os: "macos".to_owned(),
    }
}

/// An A/B run of `trees` on a host quiet at a 1-minute load of 6.5, under a
/// 10-core host's bound.
fn run(settings: &Settings, trees: &mut RecordedTrees) -> Result<ab::Finished, String> {
    let mut host = ScriptedHost::new(vec![quiet(6.5)]);
    ab::run(settings, &mut host, &probe(), trees)
}

/// The benchmark `id` of `report`.
fn find<'a>(report: &'a Report, id: &str) -> &'a ab::Benchmark {
    report
        .benchmarks
        .iter()
        .find(|b| b.id == id)
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

/// The same benchmark at a median of `ns` in every round.
fn at(id: &str, ns: u64) -> Recorded {
    Recorded::new(id, ns)
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

// Reading the rounds and judging the sides.

#[test]
fn each_side_s_value_is_the_median_of_its_rounds_medians_and_the_sides_alternate() {
    let root = checkout("ab-rounds");
    let id = "protocol/decode/snapshot-500";
    let mut trees = RecordedTrees::rounds(
        vec![
            vec![at(id, 100_000)],
            vec![at(id, 104_000)],
            vec![at(id, 99_000)],
        ],
        vec![
            vec![at(id, 101_000)],
            vec![at(id, 103_000)],
            vec![at(id, 120_000)],
        ],
    );
    let finished = run(&settings(&root), &mut trees).unwrap();
    let report = &finished.report;

    // Both trees are prepared once, then base and head take turns, the side
    // that goes first alternating, each run with a fresh criterion home, and
    // the base's checkout is removed at the end.
    let home = |round: u32, side: &str| {
        format!(
            "{}",
            root.join(format!("out/criterion/round-{round}-{side}"))
                .display()
        )
    };
    assert_eq!(
        trees.log,
        [
            format!("prepare {BASE_SHA}"),
            format!("run base {}", home(1, "base")),
            format!("run head {}", home(1, "head")),
            format!("run head {}", home(2, "head")),
            format!("run base {}", home(2, "base")),
            format!("run base {}", home(3, "base")),
            format!("run head {}", home(3, "head")),
            "finish".to_owned(),
        ]
    );
    assert_eq!(
        report.order,
        [
            [Side::Base, Side::Head],
            [Side::Head, Side::Base],
            [Side::Base, Side::Head]
        ]
    );
    assert_eq!(ab::order(4).len(), 4);
    assert_eq!(report.rounds, 3);
    assert_eq!(
        (report.base.name.as_str(), report.base.sha.as_str()),
        (BASE_REF, BASE_SHA)
    );

    // Base's rounds are 100, 104 and 99 us, head's 101, 103 and 120: the
    // medians are 100 and 103, one of each side's rounds, 3.0% apart.
    let b = find(report, id);
    let base = b.base.as_ref().unwrap();
    assert_eq!(base.rounds_ns, [100_000, 104_000, 99_000]);
    assert_eq!(base.median_ns, 100_000);
    assert_eq!(b.head.rounds_ns, [101_000, 103_000, 120_000]);
    assert_eq!(b.head.median_ns, 103_000);
    assert_eq!(b.head.samples, 100);
    assert_eq!(b.difference_pct, Some(3.0));
    assert_eq!(
        (b.time_verdict, b.verdict),
        (Some(Verdict::Same), Verdict::Same)
    );
    assert_eq!(report.summary.exit_status, 0);
    assert_eq!(report.threshold_pct, 10.0);
    assert_eq!(report.rustup_toolchain.as_deref(), Some(RUSTUP_TOOLCHAIN));
    assert_eq!(finished.leftover, None);

    // Two rounds take the lower of the two, a measured round, as every
    // median here is.
    let mut two = RecordedTrees::rounds(
        vec![vec![at(id, 100_000)], vec![at(id, 90_000)]],
        vec![vec![at(id, 95_000)], vec![at(id, 97_000)]],
    );
    let mut settings = settings(&checkout("ab-two-rounds"));
    settings.rounds = 2;
    let report = run(&settings, &mut two).unwrap().report;
    let b = find(&report, id);
    assert_eq!(
        (b.base.as_ref().unwrap().median_ns, b.head.median_ns),
        (90_000, 95_000)
    );
    assert_eq!(two.log.iter().filter(|l| l.starts_with("run ")).count(), 4);
}

#[test]
fn a_head_median_4_percent_slower_than_base_s_is_the_same_under_a_10_percent_threshold() {
    let root = checkout("ab-noise");
    let id = "mirror/pump/first-read-grid";
    let mut trees = RecordedTrees::new(
        vec![at(id, 100_000).allocating(2_092, 180_000)],
        vec![at(id, 104_000).allocating(2_092, 180_000)],
    );
    let finished = run(&settings(&root), &mut trees).unwrap();
    let b = find(&finished.report, id);
    assert_eq!(b.difference_pct, Some(4.0));
    assert_eq!(
        (b.time_verdict, b.allocations_verdict, b.verdict),
        (Some(Verdict::Same), Some(Verdict::Same), Verdict::Same)
    );
    assert_eq!(finished.report.summary.exit_status, 0);
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let (table, row) = row(&md, id);
    assert_eq!(table.cell(&row, "Base µs"), "100.000");
    assert_eq!(table.cell(&row, "Head µs"), "104.000");
    assert_eq!(table.cell(&row, "Difference"), "+4.0%");
    assert_eq!(table.cell(&row, "Verdict"), "same");
    assert!(
        md.contains("Exit status 0: no regression against main~5."),
        "{md}"
    );
}

#[test]
fn a_head_20_percent_slower_or_allocating_more_is_a_regression_with_both_values_and_exits_1() {
    let root = checkout("ab-regression");
    let slower = "protocol/decode/snapshot-500";
    let allocating = "daemon/wire/reply-snapshot-big";
    let mut trees = RecordedTrees::new(
        vec![
            at(allocating, 98_400).allocating(21, 295_346),
            at(slower, 100_000).allocating(765, 95_687),
        ],
        vec![
            at(allocating, 99_000).allocating(22, 295_400),
            at(slower, 120_000).allocating(765, 95_687),
        ],
    );
    let finished = run(&settings(&root), &mut trees).unwrap();
    let report = &finished.report;
    let b = find(report, slower);
    assert_eq!(b.difference_pct, Some(20.0));
    assert_eq!(
        (b.time_verdict, b.allocations_verdict, b.verdict),
        (
            Some(Verdict::Regression),
            Some(Verdict::Same),
            Verdict::Regression
        )
    );
    // 0.6% slower, within the threshold, with one more allocation.
    let b = find(report, allocating);
    assert_eq!(
        (b.time_verdict, b.allocations_verdict, b.verdict),
        (
            Some(Verdict::Same),
            Some(Verdict::Regression),
            Verdict::Regression
        )
    );
    assert_eq!(report.summary.regressions, 2);
    assert_eq!(report.summary.exit_status, 1);
    assert_eq!(
        report.summary.reason,
        format!("2 regressions against {BASE_REF}: {allocating}, {slower}")
    );

    // Both outputs list each measure that moved with base's and head's
    // value; what the run prints ends with them and the summary line.
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let printed = report.verdict();
    for text in [&md, &printed] {
        let table = common::table(text, "Regressions");
        let rows: Vec<Vec<&str>> = table
            .rows
            .iter()
            .map(|r| {
                [
                    "Benchmark",
                    "Measure",
                    "Base",
                    "Head",
                    "Difference",
                    "Threshold",
                ]
                .iter()
                .map(|c| table.cell(r, c))
                .collect()
            })
            .collect();
        assert_eq!(
            rows,
            [
                vec![allocating, "allocations", "21", "22", "more", "exact"],
                vec![allocating, "bytes", "295346", "295400", "more", "exact"],
                vec![slower, "median µs", "100.000", "120.000", "+20.0%", "10.0%"],
            ],
            "{text}"
        );
    }
    let (table, row) = row(&md, slower);
    assert_eq!(table.cell(&row, "Verdict"), "regressed (median)");
    let (table, row) = self::row(&md, allocating);
    assert_eq!(table.cell(&row, "Verdict"), "regressed (allocations)");
    assert_eq!(table.cell(&row, "Base allocations"), "21");
    assert_eq!(table.cell(&row, "Head allocations"), "22");
    assert!(
        printed.trim_end().ends_with(&format!(
            "Exit status 1: 2 regressions against {BASE_REF}: {allocating}, {slower}. Of 2 \
             benchmarks: 2 regressed, 0 improved, 0 the same, 0 new; 0 in base not measured \
             in head."
        )),
        "{printed}"
    );
}

#[test]
fn a_head_faster_past_the_threshold_or_allocating_less_improves_and_exits_0() {
    let root = checkout("ab-improvement");
    let faster = "mirror/snapshot/interactive-grid";
    let fewer = "protocol/encode/snapshot-500";
    let mut trees = RecordedTrees::new(
        vec![
            at(faster, 100_000).allocating(412, 40_000),
            at(fewer, 26_900).allocating(10, 100),
        ],
        vec![
            at(faster, 85_000).allocating(412, 40_000),
            at(fewer, 26_800).allocating(9, 90),
        ],
    );
    let finished = run(&settings(&root), &mut trees).unwrap();
    let report = &finished.report;
    let b = find(report, faster);
    assert_eq!(b.difference_pct, Some(-15.0));
    assert_eq!(
        (b.time_verdict, b.verdict),
        (Some(Verdict::Improvement), Verdict::Improvement)
    );
    let b = find(report, fewer);
    assert_eq!(
        (b.time_verdict, b.allocations_verdict, b.verdict),
        (
            Some(Verdict::Same),
            Some(Verdict::Improvement),
            Verdict::Improvement
        )
    );
    assert_eq!(
        (report.summary.improvements, report.summary.exit_status),
        (2, 0)
    );
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let (table, row) = row(&md, faster);
    assert_eq!(table.cell(&row, "Verdict"), "faster");
    let (table, row) = self::row(&md, fewer);
    assert_eq!(table.cell(&row, "Verdict"), "fewer allocations");
    let table = common::table(&md, "Improvements");
    assert_eq!(table.rows.len(), 3, "{md}");

    // A threshold of 20% keeps the 15% faster median the same.
    let mut settings = settings(&checkout("ab-improvement-20"));
    settings.threshold_pct = 20.0;
    let mut trees = RecordedTrees::new(vec![at(faster, 100_000)], vec![at(faster, 85_000)]);
    let report = run(&settings, &mut trees).unwrap().report;
    assert_eq!(find(&report, faster).verdict, Verdict::Same);
    assert_eq!(report.threshold_pct, 20.0);
}

#[test]
fn a_benchmark_one_side_lacks_is_new_or_missing_and_changes_no_exit_status() {
    let root = checkout("ab-new-missing");
    let both = "protocol/decode/snapshot-500";
    let added = "mirror/pump/first-read-grid-rn";
    let dropped = "mirror/settle/no-frames-grid";
    let mut trees = RecordedTrees::new(
        vec![at(both, 100_000), at(dropped, 5_000)],
        vec![at(both, 100_500), at(added, 97_400)],
    );
    let finished = run(&settings(&root), &mut trees).unwrap();
    let report = &finished.report;
    let b = find(report, added);
    assert_eq!(
        (b.base.is_none(), b.difference_pct, b.verdict),
        (true, None, Verdict::New)
    );
    assert_eq!(report.missing.len(), 1);
    assert_eq!(report.missing[0].id, dropped);
    assert_eq!(report.missing[0].base.median_ns, 5_000);
    let s = &report.summary;
    assert_eq!(
        (s.benchmarks, s.same, s.new, s.missing, s.exit_status),
        (2, 1, 1, 1, 0)
    );
    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    let table = common::table(&md, "Missing");
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.cell(&table.rows[0], "Benchmark"), dropped);
    assert_eq!(table.cell(&table.rows[0], "Base µs"), "5.000");
    let (table, row) = row(&md, added);
    assert_eq!(table.cell(&row, "Base µs"), "—");
    assert_eq!(table.cell(&row, "Verdict"), "new");
}

#[test]
fn the_base_s_checkout_is_removed_whether_or_not_the_rounds_complete() {
    let id = "protocol/decode/snapshot-500";
    let mut trees = RecordedTrees::new(vec![at(id, 100_000)], vec![at(id, 100_000)]);
    trees.fail_prepare = Some("cannot check main~5 out".to_owned());
    let error = run(&settings(&checkout("ab-prepare-fails")), &mut trees).unwrap_err();
    assert!(error.contains("cannot check main~5 out"), "{error}");
    assert_eq!(
        trees.log,
        [format!("prepare {BASE_SHA}"), "finish".to_owned()]
    );

    let mut trees = RecordedTrees::new(vec![at(id, 100_000)], vec![at(id, 100_000)]);
    trees.fail_run = Some(4);
    let root = checkout("ab-run-fails");
    let error = run(&settings(&root), &mut trees).unwrap_err();
    assert!(error.contains("run 4"), "{error}");
    assert_eq!(trees.log.len(), 6, "{:?}", trees.log);
    assert_eq!(trees.log.last().map(String::as_str), Some("finish"));
    assert!(!root.join("out/report.json").exists());
}

#[test]
fn a_ref_the_checkout_lacks_or_a_busy_host_is_refused_before_anything_is_checked_out() {
    let id = "protocol/decode/snapshot-500";
    let mut trees = RecordedTrees::new(vec![at(id, 100_000)], vec![at(id, 100_000)]);
    let mut unknown = settings(&checkout("ab-unknown-ref"));
    unknown.base = "no-such-ref".to_owned();
    let mut host = ScriptedHost::new(vec![quiet(1.0)]);
    let error = ab::run(&unknown, &mut host, &probe(), &mut trees).unwrap_err();
    assert!(error.contains("--ab no-such-ref"), "{error}");
    assert_eq!(host.checks, 0);
    assert!(trees.log.is_empty());

    // The guard keeps its busy-process check whatever the load's bound.
    let root = checkout("ab-busy");
    let mut host = ScriptedHost::new(vec![reading(2.0, vec![process(977, "java", 6.3)])]);
    let error = ab::run(&settings(&root), &mut host, &probe(), &mut trees).unwrap_err();
    for named in ["java (pid 977, 6.3% of a core)", "nothing was measured"] {
        assert!(error.contains(named), "{named:?} in {error}");
    }
    let mut host = ScriptedHost::new(vec![quiet(10.0)]);
    let error = ab::run(&settings(&root), &mut host, &probe(), &mut trees).unwrap_err();
    assert!(error.contains("10.00, not under 10"), "{error}");
    assert!(trees.log.is_empty());
    assert!(!root.join("out").exists());
}

#[test]
fn the_host_is_checked_again_once_both_trees_are_built_and_the_rounds_start_on_a_quiet_one() {
    let root = checkout("ab-guard-after-build");
    let id = "protocol/decode/snapshot-500";
    let mut trees = RecordedTrees::new(vec![at(id, 100_000)], vec![at(id, 100_000)]);
    // Quiet before the builds; once they are done, another session's build
    // is busy, then gone.
    let mut host = ScriptedHost::new(vec![
        quiet(1.0),
        reading(3.0, vec![process(4711, "rustc", 99.0)]),
        bench::micro::guard::Reading {
            load_average: Some([2.2, 2.5, 2.9]),
            busy: Some(Vec::new()),
        },
        // Once the rounds have run.
        quiet(2.4),
    ]);
    let mut settings = settings(&root);
    settings.wait = Duration::from_secs(10 * 60);
    let finished = ab::run(&settings, &mut host, &probe(), &mut trees).unwrap();
    assert_eq!(host.checks, 4);
    assert_eq!(host.waits.len(), 1);
    assert!(
        host.waits[0].1.contains("rustc (pid 4711"),
        "{:?}",
        host.waits
    );
    let p = &finished.report.provenance;
    assert_eq!(p.host.load_average, Some([2.2, 2.5, 2.9]));
    assert_eq!(p.guard.waited_s, 30);
    assert_eq!(trees.log.len(), 8, "{:?}", trees.log);

    // Without a wait, a host busy once the trees are built is refused and
    // the base's checkout removed, nothing measured.
    let mut trees = RecordedTrees::new(vec![at(id, 100_000)], vec![at(id, 100_000)]);
    let mut host = ScriptedHost::new(vec![
        quiet(1.0),
        reading(3.0, vec![process(4711, "rustc", 99.0)]),
    ]);
    let error = ab::run(
        &self::settings(&checkout("ab-guard-after-build-refused")),
        &mut host,
        &probe(),
        &mut trees,
    )
    .unwrap_err();
    assert!(error.contains("rustc (pid 4711"), "{error}");
    assert_eq!(
        trees.log,
        [format!("prepare {BASE_SHA}"), "finish".to_owned()]
    );
}

#[test]
fn both_reports_agree_on_every_number_and_name_the_base_the_rounds_and_the_order() {
    let root = checkout("ab-two-outputs");
    let mut trees = RecordedTrees::rounds(
        vec![
            vec![
                at("daemon/wire/parse-snapshot-request", 950).allocating(8, 680),
                at("protocol/decode/snapshot-500", 100_000).allocating(765, 95_687),
                at("simulator/content_hash/460x1000", 110_000),
            ],
            vec![
                at("daemon/wire/parse-snapshot-request", 940).allocating(8, 680),
                at("protocol/decode/snapshot-500", 101_000).allocating(765, 95_687),
                at("simulator/content_hash/460x1000", 111_000),
            ],
        ],
        vec![
            vec![
                at("daemon/wire/parse-snapshot-request", 961).allocating(8, 680),
                at("protocol/decode/snapshot-500", 125_000).allocating(765, 95_687),
                at("simulator/content_hash/460x1000", 90_000),
            ],
            vec![
                at("daemon/wire/parse-snapshot-request", 955).allocating(8, 680),
                at("protocol/decode/snapshot-500", 124_000).allocating(765, 95_687),
                at("simulator/content_hash/460x1000", 91_000),
            ],
        ],
    );
    let finished = run(&settings(&root), &mut trees).unwrap();
    let text = fs::read_to_string(&finished.files.json).unwrap();
    assert_eq!(finished.files.json, root.join("out/report.json"));
    assert_eq!(Report::from_json(&text).unwrap(), finished.report);
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["base"]["ref"], BASE_REF);
    assert_eq!(json["base"]["sha"], BASE_SHA);
    assert_eq!(json["order"][1][0], "head");
    assert_eq!(json["benchmarks"][1]["head"]["rounds_ns"][2], 124_000);
    assert_eq!(json["provenance"]["commit"]["sha"], SHA);
    assert_eq!(json["provenance"]["guard"]["max_load"], 10.0);

    let md = fs::read_to_string(&finished.files.markdown).unwrap();
    common::assert_every_markdown_number_is_in_the_json(&md, &json);
    assert!(
        md.starts_with("# Microbenchmark A/B report\n\nExit status 1:"),
        "{md}"
    );
    let table = common::table(&md, "Provenance");
    assert_eq!(field(&table, "Base"), format!("{BASE_REF} at {BASE_SHA}"));
    assert_eq!(field(&table, "Commit under test"), format!("{SHA} (clean)"));
    assert_eq!(
        field(&table, "Rounds"),
        "3: base then head, head then base, base then head"
    );
    assert_eq!(field(&table, "Toolchain for both sides"), RUSTUP_TOOLCHAIN);
    assert_eq!(field(&table, "Load average at start"), "6.50 3.50 4.00");
    // The third round repeats each side's second.
    let (table, row) = row(&md, "protocol/decode/snapshot-500");
    assert_eq!(
        table.cell(&row, "Base rounds µs"),
        "100.000, 101.000, 101.000"
    );
    assert_eq!(
        table.cell(&row, "Head rounds µs"),
        "125.000, 124.000, 124.000"
    );
    assert_eq!(table.cell(&row, "Base µs"), us(101_000));
    assert_eq!(table.cell(&row, "Head µs"), us(124_000));
    assert_eq!(table.cell(&row, "Difference"), "+22.8%");
    let (table, row) = self::row(&md, "simulator/content_hash/460x1000");
    assert_eq!(table.cell(&row, "Verdict"), "faster");
    assert_eq!(table.cell(&row, "Base allocations"), "—");
}

// Through the binary.

/// The load bound an A/B run through the binary is given: none on macOS,
/// where the fake sysctl reports the host's load and its 10 cores, and a
/// raised one elsewhere, where the load is the host's own.
fn bound() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        Vec::new()
    } else {
        vec!["--max-load", "1000"]
    }
}

/// `micro --ab BASE_REF`, under [`bound`], then `args`.
fn ab_args<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec!["--ab", BASE_REF];
    all.extend(bound());
    all.extend(args);
    all
}

#[test]
fn an_ab_run_s_load_bound_is_the_host_s_core_count_else_the_quiet_host_s() {
    assert_eq!(ab::default_max_load(Some(3)), 3.0);
    assert_eq!(ab::default_max_load(Some(10)), 10.0);
    assert_eq!(ab::default_max_load(None), 4.0);
}

/// The worktree the binary checks the base out into, as its git saw it.
fn worktree(git: &[String]) -> PathBuf {
    let added = git
        .iter()
        .find_map(|line| line.strip_prefix("worktree add --detach "))
        .unwrap_or_else(|| panic!("no worktree added in {git:?}"));
    let (dir, sha) = added.rsplit_once(' ').unwrap();
    assert_eq!(sha, BASE_SHA);
    PathBuf::from(dir)
}

#[test]
fn micro_ab_checks_the_base_out_beside_the_checkout_builds_both_trees_apart_and_alternates_them() {
    let id = "protocol/decode/snapshot-500";
    let other = "mirror/pump/first-read-grid";
    let tools = FakeTools::ab(
        "ab-binary",
        &[
            vec![at(id, 100_000).allocating(765, 95_687), at(other, 97_000)],
            vec![at(id, 101_000).allocating(765, 95_687), at(other, 97_500)],
        ],
        &[vec![
            at(id, 102_000).allocating(765, 95_687),
            at(other, 96_000),
        ]],
    );
    // A load of 6.5 is under the 10 cores' bound, though over the default 4.
    tools.load("{ 6.50 5.00 4.00 }");
    let out = tools.dir.join("out");
    let (status, stdout, stderr) = output(tools.micro(&ab_args(&["--out", out.to_str().unwrap()])));
    assert_eq!(status, Some(0), "stdout: {stdout}\nstderr: {stderr}");

    // The base is resolved, checked out outside the checkout and removed at
    // the end.
    let git = tools.git_log();
    let dir = worktree(&git);
    assert!(
        dir.starts_with(tools.tmp()),
        "{} in {}",
        dir.display(),
        tools.tmp().display()
    );
    assert!(!dir.starts_with(&tools.root));
    let commands: Vec<&str> = git
        .iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("rev-parse --verify") || l.starts_with("worktree"))
        .collect();
    assert_eq!(
        commands,
        [
            format!("rev-parse --verify {BASE_REF}^{{commit}}").as_str(),
            "worktree prune",
            format!("worktree add --detach {} {BASE_SHA}", dir.display()).as_str(),
            format!("worktree remove --force {}", dir.display()).as_str(),
        ]
    );
    assert!(!dir.exists());

    // Both trees are built before anything is measured, base in a target
    // directory of its own, then the rounds alternate the sides; every
    // command has the checkout's toolchain.
    let log = tools.cargo_log();
    let base_target = tools.root.join("target/bench/micro-ab/target");
    let canonical = |p: &str| fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    let steps: Vec<(String, bool)> = log
        .iter()
        .map(|l| (l[0].clone(), l[5].contains("--no-run")))
        .collect();
    let step = |side: &str, build: bool| (side.to_owned(), build);
    assert_eq!(
        steps,
        [
            step("head", true),
            step("base", true),
            step("base", false),
            step("head", false),
            step("head", false),
            step("base", false),
            step("base", false),
            step("head", false),
        ]
    );
    let mut homes = Vec::new();
    for line in &log {
        let (side, cwd, target, toolchain, home, args) =
            (&line[0], &line[1], &line[2], &line[3], &line[4], &line[5]);
        assert_eq!(toolchain, RUSTUP_TOOLCHAIN, "{line:?}");
        assert!(
            args.starts_with(
                "bench --workspace --bench codec --bench mirror --bench daemon --bench frames"
            ),
            "{line:?}"
        );
        if side == "base" {
            assert_eq!(PathBuf::from(target), base_target, "{line:?}");
        } else {
            assert_eq!(target, "", "{line:?}");
            assert_eq!(
                canonical(cwd),
                fs::canonicalize(&tools.root).unwrap(),
                "{line:?}"
            );
        }
        if !args.contains("--no-run") {
            homes.push(PathBuf::from(home));
        }
    }
    assert_eq!(homes[0], out.join("criterion/round-1-base"));
    assert_eq!(homes[3], out.join("criterion/round-2-base"));
    homes.sort();
    homes.dedup();
    assert_eq!(homes.len(), 6);

    let report = Report::from_json(&fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    let max_load = if cfg!(target_os = "macos") {
        10.0
    } else {
        1000.0
    };
    assert_eq!(report.provenance.guard.max_load, max_load);
    assert_eq!(report.provenance.host.cpu_cores, Some(10));
    assert_eq!(report.threshold_pct, 10.0);
    assert_eq!(report.base.sha, BASE_SHA);
    let b = report.benchmarks.iter().find(|b| b.id == id).unwrap();
    // Base's third round repeats its second.
    assert_eq!(
        b.base.as_ref().unwrap().rounds_ns,
        [100_000, 101_000, 101_000]
    );
    assert_eq!(b.head.median_ns, 102_000);
    assert_eq!(b.difference_pct, Some(1.0));
    assert!(out.join("report.md").is_file());
    let last = stdout.trim_end().lines().last().unwrap();
    assert!(
        last.starts_with(&format!("Exit status 0: no regression against {BASE_REF}.")),
        "{stdout}"
    );
}

#[test]
fn micro_ab_exits_1_on_a_regression_and_2_on_a_refused_option_or_a_busy_host() {
    let id = "protocol/decode/snapshot-500";
    let tools = FakeTools::ab(
        "ab-binary-exits",
        &[vec![at(id, 100_000)]],
        &[vec![at(id, 130_000)]],
    );
    let out = |name: &str| tools.dir.join(name).to_str().unwrap().to_owned();
    let (status, stdout, stderr) =
        output(tools.micro(&ab_args(&["--out", &out("out-regression")])));
    assert_eq!(status, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    let table = common::table(&stdout, "Regressions");
    assert_eq!(table.cell(&table.rows[0], "Base"), "100.000");
    assert_eq!(table.cell(&table.rows[0], "Head"), "130.000");
    assert_eq!(table.cell(&table.rows[0], "Difference"), "+30.0%");
    assert!(stdout.contains(&format!(
        "Exit status 1: 1 regression against {BASE_REF}: {id}"
    )));

    // --save-baseline writes the committed baseline, which an A/B run does
    // not measure against, and --rounds belongs to --ab.
    let (status, _, stderr) = output(tools.micro(&["--ab", BASE_REF, "--save-baseline"]));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
    let (status, _, stderr) = output(tools.micro(&["--rounds", "2"]));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(stderr.contains("--ab"), "{stderr}");

    // A busy emulator is refused whatever the load.
    tools.processes(
        "    1     0   0.1 /sbin/launchd\n \
         4242     1  17.2 /opt/android-sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64-headless\n",
    );
    let (status, _, stderr) = output(tools.micro(&ab_args(&["--out", &out("out-busy")])));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(
        stderr.contains("qemu-system-aarch64-headless (pid 4242, 17.2% of a core)"),
        "{stderr}"
    );

    // Without --ab the bound stays 4, which a load of 6.5 is not under.
    tools.processes("    1     0   0.1 /sbin/launchd\n");
    if cfg!(target_os = "macos") {
        tools.load("{ 6.50 5.00 4.00 }");
        let (status, _, stderr) = output(tools.micro(&["--out", &out("out-baseline")]));
        assert_eq!(status, Some(2), "{stderr}");
        assert!(stderr.contains("6.50, not under 4"), "{stderr}");
    }
    let (status, _, stderr) = output(tools.micro(&["--ab", "no-such-ref"]));
    assert_eq!(status, Some(2), "{stderr}");
    assert!(stderr.contains("--ab no-such-ref"), "{stderr}");
}
