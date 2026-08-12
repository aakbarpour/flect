use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use flect_core::{
    BlindAgentJob, BlindAgentSubmission, BlindBundle, BlindGuard, Config, ContextBuilder,
    ContextPolicy, EchoedSpec, GitRepository, IsolationLevel, ModelCallRecord,
    ReconciliationAgentJob, ReconciliationAgentSubmission, RunStore, VerificationRecord,
};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

use crate::{EvidenceError, validate_verdict_evidence};

const VERIFIER_INSTRUCTIONS: &str = "You are the blind Flect verifier. You have not been given the original task. Do not attempt to discover it. Inspect only the supplied sanitized patch evidence. Determine what behavior this patch appears to add, remove, or change. Return only a valid EchoedSpec matching the supplied schema. Do not perform general style review. Do not invent files, lines, requirements, or motivations. Preserve uncertainty.";
const JUDGE_INSTRUCTIONS: &str = "You are the Flect reconciliation judge. Compare IntendedSpec with EchoedSpec. Do not review unrelated code quality. Return only a valid Verdict. Use SAME only with no material divergence; PARTIAL for missing requirements, constraints, scope creep, or meaningful unexpected behavior; DIFFERENT for a materially different or contradictory change; UNCERTAIN for insufficient evidence. Never fabricate evidence.";
static JOB_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum AgentWorkflowError {
    #[error("could not discover repository: {0}")]
    Repository(String),
    #[error("could not load Flect configuration: {0}")]
    Configuration(String),
    #[error("could not load Flect run state: {0}")]
    RunState(String),
    #[error("could not capture the blind bundle: {0}")]
    Bundle(String),
    #[error("could not create agent workspace {path}: {source}")]
    WorkspaceIo {
        path: String,
        source: std::io::Error,
    },
    #[error("agent workspace must be outside the repository and cannot use a symlink boundary")]
    UnsafeWorkspace,
    #[error("invalid agent job identifier `{0}`")]
    InvalidJobId(String),
    #[error("agent job `{0}` does not exist")]
    JobNotFound(String),
    #[error("agent job `{0}` is not in the required lifecycle state")]
    InvalidJobState(String),
    #[error("agent submission job ID does not match `{0}`")]
    JobMismatch(String),
    #[error("blind response references unavailable scope `{0}`")]
    UnavailableScope(String),
    #[error("agent state is invalid: {0}")]
    InvalidState(String),
    #[error("submitted verdict failed trusted validation: {0}")]
    Evidence(#[from] EvidenceError),
}

pub struct AgentService {
    repository: GitRepository,
    workspace_root: PathBuf,
}

impl AgentService {
    /// Discovers a repository and uses the operating-system temporary directory.
    ///
    /// # Errors
    ///
    /// Returns an error when repository discovery or workspace validation fails.
    pub fn discover(start: &Path) -> Result<Self, AgentWorkflowError> {
        let repository = GitRepository::discover(start)
            .map_err(|error| AgentWorkflowError::Repository(error.to_string()))?;
        Self::with_workspace_root(repository, std::env::temp_dir().join("flect-agent-jobs"))
    }

    /// Creates a service with an explicit workspace root, primarily for embedding and tests.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace is inside the repository or crosses a symlink.
    pub fn with_workspace_root(
        repository: GitRepository,
        workspace_root: PathBuf,
    ) -> Result<Self, AgentWorkflowError> {
        let repository_root = canonical_existing(repository.root())?;
        if workspace_root.exists() {
            let workspace = canonical_existing(&workspace_root)?;
            if workspace.starts_with(&repository_root) || has_symlink_component(&workspace_root)? {
                return Err(AgentWorkflowError::UnsafeWorkspace);
            }
        } else if workspace_root.starts_with(repository.root()) {
            return Err(AgentWorkflowError::UnsafeWorkspace);
        }
        Ok(Self {
            repository,
            workspace_root,
        })
    }

    /// Captures a sanitized bundle and writes a new read-only blind-agent workspace.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid run/configuration state, unsafe paths, or capture failures.
    pub fn prepare_blind(
        &self,
        run_id: Option<&str>,
        context: Option<ContextPolicy>,
    ) -> Result<BlindAgentJob, AgentWorkflowError> {
        let run = RunStore::new(self.repository.root())
            .load_run(run_id)
            .map_err(|error| AgentWorkflowError::RunState(error.to_string()))?;
        let mut config = Config::load(&self.repository.root().join("flect.toml"))
            .map_err(|error| AgentWorkflowError::Configuration(error.to_string()))?;
        if let Some(context) = context {
            config.verification.context = context;
        }
        let bundle = self.build_bundle(&config, &run.base_revision)?;
        let job_id = generate_id("blind", &run.id)?;
        let workspace = self.create_blind_workspace(&job_id, &bundle)?;
        let job = BlindAgentJob {
            version: 1,
            job_id: job_id.clone(),
            run_id: run.id,
            isolation: IsolationLevel::Structural,
            workspace: workspace.display().to_string(),
            instructions: VERIFIER_INSTRUCTIONS.to_owned(),
            bundle,
            echoed_spec_schema: strict_schema::<EchoedSpec>()?,
            allowed_resources: vec![
                "patch.json".to_owned(),
                "context/*.json".to_owned(),
                "manifest.json".to_owned(),
                "echoed-spec.schema.json".to_owned(),
                "VERIFIER.md".to_owned(),
            ],
            excluded_resources: vec![
                "original task".to_owned(),
                "IntendedSpec".to_owned(),
                "conversation and parent reasoning".to_owned(),
                "issue, branch, and commit metadata".to_owned(),
                ".git".to_owned(),
                ".flect".to_owned(),
            ],
        };
        self.save_blind_state(&BlindState {
            version: 1,
            status: BlindStatus::Prepared,
            job: job.clone(),
            echoed_spec: None,
            model: None,
            model_selection: flect_core::AgentModelSelection::Unknown,
        })?;
        Ok(job)
    }

    /// Validates and accepts a structured blind-agent response once.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid lifecycle, job mismatch, or unavailable scope.
    pub fn submit_echo(
        &self,
        submission: BlindAgentSubmission,
    ) -> Result<EchoedSpec, AgentWorkflowError> {
        let mut state = self.load_blind_state(&submission.job_id)?;
        if state.status != BlindStatus::Prepared {
            return Err(AgentWorkflowError::InvalidJobState(submission.job_id));
        }
        if state.job.job_id != submission.job_id {
            return Err(AgentWorkflowError::JobMismatch(state.job.job_id));
        }
        let allowed = state
            .job
            .bundle
            .patch
            .files
            .iter()
            .map(|file| file.path.as_str())
            .chain(
                state
                    .job
                    .bundle
                    .context
                    .iter()
                    .map(|file| file.path.as_str()),
            )
            .collect::<Vec<_>>();
        if let Some(scope) = submission
            .echoed_spec
            .affected_scope
            .iter()
            .find(|scope| !allowed.contains(&scope.as_str()))
        {
            return Err(AgentWorkflowError::UnavailableScope(scope.clone()));
        }
        state.status = BlindStatus::EchoAccepted;
        state.echoed_spec = Some(submission.echoed_spec.clone());
        state.model = submission.model;
        state.model_selection = submission.model_selection;
        self.save_blind_state(&state)?;
        Ok(submission.echoed_spec)
    }

    /// Creates a distinct judge job from an accepted blind response.
    ///
    /// # Errors
    ///
    /// Returns an error when the blind job is missing, invalid, or incomplete.
    pub fn prepare_reconciliation(
        &self,
        blind_job_id: &str,
    ) -> Result<ReconciliationAgentJob, AgentWorkflowError> {
        let blind = self.load_blind_state(blind_job_id)?;
        if blind.status != BlindStatus::EchoAccepted {
            return Err(AgentWorkflowError::InvalidJobState(blind_job_id.to_owned()));
        }
        let echoed_spec = blind
            .echoed_spec
            .clone()
            .ok_or_else(|| AgentWorkflowError::InvalidJobState(blind_job_id.to_owned()))?;
        let run = RunStore::new(self.repository.root())
            .load_run(Some(&blind.job.run_id))
            .map_err(|error| AgentWorkflowError::RunState(error.to_string()))?;
        let job = ReconciliationAgentJob {
            version: 1,
            job_id: generate_id("judge", &blind.job.run_id)?,
            run_id: blind.job.run_id.clone(),
            blind_job_id: blind.job.job_id.clone(),
            instructions: JUDGE_INSTRUCTIONS.to_owned(),
            intended_spec: run.intended_spec,
            echoed_spec,
            available_evidence: blind.job.bundle.patch.files.clone(),
            verdict_schema: strict_schema::<flect_core::Verdict>()?,
        };
        self.save_reconciliation_state(&ReconciliationState {
            version: 1,
            status: ReconciliationStatus::Prepared,
            job: job.clone(),
            bundle: blind.job.bundle,
            blind_model: blind.model,
            blind_model_selection: blind.model_selection,
        })?;
        Ok(job)
    }

    /// Validates a judge response and persists the final verification record.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid lifecycle, fabricated evidence, or persistence failure.
    pub fn submit_verdict(
        &self,
        submission: ReconciliationAgentSubmission,
    ) -> Result<VerificationRecord, AgentWorkflowError> {
        let mut state = self.load_reconciliation_state(&submission.job_id)?;
        if state.status != ReconciliationStatus::Prepared {
            return Err(AgentWorkflowError::InvalidJobState(submission.job_id));
        }
        if state.job.job_id != submission.job_id {
            return Err(AgentWorkflowError::JobMismatch(state.job.job_id));
        }
        validate_verdict_evidence(&submission.verdict, &state.bundle)?;
        let record = VerificationRecord {
            version: 1,
            run_id: state.job.run_id.clone(),
            bundle: state.bundle.clone(),
            echoed_spec: state.job.echoed_spec.clone(),
            verdict: submission.verdict,
            isolation: IsolationLevel::Structural,
            model_calls: vec![
                agent_call(
                    "backward",
                    state.blind_model.clone(),
                    state.blind_model_selection,
                ),
                agent_call(
                    "reconciliation",
                    submission.model.clone(),
                    submission.model_selection,
                ),
            ],
            verified_unix_ms: unix_millis()?,
        };
        RunStore::new(self.repository.root())
            .save_verification(&record)
            .map_err(|error| AgentWorkflowError::RunState(error.to_string()))?;
        state.status = ReconciliationStatus::Completed;
        self.save_reconciliation_state(&state)?;
        Ok(record)
    }

    fn build_bundle(
        &self,
        config: &Config,
        base_revision: &str,
    ) -> Result<BlindBundle, AgentWorkflowError> {
        let patch = self
            .repository
            .capture_patch(
                base_revision,
                config.verification.include_untracked,
                config.privacy.respect_gitignore,
                config.verification.max_patch_bytes,
            )
            .map_err(|error| AgentWorkflowError::Bundle(error.to_string()))?;
        let context = ContextBuilder::new(self.repository.root(), config)
            .map_err(|error| AgentWorkflowError::Bundle(error.to_string()))?
            .build(patch)
            .map_err(|error| AgentWorkflowError::Bundle(error.to_string()))?;
        BlindGuard::build(context, &config.blind)
            .map_err(|error| AgentWorkflowError::Bundle(error.to_string()))
    }

    fn create_blind_workspace(
        &self,
        job_id: &str,
        bundle: &BlindBundle,
    ) -> Result<PathBuf, AgentWorkflowError> {
        validate_job_id(job_id)?;
        fs::create_dir_all(&self.workspace_root)
            .map_err(|source| workspace_error(&self.workspace_root, source))?;
        if has_symlink_component(&self.workspace_root)? {
            return Err(AgentWorkflowError::UnsafeWorkspace);
        }
        let workspace = self.workspace_root.join(job_id);
        fs::create_dir(&workspace).map_err(|source| workspace_error(&workspace, source))?;
        let context_directory = workspace.join("context");
        fs::create_dir(&context_directory)
            .map_err(|source| workspace_error(&context_directory, source))?;
        write_json_readonly(&workspace.join("patch.json"), &bundle.patch)?;
        write_json_readonly(&workspace.join("manifest.json"), &bundle.manifest)?;
        write_json_readonly(
            &workspace.join("echoed-spec.schema.json"),
            &strict_schema::<EchoedSpec>()?,
        )?;
        write_readonly(
            &workspace.join("VERIFIER.md"),
            VERIFIER_INSTRUCTIONS.as_bytes(),
        )?;
        for (index, context) in bundle.context.iter().enumerate() {
            write_json_readonly(&context_directory.join(format!("{index:04}.json")), context)?;
        }
        Ok(workspace)
    }

    fn blind_state_path(&self, job_id: &str) -> Result<PathBuf, AgentWorkflowError> {
        validate_job_id(job_id)?;
        Ok(self
            .repository
            .root()
            .join(".flect/agent/blind")
            .join(format!("{job_id}.json")))
    }

    fn reconciliation_state_path(&self, job_id: &str) -> Result<PathBuf, AgentWorkflowError> {
        validate_job_id(job_id)?;
        Ok(self
            .repository
            .root()
            .join(".flect/agent/reconciliation")
            .join(format!("{job_id}.json")))
    }

    fn save_blind_state(&self, state: &BlindState) -> Result<(), AgentWorkflowError> {
        write_state(&self.blind_state_path(&state.job.job_id)?, state)
    }

    fn load_blind_state(&self, job_id: &str) -> Result<BlindState, AgentWorkflowError> {
        read_state(&self.blind_state_path(job_id)?, job_id)
    }

    fn save_reconciliation_state(
        &self,
        state: &ReconciliationState,
    ) -> Result<(), AgentWorkflowError> {
        write_state(&self.reconciliation_state_path(&state.job.job_id)?, state)
    }

    fn load_reconciliation_state(
        &self,
        job_id: &str,
    ) -> Result<ReconciliationState, AgentWorkflowError> {
        read_state(&self.reconciliation_state_path(job_id)?, job_id)
    }
}

#[derive(Serialize, Deserialize)]
struct BlindState {
    version: u32,
    status: BlindStatus,
    job: BlindAgentJob,
    echoed_spec: Option<EchoedSpec>,
    model: Option<String>,
    model_selection: flect_core::AgentModelSelection,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlindStatus {
    Prepared,
    EchoAccepted,
}

#[derive(Serialize, Deserialize)]
struct ReconciliationState {
    version: u32,
    status: ReconciliationStatus,
    job: ReconciliationAgentJob,
    bundle: BlindBundle,
    blind_model: Option<String>,
    blind_model_selection: flect_core::AgentModelSelection,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReconciliationStatus {
    Prepared,
    Completed,
}

fn strict_schema<T: JsonSchema>() -> Result<Value, AgentWorkflowError> {
    let mut schema = serde_json::to_value(schema_for!(T))
        .map_err(|error| AgentWorkflowError::InvalidState(error.to_string()))?;
    make_objects_strict(&mut schema);
    Ok(schema)
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

fn write_state<T: Serialize>(path: &Path, value: &T) -> Result<(), AgentWorkflowError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| workspace_error(parent, source))?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| AgentWorkflowError::InvalidState(error.to_string()))?;
    fs::write(path, bytes).map_err(|source| workspace_error(path, source))
}

fn read_state<T: DeserializeOwned>(path: &Path, job_id: &str) -> Result<T, AgentWorkflowError> {
    if !path.exists() {
        return Err(AgentWorkflowError::JobNotFound(job_id.to_owned()));
    }
    let bytes = fs::read(path).map_err(|source| workspace_error(path, source))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AgentWorkflowError::InvalidState(error.to_string()))
}

fn write_json_readonly<T: Serialize>(path: &Path, value: &T) -> Result<(), AgentWorkflowError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| AgentWorkflowError::InvalidState(error.to_string()))?;
    write_readonly(path, &bytes)
}

fn write_readonly(path: &Path, bytes: &[u8]) -> Result<(), AgentWorkflowError> {
    fs::write(path, bytes).map_err(|source| workspace_error(path, source))?;
    let mut permissions = fs::metadata(path)
        .map_err(|source| workspace_error(path, source))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(|source| workspace_error(path, source))
}

fn validate_job_id(job_id: &str) -> Result<(), AgentWorkflowError> {
    let valid = (job_id.starts_with("blind_") || job_id.starts_with("judge_"))
        && job_id.split_once('_').is_some_and(|(_, value)| {
            value.len() == 16 && value.chars().all(|ch| ch.is_ascii_hexdigit())
        });
    if valid {
        Ok(())
    } else {
        Err(AgentWorkflowError::InvalidJobId(job_id.to_owned()))
    }
}

fn generate_id(kind: &str, run_id: &str) -> Result<String, AgentWorkflowError> {
    let now = unix_millis()?;
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    run_id.hash(&mut hasher);
    now.hash(&mut hasher);
    JOB_SEQUENCE
        .fetch_add(1, Ordering::Relaxed)
        .hash(&mut hasher);
    Ok(format!("{kind}_{:016x}", hasher.finish()))
}

fn unix_millis() -> Result<u64, AgentWorkflowError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AgentWorkflowError::InvalidState(error.to_string()))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|error| AgentWorkflowError::InvalidState(error.to_string()))
}

fn canonical_existing(path: &Path) -> Result<PathBuf, AgentWorkflowError> {
    fs::canonicalize(path).map_err(|source| workspace_error(path, source))
}

fn has_symlink_component(path: &Path) -> Result<bool, AgentWorkflowError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(workspace_error(&current, source)),
        }
    }
    Ok(false)
}

fn workspace_error(path: &Path, source: std::io::Error) -> AgentWorkflowError {
    AgentWorkflowError::WorkspaceIo {
        path: path.display().to_string(),
        source,
    }
}

fn agent_call(
    stage: &str,
    model: Option<String>,
    selection: flect_core::AgentModelSelection,
) -> ModelCallRecord {
    ModelCallRecord {
        stage: stage.to_owned(),
        attempt: 1,
        accepted: true,
        provider: "codex-native".to_owned(),
        model: model.unwrap_or_else(|| "unknown".to_owned()),
        latency_ms: 0,
        input_tokens: None,
        cached_input_tokens: None,
        output_tokens: None,
        estimated_cost_usd: None,
        pricing_version: None,
        escalation_reason: Some(format!("agent model selection: {selection:?}")),
    }
}
