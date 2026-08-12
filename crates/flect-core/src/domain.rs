//! Stable data structures passed between Flect's pipeline stages.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The original request captured before implementation begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    pub text: String,
}

/// A structured account of what the implementation should accomplish.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IntendedSpec {
    pub objective: String,
    pub requirements: Vec<String>,
    pub constraints: Vec<String>,
    pub non_goals: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub expected_scope: Vec<String>,
    pub uncertainties: Vec<String>,
}

impl IntendedSpec {
    /// Creates a deliberately conservative spec when no forward model is configured.
    pub fn from_task(task: &TaskInput) -> Self {
        Self {
            objective: task.text.trim().to_owned(),
            ..Self::default()
        }
    }
}

/// How a path changed relative to the captured base revision.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

/// One file in a captured patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
    pub patch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub insertions: u64,
    pub deletions: u64,
    pub binary: bool,
}

/// The complete set of working-tree changes Flect observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchSet {
    pub base_revision: String,
    pub files: Vec<ChangedFile>,
    pub renames: u64,
    pub insertions: u64,
    pub deletions: u64,
    pub binary_files: Vec<String>,
    pub untracked_files: Vec<String>,
}

/// Verifier context policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextPolicy {
    Patch,
    #[default]
    Focused,
    Repo,
}

impl fmt::Display for ContextPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Patch => formatter.write_str("patch"),
            Self::Focused => formatter.write_str("focused"),
            Self::Repo => formatter.write_str("repo"),
        }
    }
}

impl FromStr for ContextPolicy {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "patch" => Ok(Self::Patch),
            "focused" => Ok(Self::Focused),
            "repo" => Ok(Self::Repo),
            _ => Err("expected `patch`, `focused`, or `repo`"),
        }
    }
}

/// Content included alongside a patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}

/// A path excluded from verifier context and the reason for doing so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExcludedPath {
    pub path: String,
    pub reason: String,
}

/// A machine-readable description of the exact verifier input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub context_policy: ContextPolicy,
    pub patch_files: Vec<String>,
    pub context_files: Vec<String>,
    pub excluded_paths: Vec<ExcludedPath>,
    pub total_bytes: u64,
}

/// How confidently Flect can describe isolation for a potential source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IsolationKind {
    StructurallyExcluded,
    HeuristicallyChecked,
    Unknown,
}

/// One entry in a `BlindGuard` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IsolationEntry {
    pub source: String,
    pub status: String,
    pub assurance: IsolationKind,
}

/// Honest, machine-readable disclosure of the blindness boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlindnessReport {
    pub isolation: String,
    pub entries: Vec<IsolationEntry>,
    pub limitations: Vec<String>,
}

/// The only payload permitted to reach a blind verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlindBundle {
    pub patch: PatchSet,
    pub context: Vec<ContextFile>,
    pub manifest: BundleManifest,
    pub blindness_report: BlindnessReport,
}

/// One affected source file, with optional descriptive symbol detail.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AffectedScope {
    /// Exact path from Flect's visible patch or context files.
    pub file: String,
    /// Optional function, class, or other descriptive scope within `file`.
    #[serde(default)]
    pub symbol: Option<String>,
}

impl<'de> Deserialize<'de> for AffectedScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Structured {
                file: String,
                #[serde(default)]
                symbol: Option<String>,
            },
            Legacy(String),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Structured { file, symbol } => Ok(Self { file, symbol }),
            // Legacy persisted records are retained verbatim and still undergo file validation.
            Wire::Legacy(file) => Ok(Self { file, symbol: None }),
        }
    }
}

impl fmt::Display for AffectedScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.symbol {
            Some(symbol) => write!(formatter, "{}: {symbol}", self.file),
            None => formatter.write_str(&self.file),
        }
    }
}

/// The apparent behavior independently reconstructed from a patch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EchoedSpec {
    pub apparent_objective: String,
    pub behavior_before: Vec<String>,
    pub behavior_after: Vec<String>,
    pub affected_scope: Vec<AffectedScope>,
    pub side_effects: Vec<String>,
    pub assumptions: Vec<String>,
    pub uncertainties: Vec<String>,
    pub confidence: f64,
}

/// Coarse alignment between requested and reconstructed behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Alignment {
    Same,
    Partial,
    Different,
    Uncertain,
}

impl fmt::Display for Alignment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Same => "SAME",
            Self::Partial => "PARTIAL",
            Self::Different => "DIFFERENT",
            Self::Uncertain => "UNCERTAIN",
        };
        formatter.write_str(value)
    }
}

/// A location-backed reason supporting a finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_hunk: Option<String>,
    /// Stable IDs of negative findings supported by this evidence.
    #[serde(default)]
    pub finding_ids: Vec<String>,
    pub description: String,
    pub confidence: f64,
}

/// The action suggested after reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecommendedAction {
    Ship,
    RevisePatch,
    RevisitReasoning,
    ReviseBoth,
    RequestMoreContext,
}

impl fmt::Display for RecommendedAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Ship => "SHIP",
            Self::RevisePatch => "REVISE PATCH",
            Self::RevisitReasoning => "REVISIT REASONING",
            Self::ReviseBoth => "REVISE BOTH",
            Self::RequestMoreContext => "REQUEST MORE CONTEXT",
        };
        formatter.write_str(value)
    }
}

/// Structured result of comparing intended and apparent behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub alignment: Alignment,
    pub agreements: Vec<String>,
    pub missing_requirements: Vec<String>,
    pub unrequested_changes: Vec<String>,
    pub violated_constraints: Vec<String>,
    pub potential_side_effects: Vec<String>,
    pub uncertainties: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub confidence: f64,
    pub recommended_action: RecommendedAction,
}

/// Minimal verdict payload emitted by an external reconciliation judge.
///
/// Flect derives the persisted action, agreement list, and trusted evidence
/// locations. This keeps the agent-facing contract small without relaxing
/// validation of the semantic findings or their evidence associations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JudgeVerdict {
    pub alignment: Alignment,
    #[serde(default)]
    pub missing_requirements: Vec<String>,
    #[serde(default)]
    pub unrequested_changes: Vec<String>,
    #[serde(default)]
    pub violated_constraints: Vec<String>,
    #[serde(default)]
    pub potential_side_effects: Vec<String>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<JudgeEvidence>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// A judge's association between negative findings and one trusted patch hunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JudgeEvidence {
    /// Categories whose emitted findings this evidence supports.
    pub finding_categories: Vec<FindingCategory>,
    /// A stable identifier from the reconciliation job's evidence contract.
    #[serde(default)]
    pub hunk_id: Option<String>,
    pub description: String,
}

/// A negative-finding category that Flect expands into stable persisted IDs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    MissingRequirements,
    UnrequestedChanges,
    ViolatedConstraints,
    PotentialSideEffects,
}

/// Safe, persisted observability for one semantic model stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelCallRecord {
    pub stage: String,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub accepted: bool,
    pub provider: String,
    pub model: String,
    pub latency_ms: u64,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
    pub pricing_version: Option<String>,
    pub escalation_reason: Option<String>,
}

/// Persisted state captured before implementation begins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    pub version: u32,
    pub id: String,
    pub repository_root: String,
    pub base_revision: String,
    pub task: TaskInput,
    pub intended_spec: IntendedSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_calls: Vec<ModelCallRecord>,
    pub created_unix_ms: u64,
}

/// Persisted verification result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationRecord {
    pub version: u32,
    pub run_id: String,
    pub bundle: BlindBundle,
    pub echoed_spec: EchoedSpec,
    pub verdict: Verdict,
    #[serde(default)]
    pub isolation: IsolationLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_calls: Vec<ModelCallRecord>,
    pub verified_unix_ms: u64,
}

/// Assurance level actually established for a semantic reasoner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    Strict,
    Structural,
    Soft,
    #[default]
    Unknown,
}

/// How a Codex runtime selected the model used for an agent job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelSelection {
    Explicit,
    Inherited,
    #[default]
    Unknown,
}

/// Trusted handoff prepared for a blind external reasoner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindAgentJob {
    pub version: u32,
    pub job_id: String,
    pub run_id: String,
    pub isolation: IsolationLevel,
    pub workspace: String,
    pub instructions: String,
    pub bundle: BlindBundle,
    pub echoed_spec_schema: serde_json::Value,
    pub allowed_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
}

/// Typed response submitted by a blind reasoner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindAgentSubmission {
    pub job_id: String,
    pub echoed_spec: EchoedSpec,
    pub model: Option<String>,
    #[serde(default)]
    pub model_selection: AgentModelSelection,
}

/// Trusted handoff prepared for a separate reconciliation reasoner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationAgentJob {
    pub version: u32,
    pub job_id: String,
    pub run_id: String,
    pub blind_job_id: String,
    pub instructions: String,
    pub intended_spec: IntendedSpec,
    pub echoed_spec: EchoedSpec,
    pub available_evidence: Vec<ChangedFile>,
    /// Machine-readable, fail-closed rules and permitted patch locations for evidence.
    pub evidence_contract: serde_json::Value,
    pub verdict_schema: serde_json::Value,
}

/// Typed response submitted by a reconciliation reasoner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationAgentSubmission {
    pub job_id: String,
    pub verdict: JudgeVerdict,
    pub model: Option<String>,
    #[serde(default)]
    pub model_selection: AgentModelSelection,
}
