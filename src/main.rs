mod check;
mod policy;
mod process;
mod version;

use clap::{Parser, Subcommand};
use serde_json::json;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    version,
    about = "Check team tool versions and explain required upgrades"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Action>,
    /// Manifest to check (relative pins resolve beside this file)
    #[arg(long, global = true, default_value = "toolcheck.toml")]
    config: PathBuf,
    /// Emit a single deterministic JSON report
    #[arg(long, global = true)]
    json: bool,
    /// Treat warnings as unmet requirements
    #[arg(long, global = true)]
    strict: bool,
}

#[derive(Subcommand)]
enum Action {
    /// Check installed versions against the manifest (also the default action)
    Check,
}

fn emit_error(json_output: bool, kind: &str, error: &str) {
    if json_output {
        let value = json!({"schema_version": 1, "error_kind": kind, "error": error});
        let _ = writeln!(io::stdout().lock(), "{value}");
    } else {
        let _ = writeln!(io::stderr().lock(), "toolcheck: {error}");
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if error.use_stderr() {
                emit_error(
                    std::env::args_os().any(|arg| arg == "--json"),
                    "invocation",
                    &error.to_string(),
                );
                return ExitCode::from(2);
            }
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
    };
    let tools = match policy::load(&cli.config) {
        Ok(tools) => tools,
        Err(error) => {
            emit_error(cli.json, "configuration", &format!("{error:#}"));
            return ExitCode::from(2);
        }
    };
    let report = check::run(&tools, cli.strict);
    let output = if cli.json {
        serde_json::to_string_pretty(&report).expect("report serialization") + "\n"
    } else {
        check::human(&report)
    };
    if let Err(error) = io::stdout().lock().write_all(output.as_bytes()) {
        let _ = writeln!(
            io::stderr().lock(),
            "toolcheck: cannot write report: {error}"
        );
        return ExitCode::from(2);
    }
    ExitCode::from(if report.summary.acceptable { 0 } else { 1 })
}
