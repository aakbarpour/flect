//! Flect command-line entry point.

mod app;
mod eval;
mod mcp;
mod report;
mod skill;

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use flect_core::{Alignment, ContextPolicy, FindingCategory};
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
    /// Inspect or update the repository's human-readable configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Perform trusted handoffs for Codex-native agent verification.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Serve Flect tools over the Model Context Protocol on stdio.
    Mcp,
    /// Run the reproducible semantic-verification evaluation suite.
    Eval {
        /// Evaluation suite JSON file.
        #[arg(long, default_value = "fixtures/evaluation/cases.json")]
        suite: PathBuf,
        /// API profile TOML file. Omit for deterministic offline evaluation.
        #[arg(long, value_name = "PATH")]
        profiles: Option<PathBuf>,
        /// Explicitly permit model-backed requests that may incur charges.
        #[arg(long, requires = "profiles")]
        allow_paid_api: bool,
        /// Write the complete JSON report to this path.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
    },
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

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print the effective validated configuration.
    Show,
    /// Set one supported configuration key.
    Set {
        /// Dotted key, such as runner.kind or runner.model.
        key: String,
        /// New value.
        value: String,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// Prepare sanitized read-only resources for a fresh blind verifier.
    PrepareBlind {
        #[arg(long)]
        run: Option<String>,
        #[arg(long)]
        context: Option<ContextPolicy>,
    },
    /// Begin a repository-independent typed verifier submission.
    VerifierBegin {
        #[arg(long)]
        job: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, value_enum, default_value_t = ModelSelectionArg::Unknown)]
        model_selection: ModelSelectionArg,
    },
    /// Set the verifier's apparent objective from a UTF-8 text file.
    VerifierSetObjective {
        #[arg(long)]
        job: String,
        #[arg(long)]
        text_file: PathBuf,
    },
    /// Add one apparent pre-change behavior from a UTF-8 text file.
    VerifierAddBefore {
        #[arg(long)]
        job: String,
        #[arg(long)]
        text_file: PathBuf,
    },
    /// Add one apparent post-change behavior from a UTF-8 text file.
    VerifierAddAfter {
        #[arg(long)]
        job: String,
        #[arg(long)]
        text_file: PathBuf,
    },
    /// Add one allowed affected scope without repository access.
    VerifierAddScope {
        #[arg(long)]
        job: String,
        #[arg(long)]
        file: String,
        #[arg(long)]
        symbol_file: Option<PathBuf>,
    },
    /// Add one apparent side effect from a UTF-8 text file.
    VerifierAddSideEffect {
        #[arg(long)]
        job: String,
        #[arg(long)]
        text_file: PathBuf,
    },
    /// Add one verifier assumption from a UTF-8 text file.
    VerifierAddAssumption {
        #[arg(long)]
        job: String,
        #[arg(long)]
        text_file: PathBuf,
    },
    /// Add one verifier uncertainty from a UTF-8 text file.
    VerifierAddUncertainty {
        #[arg(long)]
        job: String,
        #[arg(long)]
        text_file: PathBuf,
    },
    /// Set finite verifier confidence in the inclusive range 0 through 1.
    VerifierSetConfidence {
        #[arg(long)]
        job: String,
        confidence: f64,
    },
    /// Seal a typed verifier submission in external Flect state.
    VerifierSubmit {
        #[arg(long)]
        job: String,
    },
    /// Commit a sealed verifier job into repository Flect state; parent passes only the job ID.
    VerifierCommit {
        #[arg(long)]
        job: String,
    },
    /// Prepare a separate judge job after an echo is accepted.
    PrepareReconciliation {
        #[arg(long)]
        blind_job: String,
    },
    /// Begin a typed judge submission owned by Flect.
    JudgeBegin {
        #[arg(long)]
        job: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, value_enum, default_value_t = ModelSelectionArg::Unknown)]
        model_selection: ModelSelectionArg,
    },
    /// Set the semantic alignment for a typed judge submission.
    JudgeSetAlignment {
        #[arg(long)]
        job: String,
        #[arg(value_enum)]
        alignment: AlignmentArg,
    },
    /// Add one semantic finding. Read text from a file to avoid shell quoting hazards.
    JudgeAddFinding {
        #[arg(long)]
        job: String,
        #[arg(long, value_enum)]
        kind: FindingKindArg,
        #[arg(long, value_name = "PATH")]
        text_file: PathBuf,
        #[arg(long)]
        evidence_ref: Option<String>,
    },
    /// Set the finite confidence in the inclusive range 0 through 1.
    JudgeSetConfidence {
        #[arg(long)]
        job: String,
        confidence: f64,
    },
    /// Validate and persist the Flect-owned typed judge submission.
    JudgeSubmit {
        #[arg(long)]
        job: String,
    },
    /// Delete Flect-owned completed workspaces, or explicitly selected stale jobs.
    Cleanup {
        /// Report eligible workspaces without deleting them.
        #[arg(long)]
        dry_run: bool,
        /// Include unfinished jobs. Use only when intentionally discarding forensic state.
        #[arg(long)]
        all: bool,
        /// Include jobs older than this many hours.
        #[arg(long, value_name = "HOURS")]
        older_than: Option<u64>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AlignmentArg {
    Same,
    Partial,
    Different,
    Uncertain,
}
impl From<AlignmentArg> for Alignment {
    fn from(value: AlignmentArg) -> Self {
        match value {
            AlignmentArg::Same => Self::Same,
            AlignmentArg::Partial => Self::Partial,
            AlignmentArg::Different => Self::Different,
            AlignmentArg::Uncertain => Self::Uncertain,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FindingKindArg {
    MissingRequirement,
    UnrequestedChange,
    ViolatedConstraint,
    PotentialSideEffect,
}
impl From<FindingKindArg> for FindingCategory {
    fn from(value: FindingKindArg) -> Self {
        match value {
            FindingKindArg::MissingRequirement => Self::MissingRequirements,
            FindingKindArg::UnrequestedChange => Self::UnrequestedChanges,
            FindingKindArg::ViolatedConstraint => Self::ViolatedConstraints,
            FindingKindArg::PotentialSideEffect => Self::PotentialSideEffects,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ModelSelectionArg {
    Explicit,
    Inherited,
    Unknown,
}
impl From<ModelSelectionArg> for flect_core::AgentModelSelection {
    fn from(value: ModelSelectionArg) -> Self {
        match value {
            ModelSelectionArg::Explicit => Self::Explicit,
            ModelSelectionArg::Inherited => Self::Inherited,
            ModelSelectionArg::Unknown => Self::Unknown,
        }
    }
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
