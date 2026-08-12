//! Reproducible offline and explicitly opt-in model evaluation.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use flect_core::{
    Alignment, BlindBundle, BlindnessReport, BundleManifest, ContextFile, ContextPolicy,
    EchoedSpec, IntendedSpec, IsolationEntry, IsolationKind, PatchSet, TaskInput, Verdict,
};
use flect_runner::{
    AgentRequest, AgentRunner, MockRunner, OpenAiResponsesConfig, OpenAiResponsesRunner,
    RequestPurpose, RunnerMetadata, RunnerOutput, estimate_openai_cost,
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
    finding_category: Option<String>,
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
    exact_verdicts: usize,
    verdict_accuracy: Rate,
    per_class_accuracy: BTreeMap<String, Rate>,
    confusion_matrix: BTreeMap<String, BTreeMap<String, usize>>,
    verifier_schema_compliance: Rate,
    judge_schema_compliance: Rate,
    correct_patch_acceptance: Rate,
    bad_patch_detection: Rate,
    false_positives: usize,
    false_negatives: usize,
    uncertainty: Rate,
    important_findings: Rate,
    finding_category_accuracy: Rate,
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
    actual: Alignment,
    verdict_match: bool,
    expected_findings: usize,
    matched_findings: usize,
    expected_finding_category: Option<String>,
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
    verdict: Verdict,
    calls: Vec<RunnerMetadata>,
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
                serde_json::to_value(&case.mock_verdict).into_diagnostic()?,
            ],
        );
        let output = evaluate_case(case, &runner, None, false, 0.0, false).await?;
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
        .await?;
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

async fn evaluate_case(
    case: &EvalCase,
    primary: &dyn AgentRunner,
    fallback: Option<&dyn AgentRunner>,
    escalate: bool,
    threshold: f64,
    strict_output: bool,
) -> Result<EvaluationOutput> {
    let bundle = bundle(case)?;
    let mut calls = Vec::new();
    let intended: IntendedSpec = stage(
        primary,
        RequestPurpose::AnalyzeForwardIntent,
        serde_json::to_value(TaskInput {
            text: case.original_task.clone(),
        })
        .into_diagnostic()?,
        &mut calls,
        strict_output,
    )
    .await?;
    let echoed: EchoedSpec = stage_with_fallback(
        primary,
        RequestPurpose::ReconstructPatchIntent,
        serde_json::to_value(&bundle).into_diagnostic()?,
        &mut calls,
        StageOptions {
            fallback,
            escalate,
            strict_output,
        },
        |value: &EchoedSpec| value.confidence < threshold || !value.uncertainties.is_empty(),
    )
    .await?;
    let input = json!({
        "intended_spec": intended,
        "echoed_spec": echoed,
        "available_evidence": bundle.patch.files,
    });
    let verdict: Verdict = stage_with_fallback(
        primary,
        RequestPurpose::ReconcileIntent,
        input,
        &mut calls,
        StageOptions {
            fallback,
            escalate,
            strict_output,
        },
        |value: &Verdict| value.alignment == Alignment::Uncertain || value.confidence < threshold,
    )
    .await?;
    Ok(EvaluationOutput { verdict, calls })
}

async fn stage<T: DeserializeOwned + JsonSchema>(
    runner: &dyn AgentRunner,
    purpose: RequestPurpose,
    input: Value,
    calls: &mut Vec<RunnerMetadata>,
    strict_output: bool,
) -> Result<T> {
    let output = runner
        .generate_structured(
            &AgentRequest { purpose, input },
            &output_schema::<T>(strict_output)?,
        )
        .await
        .map_err(|error| miette!("evaluation runner failed: {error}"))?;
    decode(output, calls)
}

async fn stage_with_fallback<T, F>(
    primary: &dyn AgentRunner,
    purpose: RequestPurpose,
    input: Value,
    calls: &mut Vec<RunnerMetadata>,
    options: StageOptions<'_>,
    should_escalate: F,
) -> Result<T>
where
    T: DeserializeOwned + JsonSchema,
    F: Fn(&T) -> bool,
{
    let schema = output_schema::<T>(options.strict_output)?;
    let request = AgentRequest { purpose, input };
    let primary_output = primary.generate_structured(&request, &schema).await;
    match primary_output {
        Ok(output) => {
            let value = decode(output, calls)?;
            if options.escalate && should_escalate(&value) {
                if let Some(fallback) = options.fallback {
                    return decode(
                        fallback
                            .generate_structured(&request, &schema)
                            .await
                            .map_err(|error| miette!("evaluation fallback failed: {error}"))?,
                        calls,
                    );
                }
            }
            Ok(value)
        }
        Err(primary_error) if options.escalate && options.fallback.is_some() => decode(
            options
                .fallback
                .expect("checked above")
                .generate_structured(&request, &schema)
                .await
                .map_err(|error| {
                    miette!("primary failed ({primary_error}); evaluation fallback failed: {error}")
                })?,
            calls,
        ),
        Err(error) => Err(miette!("evaluation runner failed: {error}")),
    }
}

fn decode<T: DeserializeOwned>(output: RunnerOutput, calls: &mut Vec<RunnerMetadata>) -> Result<T> {
    calls.push(output.metadata);
    serde_json::from_value(output.value)
        .into_diagnostic()
        .wrap_err("evaluation output did not match its typed schema")
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
    let finding_text = finding_text(&output.verdict);
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
        actual: output.verdict.alignment,
        verdict_match: output.verdict.alignment == case.expected.verdict,
        expected_findings: case.expected.important_findings.len(),
        matched_findings,
        expected_finding_category: case.expected.finding_category.clone(),
        finding_category_match: case
            .expected
            .finding_category
            .as_ref()
            .map(|_| matched_findings == case.expected.important_findings.len()),
        evidence_ref_validation_failures: output
            .verdict
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
            .count(),
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
    let correct = cases
        .iter()
        .filter(|case| case.expected == Alignment::Same)
        .count();
    let accepted = cases
        .iter()
        .filter(|case| case.expected == Alignment::Same && case.actual == Alignment::Same)
        .count();
    let bad = cases.len() - correct;
    let detected = cases
        .iter()
        .filter(|case| {
            case.expected != Alignment::Same
                && matches!(case.actual, Alignment::Partial | Alignment::Different)
        })
        .count();
    let uncertain = cases
        .iter()
        .filter(|case| case.actual == Alignment::Uncertain)
        .count();
    let expected_findings = cases.iter().map(|case| case.expected_findings).sum();
    let matched_findings = cases.iter().map(|case| case.matched_findings).sum();
    let calls = cases.iter().map(|case| case.requests).sum();
    let latency = cases.iter().map(|case| case.latency_ms).sum();
    let exact = cases.iter().filter(|case| case.verdict_match).count();
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
        count.0 += usize::from(case.verdict_match);
        *confusion_matrix
            .get_mut(alignment_name(case.expected))
            .expect("known alignment")
            .get_mut(alignment_name(case.actual))
            .expect("known alignment") += 1;
    }
    let per_class_accuracy = per_class_counts
        .into_iter()
        .map(|(class, (matches, total))| (class, rate(matches, total)))
        .collect();
    let category_cases = cases
        .iter()
        .filter(|case| case.expected_finding_category.is_some())
        .count();
    let category_matches = cases
        .iter()
        .filter(|case| case.finding_category_match == Some(true))
        .count();
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
        cases_persisted: cases.len(),
        exact_verdicts: exact,
        verdict_accuracy: rate(exact, cases.len()),
        per_class_accuracy,
        confusion_matrix,
        verifier_schema_compliance: rate(cases.len() * 2, cases.len() * 2),
        judge_schema_compliance: rate(cases.len(), cases.len()),
        correct_patch_acceptance: rate(accepted, correct),
        bad_patch_detection: rate(detected, bad),
        false_positives: correct - accepted,
        false_negatives: cases
            .iter()
            .filter(|case| case.expected != Alignment::Same && case.actual == Alignment::Same)
            .count(),
        uncertainty: rate(uncertain, cases.len()),
        important_findings: rate(matched_findings, expected_findings),
        finding_category_accuracy: rate(category_matches, category_cases),
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
        if case.expected.important_findings.is_empty() != case.expected.finding_category.is_none() {
            return Err(miette!(
                "evaluation case `{}` has inconsistent finding ground truth",
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
