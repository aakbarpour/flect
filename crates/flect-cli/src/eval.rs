//! Reproducible offline and explicitly opt-in model evaluation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use flect_core::{
    Alignment, BlindBundle, BlindnessReport, BundleManifest, ContextFile, ContextPolicy,
    EchoedSpec, FindingCategory, IntendedSpec, IsolationEntry, IsolationKind, JudgeFinding,
    JudgeVerdict, PatchSet, TaskInput, Verdict,
};
use flect_runner::{
    AgentRequest, AgentRunner, MockRunner, OpenAiResponsesConfig, OpenAiResponsesRunner,
    RequestPurpose, RunnerError, RunnerMetadata, RunnerOutput, estimate_openai_cost,
};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    version: u32,
    cases: Vec<EvalCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalCase {
    id: String,
    name: String,
    class: String,
    subset: String,
    base_files: Vec<ContextFile>,
    base_state: String,
    original_task: String,
    candidate_patch: PatchSet,
    change: String,
    intended_spec: IntendedSpec,
    mock_echoed_spec: EchoedSpec,
    mock_verdict: Verdict,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    verdict: Alignment,
    important_findings: Vec<String>,
    finding_categories: BTreeSet<String>,
    rationale: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiProfiles {
    version: u32,
    base_url: String,
    api_key_env: String,
    reasoning_effort: String,
    timeout_seconds: u64,
    profiles: Vec<ApiProfile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ApiProfile {
    name: String,
    model: String,
    fallback_model: Option<String>,
    escalate: bool,
    confidence_threshold: f64,
}

#[derive(Debug, Serialize)]
struct EvalReport {
    version: u32,
    suite: String,
    suite_hash: Option<String>,
    git_revision: Option<String>,
    generated_unix_ms: Option<u128>,
    mode: String,
    profiles: Vec<ProfileReport>,
    limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProfileReport {
    profile: ProfileSummary,
    metrics: Metrics,
    cases: Vec<CaseResult>,
}

#[derive(Debug, Serialize)]
struct ProfileSummary {
    name: String,
    provider: String,
    model: String,
    fallback_model: Option<String>,
    escalation_enabled: bool,
    confidence_threshold: Option<f64>,
}

#[derive(Debug, Serialize)]
struct Metrics {
    cases: usize,
    cases_attempted: usize,
    cases_persisted: usize,
    cases_failed: usize,
    failure_counts: BTreeMap<String, usize>,
    exact_verdicts: usize,
    verdict_accuracy: Rate,
    per_class_accuracy: BTreeMap<String, Rate>,
    confusion_matrix: BTreeMap<String, BTreeMap<String, usize>>,
    verifier_schema_compliance: Rate,
    judge_schema_compliance: Rate,
    correct_patch_acceptance: Rate,
    good_patch_abstentions: usize,
    bad_patch_detection: Rate,
    false_positives: usize,
    false_negatives: usize,
    uncertainty: Rate,
    important_findings: Rate,
    finding_category_exact_match: Rate,
    finding_category_recall: Rate,
    evidence_ref_validation_failures: usize,
    requests: usize,
    latency_ms: u64,
    average_latency_ms: Option<u64>,
    median_latency_ms: Option<u64>,
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
struct Rate {
    numerator: usize,
    denominator: usize,
    fraction: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CaseResult {
    id: String,
    name: String,
    class: String,
    expected: Alignment,
    actual: Option<Alignment>,
    verdict_match: Option<bool>,
    verifier_stage: StageOutcome,
    judge_stage: StageOutcome,
    evidence_validation_status: StageStatus,
    failure_category: Option<FailureCategory>,
    execution_mode: String,
    expected_findings: usize,
    matched_findings: usize,
    expected_finding_categories: BTreeSet<String>,
    actual_finding_categories: Option<BTreeSet<String>>,
    finding_category_match: Option<bool>,
    evidence_ref_validation_failures: usize,
    models: Vec<String>,
    requests: usize,
    latency_ms: u64,
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    estimated_cost_usd: Option<f64>,
}

struct EvaluationOutput {
    verdict: Option<Verdict>,
    calls: Vec<RunnerMetadata>,
    verifier_stage: StageOutcome,
    judge_stage: StageOutcome,
    evidence_validation_status: StageStatus,
    failure_category: Option<FailureCategory>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StageStatus {
    #[default]
    NotAttempted,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AttemptStatus {
    Succeeded,
    SchemaFailed,
    ProviderFailed,
}

#[derive(Debug, Clone, Default, Serialize)]
struct StageOutcome {
    attempts: Vec<AttemptStatus>,
    final_status: StageStatus,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
enum FailureCategory {
    ForwardSchemaFailure,
    VerifierSchemaFailure,
    JudgeSchemaFailure,
    EvidenceValidationFailure,
    OrchestrationFailure,
    ProviderRuntimeFailure,
}

enum StageFailure {
    Schema,
    Provider,
}

#[derive(Clone, Copy)]
struct StageOptions<'a> {
    fallback: Option<&'a dyn AgentRunner>,
    escalate: bool,
    strict_output: bool,
}

pub async fn run(
    suite_path: &Path,
    profiles_path: Option<&Path>,
    allow_paid_api: bool,
    output_path: Option<&Path>,
    json_output: bool,
) -> Result<()> {
    let suite: Suite = read_json(suite_path)?;
    validate_suite(&suite)?;
    let (mode, reports) = if let Some(profiles_path) = profiles_path {
        if !allow_paid_api {
            return Err(miette!(
                "API evaluation requires the explicit `--allow-paid-api` flag"
            ));
        }
        let profiles: ApiProfiles = read_toml(profiles_path)?;
        validate_profiles(&profiles)?;
        let mut reports = Vec::new();
        for profile in &profiles.profiles {
            reports.push(run_api_profile(&suite, &profiles, profile).await?);
        }
        ("api".to_owned(), reports)
    } else {
        if allow_paid_api {
            return Err(miette!("`--allow-paid-api` requires `--profiles`"));
        }
        ("offline".to_owned(), vec![run_offline(&suite).await?])
    };
    let report = EvalReport {
        version: 1,
        suite: suite_path.display().to_string(),
        suite_hash: command_output("git", &["hash-object", &suite_path.display().to_string()]),
        git_revision: command_output("git", &["rev-parse", "HEAD"]),
        generated_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|duration| duration.as_millis()),
        mode,
        profiles: reports,
        limitations: vec![
            "This small curated suite supports directional comparisons, not population-level or per-class precision/recall claims.".to_owned(),
            "Model-reported confidence is treated as a routing heuristic, not a calibrated probability.".to_owned(),
            "Estimated cost is emitted only for known OpenAI model pricing in the versioned Flect table.".to_owned(),
        ],
    };
    let bytes = serde_json::to_vec_pretty(&report).into_diagnostic()?;
    if let Some(path) = output_path {
        fs::write(path, &bytes)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not write evaluation report {}", path.display()))?;
    }
    if json_output {
        io::stdout().write_all(&bytes).into_diagnostic()?;
        println!();
    } else {
        print_summary(&report);
        if let Some(path) = output_path {
            println!("\nReport  {}", path.display());
        }
    }
    Ok(())
}

async fn run_offline(suite: &Suite) -> Result<ProfileReport> {
    let mut cases = Vec::new();
    for case in &suite.cases {
        let runner = MockRunner::named(
            "fixture-mock",
            [
                serde_json::to_value(&case.intended_spec).into_diagnostic()?,
                serde_json::to_value(&case.mock_echoed_spec).into_diagnostic()?,
                serde_json::to_value(mock_judge_verdict(&case.mock_verdict)).into_diagnostic()?,
            ],
        );
        let output = evaluate_case(case, &runner, None, false, 0.0, false).await;
        cases.push(case_result(case, &output, None));
    }
    Ok(profile_report(
        ProfileSummary {
            name: "offline-fixtures".to_owned(),
            provider: "mock".to_owned(),
            model: "fixture-mock".to_owned(),
            fallback_model: None,
            escalation_enabled: false,
            confidence_threshold: None,
        },
        cases,
    ))
}

async fn run_api_profile(
    suite: &Suite,
    profiles: &ApiProfiles,
    profile: &ApiProfile,
) -> Result<ProfileReport> {
    let primary = api_runner(profiles, &profile.model)?;
    let fallback = profile
        .fallback_model
        .as_deref()
        .map(|model| api_runner(profiles, model))
        .transpose()?;
    let mut cases = Vec::new();
    for case in &suite.cases {
        let output = evaluate_case(
            case,
            &primary,
            fallback.as_ref().map(|runner| runner as &dyn AgentRunner),
            profile.escalate,
            profile.confidence_threshold,
            true,
        )
        .await;
        cases.push(case_result(case, &output, Some(&profiles.base_url)));
    }
    Ok(profile_report(
        ProfileSummary {
            name: profile.name.clone(),
            provider: profiles.base_url.clone(),
            model: profile.model.clone(),
            fallback_model: profile.fallback_model.clone(),
            escalation_enabled: profile.escalate,
            confidence_threshold: Some(profile.confidence_threshold),
        },
        cases,
    ))
}

#[allow(clippy::too_many_lines)]
async fn evaluate_case(
    case: &EvalCase,
    primary: &dyn AgentRunner,
    fallback: Option<&dyn AgentRunner>,
    escalate: bool,
    threshold: f64,
    strict_output: bool,
) -> EvaluationOutput {
    let mut calls = Vec::new();
    let mut verifier_stage = StageOutcome::default();
    let mut judge_stage = StageOutcome::default();
    let Ok(bundle) = bundle(case) else {
        return failed_output(
            calls,
            verifier_stage,
            judge_stage,
            StageStatus::NotAttempted,
            FailureCategory::OrchestrationFailure,
        );
    };
    let intended: IntendedSpec = match stage(
        primary,
        RequestPurpose::AnalyzeForwardIntent,
        match serde_json::to_value(TaskInput {
            text: case.original_task.clone(),
        }) {
            Ok(value) => value,
            Err(_) => {
                return failed_output(
                    calls,
                    verifier_stage,
                    judge_stage,
                    StageStatus::NotAttempted,
                    FailureCategory::OrchestrationFailure,
                );
            }
        },
        &mut calls,
        strict_output,
    )
    .await
    {
        Ok(value) => value,
        Err(StageFailure::Schema) => {
            return failed_output(
                calls,
                verifier_stage,
                judge_stage,
                StageStatus::NotAttempted,
                FailureCategory::ForwardSchemaFailure,
            );
        }
        Err(StageFailure::Provider) => {
            return failed_output(
                calls,
                verifier_stage,
                judge_stage,
                StageStatus::NotAttempted,
                FailureCategory::ProviderRuntimeFailure,
            );
        }
    };
    let echoed: EchoedSpec = match stage_with_fallback(
        primary,
        RequestPurpose::ReconstructPatchIntent,
        match serde_json::to_value(&bundle) {
            Ok(value) => value,
            Err(_) => {
                return failed_output(
                    calls,
                    verifier_stage,
                    judge_stage,
                    StageStatus::NotAttempted,
                    FailureCategory::OrchestrationFailure,
                );
            }
        },
        &mut calls,
        &mut verifier_stage,
        StageOptions {
            fallback,
            escalate,
            strict_output,
        },
        |value: &EchoedSpec| value.confidence < threshold || !value.uncertainties.is_empty(),
    )
    .await
    {
        Ok(value) => value,
        Err(StageFailure::Schema) => {
            return failed_output(
                calls,
                verifier_stage,
                judge_stage,
                StageStatus::NotAttempted,
                FailureCategory::VerifierSchemaFailure,
            );
        }
        Err(StageFailure::Provider) => {
            return failed_output(
                calls,
                verifier_stage,
                judge_stage,
                StageStatus::NotAttempted,
                FailureCategory::ProviderRuntimeFailure,
            );
        }
    };
    let input = json!({
        "intended_spec": intended,
        "echoed_spec": echoed,
        "available_evidence": bundle.patch.files,
    });
    let judge: JudgeVerdict = match stage_with_fallback(
        primary,
        RequestPurpose::ReconcileIntent,
        input,
        &mut calls,
        &mut judge_stage,
        StageOptions {
            fallback,
            escalate,
            strict_output,
        },
        |value: &JudgeVerdict| {
            value.alignment == Alignment::Uncertain || value.confidence < threshold
        },
    )
    .await
    {
        Ok(value) => value,
        Err(StageFailure::Schema) => {
            return failed_output(
                calls,
                verifier_stage,
                judge_stage,
                StageStatus::NotAttempted,
                FailureCategory::JudgeSchemaFailure,
            );
        }
        Err(StageFailure::Provider) => {
            return failed_output(
                calls,
                verifier_stage,
                judge_stage,
                StageStatus::NotAttempted,
                FailureCategory::ProviderRuntimeFailure,
            );
        }
    };
    match flect_app::materialize_judge_verdict(judge, &bundle) {
        Ok(verdict) => EvaluationOutput {
            verdict: Some(verdict),
            calls,
            verifier_stage,
            judge_stage,
            evidence_validation_status: StageStatus::Succeeded,
            failure_category: None,
        },
        Err(_) => failed_output(
            calls,
            verifier_stage,
            judge_stage,
            StageStatus::Failed,
            FailureCategory::EvidenceValidationFailure,
        ),
    }
}

fn failed_output(
    calls: Vec<RunnerMetadata>,
    verifier_stage: StageOutcome,
    judge_stage: StageOutcome,
    evidence_validation_status: StageStatus,
    failure_category: FailureCategory,
) -> EvaluationOutput {
    EvaluationOutput {
        verdict: None,
        calls,
        verifier_stage,
        judge_stage,
        evidence_validation_status,
        failure_category: Some(failure_category),
    }
}

async fn stage<T: DeserializeOwned + JsonSchema>(
    runner: &dyn AgentRunner,
    purpose: RequestPurpose,
    input: Value,
    calls: &mut Vec<RunnerMetadata>,
    strict_output: bool,
) -> std::result::Result<T, StageFailure> {
    let schema = output_schema::<T>(strict_output).map_err(|_| StageFailure::Schema)?;
    let output = runner
        .generate_structured(&AgentRequest { purpose, input }, &schema)
        .await
        .map_err(|error| classify_runner_error(&error))?;
    decode(output, calls)
}

async fn stage_with_fallback<T, F>(
    primary: &dyn AgentRunner,
    purpose: RequestPurpose,
    input: Value,
    calls: &mut Vec<RunnerMetadata>,
    outcome: &mut StageOutcome,
    options: StageOptions<'_>,
    should_escalate: F,
) -> std::result::Result<T, StageFailure>
where
    T: DeserializeOwned + JsonSchema,
    F: Fn(&T) -> bool,
{
    let schema = output_schema::<T>(options.strict_output).map_err(|_| StageFailure::Schema)?;
    let request = AgentRequest { purpose, input };
    let primary_output = primary.generate_structured(&request, &schema).await;
    match primary_output {
        Ok(output) => {
            let value = match decode(output, calls) {
                Ok(value) => {
                    outcome.attempts.push(AttemptStatus::Succeeded);
                    value
                }
                Err(error) => {
                    outcome.attempts.push(AttemptStatus::SchemaFailed);
                    if options.escalate {
                        if let Some(fallback) = options.fallback {
                            return stage_fallback_attempt(
                                fallback, &request, &schema, calls, outcome,
                            )
                            .await;
                        }
                    }
                    outcome.final_status = StageStatus::Failed;
                    return Err(error);
                }
            };
            if options.escalate && should_escalate(&value) {
                if let Some(fallback) = options.fallback {
                    return stage_fallback_attempt(fallback, &request, &schema, calls, outcome)
                        .await;
                }
            }
            outcome.final_status = StageStatus::Succeeded;
            Ok(value)
        }
        Err(error) if options.escalate && options.fallback.is_some() => {
            outcome
                .attempts
                .push(attempt_status(&classify_runner_error(&error)));
            stage_fallback_attempt(
                options.fallback.expect("checked above"),
                &request,
                &schema,
                calls,
                outcome,
            )
            .await
        }
        Err(error) => {
            let error = classify_runner_error(&error);
            outcome.attempts.push(attempt_status(&error));
            outcome.final_status = StageStatus::Failed;
            Err(error)
        }
    }
}

async fn stage_fallback_attempt<T: DeserializeOwned>(
    fallback: &dyn AgentRunner,
    request: &AgentRequest,
    schema: &Value,
    calls: &mut Vec<RunnerMetadata>,
    outcome: &mut StageOutcome,
) -> std::result::Result<T, StageFailure> {
    let result = match fallback.generate_structured(request, schema).await {
        Ok(output) => decode(output, calls),
        Err(error) => Err(classify_runner_error(&error)),
    };
    match result {
        Ok(value) => {
            outcome.attempts.push(AttemptStatus::Succeeded);
            outcome.final_status = StageStatus::Succeeded;
            Ok(value)
        }
        Err(error) => {
            outcome.attempts.push(attempt_status(&error));
            outcome.final_status = StageStatus::Failed;
            Err(error)
        }
    }
}

fn attempt_status(error: &StageFailure) -> AttemptStatus {
    match error {
        StageFailure::Schema => AttemptStatus::SchemaFailed,
        StageFailure::Provider => AttemptStatus::ProviderFailed,
    }
}

fn decode<T: DeserializeOwned>(
    output: RunnerOutput,
    calls: &mut Vec<RunnerMetadata>,
) -> std::result::Result<T, StageFailure> {
    calls.push(output.metadata);
    serde_json::from_value(output.value).map_err(|_| StageFailure::Schema)
}

fn classify_runner_error(error: &RunnerError) -> StageFailure {
    match error {
        RunnerError::InvalidJson(_)
        | RunnerError::SchemaValidation(_)
        | RunnerError::MissingOutput => StageFailure::Schema,
        _ => StageFailure::Provider,
    }
}

fn bundle(case: &EvalCase) -> Result<BlindBundle> {
    let patch_files = case
        .candidate_patch
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let context_files = case
        .base_files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let total_bytes = case
        .candidate_patch
        .files
        .iter()
        .map(|file| file.patch.len())
        .chain(case.base_files.iter().map(|file| file.content.len()))
        .try_fold(0_u64, |total, bytes| {
            u64::try_from(bytes)
                .map(|bytes| total.saturating_add(bytes))
                .into_diagnostic()
        })?;
    Ok(BlindBundle {
        patch: case.candidate_patch.clone(),
        context: case.base_files.clone(),
        manifest: BundleManifest {
            context_policy: ContextPolicy::Focused,
            patch_files,
            context_files,
            excluded_paths: Vec::new(),
            total_bytes,
        },
        blindness_report: BlindnessReport {
            isolation: "strict".to_owned(),
            entries: vec![IsolationEntry {
                source: "original task".to_owned(),
                status: "not present in evaluation bundle".to_owned(),
                assurance: IsolationKind::StructurallyExcluded,
            }],
            limitations: vec!["Fixture source text may itself reveal task semantics.".to_owned()],
        },
    })
}

fn case_result(
    case: &EvalCase,
    output: &EvaluationOutput,
    pricing_base_url: Option<&str>,
) -> CaseResult {
    let finding_text = output
        .verdict
        .as_ref()
        .map(finding_text)
        .unwrap_or_default();
    let matched_findings = case
        .expected
        .important_findings
        .iter()
        .filter(|finding| finding_text.contains(&finding.to_ascii_lowercase()))
        .count();
    let usage = aggregate_calls(&output.calls, pricing_base_url);
    CaseResult {
        id: case.id.clone(),
        name: case.name.clone(),
        class: case.class.clone(),
        expected: case.expected.verdict,
        actual: output.verdict.as_ref().map(|verdict| verdict.alignment),
        verdict_match: output
            .verdict
            .as_ref()
            .map(|verdict| verdict.alignment == case.expected.verdict),
        verifier_stage: output.verifier_stage.clone(),
        judge_stage: output.judge_stage.clone(),
        evidence_validation_status: output.evidence_validation_status,
        failure_category: output.failure_category,
        execution_mode: if pricing_base_url.is_some() {
            "http_responses_api"
        } else {
            "deterministic_fixture"
        }
        .to_owned(),
        expected_findings: case.expected.important_findings.len(),
        matched_findings,
        expected_finding_categories: case.expected.finding_categories.clone(),
        actual_finding_categories: output.verdict.as_ref().map(verdict_categories),
        finding_category_match: output
            .verdict
            .as_ref()
            .map(|verdict| verdict_categories(verdict) == case.expected.finding_categories),
        evidence_ref_validation_failures: output.verdict.as_ref().map_or(
            usize::from(output.evidence_validation_status == StageStatus::Failed),
            |verdict| {
                verdict
                    .evidence
                    .iter()
                    .filter(|evidence| {
                        evidence.file.as_ref().is_some_and(|evidence_path| {
                            !case
                                .candidate_patch
                                .files
                                .iter()
                                .any(|file| &file.path == evidence_path)
                        })
                    })
                    .count()
            },
        ),
        models: output.calls.iter().map(|call| call.model.clone()).collect(),
        requests: output.calls.len(),
        latency_ms: usage.latency_ms,
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        estimated_cost_usd: usage.estimated_cost_usd,
    }
}

#[allow(clippy::too_many_lines)]
fn profile_report(profile: ProfileSummary, cases: Vec<CaseResult>) -> ProfileReport {
    let persisted = cases.iter().filter(|case| case.actual.is_some()).count();
    let correct = cases
        .iter()
        .filter(|case| case.expected == Alignment::Same)
        .count();
    let accepted = cases
        .iter()
        .filter(|case| case.expected == Alignment::Same && case.actual == Some(Alignment::Same))
        .count();
    let bad = cases.len() - correct;
    let detected = cases
        .iter()
        .filter(|case| {
            case.expected != Alignment::Same
                && matches!(case.actual, Some(Alignment::Partial | Alignment::Different))
        })
        .count();
    let uncertain = cases
        .iter()
        .filter(|case| case.actual == Some(Alignment::Uncertain))
        .count();
    let expected_findings = cases.iter().map(|case| case.expected_findings).sum();
    let matched_findings = cases.iter().map(|case| case.matched_findings).sum();
    let calls = cases.iter().map(|case| case.requests).sum();
    let latency = cases.iter().map(|case| case.latency_ms).sum();
    let exact = cases
        .iter()
        .filter(|case| case.verdict_match == Some(true))
        .count();
    let mut per_class_counts = BTreeMap::<String, (usize, usize)>::new();
    let mut confusion_matrix = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for label in ["SAME", "PARTIAL", "DIFFERENT", "UNCERTAIN"] {
        confusion_matrix.insert(
            label.to_owned(),
            BTreeMap::from([
                ("SAME".to_owned(), 0),
                ("PARTIAL".to_owned(), 0),
                ("DIFFERENT".to_owned(), 0),
                ("UNCERTAIN".to_owned(), 0),
            ]),
        );
    }
    for case in &cases {
        let count = per_class_counts.entry(case.class.clone()).or_default();
        count.1 += 1;
        count.0 += usize::from(case.verdict_match == Some(true));
        let Some(actual) = case.actual else { continue };
        *confusion_matrix
            .get_mut(alignment_name(case.expected))
            .expect("known alignment")
            .get_mut(alignment_name(actual))
            .expect("known alignment") += 1;
    }
    let per_class_accuracy = per_class_counts
        .into_iter()
        .map(|(class, (matches, total))| (class, rate(matches, total)))
        .collect();
    let category_cases = cases
        .iter()
        .filter(|case| !case.expected_finding_categories.is_empty())
        .count();
    let category_matches = cases
        .iter()
        .filter(|case| case.finding_category_match == Some(true))
        .count();
    let expected_categories = cases
        .iter()
        .map(|case| case.expected_finding_categories.len())
        .sum();
    let matched_categories = cases
        .iter()
        .map(|case| {
            case.actual_finding_categories.as_ref().map_or(0, |actual| {
                case.expected_finding_categories
                    .intersection(actual)
                    .count()
            })
        })
        .sum();
    let evidence_failures = cases
        .iter()
        .map(|case| case.evidence_ref_validation_failures)
        .sum();
    let mut latencies = cases.iter().map(|case| case.latency_ms).collect::<Vec<_>>();
    latencies.sort_unstable();
    let average_latency_ms = u64::try_from(latencies.len())
        .ok()
        .filter(|count| *count > 0)
        .map(|count| latency / count);
    let median_latency_ms = median(&latencies);
    let metrics = Metrics {
        cases: cases.len(),
        cases_attempted: cases.len(),
        cases_persisted: persisted,
        cases_failed: cases.len() - persisted,
        failure_counts: failure_counts(&cases),
        exact_verdicts: exact,
        verdict_accuracy: rate(exact, persisted),
        per_class_accuracy,
        confusion_matrix,
        verifier_schema_compliance: stage_rate(&cases, |case| &case.verifier_stage),
        judge_schema_compliance: stage_rate(&cases, |case| &case.judge_stage),
        correct_patch_acceptance: rate(accepted, correct),
        good_patch_abstentions: cases
            .iter()
            .filter(|case| {
                case.expected == Alignment::Same && case.actual == Some(Alignment::Uncertain)
            })
            .count(),
        bad_patch_detection: rate(detected, bad),
        false_positives: cases
            .iter()
            .filter(|case| {
                case.expected == Alignment::Same
                    && matches!(case.actual, Some(Alignment::Partial | Alignment::Different))
            })
            .count(),
        false_negatives: cases
            .iter()
            .filter(|case| case.expected != Alignment::Same && case.actual == Some(Alignment::Same))
            .count(),
        uncertainty: rate(uncertain, cases.len()),
        important_findings: rate(matched_findings, expected_findings),
        finding_category_exact_match: rate(category_matches, category_cases),
        finding_category_recall: rate(matched_categories, expected_categories),
        evidence_ref_validation_failures: evidence_failures,
        requests: calls,
        latency_ms: latency,
        average_latency_ms,
        median_latency_ms,
        input_tokens: sum_optional(cases.iter().map(|case| case.input_tokens)),
        cached_input_tokens: sum_optional(cases.iter().map(|case| case.cached_input_tokens)),
        output_tokens: sum_optional(cases.iter().map(|case| case.output_tokens)),
        estimated_cost_usd: sum_optional_f64(cases.iter().map(|case| case.estimated_cost_usd)),
    };
    ProfileReport {
        profile,
        metrics,
        cases,
    }
}

fn alignment_name(alignment: Alignment) -> &'static str {
    match alignment {
        Alignment::Same => "SAME",
        Alignment::Partial => "PARTIAL",
        Alignment::Different => "DIFFERENT",
        Alignment::Uncertain => "UNCERTAIN",
    }
}

fn median(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        values[middle - 1].midpoint(values[middle])
    } else {
        values[middle]
    })
}

struct CallAggregate {
    latency_ms: u64,
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    estimated_cost_usd: Option<f64>,
}

fn aggregate_calls(calls: &[RunnerMetadata], pricing_base_url: Option<&str>) -> CallAggregate {
    CallAggregate {
        latency_ms: calls.iter().map(|call| call.latency_ms).sum(),
        input_tokens: sum_optional(calls.iter().map(|call| call.usage.input_tokens)),
        cached_input_tokens: sum_optional(calls.iter().map(|call| call.usage.cached_input_tokens)),
        output_tokens: sum_optional(calls.iter().map(|call| call.usage.output_tokens)),
        estimated_cost_usd: pricing_base_url.and_then(|base_url| {
            sum_optional_f64(
                calls
                    .iter()
                    .map(|metadata| estimate_openai_cost(metadata, base_url)),
            )
        }),
    }
}

fn rate(numerator: usize, denominator: usize) -> Rate {
    let fraction = u32::try_from(numerator)
        .ok()
        .zip(u32::try_from(denominator).ok())
        .and_then(|(numerator, denominator)| {
            (denominator != 0).then(|| f64::from(numerator) / f64::from(denominator))
        });
    Rate {
        numerator,
        denominator,
        fraction,
    }
}

fn sum_optional(mut values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.try_fold(0_u64, |total, value| Some(total + value?))
}

fn sum_optional_f64(mut values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.try_fold(0.0, |total, value| Some(total + value?))
}

fn finding_text(verdict: &Verdict) -> String {
    verdict
        .missing_requirements
        .iter()
        .chain(&verdict.unrequested_changes)
        .chain(&verdict.violated_constraints)
        .chain(&verdict.potential_side_effects)
        .chain(&verdict.uncertainties)
        .chain(
            verdict
                .evidence
                .iter()
                .map(|evidence| &evidence.description),
        )
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn verdict_categories(verdict: &Verdict) -> BTreeSet<String> {
    [
        (
            !verdict.missing_requirements.is_empty(),
            "missing_requirement",
        ),
        (
            !verdict.unrequested_changes.is_empty(),
            "unrequested_change",
        ),
        (
            !verdict.violated_constraints.is_empty(),
            "violated_constraint",
        ),
        (
            !verdict.potential_side_effects.is_empty(),
            "potential_side_effect",
        ),
    ]
    .into_iter()
    .filter(|(present, _)| *present)
    .map(|(_, category)| category.to_owned())
    .collect()
}

fn mock_judge_verdict(verdict: &Verdict) -> JudgeVerdict {
    let findings = [
        (
            FindingCategory::MissingRequirements,
            &verdict.missing_requirements,
        ),
        (
            FindingCategory::UnrequestedChanges,
            &verdict.unrequested_changes,
        ),
        (
            FindingCategory::ViolatedConstraints,
            &verdict.violated_constraints,
        ),
        (
            FindingCategory::PotentialSideEffects,
            &verdict.potential_side_effects,
        ),
    ]
    .into_iter()
    .flat_map(|(kind, texts)| {
        texts.iter().map(move |text| JudgeFinding {
            kind,
            text: text.clone(),
            evidence_ref: None,
        })
    })
    .collect();
    JudgeVerdict {
        alignment: verdict.alignment,
        findings,
        confidence: verdict.confidence,
    }
}

fn stage_rate<'a>(
    cases: &'a [CaseResult],
    outcome: impl Fn(&'a CaseResult) -> &'a StageOutcome,
) -> Rate {
    let attempts = cases.iter().flat_map(|case| &outcome(case).attempts);
    let denominator = attempts.clone().count();
    let numerator = attempts
        .filter(|status| **status == AttemptStatus::Succeeded)
        .count();
    rate(numerator, denominator)
}

fn failure_counts(cases: &[CaseResult]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for failure in cases.iter().filter_map(|case| case.failure_category) {
        let key = serde_json::to_value(failure)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "orchestration_failure".to_owned());
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

fn strict_schema<T: JsonSchema>() -> Result<Value> {
    let mut schema = serde_json::to_value(schema_for!(T)).into_diagnostic()?;
    make_objects_strict(&mut schema);
    Ok(schema)
}

fn output_schema<T: JsonSchema>(strict: bool) -> Result<Value> {
    if strict {
        strict_schema::<T>()
    } else {
        serde_json::to_value(schema_for!(T)).into_diagnostic()
    }
}

fn make_objects_strict(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("$schema");
            if let Some(Value::Object(properties)) = object.get("properties") {
                object.insert(
                    "required".to_owned(),
                    Value::Array(properties.keys().cloned().map(Value::String).collect()),
                );
                object.insert("additionalProperties".to_owned(), Value::Bool(false));
            }
            object.values_mut().for_each(make_objects_strict);
        }
        Value::Array(values) => values.iter_mut().for_each(make_objects_strict),
        _ => {}
    }
}

fn api_runner(profiles: &ApiProfiles, model: &str) -> Result<OpenAiResponsesRunner> {
    OpenAiResponsesRunner::from_env(OpenAiResponsesConfig {
        base_url: profiles.base_url.clone(),
        api_key_env: profiles.api_key_env.clone(),
        model: model.to_owned(),
        reasoning_effort: profiles.reasoning_effort.clone(),
        timeout: Duration::from_secs(profiles.timeout_seconds),
    })
    .map_err(|error| miette!("could not initialize evaluation runner: {error}"))
}

fn validate_suite(suite: &Suite) -> Result<()> {
    if suite.version != 1 {
        return Err(miette!(
            "unsupported evaluation suite version {}",
            suite.version
        ));
    }
    if suite.cases.len() < 30 {
        return Err(miette!("benchmark v1 requires at least 30 cases"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for case in &suite.cases {
        if !ids.insert(&case.id)
            || case.id.trim().is_empty()
            || case.name.trim().is_empty()
            || case.class.trim().is_empty()
            || case.subset.trim().is_empty()
            || case.base_state.trim().is_empty()
            || case.change.trim().is_empty()
            || case.original_task.trim().is_empty()
            || case.expected.rationale.trim().is_empty()
            || case.base_files.is_empty()
            || case.candidate_patch.files.is_empty()
        {
            return Err(miette!("evaluation case `{}` is incomplete", case.name));
        }
        if case.expected.important_findings.is_empty()
            != case.expected.finding_categories.is_empty()
        {
            return Err(miette!(
                "evaluation case `{}` has inconsistent finding ground truth",
                case.name
            ));
        }
        if case.expected.finding_categories.iter().any(|category| {
            ![
                "missing_requirement",
                "unrequested_change",
                "violated_constraint",
                "potential_side_effect",
            ]
            .contains(&category.as_str())
        }) {
            return Err(miette!(
                "evaluation case `{}` has an invalid finding category",
                case.name
            ));
        }
        let serialized = serde_json::to_string(&bundle(case)?).into_diagnostic()?;
        if serialized.contains(&case.original_task) {
            return Err(miette!(
                "evaluation case `{}` leaks its original task into the blind bundle",
                case.name
            ));
        }
    }
    let canonical = suite
        .cases
        .iter()
        .filter(|case| case.subset == "canonical-5")
        .map(|case| (case.class.as_str(), case.expected.verdict))
        .collect::<Vec<_>>();
    let expected = vec![
        ("correct_patch", Alignment::Same),
        ("partial_implementation", Alignment::Partial),
        ("scope_creep", Alignment::Partial),
        ("constraint_violation", Alignment::Partial),
        ("wrong_component", Alignment::Different),
    ];
    if canonical != expected {
        return Err(miette!("canonical-5 labels or ordering drifted"));
    }
    Ok(())
}

fn validate_profiles(profiles: &ApiProfiles) -> Result<()> {
    if profiles.version != 1 || profiles.profiles.is_empty() || profiles.timeout_seconds == 0 {
        return Err(miette!("evaluation profile configuration is invalid"));
    }
    for profile in &profiles.profiles {
        if profile.name.trim().is_empty()
            || profile.model.trim().is_empty()
            || !(0.0..=1.0).contains(&profile.confidence_threshold)
            || (profile.escalate && profile.fallback_model.is_none())
        {
            return Err(miette!("evaluation profile `{}` is invalid", profile.name));
        }
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .into_diagnostic()
        .wrap_err_with(|| format!("invalid JSON in {}", path.display()))
}

fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not read {}", path.display()))?;
    toml::from_str(&text)
        .into_diagnostic()
        .wrap_err_with(|| format!("invalid TOML in {}", path.display()))
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn print_summary(report: &EvalReport) {
    println!("Flect evaluation\n");
    println!("Mode     {}", report.mode);
    println!("Suite    {}", report.suite);
    for profile in &report.profiles {
        let metrics = &profile.metrics;
        println!("\n{}", profile.profile.name);
        println!(
            "  Exact verdicts       {}/{}",
            metrics.exact_verdicts, metrics.cases
        );
        println!(
            "  Correct acceptance   {}",
            display_rate(&metrics.correct_patch_acceptance)
        );
        println!(
            "  Bad patch detection  {}",
            display_rate(&metrics.bad_patch_detection)
        );
        println!(
            "  Uncertainty          {}",
            display_rate(&metrics.uncertainty)
        );
        println!(
            "  Findings             {}",
            display_rate(&metrics.important_findings)
        );
        println!("  Requests             {}", metrics.requests);
        println!("  Latency              {} ms", metrics.latency_ms);
        println!(
            "  Estimated cost       {}",
            metrics
                .estimated_cost_usd
                .map_or_else(|| "unknown".to_owned(), |cost| format!("${cost:.6}"))
        );
    }
}

fn display_rate(rate: &Rate) -> String {
    rate.fraction.map_or_else(
        || "n/a".to_owned(),
        |value| {
            format!(
                "{}/{} ({:.1}%)",
                rate.numerator,
                rate.denominator,
                value * 100.0
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_case() -> EvalCase {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let suite: Suite = read_json(&root.join("fixtures/evaluation/cases.json")).unwrap();
        suite.cases.into_iter().next().unwrap()
    }

    fn output_with_verdict(verdict: Option<Verdict>) -> EvaluationOutput {
        EvaluationOutput {
            verdict,
            calls: Vec::new(),
            verifier_stage: StageOutcome {
                attempts: vec![AttemptStatus::Succeeded],
                final_status: StageStatus::Succeeded,
            },
            judge_stage: StageOutcome {
                attempts: vec![AttemptStatus::Succeeded],
                final_status: StageStatus::Succeeded,
            },
            evidence_validation_status: StageStatus::Succeeded,
            failure_category: None,
        }
    }

    #[tokio::test]
    async fn schema_failure_is_a_typed_case_result() {
        let case = first_case();
        let runner = MockRunner::named("malformed", [json!({"unexpected": true})]);
        let output = evaluate_case(&case, &runner, None, false, 0.0, false).await;
        assert_eq!(
            output.failure_category,
            Some(FailureCategory::ForwardSchemaFailure)
        );
        assert_eq!(
            output.verifier_stage.final_status,
            StageStatus::NotAttempted
        );
        assert!(output.verdict.is_none());
    }

    #[tokio::test]
    async fn compact_judge_evidence_failure_is_closed() {
        let case = first_case();
        let judge = JudgeVerdict {
            alignment: Alignment::Partial,
            findings: vec![JudgeFinding {
                kind: FindingCategory::MissingRequirements,
                text: "missing behavior".to_owned(),
                evidence_ref: Some("hunk/999".to_owned()),
            }],
            confidence: 0.8,
        };
        let runner = MockRunner::named(
            "invalid-evidence",
            [
                serde_json::to_value(&case.intended_spec).unwrap(),
                serde_json::to_value(&case.mock_echoed_spec).unwrap(),
                serde_json::to_value(judge).unwrap(),
            ],
        );
        let output = evaluate_case(&case, &runner, None, false, 0.0, false).await;
        assert_eq!(
            output.failure_category,
            Some(FailureCategory::EvidenceValidationFailure)
        );
        assert_eq!(output.judge_stage.final_status, StageStatus::Succeeded);
        assert_eq!(output.evidence_validation_status, StageStatus::Failed);
        assert!(output.verdict.is_none());
    }

    #[test]
    fn good_patch_errors_and_abstentions_are_not_false_positives() {
        let mut case = first_case();
        case.expected.verdict = Alignment::Same;
        let actuals = [Some(Alignment::Partial), Some(Alignment::Uncertain), None];
        let cases = actuals
            .into_iter()
            .map(|actual| {
                let verdict = actual.map(|alignment| {
                    let mut verdict = case.mock_verdict.clone();
                    verdict.alignment = alignment;
                    verdict
                });
                case_result(&case, &output_with_verdict(verdict), None)
            })
            .collect();
        let report = profile_report(test_profile(), cases);
        assert_eq!(report.metrics.false_positives, 1);
        assert_eq!(report.metrics.good_patch_abstentions, 1);
        assert_eq!(report.metrics.correct_patch_acceptance.denominator, 3);
    }

    #[test]
    fn finding_category_exact_match_is_order_independent() {
        let mut case = first_case();
        case.expected.finding_categories = BTreeSet::from([
            "unrequested_change".to_owned(),
            "missing_requirement".to_owned(),
        ]);
        let mut verdict = case.mock_verdict.clone();
        verdict.missing_requirements = vec!["first".to_owned()];
        verdict.unrequested_changes = vec!["second".to_owned()];
        let result = case_result(&case, &output_with_verdict(Some(verdict)), None);
        assert_eq!(
            result.actual_finding_categories,
            Some(BTreeSet::from([
                "missing_requirement".to_owned(),
                "unrequested_change".to_owned(),
            ]))
        );
        assert_eq!(result.finding_category_match, Some(true));
    }

    #[test]
    fn category_recall_counts_expected_labels_from_failed_cases() {
        let mut case = first_case();
        case.expected.finding_categories = BTreeSet::from([
            "missing_requirement".to_owned(),
            "violated_constraint".to_owned(),
        ]);
        let result = case_result(&case, &output_with_verdict(None), None);
        let report = profile_report(test_profile(), vec![result]);
        assert_eq!(report.metrics.finding_category_recall.numerator, 0);
        assert_eq!(report.metrics.finding_category_recall.denominator, 2);
    }

    #[tokio::test]
    async fn fallback_preserves_each_verifier_attempt_outcome() {
        let case = first_case();
        let primary = MockRunner::named(
            "primary",
            [
                serde_json::to_value(&case.intended_spec).unwrap(),
                json!({"invalid": true}),
                serde_json::to_value(mock_judge_verdict(&case.mock_verdict)).unwrap(),
            ],
        );
        let fallback = MockRunner::named(
            "fallback",
            [serde_json::to_value(&case.mock_echoed_spec).unwrap()],
        );
        let output = evaluate_case(&case, &primary, Some(&fallback), true, 0.0, false).await;
        assert_eq!(
            output.verifier_stage.attempts,
            vec![AttemptStatus::SchemaFailed, AttemptStatus::Succeeded]
        );
        assert_eq!(output.verifier_stage.final_status, StageStatus::Succeeded);
    }

    fn test_profile() -> ProfileSummary {
        ProfileSummary {
            name: "test".to_owned(),
            provider: "mock".to_owned(),
            model: "mock".to_owned(),
            fallback_model: None,
            escalation_enabled: false,
            confidence_threshold: None,
        }
    }
}
