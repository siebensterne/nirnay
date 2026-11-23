//! Locate and drive the locally installed KDB-X engine, then pin its output.
//!
//! The runner always passes absolute paths and never mutates the input. Every
//! invocation is recorded by input and output digests so a result can be
//! reproduced or compared across machines.

use std::path::{
    Path, //
    PathBuf,
};
use std::process::Command;

use anyhow::{
    Context, //
    Result,
    bail,
};
use sha2::{
    Digest, //
    Sha256,
};

use crate::metrics::{
    MbpMetrics, //
    parse_metrics_csv,
};
use crate::observation::parse_observations_csv;

/// Default local KDB-X binary used by the owner environment.
pub const DEFAULT_Q_BIN: &str = "/Users/tourist/.kx/bin/q";
/// Override for the q binary, useful on a different owner machine or in tests.
pub const Q_BIN_ENV: &str = "NIRNAY_Q_BIN";

/// One reproducible analytics invocation.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub metrics: MbpMetrics,
    pub stdout: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_rows: u64,
    pub output_bytes: u64,
    pub binary: String,
}

/// Resolve the q binary without printing or exporting any credential.
pub fn find_q() -> Result<PathBuf> {
    let candidate = std::env::var_os(Q_BIN_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_Q_BIN));
    if candidate.is_file() {
        Ok(candidate)
    } else {
        bail!("KDB-X q binary not found at {}", candidate.display())
    }
}

/// Absolute path of the checked-in q analytics contract.
pub fn q_script_path() -> Result<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../analytics/kdb/mbp_metrics.q");
    path.canonicalize()
        .with_context(|| format!("cannot resolve q script at {}", path.display()))
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Validate the input contract and run q once, returning typed metrics and
/// digests. Fails closed on a bad input, a bad exit status or a drifted output.
pub fn analyze(input: &Path, source_id: &str, q_bin: &Path) -> Result<Analysis> {
    let input_bytes =
        std::fs::read(input).with_context(|| format!("cannot read input {}", input.display()))?;
    let input_text = std::str::from_utf8(&input_bytes).context("input is not UTF-8")?;
    let rows = parse_observations_csv(input_text)
        .context("input does not satisfy the KDB-X analytics contract")?;
    let input_abs = input
        .canonicalize()
        .with_context(|| format!("cannot resolve input {}", input.display()))?;
    let script = q_script_path()?;

    let output = Command::new(q_bin)
        .arg(&script)
        .arg(&input_abs)
        .arg(source_id)
        .output()
        .with_context(|| format!("cannot launch q at {}", q_bin.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "q exited with {:?}: {}",
            output.status.code(),
            stderr.trim()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("q stdout is not UTF-8")?;
    let metrics = parse_metrics_csv(&stdout).context("q output failed the metrics contract")?;
    if metrics.source_id != source_id {
        bail!(
            "q echoed source_id {:?}, expected {source_id:?}",
            metrics.source_id
        );
    }
    if metrics.rows != rows.len() as u64 {
        bail!(
            "q reported {} rows but the input contains {}",
            metrics.rows,
            rows.len()
        );
    }

    Ok(Analysis {
        metrics,
        output_sha256: digest(stdout.as_bytes()),
        output_bytes: stdout.len() as u64,
        input_sha256: digest(&input_bytes),
        input_rows: rows.len() as u64,
        binary: q_bin.to_string_lossy().into_owned(),
        stdout,
    })
}
