//! CLI orchestration. Domain policy remains in `flect-core`.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use flect_core::{
    BlindBundle, BlindGuard, Config, ContextBuilder, ContextPolicy, EchoedSpec, FileStatus,
    GitRepository, IntendedSpec, RunRecord, RunStore, TaskInput, VerificationRecord, reconcile,
};
use flect_runner::{AgentRequest, AgentRunner, MockRunner, RequestPurpose};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use schemars::schema_for;
use serde_json::json;

use crate::Command;
use crate::report;

pub fn run(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::Init => init(json_output),
        Command::Start {
            task,
            task_file,
            spec_file,
        } => start(
            task,
            task_file.as_deref(),
            spec_file.as_deref(),
            json_output,
        ),
        Command::Verify {
            run,
            echoed_spec,
            context,
        } => verify(run.as_deref(), echoed_spec.as_deref(), context, json_output),
        Command::Echo {
            revision,
            echoed_spec,
            context,
        } => echo(
            revision.as_deref(),
            echoed_spec.as_deref(),
            context,
            json_output,
        ),
        Command::Inspect { run, context } => inspect(run.as_deref(), context, json_output),
        Command::Doctor => doctor(json_output),
    }
}

fn init(json_output: bool) -> Result<()> {
    let repository = discover_current()?;
    let config_path = repository.root().join("flect.toml");
    let config_created = if config_path.exists() {
        false
    } else {
        fs::write(&config_path, Config::default_document())
            .into_diagnostic()
            .wrap_err_with(|| format!("could not write {}", config_path.display()))?;
        true
    };
    let ignore_updated = ensure_state_ignored(&repository.root().join(".gitignore"))?;

    if json_output {
        print_json(&json!({
            "repository": repository.root(),
            "config": config_path,
            "config_created": config_created,
            "gitignore_updated": ignore_updated,
        }))?;
    } else {
        println!("Flect initialized\n");
        println!("Repository  {}", repository.root().display());
        println!(
            "Config      {} ({})",
            config_path.display(),
            if config_created {
                "created"
            } else {
                "already present"
            }
        );
        println!(
            "State       .flect/ ({})",
            if ignore_updated {
                "added to .gitignore"
            } else {
                "already ignored"
            }
        );
    }
    Ok(())
}

fn start(
    task_argument: Option<String>,
    task_file: Option<&Path>,
    spec_file: Option<&Path>,
    json_output: bool,
) -> Result<()> {
    let repository = discover_current()?;
    let config = load_config(repository.root())?;
    ensure_mock_provider(&config)?;
    let base_revision = repository.head_revision().map_err(to_report)?;
    let task = TaskInput {
        text: read_task(task_argument, task_file)?,
    };
    if task.text.trim().is_empty() {
        return Err(miette!("the original task cannot be empty"));
    }
    let intended_spec = match spec_file {
        Some(path) => read_json_file(path)?,
        None => IntendedSpec::from_task(&task),
    };
    let now = unix_millis()?;
    let run = RunRecord {
        version: 1,
        id: generate_run_id(now, &base_revision, &task.text),
        repository_root: repository.root().display().to_string(),
        base_revision,
        task,
        intended_spec,
        created_unix_ms: now,
    };
    RunStore::new(repository.root())
        .save_run(&run)
        .map_err(to_report)?;

    if json_output {
        print_json(&run)?;
    } else {
        report::run_created(&run);
    }
    Ok(())
}

fn verify(
    run_id: Option<&str>,
    echoed_spec_path: Option<&Path>,
    context: Option<ContextPolicy>,
    json_output: bool,
) -> Result<()> {
    let repository = discover_current()?;
    let mut config = load_config(repository.root())?;
    ensure_mock_provider(&config)?;
    if let Some(context) = context {
        config.verification.context = context;
    }
    let store = RunStore::new(repository.root());
    let run = store.load_run(run_id).map_err(to_report)?;
    if Path::new(&run.repository_root) != repository.root() {
        return Err(miette!(
            "run {} belongs to {}, not {}",
            run.id,
            run.repository_root,
            repository.root().display()
        ));
    }
    let bundle = build_bundle(&repository, &config, &run.base_revision)?;
    let echoed = reconstruct(&bundle, echoed_spec_path)?;
    let verdict = reconcile(&run.intended_spec, &echoed);
    let record = VerificationRecord {
        version: 1,
        run_id: run.id,
        bundle,
        echoed_spec: echoed,
        verdict,
        verified_unix_ms: unix_millis()?,
    };
    store.save_verification(&record).map_err(to_report)?;

    if json_output {
        print_json(&record)?;
    } else {
        report::verification(&record);
    }
    Ok(())
}

fn echo(
    revision: Option<&str>,
    echoed_spec_path: Option<&Path>,
    context: Option<ContextPolicy>,
    json_output: bool,
) -> Result<()> {
    let repository = discover_current()?;
    let mut config = load_config(repository.root())?;
    ensure_mock_provider(&config)?;
    if let Some(context) = context {
        config.verification.context = context;
    }
    let base = match revision {
        Some(revision) => repository.resolve_revision(revision).map_err(to_report)?,
        None => repository.head_revision().map_err(to_report)?,
    };
    let bundle = build_bundle(&repository, &config, &base)?;
    let echoed = reconstruct(&bundle, echoed_spec_path)?;
    if json_output {
        print_json(&json!({ "bundle_manifest": bundle.manifest, "echoed_spec": echoed }))?;
    } else {
        report::echo(&echoed);
    }
    Ok(())
}

fn inspect(run_id: Option<&str>, context: Option<ContextPolicy>, json_output: bool) -> Result<()> {
    let repository = discover_current()?;
    let mut config = load_config(repository.root())?;
    if let Some(context) = context {
        config.verification.context = context;
    }
    let run = RunStore::new(repository.root())
        .load_run(run_id)
        .map_err(to_report)?;
    let bundle = build_bundle(&repository, &config, &run.base_revision)?;
    if json_output {
        print_json(&bundle)?;
    } else {
        report::inspection(&bundle);
    }
    Ok(())
}

fn doctor(json_output: bool) -> Result<()> {
    let git = ProcessCommand::new("git").arg("--version").output();
    let git_version = match git {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        _ => "unavailable".to_owned(),
    };
    let current = std::env::current_dir().into_diagnostic()?;
    let repository = GitRepository::discover(&current);
    let (root, config_status, provider) = match repository {
        Ok(repository) => match load_config(repository.root()) {
            Ok(config) => (
                repository.root().display().to_string(),
                "valid".to_owned(),
                config.runner.provider,
            ),
            Err(error) => (
                repository.root().display().to_string(),
                format!("invalid: {error}"),
                "unknown".to_owned(),
            ),
        },
        Err(error) => (
            format!("unavailable: {error}"),
            "not checked".to_owned(),
            "not checked".to_owned(),
        ),
    };
    let ready = git_version != "unavailable" && config_status == "valid" && provider == "mock";
    let result = json!({
        "git": git_version,
        "repository": root,
        "configuration": config_status,
        "runner_provider": provider,
        "ready": ready,
    });
    if json_output {
        print_json(&result)?;
    } else {
        report::doctor(&result);
    }
    Ok(())
}

fn build_bundle(repository: &GitRepository, config: &Config, base: &str) -> Result<BlindBundle> {
    let patch = repository
        .capture_patch(
            base,
            config.verification.include_untracked,
            config.privacy.respect_gitignore,
            config.verification.max_patch_bytes,
        )
        .map_err(to_report)?;
    let context = ContextBuilder::new(repository.root(), config)
        .map_err(to_report)?
        .build(patch)
        .map_err(to_report)?;
    BlindGuard::build(context, &config.blind).map_err(to_report)
}

fn reconstruct(bundle: &BlindBundle, echoed_spec_path: Option<&Path>) -> Result<EchoedSpec> {
    let response = match echoed_spec_path {
        Some(path) => read_json_file(path)?,
        None => deterministic_echo(bundle),
    };
    let runner = MockRunner::with_response(&response).map_err(to_report)?;
    let request = AgentRequest {
        purpose: RequestPurpose::ReconstructPatchIntent,
        input: serde_json::to_value(bundle).into_diagnostic()?,
    };
    let schema = serde_json::to_value(schema_for!(EchoedSpec)).into_diagnostic()?;
    let value = runner
        .generate_structured(&request, &schema)
        .map_err(to_report)?;
    serde_json::from_value(value)
        .into_diagnostic()
        .wrap_err("runner response did not match the EchoedSpec schema")
}

fn deterministic_echo(bundle: &BlindBundle) -> EchoedSpec {
    let behavior_after = bundle
        .patch
        .files
        .iter()
        .map(|file| {
            let verb = match file.status {
                FileStatus::Added | FileStatus::Untracked => "Adds",
                FileStatus::Modified => "Modifies",
                FileStatus::Deleted => "Deletes",
                FileStatus::Renamed => "Renames",
            };
            format!("{verb} {}", file.path)
        })
        .collect();
    let affected_scope = bundle
        .patch
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    EchoedSpec {
        apparent_objective: if affected_scope.is_empty() {
            "No text changes detected".to_owned()
        } else {
            format!(
                "Change {} file(s): {}",
                affected_scope.len(),
                affected_scope.join(", ")
            )
        },
        behavior_after,
        affected_scope,
        uncertainties: vec![
            "Milestone 1 has no real verifier; file-level reconstruction is intentionally uncertain"
                .to_owned(),
        ],
        confidence: 0.35,
        ..EchoedSpec::default()
    }
}

fn discover_current() -> Result<GitRepository> {
    let current = std::env::current_dir().into_diagnostic()?;
    GitRepository::discover(&current).map_err(to_report)
}

fn load_config(root: &Path) -> Result<Config> {
    Config::load(&root.join("flect.toml")).map_err(to_report)
}

fn ensure_mock_provider(config: &Config) -> Result<()> {
    if config.runner.provider == "mock" {
        Ok(())
    } else {
        Err(miette!(
            "runner provider `{}` is configured, but Milestone 1 supports only `mock`; real providers are introduced in Milestone 2",
            config.runner.provider
        ))
    }
}

fn read_task(argument: Option<String>, path: Option<&Path>) -> Result<String> {
    if let Some(task) = argument {
        return Ok(task);
    }
    if let Some(path) = path {
        return fs::read_to_string(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not read task file {}", path.display()));
    }
    if io::stdin().is_terminal() {
        return Err(miette!(
            "provide the original task with `--task`, `--task-file`, or piped stdin"
        ));
    }
    let mut task = String::new();
    io::stdin()
        .read_to_string(&mut task)
        .into_diagnostic()
        .wrap_err("could not read task from stdin")?;
    Ok(task)
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .into_diagnostic()
        .wrap_err_with(|| format!("{} is not valid expected JSON", path.display()))
}

fn ensure_state_ignored(path: &Path) -> Result<bool> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).into_diagnostic(),
    };
    if existing.lines().any(|line| line.trim() == ".flect/") {
        return Ok(false);
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not update {}", path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file).into_diagnostic()?;
    }
    writeln!(file, ".flect/").into_diagnostic()?;
    Ok(true)
}

fn unix_millis() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .into_diagnostic()
        .wrap_err("system clock is before the Unix epoch")?;
    u64::try_from(elapsed.as_millis()).into_diagnostic()
}

fn generate_run_id(now: u64, revision: &str, task: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    now.hash(&mut hasher);
    revision.hash(&mut hasher);
    task.hash(&mut hasher);
    format!("fl_{:016x}", hasher.finish())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value).into_diagnostic()?;
    writeln!(output).into_diagnostic()
}

fn to_report(error: impl std::fmt::Display) -> miette::Report {
    miette!("{error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_echo_never_claims_confidence() {
        let bundle: BlindBundle = serde_json::from_value(json!({
            "patch": {
                "base_revision": "abc", "files": [], "renames": 0,
                "insertions": 0, "deletions": 0, "binary_files": [], "untracked_files": []
            },
            "context": [],
            "manifest": { "context_policy": "patch", "patch_files": [], "context_files": [], "excluded_paths": [], "total_bytes": 0 },
            "blindness_report": { "isolation": "strict", "entries": [], "limitations": [] }
        })).unwrap();
        assert!(deterministic_echo(&bundle).confidence < 0.5);
    }
}
