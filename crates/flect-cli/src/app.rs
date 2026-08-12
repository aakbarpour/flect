//! CLI orchestration. Domain policy remains in `flect-core`.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use flect_core::config::RunnerConfig;
use flect_core::{
    BlindBundle, BlindGuard, Config, ContextBuilder, ContextPolicy, EchoedSpec, Evidence,
    FileStatus, GitRepository, IntendedSpec, ModelCallRecord, RunRecord, RunStore, RunnerKind,
    TaskInput, Verdict, VerificationRecord, reconcile,
};
use flect_runner::{
    AgentRequest, AgentRunner, MockRunner, OpenAiResponsesConfig, OpenAiResponsesRunner,
    RequestPurpose, RunnerError, RunnerMetadata,
};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::report;
use crate::{Command, SkillCommand};

const PRICING_VERSION: &str = "openai-2026-08-12";
const PRICING_TABLE: &[ModelPrice] = &[
    ModelPrice {
        model: "gpt-5.6-luna",
        input_per_million: 1.0,
        cached_input_per_million: 0.1,
        output_per_million: 6.0,
    },
    ModelPrice {
        model: "gpt-5.6-terra",
        input_per_million: 2.5,
        cached_input_per_million: 0.25,
        output_per_million: 15.0,
    },
];

struct ModelPrice {
    model: &'static str,
    input_per_million: f64,
    cached_input_per_million: f64,
    output_per_million: f64,
}

struct RoutedOutput<T> {
    value: T,
    attempts: Vec<AttemptRecord>,
}

struct AttemptRecord {
    metadata: RunnerMetadata,
    accepted: bool,
    escalation_reason: Option<String>,
}

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
        Command::Mcp => crate::mcp::run(),
        Command::Skill { command } => skill(&command, json_output),
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
            let output = generate_api::<IntendedSpec>(
                &config,
                RequestPurpose::AnalyzeForwardIntent,
                serde_json::to_value(&task).into_diagnostic()?,
            )
            .await?;
            (
                output.value,
                model_calls("forward", output.attempts, &config),
            )
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
    let model_calls = model_calls("backward", backward_metadata, &config)
        .into_iter()
        .chain(model_calls(
            "reconciliation",
            reconciliation_metadata,
            &config,
        ))
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
            "model_calls": model_calls("backward", metadata, &config),
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

fn skill(command: &SkillCommand, json_output: bool) -> Result<()> {
    let repository = discover_current()?;
    crate::skill::run(repository.root(), command, json_output)
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
) -> Result<(EchoedSpec, Vec<AttemptRecord>)> {
    if let Some(path) = echoed_spec_path {
        let response: EchoedSpec = read_json_file(path)?;
        let runner = MockRunner::with_response(&response).map_err(to_report)?;
        let (echoed, _) = generate_typed(
            &runner,
            RequestPurpose::ReconstructPatchIntent,
            blind_input(bundle)?,
        )
        .await?;
        return Ok((echoed, Vec::new()));
    }
    if config.runner.kind == RunnerKind::Mock {
        return Ok((deterministic_echo(bundle), Vec::new()));
    }
    let output = generate_api::<EchoedSpec>(
        config,
        RequestPurpose::ReconstructPatchIntent,
        blind_input(bundle)?,
    )
    .await?;
    Ok((output.value, output.attempts))
}

async fn reconcile_semantically(
    config: &Config,
    intended: &IntendedSpec,
    echoed: &EchoedSpec,
    bundle: &BlindBundle,
) -> Result<(Verdict, Vec<AttemptRecord>)> {
    if config.runner.kind == RunnerKind::Mock {
        return Ok((reconcile(intended, echoed), Vec::new()));
    }
    let input = json!({
        "intended_spec": intended,
        "echoed_spec": echoed,
        "available_evidence": bundle.patch.files,
    });
    let output = generate_api::<Verdict>(config, RequestPurpose::ReconcileIntent, input).await?;
    Ok((output.value, output.attempts))
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

async fn generate_api<T>(
    config: &Config,
    purpose: RequestPurpose,
    input: Value,
) -> Result<RoutedOutput<T>>
where
    T: DeserializeOwned + JsonSchema,
{
    let primary_model = config
        .runner
        .model
        .as_deref()
        .ok_or_else(|| miette!("runner.model is required for the API runner"))?;
    let primary = api_runner(config, primary_model)?;
    let fallback_model = config
        .runner
        .fallback_model
        .as_deref()
        .filter(|model| *model != primary_model);
    let fallback = fallback_model
        .map(|model| api_runner(config, model))
        .transpose()?;
    let fallback_candidate = fallback
        .as_ref()
        .zip(fallback_model)
        .map(|(runner, model)| (runner as &dyn AgentRunner, model));
    generate_routed(
        &primary,
        primary_model,
        fallback_candidate,
        purpose,
        input,
        &config.runner,
    )
    .await
}

async fn generate_routed<T>(
    primary: &dyn AgentRunner,
    primary_model: &str,
    fallback: Option<(&dyn AgentRunner, &str)>,
    purpose: RequestPurpose,
    input: Value,
    config: &RunnerConfig,
) -> Result<RoutedOutput<T>>
where
    T: DeserializeOwned + JsonSchema,
{
    let request = AgentRequest { purpose, input };
    let schema = strict_schema::<T>()?;
    let complexity = complexity_signal(&request.input, config);
    let started = Instant::now();
    let primary_result = primary.generate_structured(&request, &schema).await;
    let (primary_output, primary_decode_error) = match primary_result {
        Ok(output) => {
            let decode_error = serde_json::from_value::<T>(output.value.clone())
                .err()
                .map(|error| format!("malformed domain output: {error}"));
            (output, decode_error)
        }
        Err(error) => {
            if config.escalate_on_uncertain && malformed_output_error(&error) {
                if let Some(fallback) = fallback {
                    let reason = format!("primary output failure: {error}");
                    let primary_attempt = AttemptRecord {
                        metadata: failed_metadata(primary_model, started),
                        accepted: false,
                        escalation_reason: Some(reason.clone()),
                    };
                    return fallback_attempt(
                        fallback,
                        &request,
                        &schema,
                        vec![primary_attempt],
                        reason,
                    )
                    .await;
                }
            }
            return Err(to_report(error));
        }
    };
    let signal = primary_decode_error
        .or(complexity)
        .or_else(|| output_signal(&primary_output.value, config.confidence_threshold));
    if config.escalate_on_uncertain {
        if let (Some(reason), Some(fallback)) = (signal.clone(), fallback) {
            let primary_attempt = AttemptRecord {
                metadata: primary_output.metadata,
                accepted: false,
                escalation_reason: Some(reason.clone()),
            };
            return fallback_attempt(fallback, &request, &schema, vec![primary_attempt], reason)
                .await;
        }
    }
    let value = serde_json::from_value(primary_output.value)
        .into_diagnostic()
        .wrap_err("runner response did not match the requested domain schema")?;
    Ok(RoutedOutput {
        value,
        attempts: vec![AttemptRecord {
            metadata: primary_output.metadata,
            accepted: true,
            escalation_reason: signal,
        }],
    })
}

async fn fallback_attempt<T>(
    fallback: (&dyn AgentRunner, &str),
    request: &AgentRequest,
    schema: &Value,
    mut attempts: Vec<AttemptRecord>,
    reason: String,
) -> Result<RoutedOutput<T>>
where
    T: DeserializeOwned,
{
    let started = Instant::now();
    let output = fallback
        .0
        .generate_structured(request, schema)
        .await
        .map_err(|error| {
            miette!(
                "fallback model `{}` failed after escalation ({reason}): {error}",
                fallback.1
            )
        })?;
    let value = serde_json::from_value(output.value)
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "fallback model `{}` returned malformed domain output after escalation ({reason})",
                fallback.1
            )
        })?;
    let mut metadata = output.metadata;
    if metadata.latency_ms == 0 {
        metadata.latency_ms = elapsed_millis(started);
    }
    attempts.push(AttemptRecord {
        metadata,
        accepted: true,
        escalation_reason: Some(reason),
    });
    Ok(RoutedOutput { value, attempts })
}

fn complexity_signal(input: &Value, config: &RunnerConfig) -> Option<String> {
    let file_count = input
        .pointer("/patch/files")
        .or_else(|| input.get("available_evidence"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if file_count >= config.complexity_file_threshold {
        return Some(format!(
            "complexity signal: {file_count} files meets the configured threshold of {}",
            config.complexity_file_threshold
        ));
    }
    let payload_bytes = serde_json::to_vec(input).map_or(u64::MAX, |bytes| bytes.len() as u64);
    (payload_bytes >= config.complexity_byte_threshold).then(|| {
        format!(
            "complexity signal: {payload_bytes} input bytes meets the configured threshold of {}",
            config.complexity_byte_threshold
        )
    })
}

fn output_signal(output: &Value, confidence_threshold: f64) -> Option<String> {
    if output
        .get("confidence")
        .and_then(Value::as_f64)
        .is_some_and(|confidence| confidence < confidence_threshold)
    {
        return Some(format!(
            "confidence is below the advisory threshold of {confidence_threshold:.2}"
        ));
    }
    if output.get("alignment").and_then(Value::as_str) == Some("UNCERTAIN") {
        return Some("primary result is UNCERTAIN".to_owned());
    }
    if output
        .get("uncertainties")
        .and_then(Value::as_array)
        .is_some_and(|values| !values.is_empty())
    {
        return Some("primary result contains explicit uncertainties".to_owned());
    }
    let has_negative_findings = [
        "missing_requirements",
        "unrequested_changes",
        "violated_constraints",
        "potential_side_effects",
    ]
    .iter()
    .any(|field| {
        output
            .get(*field)
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty())
    });
    if has_negative_findings
        && output
            .get("evidence")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        return Some("primary result has negative findings without structured evidence".to_owned());
    }
    None
}

fn malformed_output_error(error: &RunnerError) -> bool {
    matches!(
        error,
        RunnerError::InvalidJson(_)
            | RunnerError::SchemaValidation(_)
            | RunnerError::MissingOutput
            | RunnerError::Incomplete(_)
            | RunnerError::Refusal(_)
    )
}

fn failed_metadata(model: &str, started: Instant) -> RunnerMetadata {
    RunnerMetadata {
        provider: "openai-compatible".to_owned(),
        model: model.to_owned(),
        latency_ms: elapsed_millis(started),
        usage: flect_runner::TokenUsage::default(),
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn api_runner(config: &Config, model: &str) -> Result<OpenAiResponsesRunner> {
    OpenAiResponsesRunner::from_env(OpenAiResponsesConfig {
        base_url: config.runner.base_url.clone(),
        api_key_env: config.runner.api_key_env.clone(),
        model: model.to_owned(),
        reasoning_effort: config.runner.reasoning_effort.clone(),
        timeout: Duration::from_secs(config.runner.timeout_seconds),
    })
    .map_err(to_report)
}

fn model_calls(stage: &str, attempts: Vec<AttemptRecord>, config: &Config) -> Vec<ModelCallRecord> {
    attempts
        .into_iter()
        .enumerate()
        .map(|(index, attempt)| {
            let cost = estimate_cost(&attempt.metadata, &config.runner.base_url);
            ModelCallRecord {
                stage: stage.to_owned(),
                attempt: u32::try_from(index + 1).unwrap_or(u32::MAX),
                accepted: attempt.accepted,
                provider: attempt.metadata.provider,
                model: attempt.metadata.model,
                latency_ms: attempt.metadata.latency_ms,
                input_tokens: attempt.metadata.usage.input_tokens,
                cached_input_tokens: attempt.metadata.usage.cached_input_tokens,
                output_tokens: attempt.metadata.usage.output_tokens,
                estimated_cost_usd: cost,
                pricing_version: cost.map(|_| PRICING_VERSION.to_owned()),
                escalation_reason: attempt.escalation_reason,
            }
        })
        .collect()
}

fn estimate_cost(metadata: &RunnerMetadata, base_url: &str) -> Option<f64> {
    if !base_url
        .trim_end_matches('/')
        .eq_ignore_ascii_case("https://api.openai.com/v1")
    {
        return None;
    }
    let price = PRICING_TABLE
        .iter()
        .find(|price| price.model == metadata.model)?;
    let input = metadata.usage.input_tokens?;
    let cached = metadata.usage.cached_input_tokens.unwrap_or(0).min(input);
    let output = metadata.usage.output_tokens?;
    let uncached = input.saturating_sub(cached);
    let long_context = input > 272_000;
    let uncached = f64::from(u32::try_from(uncached).ok()?);
    let cached = f64::from(u32::try_from(cached).ok()?);
    let output = f64::from(u32::try_from(output).ok()?);
    Some(
        ((uncached * price.input_per_million * if long_context { 2.0 } else { 1.0 })
            + (cached * price.cached_input_per_million * if long_context { 2.0 } else { 1.0 })
            + (output * price.output_per_million * if long_context { 1.5 } else { 1.0 }))
            / 1_000_000.0,
    )
}

fn verification_dry_run(config: &Config, bundle: &BlindBundle, json_output: bool) -> Result<()> {
    let value = json!({
        "dry_run": true,
        "request_sent": false,
        "runner": {
            "kind": config.runner.kind.to_string(),
            "provider": if config.runner.kind == RunnerKind::Api { "openai-compatible" } else { "mock" },
            "model": config.runner.model,
            "fallback_model": config.runner.fallback_model,
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

    fn echoed(confidence: f64, uncertainties: Vec<String>) -> EchoedSpec {
        EchoedSpec {
            apparent_objective: "Validate input".to_owned(),
            behavior_after: vec!["Input is validated".to_owned()],
            uncertainties,
            confidence,
            ..EchoedSpec::default()
        }
    }

    #[tokio::test]
    async fn confident_luna_result_does_not_invoke_terra() {
        let primary = MockRunner::named(
            "gpt-5.6-luna",
            [serde_json::to_value(echoed(0.9, Vec::new())).unwrap()],
        );
        let fallback = MockRunner::named(
            "gpt-5.6-terra",
            [serde_json::to_value(echoed(0.95, Vec::new())).unwrap()],
        );
        let result = generate_routed::<EchoedSpec>(
            &primary,
            "gpt-5.6-luna",
            Some((&fallback, "gpt-5.6-terra")),
            RequestPurpose::ReconstructPatchIntent,
            json!({"patch": {"files": []}}),
            &RunnerConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.attempts.len(), 1);
        assert_eq!(result.attempts[0].metadata.model, "gpt-5.6-luna");
        assert!(result.attempts[0].accepted);
    }

    #[tokio::test]
    async fn low_confidence_escalates_from_luna_to_terra_once() {
        let primary = MockRunner::named(
            "gpt-5.6-luna",
            [serde_json::to_value(echoed(0.4, Vec::new())).unwrap()],
        );
        let fallback = MockRunner::named(
            "gpt-5.6-terra",
            [serde_json::to_value(echoed(0.9, Vec::new())).unwrap()],
        );
        let result = generate_routed::<EchoedSpec>(
            &primary,
            "gpt-5.6-luna",
            Some((&fallback, "gpt-5.6-terra")),
            RequestPurpose::ReconstructPatchIntent,
            json!({"patch": {"files": []}}),
            &RunnerConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.attempts.len(), 2);
        assert!(!result.attempts[0].accepted);
        assert!(result.attempts[1].accepted);
        assert_eq!(result.attempts[1].metadata.model, "gpt-5.6-terra");
        assert!(
            result.attempts[1]
                .escalation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("confidence"))
        );
        let records = model_calls("backward", result.attempts, &Config::default());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].attempt, 1);
        assert!(!records[0].accepted);
        assert_eq!(records[1].attempt, 2);
        assert!(records[1].accepted);
    }

    #[tokio::test]
    async fn malformed_primary_output_escalates_once() {
        let primary = MockRunner::named("gpt-5.6-luna", [json!({"wrong": true})]);
        let fallback = MockRunner::named(
            "gpt-5.6-terra",
            [serde_json::to_value(echoed(0.9, Vec::new())).unwrap()],
        );
        let result = generate_routed::<EchoedSpec>(
            &primary,
            "gpt-5.6-luna",
            Some((&fallback, "gpt-5.6-terra")),
            RequestPurpose::ReconstructPatchIntent,
            json!({"patch": {"files": []}}),
            &RunnerConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.attempts.len(), 2);
        assert!(result.attempts[1].accepted);
    }

    #[tokio::test]
    async fn configured_complexity_signal_escalates_once() {
        let primary = MockRunner::named(
            "gpt-5.6-luna",
            [serde_json::to_value(echoed(0.9, Vec::new())).unwrap()],
        );
        let fallback = MockRunner::named(
            "gpt-5.6-terra",
            [serde_json::to_value(echoed(0.9, Vec::new())).unwrap()],
        );
        let config = RunnerConfig {
            complexity_file_threshold: 1,
            ..RunnerConfig::default()
        };
        let result = generate_routed::<EchoedSpec>(
            &primary,
            "gpt-5.6-luna",
            Some((&fallback, "gpt-5.6-terra")),
            RequestPurpose::ReconstructPatchIntent,
            json!({"patch": {"files": [{"path": "src/lib.rs"}]}}),
            &config,
        )
        .await
        .unwrap();
        assert_eq!(result.attempts.len(), 2);
        assert!(
            result.attempts[0]
                .escalation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("complexity signal"))
        );
    }

    #[test]
    fn estimates_only_versioned_known_openai_pricing() {
        let metadata = RunnerMetadata {
            provider: "openai-compatible".to_owned(),
            model: "gpt-5.6-luna".to_owned(),
            latency_ms: 1,
            usage: flect_runner::TokenUsage {
                input_tokens: Some(100_000),
                cached_input_tokens: Some(50_000),
                output_tokens: Some(10_000),
            },
        };
        assert_eq!(
            estimate_cost(&metadata, "https://api.openai.com/v1"),
            Some(0.115)
        );
        assert_eq!(estimate_cost(&metadata, "https://example.com/v1"), None);
        let unknown = RunnerMetadata {
            model: "custom-model".to_owned(),
            ..metadata
        };
        assert_eq!(estimate_cost(&unknown, "https://api.openai.com/v1"), None);
    }
}
