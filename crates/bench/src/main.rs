//! `mobile-agent-bench`: benchmark harness and baseline adapters.
//!
//! Exit status: 0 when every gate passed or, with `--gates none`, none was
//! judged, 1 when any gate failed, 2 when the harness itself could not run.
//! `micro`, with `--ab` too: 0 without a regression, 1 on any, 2 when the
//! host was busy or the run could not complete. `trend`: 0 when its two
//! reports read and are of one kind, 2 otherwise.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use bench::adapter::{Adapter, Detection};
use bench::baseline::{BaselineConfig, baselines};
use bench::cli::{self, CliAdapter, CliConfig};
use bench::fake::FakeAdapter;
use bench::fixture::Fixture;
use bench::flow;
use bench::gates::GateSet;
use bench::harness::{Harness, RunConfig};
use bench::install;
use bench::micro::guard::SystemHost;
use bench::micro::{self, CargoBench, Settings, ab};
use bench::plan;
use bench::primitive::Primitive;
use bench::provenance::{Probe, SystemProbe, capture_host, compact_timestamp};
use bench::ratio;
use bench::report::Report;
use bench::target::TargetClass;
use bench::trend;
use clap::builder::PossibleValuesParser;
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mobile-agent-bench",
    version,
    about = "Benchmark harness for mobile-agent: measures primitives, writes report.json and report.md, and judges latency gates"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every series, write report.json and report.md, and judge the gates, or with --gates none report mobile-agent's ratios against the baselines alone
    Run(RunArgs),
    /// Judge the gates on a report.json written by `run`
    Gates(GatesArgs),
    /// Print the default gates in the gate file format
    DefaultGates,
    /// Look for Maestro, maestro-runner, agent-device and Appium's WebDriverAgent on PATH and print what was found
    Baselines,
    /// Run the host microbenchmarks on a quiet host, compare them with reports/micro/baseline.json, or with --ab with another ref built and run on this host, and write report.json and report.md
    Micro(MicroArgs),
    /// Print a report beside the previous scheduled run's of the same kind, two microbenchmark A/B reports or two runs', with the changes, shown and never judged
    Trend(TrendArgs),
    /// Start WebDriverAgent on a physical iPhone through its test manager, print its localhost URL and serve until standard input closes: the WebDriverAgent baseline's helper
    #[command(hide = true)]
    WdaSession(WdaSessionArgs),
}

// Only macOS reaches an iPhone, so elsewhere the arguments go unread.
#[derive(Args)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct WdaSessionArgs {
    /// The iPhone's identifier, on USB
    #[arg(long, value_name = "UDID")]
    udid: String,

    /// The bundle identifier of the WebDriverAgent runner installed on it
    #[arg(long, value_name = "ID")]
    bundle_id: String,

    /// How long WebDriverAgent has to answer once its test starts, in seconds
    #[arg(long, value_name = "SECONDS", default_value_t = 90)]
    readiness: u64,
}

#[derive(Args)]
struct MicroArgs {
    /// Wait up to this many minutes for a quiet host, checking every 30 s [default: refuse a busy host at once]
    #[arg(long, value_name = "MINUTES")]
    wait: Option<u32>,

    /// Measure only while the 1-minute load average is under this; --save-baseline takes at most the default [default: 4, or the host's core count with --ab]
    #[arg(long, value_name = "N", value_parser = positive)]
    max_load: Option<f64>,

    /// A median slower or faster than the baseline's by more than this percentage, and outside the interval the baseline recorded around its own median, is a regression or an improvement; with --ab, a head median slower or faster than base's by more than it [default: 5, or 10 with --ab]
    #[arg(long, value_name = "PERCENT", value_parser = positive)]
    threshold: Option<f64>,

    /// Compare the checkout with this ref instead of reports/micro/baseline.json: the ref checked out into a worktree of its own, both built on this host and their suites run in rounds, the side that goes first alternating
    #[arg(long, value_name = "REF", conflicts_with = "save_baseline")]
    ab: Option<String>,

    /// With --ab, how many times each side runs the suites [default: 3]
    #[arg(long, value_name = "N", requires = "ab", value_parser = clap::value_parser!(u32).range(1..))]
    rounds: Option<u32>,

    /// Run only the benchmarks whose ids match this regular expression, criterion's filter, such as protocol/decode
    #[arg(long, value_name = "NAME")]
    filter: Option<String>,

    /// Rewrite reports/micro/baseline.json with this run's values and provenance, after comparing; refused with --filter, a raised --max-load or --ab
    #[arg(long)]
    save_baseline: bool,

    /// Directory for report.json, report.md and criterion's files [default: target/bench/micro/, or target/bench/micro-ab/ with --ab, and the UTC start time, in the repository root]
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
}

/// A positive, finite number.
fn positive(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => Ok(n),
        _ => Err(format!("{text} is not a positive number")),
    }
}

#[derive(Args)]
struct RunArgs {
    /// A target to measure mobile-agent on, as `mobile-agent devices` lists it:
    /// a Simulator's UDID, an adb serial or a physical iPhone's identifier;
    /// repeatable
    #[arg(
        long = "target",
        value_name = "ID",
        required_unless_present = "fake",
        conflicts_with = "fake"
    )]
    targets: Vec<String>,

    /// A fixture app to run on every target: swiftui, expo or compose; repeatable [default: each target's platform's, swiftui and expo on a Simulator or an iPhone, expo and compose on Android]
    #[arg(long = "fixture", value_name = "FIXTURE", value_parser = ["swiftui", "expo", "compose"], conflicts_with = "fake")]
    fixtures: Vec<String>,

    /// A reference flow to run alone on every target's fixtures, after its golden check, without the primitives --primitive does not name: login, list-scroll or big-screen; repeatable [default: every primitive and flow]
    #[arg(long = "flow", value_name = "FLOW", value_parser = [flow::LOGIN, flow::LIST_SCROLL, flow::BIG_SCREEN], conflicts_with = "fake")]
    flows: Vec<String>,

    /// A primitive to measure alone on every target's fixtures, or beside the flows --flow names: cli-start, cold-attach, first-snapshot, warm-snapshot, tap-dispatch, tap-to-frame-change, tap-to-tree-delta, text-entry, screenshot or settle; repeatable [default: every primitive and flow]
    #[arg(long = "primitive", value_name = "PRIMITIVE", value_parser = PossibleValuesParser::new(Primitive::ALL.map(Primitive::id)), conflicts_with = "fake")]
    primitives: Vec<String>,

    /// The tool under test
    #[arg(long, value_name = "TOOL", value_parser = ["mobile-agent"], default_value = "mobile-agent")]
    tool: String,

    /// The run plan: full, every fixture of each target's platform at 50 measured iterations after 3 warm-up, or ci, a runner's, one fixture per platform, swiftui on iOS and compose on Android, at 10 after 2
    #[arg(long, value_name = "PLAN", value_parser = ["full", "ci"], default_value = "full")]
    plan: String,

    /// The mobile-agent executable to drive [default: the mobile-agent beside mobile-agent-bench, else mobile-agent on PATH]
    #[arg(long, value_name = "PATH", conflicts_with = "fake")]
    cli: Option<PathBuf>,

    /// Drive the scripted fake adapter described by this JSON file as the tool
    /// under test instead of mobile-agent; its targets, series and flows make
    /// up the run
    #[arg(long, value_name = "SCRIPT")]
    fake: Option<PathBuf>,

    /// Directory for report.json and report.md [default: target/bench/ and the UTC start time, such as target/bench/20260927T101500Z]
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,

    /// Measured iterations per series and tool [default: 50, or 10 with --plan ci]
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    iterations: Option<u32>,

    /// Warm-up iterations per series and tool, run first and excluded from statistics [default: 3, or 2 with --plan ci]
    #[arg(long)]
    warmup: Option<u32>,

    /// Retries of a failed attempt within one iteration; an iteration that passes
    /// on a retry counts as a flake, one that never passes as a failure
    #[arg(long, default_value_t = 1)]
    retries: u32,

    /// Gate file replacing the default gates, or none to judge no gate, as on a CI runner, whose run the ratios against the baselines judge instead
    #[arg(long, value_name = "FILE|none")]
    gates: Option<PathBuf>,

    /// Leave out Maestro, maestro-runner, agent-device and WebDriverAgent even when installed
    #[arg(long)]
    no_baselines: bool,

    /// A fixture's device build to install on every physical iPhone of the run before anything runs, through CoreDevice: swiftui or expo, then = and the .app; repeatable
    #[arg(long = "ios-app", value_name = "FIXTURE=PATH", conflicts_with = "fake")]
    ios_apps: Vec<String>,

    /// States that every physical iPhone of the run has Auto-Lock off (Never), which no tool can read, so the report records it as stated
    #[arg(long, conflicts_with = "fake")]
    auto_lock_off: bool,
}

#[derive(Args)]
struct TrendArgs {
    /// The previous scheduled run's report.json
    previous: PathBuf,

    /// This run's report.json
    report: PathBuf,
}

#[derive(Args)]
struct GatesArgs {
    /// A report.json written by `run`
    report: PathBuf,

    /// Gate file replacing the default gates
    #[arg(long, value_name = "FILE")]
    gates: Option<PathBuf>,
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Run(args) => run(&args),
        Command::Gates(args) => judge(&args),
        Command::DefaultGates => {
            println!("{}", GateSet::defaults().to_json());
            Ok(ExitCode::SUCCESS)
        }
        Command::Baselines => {
            list_baselines();
            Ok(ExitCode::SUCCESS)
        }
        Command::Micro(args) => run_micro(&args),
        Command::Trend(args) => compare_trend(&args),
        Command::WdaSession(args) => wda_session(&args),
    };
    result.unwrap_or_else(|message| {
        eprintln!("mobile-agent-bench: {message}");
        ExitCode::from(2)
    })
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn load_gates(path: Option<&Path>) -> Result<GateSet, String> {
    match path {
        None => Ok(GateSet::defaults()),
        Some(path) => GateSet::from_json(&read(path)?),
    }
}

fn run(args: &RunArgs) -> Result<ExitCode, String> {
    // `--gates none` judges no gate; a gate file, or the defaults, judge the run.
    let gates = match args.gates.as_deref() {
        Some(path) if path == Path::new("none") => None,
        path => Some(load_gates(path)?),
    };
    let plan = plan::Plan::from_id(&args.plan).unwrap_or(plan::Plan::Full);
    let planned = plan.config();
    let config = RunConfig {
        iterations: args.iterations.unwrap_or(planned.iterations),
        warmup: args.warmup.unwrap_or(planned.warmup),
        retries: args.retries,
    };
    let dir = args.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/bench").join(compact_timestamp(SystemTime::now()))
    });
    let (mut harness, classes) = match &args.fake {
        Some(script) => {
            let fake = FakeAdapter::from_json(&read(script)?)?;
            let classes = fake.targets().iter().map(|t| t.class).collect();
            let harness = Harness::new(config, fake.targets(), fake.series())
                .flows(fake.flows())
                .tool(Box::new(fake));
            (harness, classes)
        }
        None => mobile_agent(args, plan, config, &dir)?,
    };
    if !args.no_baselines {
        let settings = BaselineConfig::new(dir.join("flows")).for_targets(classes);
        for baseline in baselines(&settings) {
            harness = harness.tool(baseline);
        }
    }
    let mut report = harness.run()?;
    report.ratios = Some(ratio::of(&report));
    let verdict = gates.map(|set| set.evaluate(&report));
    report.gates = verdict.clone();
    let files = report
        .write(&dir)
        .map_err(|e| format!("cannot write the report into {}: {e}", dir.display()))?;
    println!(
        "wrote {} and {}\n",
        files.json.display(),
        files.markdown.display()
    );
    match verdict {
        Some(verdict) => {
            print!("{}", verdict.table());
            Ok(ExitCode::from(verdict.exit_code()))
        }
        None => {
            // The ratios stand in for the gate table.
            if let Some(ratios) = report.ratios.as_ref().filter(|r| !r.is_empty()) {
                println!("{}\n", ratios.table().trim_end());
            }
            println!("No gate judged (--gates none).");
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// The run of mobile-agent through the CLI adapter: the targets `--target`
/// names among those `mobile-agent devices` lists, each with the fixtures
/// `plan` runs on it, and the plan's series and flows for them, with the
/// targets' classes. On a physical iPhone the fixtures' builds `--ios-app`
/// names are installed first, and `--auto-lock-off` is recorded as stated.
/// The golden checks run first, as in every run, then the timed series. The
/// ci plan refuses `--fixture`, since it picks the fixtures.
fn mobile_agent(
    args: &RunArgs,
    plan: plan::Plan,
    config: RunConfig,
    dir: &Path,
) -> Result<(Harness, Vec<TargetClass>), String> {
    if plan == plan::Plan::Ci && !args.fixtures.is_empty() {
        return Err(format!(
            "--plan ci runs one fixture per platform, swiftui on iOS and compose on Android; \
             --fixture names {}",
            args.fixtures.join(", ")
        ));
    }
    let apps = args
        .ios_apps
        .iter()
        .map(|text| install::parse_app(text))
        .collect::<Result<Vec<_>, _>>()?;
    let mut settings = CliConfig::new(dir.join("cli"));
    settings.program = args.cli.clone().unwrap_or_else(cli::default_program);
    let mut adapter = CliAdapter::new(settings);
    if let Detection::Absent { note } = adapter.detect() {
        return Err(format!("cannot run {}: {note}", args.tool));
    }
    let connected = adapter
        .devices()
        .map_err(|e| format!("cannot list the targets: {e}"))?;
    let chosen: Vec<Fixture> = args
        .fixtures
        .iter()
        .filter_map(|id| plan::FIXTURES.into_iter().find(|f| f.id() == id))
        .collect();
    let primitives: Vec<Primitive> = args
        .primitives
        .iter()
        .filter_map(|id| Primitive::ALL.into_iter().find(|p| p.id() == id))
        .collect();
    let mut targets = Vec::new();
    let mut series = Vec::new();
    let mut fixtures: Vec<Fixture> = Vec::new();
    for (mut target, runs) in plan::select(&connected, &args.targets, &chosen)? {
        let runs = match plan {
            plan::Plan::Full => runs,
            plan::Plan::Ci => plan.fixtures(target.class.platform()),
        };
        series.extend(plan::only(
            plan::series(&target, &runs),
            &primitives,
            &args.flows,
        ));
        for fixture in runs {
            if !fixtures.contains(&fixture) {
                fixtures.push(fixture);
            }
        }
        if target.class == TargetClass::IosDevice && args.auto_lock_off {
            target.auto_lock_off = Some(true);
        }
        targets.push(target);
    }
    fixtures.sort();
    if !apps.is_empty() && !targets.iter().any(|t| t.class == TargetClass::IosDevice) {
        return Err(
            "--ios-app installs on a physical iPhone, and the run has no ios-device target"
                .to_owned(),
        );
    }
    for target in targets
        .iter_mut()
        .filter(|t| t.class == TargetClass::IosDevice)
    {
        for (fixture, bundle) in &apps {
            let installed = install::install(target, *fixture, bundle)?;
            println!(
                "installed the {fixture} fixture's build on {} in {} ms",
                target.id,
                installed.elapsed_us / 1000
            );
            target.installs.push(installed);
        }
    }
    let classes = targets.iter().map(|t| t.class).collect();
    let harness = Harness::new(config, targets, series)
        .flows(plan::flows(&fixtures))
        .tool(Box::new(adapter));
    Ok((harness, classes))
}

/// Prints each baseline tool as detection finds it.
fn list_baselines() {
    println!("| Tool | Status | Version | Note |");
    println!("|---|---|---|---|");
    for mut tool in baselines(&BaselineConfig::new(std::env::temp_dir())) {
        let row = match tool.detect() {
            Detection::Available { version } => format!(
                "| {} | available | {} | — |",
                tool.name(),
                version.as_deref().unwrap_or("—")
            ),
            Detection::Absent { note } => format!("| {} | skipped | — | {note} |", tool.name()),
        };
        println!("{row}");
    }
}

/// The microbenchmarks of the checkout the current directory is in, run
/// through `cargo bench` behind the guard on this host.
fn run_micro(args: &MicroArgs) -> Result<ExitCode, String> {
    let probe = SystemProbe;
    let root = probe
        .output("git", &["rev-parse", "--show-toplevel"])
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .ok_or("micro runs in the repository's checkout, and git finds none here")?;
    if let Some(base) = &args.ab {
        return run_micro_ab(args, base, &root);
    }
    let settings = Settings {
        out: args.out.clone(),
        max_load: args.max_load.unwrap_or(micro::MAX_LOAD),
        wait: Duration::from_secs(u64::from(args.wait.unwrap_or(0)) * 60),
        threshold_pct: args.threshold.unwrap_or(micro::THRESHOLD_PCT),
        filter: args.filter.clone(),
        save_baseline: args.save_baseline,
        ..Settings::new(&root)
    };
    let mut host = SystemHost::new(
        Box::new(SystemProbe),
        std::env::consts::OS,
        std::process::id(),
    );
    let mut suites = CargoBench::new(&root);
    let finished = micro::run(&settings, &mut host, &probe, &mut suites)?;
    println!(
        "\nwrote {} and {}",
        finished.files.json.display(),
        finished.files.markdown.display()
    );
    if let Some(baseline) = &finished.baseline {
        println!("wrote the baseline {}", baseline.display());
    }
    println!();
    print!("{}", finished.report.verdict());
    Ok(ExitCode::from(finished.report.summary.exit_status))
}

/// The A/B mode of `micro`: the checkout at `root` against `base`, both
/// built and run on this host behind the guard, whose load bound is the
/// host's core count unless `--max-load` gives one. The base is checked out
/// into the system's temporary directory and built in
/// `target/bench/micro-ab/target/`, and both sides build and run with the
/// toolchain `RUSTUP_TOOLCHAIN` names, else the one rustup resolves in the
/// checkout.
fn run_micro_ab(args: &MicroArgs, base: &str, root: &Path) -> Result<ExitCode, String> {
    let probe = SystemProbe;
    let os = std::env::consts::OS;
    let cores = capture_host(&probe, os).cpu_cores.or_else(|| {
        std::thread::available_parallelism()
            .ok()
            .and_then(|n| u32::try_from(n.get()).ok())
    });
    let toolchain = std::env::var("RUSTUP_TOOLCHAIN")
        .ok()
        .filter(|name| !name.is_empty())
        .or_else(|| {
            probe
                .output("rustup", &["show", "active-toolchain"])
                .and_then(|line| line.split_whitespace().next().map(str::to_owned))
        });
    let settings = ab::Settings {
        root: root.to_owned(),
        base: base.to_owned(),
        out: args.out.clone(),
        rounds: args.rounds.unwrap_or(ab::ROUNDS),
        max_load: args.max_load.unwrap_or_else(|| ab::default_max_load(cores)),
        wait: Duration::from_secs(u64::from(args.wait.unwrap_or(0)) * 60),
        threshold_pct: args.threshold.unwrap_or(ab::THRESHOLD_PCT),
        filter: args.filter.clone(),
        toolchain: toolchain.clone(),
        os: os.to_owned(),
    };
    let dir = std::env::temp_dir().join(format!("mobile-agent-bench-ab-{}", std::process::id()));
    let mut trees = ab::Worktrees::new(
        root,
        dir,
        root.join("target/bench/micro-ab/target"),
        toolchain,
    );
    let mut host = SystemHost::new(Box::new(SystemProbe), os, std::process::id());
    let finished = ab::run(&settings, &mut host, &probe, &mut trees)?;
    println!(
        "\nwrote {} and {}",
        finished.files.json.display(),
        finished.files.markdown.display()
    );
    if let Some(leftover) = &finished.leftover {
        eprintln!("mobile-agent-bench: {leftover}");
    }
    println!();
    print!("{}", finished.report.verdict());
    Ok(ExitCode::from(finished.report.summary.exit_status))
}

/// The WebDriverAgent baseline's helper: WebDriverAgent started through the
/// iPhone's test manager and served on localhost until standard input closes.
#[cfg(target_os = "macos")]
fn wda_session(args: &WdaSessionArgs) -> Result<ExitCode, String> {
    bench::baseline::wda_xctest::serve(
        &args.udid,
        &args.bundle_id,
        Duration::from_secs(args.readiness),
    )?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(target_os = "macos"))]
fn wda_session(_: &WdaSessionArgs) -> Result<ExitCode, String> {
    Err("WebDriverAgent on a physical iPhone needs macOS".to_owned())
}

/// A report beside the previous scheduled run's, printed as markdown; the
/// changes are shown and never judged, so the status is 0 whenever both
/// reports read and are of one kind.
fn compare_trend(args: &TrendArgs) -> Result<ExitCode, String> {
    let markdown = trend::compare(&read(&args.previous)?, &read(&args.report)?)?;
    print!("{markdown}");
    Ok(ExitCode::SUCCESS)
}

fn judge(args: &GatesArgs) -> Result<ExitCode, String> {
    let report = Report::from_json(&read(&args.report)?)?;
    let verdict = load_gates(args.gates.as_deref())?.evaluate(&report);
    print!("{}", verdict.table());
    Ok(ExitCode::from(verdict.exit_code()))
}
