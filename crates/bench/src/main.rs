//! `mobile-agent-bench`: benchmark harness and baseline adapters.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;

use bench::fake::FakeAdapter;
use bench::harness::{Harness, RunConfig};
use bench::provenance::compact_timestamp;
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mobile-agent-bench",
    version,
    about = "Benchmark harness for mobile-agent: measures primitives and writes report.json and report.md"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every series and write report.json and report.md
    Run(RunArgs),
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
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Run(args) => run(&args),
    };
    result.unwrap_or_else(|message| {
        eprintln!("mobile-agent-bench: {message}");
        ExitCode::from(2)
    })
}

fn run(args: &RunArgs) -> Result<ExitCode, String> {
    let script = fs::read_to_string(&args.fake)
        .map_err(|e| format!("cannot read {}: {e}", args.fake.display()))?;
    let fake = FakeAdapter::from_json(&script)?;
    let config = RunConfig {
        iterations: args.iterations,
        warmup: args.warmup,
    };
    let report = Harness::new(config, fake.targets(), fake.series())
        .tool(Box::new(fake))
        .run()?;
    let dir = args.out.clone().unwrap_or_else(|| {
        PathBuf::from("target/bench").join(compact_timestamp(SystemTime::now()))
    });
    let files = report
        .write(&dir)
        .map_err(|e| format!("cannot write the report into {}: {e}", dir.display()))?;
    println!(
        "wrote {} and {}",
        files.json.display(),
        files.markdown.display()
    );
    Ok(ExitCode::SUCCESS)
}
