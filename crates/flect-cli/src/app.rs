//! CLI orchestration. Domain policy remains in `flect-core`.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flect_core::{
    BlindBundle, BlindGuard, Config, ContextBuilder, ContextPolicy, EchoedSpec, Evidence,
    FileStatus, GitRepository, IntendedSpec, ModelCallRecord, RunRecord, RunStore, RunnerKind,
    TaskInput, Verdict, VerificationRecord, reconcile,
};
use flect_runner::{
    AgentRequest, AgentRunner, MockRunner, OpenAiResponsesConfig, OpenAiResponsesRunner,
    RequestPurpose, RunnerMetadata,
};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Command;
use crate::report;

pub async fn run(command: Command, json_output: bool) -> Result<()> {
    match command {
        Command::Init => init(json_output),
        Command::Start {
            task,
            task_file,
            spec_file,
        } => {
            start(
                task,
                task_file.as_deref(),
                spec_file.as_deref(),
                json_output,
            )
            .await
        }
        Command::Verify {
            run,
            echoed_spec,
            context,
            dry_run,
        } => {
            verify(
                run.as_deref(),
                echoed_spec.as_deref(),
                context,
                dry_run,
                json_output,
            )
            .await
        }
        Command::Echo {
            revision,
            echoed_spec,
            context,
        } => {
            echo(
                revision.as_deref(),
                echoed_spec.as_deref(),
                context,
                json_output,
            )
            .await
        }
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

async fn start(
    task_argument: Option<String>,
    task_file: Option<&Path>,
    spec_file: Option<&Path>,
    json_output: bool,
) -> Result<()> {
    let repository = discover_current()?;
    let config = load_config(repository.root())?;
    let base_revision = repository.head_revision().map_err(to_report)?;
    let task = TaskInput {
        text: read_task(task_argument, task_file)?,
    };
    if task.text.trim().is_empty() {
        return Err(miette!("the original task cannot be empty"));
    }
    let (intended_spec, model_calls) = match spec_file {
        Some(path) => (read_json_file(path)?, Vec::new()),
        None if config.runner.kind == RunnerKind::Mock => {
            (IntendedSpec::from_task(&task), Vec::new())
        }
        None => {
            let runner = api_runner(&config)?;
            let (spec, metadata) = generate_typed(
                &runner,
                RequestPurpose::AnalyzeForwardIntent,
                serde_json::to_value(&task).into_diagnostic()?,
            )
            .await?;
            (spec, vec![model_call("forward", metadata)])
        }
    };
    let now = unix_millis()?;
    let run = RunRecord {
        version: 1,
        id: generate_run_id(now, &base_revision, &task.text),
        repository_root: repository.root().display().to_string(),
        base_revision,
        task,
        intended_spec,
        model_calls,
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

async fn verify(
    run_id: Option<&str>,
    echoed_spec_path: Option<&Path>,
    context: Option<ContextPolicy>,
    dry_run: bool,
    json_output: bool,
) -> Result<()> {
    let repository = discover_current()?;
    let mut config = load_config(repository.root())?;
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
    if dry_run {
        return verification_dry_run(&config, &bundle, json_output);
    }
    let (echoed, backward_metadata) = reconstruct(&config, &bundle, echoed_spec_path).await?;
    let (mut verdict, reconciliation_metadata) =
        reconcile_semantically(&config, &run.intended_spec, &echoed, &bundle).await?;
    validate_evidence(&mut verdict, &bundle);
    let model_calls = backward_metadata
        .into_iter()
        .map(|metadata| model_call("backward", metadata))
        .chain(
            reconciliation_metadata
                .into_iter()
                .map(|metadata| model_call("reconciliation", metadata)),
        )
        .collect();
    let record = VerificationRecord {
        version: 1,
        run_id: run.id,
        bundle,
        echoed_spec: echoed,
        verdict,
        model_calls,
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

async fn echo(
    revision: Option<&str>,
    echoed_spec_path: Option<&Path>,
    context: Option<ContextPolicy>,
    json_output: bool,
) -> Result<()> {
    let repository = discover_current()?;
    let mut config = load_config(repository.root())?;
    if let Some(context) = context {
        config.verification.context = context;
    }
    let base = match revision {
        Some(revision) => repository.resolve_revision(revision).map_err(to_report)?,
        None => repository.head_revision().map_err(to_report)?,
    };
    let bundle = build_bundle(&repository, &config, &base)?;
    let (echoed, metadata) = reconstruct(&config, &bundle, echoed_spec_path).await?;
    if json_output {
        print_json(&json!({
            "bundle_manifest": bundle.manifest,
            "echoed_spec": echoed,
            "model_call": metadata.map(|value| model_call("backward", value)),
        }))?;
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
    let (root, config_status, runner) = match repository {
        Ok(repository) => match load_config(repository.root()) {
            Ok(config) => {
                let credential = if config.runner.kind == RunnerKind::Api {
                    Some(json!({
                        "environment": config.runner.api_key_env,
                        "available": std::env::var_os(&config.runner.api_key_env)
                            .is_some_and(|value| !value.is_empty()),
                    }))
                } else {
                    None
                };
                (
                    repository.root().display().to_string(),
                    "valid".to_owned(),
                    json!({
                        "kind": config.runner.kind.to_string(),
                        "protocol": format!("{:?}", config.runner.protocol).to_ascii_lowercase(),
                        "base_url": config.runner.base_url,
                        "model": config.runner.model,
                        "fallback_model": config.runner.fallback_model,
                        "credential": credential,
                    }),
                )
            }
            Err(error) => (
                repository.root().display().to_string(),
                format!("invalid: {error}"),
                json!({"kind": "unknown"}),
            ),
        },
        Err(error) => (
            format!("unavailable: {error}"),
            "not checked".to_owned(),
            json!({"kind": "not checked"}),
        ),
    };
    let credential_ready = runner["credential"]
        .as_object()
        .is_none_or(|credential| credential["available"].as_bool().unwrap_or(false));
    let ready = git_version != "unavailable" && config_status == "valid" && credential_ready;
    let result = json!({
        "git": git_version,
        "repository": root,
        "configuration": config_status,
        "runner": runner,
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

async fn reconstruct(
    config: &Config,
    bundle: &BlindBundle,
    echoed_spec_path: Option<&Path>,
) -> Result<(EchoedSpec, Option<RunnerMetadata>)> {
    if let Some(path) = echoed_spec_path {
        let response: EchoedSpec = read_json_file(path)?;
        let runner = MockRunner::with_response(&response).map_err(to_report)?;
        let (echoed, _) = generate_typed(
            &runner,
            RequestPurpose::ReconstructPatchIntent,
            blind_input(bundle)?,
        )
        .await?;
        return Ok((echoed, None));
    }
    if config.runner.kind == RunnerKind::Mock {
        return Ok((deterministic_echo(bundle), None));
    }
    let runner = api_runner(config)?;
    let (echoed, metadata) = generate_typed(
        &runner,
        RequestPurpose::ReconstructPatchIntent,
        blind_input(bundle)?,
    )
    .await?;
    Ok((echoed, Some(metadata)))
}

async fn reconcile_semantically(
    config: &Config,
    intended: &IntendedSpec,
    echoed: &EchoedSpec,
    bundle: &BlindBundle,
) -> Result<(Verdict, Option<RunnerMetadata>)> {
    if config.runner.kind == RunnerKind::Mock {
        return Ok((reconcile(intended, echoed), None));
    }
    let runner = api_runner(config)?;
    let input = json!({
        "intended_spec": intended,
        "echoed_spec": echoed,
        "available_evidence": bundle.patch.files,
    });
    let (verdict, metadata) =
        generate_typed(&runner, RequestPurpose::ReconcileIntent, input).await?;
    Ok((verdict, Some(metadata)))
}

fn blind_input(bundle: &BlindBundle) -> Result<Value> {
    serde_json::to_value(bundle)
        .into_diagnostic()
        .wrap_err("could not serialize the strict blind verifier bundle")
}

async fn generate_typed<T>(
    runner: &dyn AgentRunner,
    purpose: RequestPurpose,
    input: Value,
) -> Result<(T, RunnerMetadata)>
where
    T: DeserializeOwned + JsonSchema,
{
    let request = AgentRequest { purpose, input };
    let schema = strict_schema::<T>()?;
    let output = runner
        .generate_structured(&request, &schema)
        .await
        .map_err(to_report)?;
    let value = serde_json::from_value(output.value)
        .into_diagnostic()
        .wrap_err("runner response did not match the requested domain schema")?;
    Ok((value, output.metadata))
}

fn strict_schema<T: JsonSchema>() -> Result<Value> {
    let mut schema = serde_json::to_value(schema_for!(T)).into_diagnostic()?;
    make_objects_strict(&mut schema);
    Ok(schema)
}

fn make_objects_strict(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("$schema");
            if let Some(Value::Object(properties)) = object.get("properties") {
                let required = properties.keys().cloned().map(Value::String).collect();
                object.insert("required".to_owned(), Value::Array(required));
                object.insert("additionalProperties".to_owned(), Value::Bool(false));
            }
            for child in object.values_mut() {
                make_objects_strict(child);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(make_objects_strict),
        _ => {}
    }
}

fn api_runner(config: &Config) -> Result<OpenAiResponsesRunner> {
    let model = config
        .runner
        .model
        .clone()
        .ok_or_else(|| miette!("runner.model is required for the API runner"))?;
    OpenAiResponsesRunner::from_env(OpenAiResponsesConfig {
        base_url: config.runner.base_url.clone(),
        api_key_env: config.runner.api_key_env.clone(),
        model,
        reasoning_effort: config.runner.reasoning_effort.clone(),
        timeout: Duration::from_secs(config.runner.timeout_seconds),
    })
    .map_err(to_report)
}

fn model_call(stage: &str, metadata: RunnerMetadata) -> ModelCallRecord {
    ModelCallRecord {
        stage: stage.to_owned(),
        provider: metadata.provider,
        model: metadata.model,
        latency_ms: metadata.latency_ms,
        input_tokens: metadata.usage.input_tokens,
        cached_input_tokens: metadata.usage.cached_input_tokens,
        output_tokens: metadata.usage.output_tokens,
    }
}

fn verification_dry_run(config: &Config, bundle: &BlindBundle, json_output: bool) -> Result<()> {
    let value = json!({
        "dry_run": true,
        "request_sent": false,
        "runner": {
            "kind": config.runner.kind.to_string(),
            "provider": if config.runner.kind == RunnerKind::Api { "openai-compatible" } else { "mock" },
            "model": config.runner.model,
        },
        "context_policy": bundle.manifest.context_policy,
        "included": {
            "patch_files": bundle.manifest.patch_files,
            "context_files": bundle.manifest.context_files,
        },
        "excluded": bundle.manifest.excluded_paths,
        "blindness_report": bundle.blindness_report,
    });
    if json_output {
        print_json(&value)
    } else {
        report::dry_run(&value);
        Ok(())
    }
}

fn validate_evidence(verdict: &mut Verdict, bundle: &BlindBundle) {
    for evidence in &mut verdict.evidence {
        validate_evidence_location(evidence, bundle);
    }
    let findings = verdict
        .missing_requirements
        .iter()
        .chain(verdict.unrequested_changes.iter())
        .chain(verdict.violated_constraints.iter())
        .chain(verdict.potential_side_effects.iter());
    for finding in findings {
        if !verdict
            .evidence
            .iter()
            .any(|evidence| evidence.description.contains(finding))
        {
            verdict.evidence.push(Evidence {
                file: None,
                line_start: None,
                line_end: None,
                patch_hunk: None,
                description: finding.clone(),
                confidence: verdict.confidence,
            });
        }
    }
}

fn validate_evidence_location(evidence: &mut Evidence, bundle: &BlindBundle) {
    let Some(file) = evidence.file.as_deref() else {
        evidence.line_start = None;
        evidence.line_end = None;
        evidence.patch_hunk = None;
        return;
    };
    let Some(changed) = bundle
        .patch
        .files
        .iter()
        .find(|changed| changed.path == file)
    else {
        evidence.file = None;
        evidence.line_start = None;
        evidence.line_end = None;
        evidence.patch_hunk = None;
        return;
    };
    if evidence
        .patch_hunk
        .as_deref()
        .is_some_and(|hunk| !changed.patch.contains(hunk))
    {
        evidence.patch_hunk = None;
    }
    let valid_lines = evidence
        .patch_hunk
        .as_deref()
        .and_then(new_line_range)
        .is_some_and(|(first, last)| {
            matches!(
                (evidence.line_start, evidence.line_end),
                (Some(start), Some(end)) if start <= end && start >= first && end <= last
            )
        });
    if !valid_lines {
        evidence.line_start = None;
        evidence.line_end = None;
    }
}

fn new_line_range(hunk: &str) -> Option<(u32, u32)> {
    let header = hunk.lines().next()?;
    let new_range = header
        .split_whitespace()
        .find(|part| part.starts_with('+'))?;
    let (start, count) = new_range[1..]
        .split_once(',')
        .map_or((new_range.trim_start_matches('+'), "1"), |parts| parts);
    let start = start.parse::<u32>().ok()?;
    let count = count.parse::<u32>().ok()?;
    (count > 0).then(|| (start, start.saturating_add(count - 1)))
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
            "The offline mock baseline has no semantic verifier; file-level reconstruction is intentionally uncertain"
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
    use flect_core::{Alignment, RecommendedAction};

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

    #[tokio::test]
    async fn mock_runner_exercises_all_semantic_stages_without_leaking_forward_input() {
        let task = TaskInput {
            text: "SECRET_ORIGINAL_TASK_SENTINEL".to_owned(),
        };
        let intended = IntendedSpec {
            objective: "Add safe behavior".to_owned(),
            requirements: vec!["Validate input".to_owned()],
            ..IntendedSpec::default()
        };
        let bundle: BlindBundle = serde_json::from_value(json!({
            "patch": {
                "base_revision": "abc", "files": [{
                    "path": "src/lib.rs", "status": "modified",
                    "patch": "@@ -1 +1 @@\n-old\n+new", "insertions": 1,
                    "deletions": 1, "binary": false
                }], "renames": 0, "insertions": 1, "deletions": 1,
                "binary_files": [], "untracked_files": []
            },
            "context": [],
            "manifest": { "context_policy": "patch", "patch_files": ["src/lib.rs"], "context_files": [], "excluded_paths": [], "total_bytes": 20 },
            "blindness_report": { "isolation": "strict", "entries": [], "limitations": [] }
        }))
        .unwrap();
        let echoed = EchoedSpec {
            apparent_objective: "Add safe behavior".to_owned(),
            behavior_after: vec!["Validates input".to_owned()],
            affected_scope: vec!["src/lib.rs".to_owned()],
            confidence: 0.9,
            ..EchoedSpec::default()
        };
        let verdict = json!({
            "alignment": "SAME",
            "agreements": ["Validate input"],
            "missing_requirements": [],
            "unrequested_changes": [],
            "violated_constraints": [],
            "potential_side_effects": [],
            "uncertainties": [],
            "evidence": [{
                "file": null, "line_start": null, "line_end": null,
                "patch_hunk": null, "description": "Patch validates input",
                "confidence": 0.9
            }],
            "confidence": 0.9,
            "recommended_action": "SHIP"
        });
        let runner = MockRunner::new([
            serde_json::to_value(&intended).unwrap(),
            serde_json::to_value(&echoed).unwrap(),
            verdict,
        ]);

        let (actual_intended, _) = generate_typed::<IntendedSpec>(
            &runner,
            RequestPurpose::AnalyzeForwardIntent,
            serde_json::to_value(&task).unwrap(),
        )
        .await
        .unwrap();
        let serialized_blind_input = serde_json::to_string(&blind_input(&bundle).unwrap()).unwrap();
        assert!(!serialized_blind_input.contains("SECRET_ORIGINAL_TASK_SENTINEL"));
        for forbidden in [
            "original_task",
            "intended_spec",
            "conversation",
            "commit_message",
        ] {
            assert!(!serialized_blind_input.contains(forbidden));
        }
        let (actual_echoed, _) = generate_typed::<EchoedSpec>(
            &runner,
            RequestPurpose::ReconstructPatchIntent,
            blind_input(&bundle).unwrap(),
        )
        .await
        .unwrap();
        let (actual_verdict, _) = generate_typed::<Verdict>(
            &runner,
            RequestPurpose::ReconcileIntent,
            json!({"intended_spec": actual_intended, "echoed_spec": actual_echoed}),
        )
        .await
        .unwrap();
        assert_eq!(actual_verdict.alignment, Alignment::Same);
        assert_eq!(actual_verdict.recommended_action, RecommendedAction::Ship);
    }

    #[test]
    fn evidence_validation_removes_fabricated_locations() {
        let bundle: BlindBundle = serde_json::from_value(json!({
            "patch": {
                "base_revision": "abc", "files": [{
                    "path": "src/lib.rs", "status": "modified", "patch": "@@ -1 +1 @@\n-old\n+new",
                    "insertions": 1, "deletions": 1, "binary": false
                }], "renames": 0, "insertions": 1, "deletions": 1,
                "binary_files": [], "untracked_files": []
            }, "context": [],
            "manifest": { "context_policy": "patch", "patch_files": ["src/lib.rs"], "context_files": [], "excluded_paths": [], "total_bytes": 20 },
            "blindness_report": { "isolation": "strict", "entries": [], "limitations": [] }
        })).unwrap();
        let mut verdict = Verdict {
            alignment: Alignment::Partial,
            agreements: Vec::new(),
            missing_requirements: vec!["Missing validation".to_owned()],
            unrequested_changes: Vec::new(),
            violated_constraints: Vec::new(),
            potential_side_effects: Vec::new(),
            uncertainties: Vec::new(),
            evidence: vec![Evidence {
                file: Some("invented.rs".to_owned()),
                line_start: Some(999),
                line_end: Some(1000),
                patch_hunk: Some("fabricated".to_owned()),
                description: "Unrelated claim".to_owned(),
                confidence: 0.8,
            }],
            confidence: 0.8,
            recommended_action: RecommendedAction::RevisePatch,
        };
        validate_evidence(&mut verdict, &bundle);
        assert!(verdict.evidence[0].file.is_none());
        assert!(verdict.evidence[0].line_start.is_none());
        assert!(
            verdict
                .evidence
                .iter()
                .any(|evidence| evidence.description == "Missing validation")
        );
    }
}
