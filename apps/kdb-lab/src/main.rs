#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
use std::io::{
    self, //
    Write,
};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{
    Context, //
    Result,
};
use clap::{
    Args, //
    Parser,
    Subcommand,
};

use kdb_lab::{
    analyze, //
    build_report,
    find_q,
};

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// Header-prefixed MBP observation CSV.
    #[arg(long)]
    input: PathBuf,
    /// Source identifier, echoed by the q contract.
    #[arg(long)]
    source: String,
    /// Output directory for metrics.csv and report.json.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the checked-in KDB-X analytics contract on one source.
    Analyze(AnalyzeArgs),
}

#[derive(Debug, Parser)]
#[command(name = "nirnay-kdb", about = "Nirnay KDB-X analytics contract harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn analyze_command(args: AnalyzeArgs) -> Result<()> {
    let q_bin = find_q()?;
    let result = analyze(&args.input, &args.source, &q_bin)?;
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("cannot create {}", args.out.display()))?;
    let metrics_path = args.out.join("metrics.csv");
    let report_path = args.out.join("report.json");
    std::fs::write(&metrics_path, result.stdout.as_bytes())
        .with_context(|| format!("cannot write {}", metrics_path.display()))?;
    let report = build_report(&result, &args.input.to_string_lossy());
    let report_text = serde_json::to_string_pretty(&report).context("cannot encode report")?;
    std::fs::write(&report_path, format!("{report_text}\n"))
        .with_context(|| format!("cannot write {}", report_path.display()))?;
    eprintln!(
        "kdb-lab: source={} rows={} input_sha256={} output_sha256={}",
        result.metrics.source_id, result.input_rows, result.input_sha256, result.output_sha256
    );
    eprintln!(
        "kdb-lab: wrote {} and {}",
        metrics_path.display(),
        report_path.display()
    );
    Ok(())
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze(args) => analyze_command(args),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "nirnay-kdb: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn analyze_subcommand_parses() {
        let cli = Cli::try_parse_from([
            "nirnay-kdb",
            "analyze",
            "--input",
            "book.csv",
            "--source",
            "upstox-live",
            "--out",
            "var/kdb",
        ])
        .expect("analyze arguments must parse");
        assert!(
            matches!(cli.command, super::Command::Analyze(_)),
            "analyze variant"
        );
    }

    #[test]
    fn missing_required_argument_fails() {
        assert!(
            Cli::try_parse_from(["nirnay-kdb", "analyze"]).is_err(),
            "analyze requires --input, --source and --out"
        );
    }
}
