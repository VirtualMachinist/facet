//! Facet command-line adapter.
//!
//! Facet is Probe cut for the terminal with Lattice underneath. Every Probe
//! command is delegated to `probe-cli` verbatim, so the upstream JSON contract
//! (`docs/CLI.md`) holds unchanged. Facet owns four things:
//!
//! - `request run` executes through the same core and HTTP engine as Probe,
//!   then records the run in the workspace Lattice store.
//! - `history`, `blob`, and `gc` read and maintain that store.
//! - `session` starts, ends, lists, and shows agent sessions in the machine store.
//! - `replay` re-sends a recorded run from the current YAML; `diff` compares two runs.
//! - `tui` opens the terminal UI.
//!
//! Contract details for the Facet-only commands live in `docs/FACET.md`.

#![forbid(unsafe_code)]

use std::io::{self, Read};

use serde_json::{Value, json};

mod args;
mod diff;
mod error;
mod history;
mod presentation;
mod replay;
mod run;
mod session;
mod tui;
mod workspace;

pub use error::FacetError;
pub use probe_cli::{
    CONFIGURATION_EXIT_CODE, EXECUTION_EXIT_CODE, IMPORT_EXIT_CODE, INVALID_ARGUMENTS_EXIT_CODE,
    INVALID_WORKSPACE_EXIT_CODE, JSON_SCHEMA_VERSION, PERSISTENCE_EXIT_CODE,
    REQUEST_NOT_FOUND_EXIT_CODE,
};
pub use tui::run_tui;

/// Exit code used when the Lattice store cannot be opened, read, or written.
/// Extends the upstream exit-code table; never renumbers it.
pub const LATTICE_EXIT_CODE: u8 = 9;

/// Exit code for the assertion family: the check the caller asked for is
/// false (`replay --frozen` refused, `diff` found differences). Unix's `1`,
/// as in `test`, `grep`, `diff`, `cmp`; unused upstream (constants start at
/// 2), so it never collides.
pub const ASSERTION_EXIT_CODE: u8 = 1;

/// Captured CLI output and process status. `stdout` is bytes because
/// `facet blob` writes a raw body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutput {
    /// Command output written to stdout.
    pub stdout: Vec<u8>,
    /// Diagnostics written to stderr.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: u8,
}

impl RunOutput {
    fn success(stdout: Vec<u8>, stderr: String) -> Self {
        Self {
            stdout,
            stderr,
            exit_code: 0,
        }
    }

    fn failure(error: FacetError, json_output: bool) -> Self {
        if json_output {
            let mut value = json!({
                "schemaVersion": JSON_SCHEMA_VERSION,
                "error": {
                    "category": error.category,
                    "exitCode": error.exit_code,
                    "message": error.message,
                }
            });
            if let Some(details) = error.details {
                value["error"]["details"] = details;
            }
            Self {
                stdout: pretty_json(&value).into_bytes(),
                stderr: String::new(),
                exit_code: error.exit_code,
            }
        } else {
            Self {
                stdout: Vec::new(),
                stderr: format!("error[{}]: {}\n", error.category, error.message),
                exit_code: error.exit_code,
            }
        }
    }
}

/// A command's human and JSON renderings plus any stderr diagnostics.
#[derive(Debug)]
pub(crate) struct CommandOutput {
    human: Vec<u8>,
    json: Value,
    warnings: Vec<String>,
    exit_code: u8,
}

impl CommandOutput {
    pub(crate) fn new(human: impl Into<Vec<u8>>, json: Value) -> Self {
        Self {
            human: human.into(),
            json,
            warnings: Vec::new(),
            exit_code: 0,
        }
    }

    pub(crate) fn warn(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// A successful document that still signals an assertion outcome
    /// (`diff` differs). Output is rendered as usual; only the code changes.
    pub(crate) fn with_exit_code(mut self, exit_code: u8) -> Self {
        self.exit_code = exit_code;
        self
    }

    fn render(self, json_output: bool, quiet: bool) -> RunOutput {
        let stdout = if quiet {
            Vec::new()
        } else if json_output {
            pretty_json(&versioned_json(self.json)).into_bytes()
        } else {
            self.human
        };
        let stderr = self
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}\n"))
            .collect();
        RunOutput {
            stdout,
            stderr,
            exit_code: self.exit_code,
        }
    }
}

/// Returns the stable top-level help text.
#[must_use]
pub const fn help() -> &'static str {
    concat!(
        "Facet - Probe, cut for the terminal, with Lattice underneath\n",
        "\n",
        "Usage: facet [OPTIONS] <COMMAND>\n",
        "\n",
        "Every Probe command works unchanged (see `probe --help`). Facet adds:\n",
        "\n",
        "Commands:\n",
        "  request run <path> <selector>       Execute an HTTP request and record it in Lattice\n",
        "  history [<path>]                    List recorded runs, newest first (metadata only)\n",
        "  history [<path>] --id <ulid>        Show one recorded run by id\n",
        "  history [<path>] --sql <query>      Run read-only SQL against the workspace store\n",
        "  session start                       Start a session; prints its ULID (use with FACET_SESSION)\n",
        "  session end [<id>|current]          End a session (default: $FACET_SESSION); idempotent\n",
        "  session list                        List sessions, newest first\n",
        "  session show <id>|current           Show one session\n",
        "  replay <runId> [<path>]             Re-send a recorded run from the current YAML\n",
        "  diff <idA> <idB> [<path>]           Compare two recorded runs, hashes first (exit 1 if different)\n",
        "  blob <hash> [<path>]                Fetch one stored body by SHA-256\n",
        "  gc [<path>] [--yes]                 Expire old runs and sweep orphaned blobs\n",
        "  tui [<path>]                        Open the terminal UI\n",
        "\n",
        "Options:\n",
        "      --no-record             Execute without writing to Lattice\n",
        "      --tag <tag>             Tag the recorded run, or filter history by tag (AND); may be repeated\n",
        "      --inline-body-max <n>   Inline bodies at or under this size (e.g. 64KiB)\n",
        "      --history-retention <r> Retention window for gc (unlimited or e.g. 30d)\n",
        "      --limit <n>             Maximum history rows (default 50)\n",
        "      --request <selector>    Only history for one request\n",
        "      --status <code>         Only history with one HTTP status\n",
        "      --actor <name>          Only history or sessions by one actor; session start actor\n",
        "      --since <unix-ms>       Only history started at or after this time\n",
        "      --session <id>|current  Only history recorded in one session\n",
        "      --environment <name>    Only history resolved with one environment\n",
        "      --hash <sha256>         Only history whose request, request body, or response body hash matches\n",
        "      --id <ulid>             One run by id (exclusive with the filters above)\n",
        "      --bodies                Include inline bodies in history JSON; unified body diff for diff\n",
        "      --frozen                Refuse to replay when the request hash changed (exit 1)\n",
        "      --meta <json>           Session metadata object (session start)\n",
        "      --open                  Only sessions still open (session list)\n",
        "      --output <file>         Write a blob to a file instead of stdout\n",
        "      --yes                   Apply gc deletions (default is a dry run)\n",
        "      --appearance <name>     TUI appearance: graphite or porcelain\n",
        "      --json                  Emit versioned deterministic JSON\n",
        "  -q, --quiet                 Suppress successful command output\n",
        "  -h, --help                  Print help\n",
        "  -V, --version               Print version\n",
        "\n",
        "Environment: FACET_ACTOR (run actor, default human), FACET_SESSION (session id),\n",
        "FACET_NO_RECORD=1 (never record), FACET_DATA_DIR / FACET_CONFIG_DIR (machine store paths).\n",
    )
}

/// Returns the Facet version, sourced from the Cargo workspace version.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Runs the CLI adapter for arguments that exclude the executable name.
#[must_use]
pub fn run<I, S>(args: I) -> RunOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    run_with_stdin(args, &mut io::empty())
}

/// Runs the CLI adapter with a reader used when the workspace path is `-`.
#[must_use]
pub fn run_with_stdin<I, S, R>(args: I, stdin: &mut R) -> RunOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    R: Read,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect();

    let owned = match args.first().map(String::as_str) {
        None => true,
        Some(
            "history" | "session" | "replay" | "diff" | "blob" | "gc" | "tui" | "-V" | "--version"
            | "-h" | "--help",
        ) => true,
        Some("request") => args.get(1).map(String::as_str) == Some("run"),
        Some(_) => false,
    };
    if !owned {
        let output = probe_cli::run_with_stdin(args, stdin);
        return RunOutput {
            stdout: output.stdout.into_bytes(),
            stderr: output.stderr,
            exit_code: output.exit_code,
        };
    }

    let mut args = args;
    let (json_output, quiet) = match strip_global_flags(&mut args) {
        Ok(flags) => flags,
        Err((error, json_output)) => return RunOutput::failure(error, json_output),
    };

    if args.is_empty() || args == ["-h"] || args == ["--help"] {
        return RunOutput::success(help().as_bytes().to_vec(), String::new());
    }
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return RunOutput::success(help().as_bytes().to_vec(), String::new());
    }
    if args == ["-V"] || args == ["--version"] {
        let output = CommandOutput::new(
            format!("facet {} (probe {})\n", version(), probe_cli::version()),
            json!({
                "name": "facet",
                "version": version(),
                "probeVersion": probe_cli::version(),
            }),
        );
        return output.render(json_output, quiet);
    }

    let result = match args[0].as_str() {
        "request" => run::run(&args[2..], stdin),
        "history" => history::history(&args[1..]),
        "session" => session::session(&args[1..]),
        "replay" => replay::replay(&args[1..], stdin),
        "diff" => diff::diff(&args[1..]),
        "blob" => history::blob(&args[1..]),
        "gc" => history::gc(&args[1..]),
        "tui" => Err(FacetError::invalid_arguments(
            "tui is interactive and must be started from the facet binary",
        )),
        _ => unreachable!("owned commands are routed above"),
    };
    match result {
        Ok(output) => output.render(json_output, quiet),
        Err(error) => RunOutput::failure(error, json_output),
    }
}

/// Removes `--json` and `--quiet` with upstream's exact rules.
fn strip_global_flags(args: &mut Vec<String>) -> Result<(bool, bool), (FacetError, bool)> {
    let json_count = remove_flags(args, &["--json"]);
    let json_output = json_count == 1;
    let quiet_count = remove_flags(args, &["-q", "--quiet"]);
    let quiet = quiet_count == 1;
    if json_count > 1 {
        return Err((
            FacetError::invalid_arguments("--json may only be specified once"),
            true,
        ));
    }
    if quiet_count > 1 {
        return Err((
            FacetError::invalid_arguments("--quiet may only be specified once"),
            json_output,
        ));
    }
    if json_output && quiet {
        return Err((
            FacetError::invalid_arguments("--json and --quiet cannot be used together"),
            true,
        ));
    }
    Ok((json_output, quiet))
}

fn remove_flags(args: &mut Vec<String>, flags: &[&str]) -> usize {
    let count = args
        .iter()
        .filter(|argument| flags.contains(&argument.as_str()))
        .count();
    args.retain(|argument| !flags.contains(&argument.as_str()));
    count
}

fn versioned_json(mut value: Value) -> Value {
    value
        .as_object_mut()
        .expect("command JSON output must be an object")
        .insert("schemaVersion".to_owned(), json!(JSON_SCHEMA_VERSION));
    value
}

fn pretty_json(value: &Value) -> String {
    let mut output =
        serde_json::to_string_pretty(value).expect("JSON value serialization cannot fail");
    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use super::{INVALID_ARGUMENTS_EXIT_CODE, run};

    #[test]
    fn version_names_both_binaries() {
        let output = run(["--version", "--json"]);
        assert_eq!(output.exit_code, 0);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["name"], "facet");
        assert_eq!(value["schemaVersion"], 1);
        assert!(value["probeVersion"].is_string());
    }

    #[test]
    fn delegates_unknown_commands_to_probe() {
        let output = run(["collection", "validate", "/nonexistent/path.yml", "--json"]);
        assert_ne!(output.exit_code, 0);
        assert!(String::from_utf8_lossy(&output.stdout).contains("invalid_workspace"));
    }

    #[test]
    fn rejects_json_with_quiet_like_upstream() {
        let output = run(["history", "--json", "--quiet"]);
        assert_eq!(output.exit_code, INVALID_ARGUMENTS_EXIT_CODE);
        assert!(String::from_utf8_lossy(&output.stdout).contains("invalid_arguments"));
    }
}
