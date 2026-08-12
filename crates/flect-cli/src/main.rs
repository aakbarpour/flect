//! Flect command-line entry point.

mod app;
mod report;
mod skill;

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};
use flect_core::ContextPolicy;
use miette::Result;

#[derive(Debug, Parser)]
#[command(
    name = "flect",
    version,
    about = "Independent intent verification for AI-written patches"
)]
struct Cli {
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    /// Show diagnostic logging. Repeat for debug logging.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize Flect configuration in the current repository.
    Init,
    /// Capture a task and immutable base revision before implementation.
    Start {
        /// Original task text. Reads stdin when omitted and input is piped.
        #[arg(long, conflicts_with = "task_file")]
        task: Option<String>,
        /// Read the original task from a UTF-8 file.
        #[arg(long, value_name = "PATH", conflicts_with = "task")]
        task_file: Option<PathBuf>,
        /// Use a pre-authored `IntendedSpec` JSON document.
        #[arg(long, value_name = "PATH")]
        spec_file: Option<PathBuf>,
    },
    /// Capture the patch, reconstruct intent, and compare it with the run spec.
    Verify {
        /// Run ID. Defaults to the latest run.
        #[arg(long)]
        run: Option<String>,
        /// Deterministic `EchoedSpec` JSON input for tests and offline workflows.
        #[arg(long, value_name = "PATH")]
        echoed_spec: Option<PathBuf>,
        /// Override the configured context policy.
        #[arg(long)]
        context: Option<ContextPolicy>,
        /// Show the exact request boundary and runner selection without invoking a model.
        #[arg(long)]
        dry_run: bool,
    },
    /// Reconstruct what the current patch appears to do without an original task.
    Echo {
        /// Base revision; for example, HEAD~1. Defaults to HEAD.
        revision: Option<String>,
        /// Deterministic `EchoedSpec` JSON input for tests and offline workflows.
        #[arg(long, value_name = "PATH")]
        echoed_spec: Option<PathBuf>,
        /// Override the configured context policy.
        #[arg(long)]
        context: Option<ContextPolicy>,
    },
    /// Show exactly what a verifier would receive, without invoking it.
    Inspect {
        /// Run ID. Defaults to the latest run.
        #[arg(long)]
        run: Option<String>,
        /// Override the configured context policy.
        #[arg(long)]
        context: Option<ContextPolicy>,
    },
    /// Diagnose the local Git, repository, and Flect configuration.
    Doctor,
    /// Manage the project-local Codex Skill integration.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SkillCommand {
    /// Install the bundled Skill under `.agents/skills/flect`.
    Install,
    /// Report whether the project-local Skill is current, missing, or modified.
    Status,
    /// Remove only unmodified files owned by Flect.
    Uninstall,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    app::run(cli.command, cli.json).await
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(level)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}
