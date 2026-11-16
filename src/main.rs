mod check;
mod init;
mod output;
mod policy;
mod process;
mod version;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;
use std::io::{self, IsTerminal, Write};
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
    /// Manifest to check or create (relative pins resolve beside this file)
    #[arg(long, global = true, default_value = "toolcheck.toml")]
    config: PathBuf,
    /// Emit a single deterministic JSON report
    #[arg(long, global = true)]
    json: bool,
    /// Treat warnings as unmet requirements
    #[arg(long, global = true)]
    strict: bool,
    /// Show executable paths, reasons, and all shadowed installations
    #[arg(short, long, global = true)]
    verbose: bool,
    /// When to colorize human-readable check output
    #[arg(long, global = true, value_enum, default_value = "auto")]
    color: ColorMode,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn enabled(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => {
                io::stdout().is_terminal()
                    && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
                    && std::env::var_os("TERM").is_none_or(|value| value != "dumb")
            }
        }
    }
}

#[derive(Subcommand)]
enum Action {
    /// Check installed versions against the manifest (also the default action)
    Check,
    /// Create a starter policy from nearby version pins and commented examples
    Init,
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
    let (output, code) = match run(&cli) {
        Ok(result) => result,
        Err(error) => {
            emit_error(cli.json, "configuration", &format!("{error:#}"));
            return ExitCode::from(2);
        }
    };
    if let Err(error) = io::stdout().lock().write_all(output.as_bytes()) {
        let _ = writeln!(
            io::stderr().lock(),
            "toolcheck: cannot write report: {error}"
        );
        return ExitCode::from(2);
    }
    ExitCode::from(code)
}

fn run(cli: &Cli) -> anyhow::Result<(String, u8)> {
    if matches!(cli.command, Some(Action::Init)) {
        let report = init::create(&cli.config)?;
        let output = if cli.json {
            serde_json::to_string_pretty(&report)? + "\n"
        } else {
            init::human(&report)
        };
        return Ok((output, 0));
    }
    let tools = policy::load(&cli.config)?;
    let report = check::run(&tools, cli.strict);
    let output = if cli.json {
        serde_json::to_string_pretty(&report)? + "\n"
    } else {
        output::human(&report, cli.verbose, cli.color.enabled())
    };
    Ok((output, if report.summary.acceptable { 0 } else { 1 }))
}
