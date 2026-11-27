#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Run one bounded paper pass from a JSON input and publish its report.
//!
//! STARTUP INTEGRATION BOUNDARY ONLY: the default path is an explicitly labelled
//! EphemeralSimulation. `--persistent` is an explicit opt-in to the recovery
//! transaction and fails closed here because this harness has no owner-provisioned
//! recovery root, real durable adapters or authority locks. It never contacts a
//! provider, collects market data, runs an experiment or holdout, or reaches a
//! live execution adapter.

use std::{
    fs, //
    io::Write,
    path::PathBuf,
};

use paper_runner::PaperInput;

const USAGE: &str = "usage: paper_run <input.json> <output.json> <oms-dir> [--persistent]";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let input = arguments.next().ok_or(USAGE)?;
    let output = arguments.next().ok_or(USAGE)?;
    let oms_dir = arguments.next().ok_or(USAGE)?;

    let persistent = arguments.any(|argument| argument == "--persistent");
    if persistent {
        // No real durable control/decision adapters or owner-provisioned
        // recovery root exist in this harness. Fail closed rather than fabricate
        // an authority from simulation state.
        return Err(concat!(
            "NOT-READY: persistent startup requires an owner-provisioned recovery ",
            "root and real durable control/decision adapters; none are available in ",
            "this harness"
        )
        .into());
    }

    let bytes = fs::read(&input)?;
    let paper_input: PaperInput = serde_json::from_slice(&bytes)?;
    let oms_path = PathBuf::from(&oms_dir).join("oms.journal");
    if let Some(parent) = oms_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let run =
        paper_runner::run_paper(&paper_input, &oms_path).map_err(|error| error.to_string())?;
    let report = serde_json::to_vec_pretty(&run.report)?;
    let mut file = fs::File::create(&output)?;
    file.write_all(&report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    eprintln!("mode={}", run.identity.label);
    eprintln!("digest={}", run.report.run_digest);
    Ok(())
}
