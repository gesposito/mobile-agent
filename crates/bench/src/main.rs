//! `mobile-agent-bench`: benchmark harness and baseline adapters.
//!
//! Exit status: 0 when every gate passed, 1 when any gate failed, 2 when the
//! harness itself could not run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use bench::adapter::{Adapter, Detection};
use bench::baseline::{BaselineConfig, baselines};
use bench::cli::{self, CliAdapter, CliConfig};
use bench::fake::FakeAdapter;
use bench::fixture::Fixture;
use bench::flow;
use bench::gates::GateSet;
use bench::harness::{Harness, RunConfig};
use bench::plan;
use bench::primitive::Primitive;
use bench::provenance::compact_timestamp;
use bench::report::Report;
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
    /// Run every series, write report.json and report.md, and judge the gates
    Run(RunArgs),
    /// Judge the gates on a report.json written by `run`
    Gates(GatesArgs),
    /// Print the default gates in the gate file format
    DefaultGates,
    /// Look for Maestro, maestro-runner and agent-device on PATH and print what was found
    Baselines,
}

#[derive(Args)]
struct RunArgs {
    /// A target to measure mobile-agent on, as `mobile-agent devices` lists it:
    /// a Simulator's UDID or an adb serial; repeatable
    #[arg(
        long = "target",
        value_name = "ID",
        required_unless_present = "fake",
        conflicts_with = "fake"
    )]
    targets: Vec<String>,

    /// A fixture app to run on every target: swiftui, expo or compose; repeatable [default: each target's platform's, swiftui and expo on a Simulator, expo and compose on Android]
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

    /// Measured iterations per series and tool
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
    iterations: u32,

    /// Warm-up iterations per series and tool, run first and excluded from statistics
    #[arg(long, default_value_t = 3)]
    warmup: u32,

    /// Retries of a failed attempt within one iteration; an iteration that passes
    /// on a retry counts as a flake, one that never passes as a failure
    #[arg(long, default_value_t = 1)]
    retries: u32,

    /// Gate file replacing the default gates
    #[arg(long, value_name = "FILE")]
    gates: Option<PathBuf>,

    /// Leave out Maestro, maestro-runner and agent-device even when installed
    #[arg(long)]
    no_baselines: bool,
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
    let gates = load_gates(args.gates.as_deref())?;
    let config = RunConfig {
        iterations: args.iterations,
        warmup: args.warmup,
        retries: args.retries,
    };
    let dir = args.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/bench").join(compact_timestamp(SystemTime::now()))
    });
    let mut harness = match &args.fake {
        Some(script) => {
            let fake = FakeAdapter::from_json(&read(script)?)?;
            Harness::new(config, fake.targets(), fake.series())
                .flows(fake.flows())
                .tool(Box::new(fake))
        }
        None => mobile_agent(args, config, &dir)?,
    };
    if !args.no_baselines {
        for baseline in baselines(&BaselineConfig::new(dir.join("flows"))) {
            harness = harness.tool(baseline);
        }
    }
    let mut report = harness.run()?;
    let verdict = gates.evaluate(&report);
    report.gates = Some(verdict.clone());
    let files = report
        .write(&dir)
        .map_err(|e| format!("cannot write the report into {}: {e}", dir.display()))?;
    println!(
        "wrote {} and {}\n",
        files.json.display(),
        files.markdown.display()
    );
    print!("{}", verdict.table());
    Ok(ExitCode::from(verdict.exit_code()))
}

/// The run of mobile-agent through the CLI adapter: the targets `--target`
/// names among those `mobile-agent devices` lists, each with its fixtures,
/// and the plan's series and flows for them. The golden checks run first, as
/// in every run, then the timed series.
fn mobile_agent(args: &RunArgs, config: RunConfig, dir: &Path) -> Result<Harness, String> {
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
    for (target, runs) in plan::select(&connected, &args.targets, &chosen)? {
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
        targets.push(target);
    }
    fixtures.sort();
    Ok(Harness::new(config, targets, series)
        .flows(plan::flows(&fixtures))
        .tool(Box::new(adapter)))
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

fn judge(args: &GatesArgs) -> Result<ExitCode, String> {
    let report = Report::from_json(&read(&args.report)?)?;
    let verdict = load_gates(args.gates.as_deref())?.evaluate(&report);
    print!("{}", verdict.table());
    Ok(ExitCode::from(verdict.exit_code()))
}
