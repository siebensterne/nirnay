#![expect(
    clippy::print_stdout,
    reason = "a task runner reports progress on stdout by definition"
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Repository task runner: a typed, discoverable CLI over the project gates.
//!
//! The task set is a clap subcommand enum: one source of truth for names,
//! help text and delegation. Every gate runs through [`Cmd`] with an explicit
//! argument vector — never a shell — and its exit status is propagated so CI
//! sees the gate's own result. The first failing step stops the run; nothing
//! is skipped silently.

use std::{
    io::{
        BufWriter, //
        Write,
        stdout,
    },
    path::{
        Path, //
        PathBuf,
    },
    process::{
        Command, //
        ExitCode,
        ExitStatus,
    },
};

use anyhow::{
    Context, //
    Result,
    ensure,
};
use clap::{
    CommandFactory, //
    Parser,
    Subcommand,
};

/// A single process invocation with explicit arguments and optional environment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cmd {
    /// Executable name or path.
    program: &'static str,
    /// Arguments passed verbatim; no shell parsing.
    args: Vec<&'static str>,
    /// Extra environment variables.
    env: Vec<(&'static str, &'static str)>,
}

impl Cmd {
    /// Construct a command with no environment overrides.
    fn new(program: &'static str, args: &'static [&'static str]) -> Self {
        Self {
            program,
            args: args.to_vec(),
            env: Vec::new(),
        }
    }

    /// Add an environment variable.
    fn with_env(mut self, key: &'static str, value: &'static str) -> Self {
        self.env.push((key, value));
        self
    }

    /// Human-readable rendering for progress output.
    fn line(&self) -> String {
        let mut parts = vec![self.program.to_owned()];
        parts.extend(self.args.iter().map(|arg| (*arg).to_owned()));
        parts.join(" ")
    }

    /// Execute the command from the workspace root.
    fn run(&self, root: &Path) -> Result<ExitStatus> {
        let status = Command::new(self.program)
            .args(&self.args)
            .envs(self.env.iter().copied())
            .current_dir(root)
            .status()
            .with_context(|| format!("running {}", self.line()))?;
        Ok(status)
    }
}

/// Repository gates exposed as clap subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
enum Task {
    /// Run the full policy gate suite via the repository's single registry (make check).
    Ci,
    /// Run the full assurance superset (polish, update and upgrade gates).
    Assure,
    /// Run every repository policy gate via check_repo.py.
    Check,
    /// Check the architecture dependency graph and manifest hygiene.
    Arch,
    /// Check dependency waivers.
    Waivers,
    /// Check claims and their falsifiers.
    Claims,
    /// Run every source and standards style gate.
    Style,
    /// Check Rust formatting.
    Fmt,
    /// Run Clippy with warnings denied.
    Lint,
    /// Run all-target and doc tests.
    Test,
    /// Build rustdoc with warnings and broken links denied.
    Docs,
    /// List the available tasks.
    List,
}

/// Parse errors are handled by clap; this enum is the single source of truth.
#[derive(Debug, Parser)]
#[command(name = "xtask", version, about, long_about = None)]
struct Cli {
    /// Gate to run.
    #[command(subcommand)]
    task: Task,
}

impl Task {
    /// Commands for this task, in execution order. Discovery tasks have none.
    fn commands(self) -> Vec<Cmd> {
        match self {
            Task::Ci => vec![Cmd::new("make", &["check"])],
            Task::Assure => vec![Cmd::new("scripts/assurance/polish.sh", &[])],
            Task::Check => vec![script("check")],
            Task::Arch => vec![script("arch")],
            Task::Waivers => vec![script("waivers")],
            Task::Claims => vec![script("claims")],
            Task::Style => vec![Cmd::new("make", &["style"])],
            Task::Fmt => vec![Cmd::new("cargo", &["fmt", "--all", "--", "--check"])],
            Task::Lint => vec![Cmd::new(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--locked",
                    "--",
                    "-D",
                    "warnings",
                ],
            )],
            Task::Test => vec![
                Cmd::new(
                    "cargo",
                    &["test", "--workspace", "--all-targets", "--locked"],
                ),
                Cmd::new("cargo", &["test", "--workspace", "--doc", "--locked"]),
            ],
            Task::Docs => vec![
                Cmd::new("cargo", &["doc", "--no-deps", "--workspace", "--locked"]).with_env(
                    "RUSTDOCFLAGS",
                    "-D warnings -D rustdoc::broken_intra_doc_links -A rustdoc::private_intra_doc_links",
                ),
            ],
            Task::List => Vec::new(),
        }
    }
}

/// Build the typed invocation of the repository policy script.
fn script(argument: &'static str) -> Cmd {
    Cmd {
        program: "python3",
        args: vec!["scripts/check_repo.py", argument],
        env: Vec::new(),
    }
}

/// Locate the workspace root from the compiled crate location.
fn workspace_root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .context("xtask must live directly under the workspace root")?
        .to_path_buf();
    ensure!(
        root.join("Cargo.toml").is_file(),
        "workspace manifest not found under {}",
        root.display()
    );
    Ok(root)
}

/// Task names taken from the clap command definition, so they cannot drift.
fn task_names() -> Vec<String> {
    let mut names: Vec<String> = Cli::command()
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_owned())
        .collect();
    names.sort_unstable();
    names
}

/// Map a child status to this process's exit code.
///
/// A successful child stays successful. A failed child keeps its own code when
/// it fits in `u8`; a signal termination or out-of-range code becomes the
/// generic failure code. Keeping this mapping pure makes propagation testable.
fn exit_code_for(status: ExitStatus) -> ExitCode {
    if status.success() {
        return ExitCode::SUCCESS;
    }
    status
        .code()
        .and_then(|value| u8::try_from(value).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from)
}

/// Run a task's commands in order, stopping at the first failure.
fn run_task(root: &Path, task: Task) -> Result<ExitCode> {
    let commands = task.commands();
    ensure!(
        !commands.is_empty(),
        "task has no executable commands; use `list` for discovery"
    );
    for command in commands {
        let line = command.line();
        let status = command.run(root)?;
        let code = exit_code_for(status);
        println!("[{code:?}] {line}");
        if code != ExitCode::SUCCESS {
            return Ok(code);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let root = workspace_root()?;
    match cli.task {
        Task::List => {
            let output = stdout();
            let mut writer = BufWriter::new(output.lock());
            for name in task_names() {
                writeln!(writer, "{name}").context("writing the task list")?;
            }
            writer.flush().context("flushing the task list")?;
            Ok(ExitCode::SUCCESS)
        }
        task => run_task(&root, task),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_gate_has_at_least_one_typed_command() {
        for (name, commands) in [
            ("ci", Task::Ci.commands()),
            ("assure", Task::Assure.commands()),
            ("check", Task::Check.commands()),
            ("arch", Task::Arch.commands()),
            ("waivers", Task::Waivers.commands()),
            ("claims", Task::Claims.commands()),
            ("style", Task::Style.commands()),
            ("fmt", Task::Fmt.commands()),
            ("lint", Task::Lint.commands()),
            ("test", Task::Test.commands()),
            ("docs", Task::Docs.commands()),
        ] {
            assert!(!commands.is_empty(), "gate `{name}` has no commands");
            for command in commands {
                assert!(!command.program.is_empty(), "gate `{name}` has no program");
                assert!(
                    !command.line().is_empty(),
                    "gate `{name}` has no command line"
                );
            }
        }
    }

    #[test]
    fn discovery_task_has_no_commands() {
        assert!(Task::List.commands().is_empty(), "list is discovery");
    }

    #[test]
    fn listed_tasks_come_from_the_cli_definition() {
        assert_eq!(
            task_names(),
            vec![
                "arch", "assure", "check", "ci", "claims", "docs", "fmt", "lint", "list", "style",
                "test", "waivers"
            ],
            "the task list reflects the subcommand enum"
        );
    }

    #[test]
    fn no_command_uses_a_shell_or_unquoted_path() {
        for (name, commands) in [
            ("ci", Task::Ci.commands()),
            ("assure", Task::Assure.commands()),
            ("style", Task::Style.commands()),
            ("check", Task::Check.commands()),
            ("fmt", Task::Fmt.commands()),
            ("lint", Task::Lint.commands()),
            ("test", Task::Test.commands()),
            ("docs", Task::Docs.commands()),
        ] {
            for command in commands {
                assert_ne!(command.program, "sh", "gate `{name}` must not use a shell");
                assert_ne!(
                    command.program, "bash",
                    "gate `{name}` must not use a shell"
                );
                for arg in &command.args {
                    assert!(
                        !arg.contains(';') && !arg.contains('$'),
                        "gate `{name}` argument `{arg}` contains shell metacharacters"
                    );
                }
            }
        }
    }

    #[test]
    fn each_subcommand_parses_to_its_task() {
        for (input, expected) in [
            ("ci", Task::Ci),
            ("assure", Task::Assure),
            ("check", Task::Check),
            ("arch", Task::Arch),
            ("waivers", Task::Waivers),
            ("claims", Task::Claims),
            ("style", Task::Style),
            ("fmt", Task::Fmt),
            ("lint", Task::Lint),
            ("test", Task::Test),
            ("docs", Task::Docs),
            ("list", Task::List),
        ] {
            let cli = Cli::try_parse_from(["xtask", input])
                .unwrap_or_else(|error| panic!("parsing {input} failed: {error}"));
            assert_eq!(cli.task, expected, "subcommand {input} parses");
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(
            Cli::try_parse_from(["xtask", "not-a-task"]).is_err(),
            "an unknown task is refused rather than forwarded to a script"
        );
    }

    fn child_status(code: i32) -> ExitStatus {
        Command::new("python3")
            .args(["-c", &format!("raise SystemExit({code})")])
            .status()
            .expect("spawn status probe")
    }

    #[test]
    fn successful_child_stays_successful() {
        assert_eq!(
            format!("{:?}", exit_code_for(child_status(0))),
            format!("{:?}", ExitCode::SUCCESS),
            "a passing gate exits successfully"
        );
    }

    #[test]
    fn failing_child_exit_code_is_propagated() {
        assert_eq!(
            format!("{:?}", exit_code_for(child_status(7))),
            format!("{:?}", ExitCode::from(7u8)),
            "the gate's own exit code reaches CI"
        );
    }

    #[test]
    fn delegated_arguments_are_accepted_by_the_gate_script() {
        let root = workspace_root().expect("workspace root");
        let script = root.join("scripts").join("check_repo.py");
        for (task, argument) in [
            (Task::Check, "check"),
            (Task::Arch, "arch"),
            (Task::Waivers, "waivers"),
            (Task::Claims, "claims"),
        ] {
            let command = task.commands();
            assert_eq!(command.len(), 1, "gate must run exactly one script");
            assert_eq!(
                command[0].args,
                vec!["scripts/check_repo.py", argument],
                "gate must delegate to check_repo.py"
            );
            let status = Command::new("python3")
                .arg(&script)
                .arg(argument)
                .current_dir(&root)
                .status()
                .expect("run gate script");
            assert!(
                status.success(),
                "gate `{argument}` is accepted and passes in the current tree"
            );
        }
    }
}
