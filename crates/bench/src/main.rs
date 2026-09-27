//! `mobile-agent-bench`: benchmark harness and baseline adapters.
//!
//! Exit status: 0 when every gate passed, 1 when any gate failed, 2 when the
//! harness itself could not run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use bench::fake::FakeAdapter;
use bench::gates::GateSet;
use bench::harness::{Harness, RunConfig};
use bench::provenance::compact_timestamp;
use bench::report::Report;
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
}

#[derive(Args)]
struct RunArgs {
    /// Drive the scripted fake adapter described by this JSON file as the tool
    /// under test; its targets and series make up the run
    #[arg(long, value_name = "SCRIPT")]
    fake: PathBuf,

    /// Directory for report.json and report.md [default: target/bench/<UTC time>]
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,

    /// Measured iterations per series and tool
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..))]
    iterations: u32,

    /// Warm-up iterations per series and tool, run first and excluded from statistics
    #[arg(long, default_value_t = 3)]
    warmup: u32,

    /// Gate file replacing the default gates
    #[arg(long, value_name = "FILE")]
    gates: Option<PathBuf>,
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
    let fake = FakeAdapter::from_json(&read(&args.fake)?)?;
    let config = RunConfig {
        iterations: args.iterations,
        warmup: args.warmup,
    };
    let mut report = Harness::new(config, fake.targets(), fake.series())
        .tool(Box::new(fake))
        .run()?;
    let verdict = gates.evaluate(&report);
    report.gates = Some(verdict.clone());
    let dir = args.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/bench").join(compact_timestamp(SystemTime::now()))
    });
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

fn judge(args: &GatesArgs) -> Result<ExitCode, String> {
    let report = Report::from_json(&read(&args.report)?)?;
    let verdict = load_gates(args.gates.as_deref())?.evaluate(&report);
    print!("{}", verdict.table());
    Ok(ExitCode::from(verdict.exit_code()))
}
