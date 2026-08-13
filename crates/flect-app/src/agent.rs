use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flect_core::{
    AgentModelSelection, Alignment, BlindAgentJob, BlindAgentSubmission, BlindBundle, BlindGuard,
    Config, ContextBuilder, ContextPolicy, EchoedSpec, FindingCategory, GitRepository,
    IsolationLevel, JudgeFinding, JudgeVerdict, ModelCallRecord, ReconciliationAgentJob, RunStore,
    VerificationRecord,
};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

use crate::{EvidenceError, materialize_judge_verdict};

const VERIFIER_INSTRUCTIONS: &str = "You are the blind Flect verifier. You have not been given the original task. Do not attempt to discover it. Inspect only the supplied sanitized patch evidence and determine what behavior the patch appears to add, remove, or change. Do not execute Flect or any repository command, write JSON, write to the repository, or use chat as the protocol payload. Write only the permitted primitive UTF-8 files in the assigned Flect draft root. Write every non-marker value with no UTF-8 BOM and no trailing carriage return or newline; use an exact-byte file API rather than a line-oriented writer. Scalar files are named exactly `objective`, `confidence`, `model`, and `model_selection`, with no extension. `model` records the actual runtime model and `model_selection` is exactly `explicit`, `inherited`, or `unknown`. List entries use consecutive zero-based six-digit names beginning at `000000.txt` in `behavior_before`, `behavior_after`, `side_effects`, `assumptions`, and `uncertainties`. Scope entries use consecutive zero-based six-digit directories beginning at `affected_scope/000000`, each containing extensionless `file` and optional `symbol`. Each scope file's exact bytes must equal one path in the supplied manifest. Leave a list directory empty when there are no entries; never invent placeholder content. Create `submitted` last as an exactly zero-byte file, then verify its length is zero. Do not add any other file or directory. Flect owns job binding, structure, validation, and serialization. Do not perform general style review or invent files, lines, requirements, or motivations. Preserve uncertainty.";
const JUDGE_INSTRUCTIONS: &str = "You are the Flect reconciliation judge. Do not execute Flect or repository commands, write JSON, write to the repository, or use chat as the protocol payload. Write only the permitted primitive UTF-8 files in the assigned Flect draft root. Write every non-marker value with no UTF-8 BOM and no trailing carriage return or newline; use an exact-byte file API rather than a line-oriented writer. Scalar files are named exactly `confidence`, `model`, and `model_selection`, with no extension. `model` records the actual runtime model and `model_selection` is exactly `explicit`, `inherited`, or `unknown`. Create exactly one empty alignment marker directory: `alignment/SAME`, `alignment/PARTIAL`, `alignment/DIFFERENT`, or `alignment/UNCERTAIN`. Finding entries use consecutive zero-based six-digit directories beginning at `findings/000000`; each contains one empty kind-marker directory named `missing_requirement`, `unrequested_change`, `violated_constraint`, or `potential_side_effect`, plus extensionless `text` and optional `evidence_ref`. Use only IDs from evidence_ref_contract. For every listed `side_effect/<n>`, create exactly one disposition under `side_effect_dispositions/side_effect/<n>`: either `finding` containing extensionless `text` and `evidence_ref`, or `not-distinct` containing extensionless `reason`. A verifier side-effect candidate is distinct only when it describes a downstream consequence beyond the base divergence itself, even when both arise from the same changed hunk. Caller migration or newly exposed sensitive data are downstream consequences; specifically, logging request-derived identifiers, keys, or values is both an unrequested behavior and a potential disclosure/observability side effect. The changed API, added output, wrong-component behavior, or altered label itself is only the violated constraint or unrequested change. Mark a candidate not-distinct when its semantic claim is the base behavior itself, but not merely because it shares a cause or evidence reference with a genuine downstream consequence. Create `submitted` last as an exactly zero-byte file, then verify its length is zero. Do not add any other file or directory. Compare IntendedSpec with EchoedSpec for complete alignment: surface scope creep, unrelated behavior, added functionality, and task-boundary violations. A constraint violation or downstream side effect alone does not establish DIFFERENT; use PARTIAL when the requested objective is materially advanced with divergence. DIFFERENT requires a missing requirement or unrequested change that supports unrelated, replacing, or contradictory work. Each verifier-reported side effect must be explicitly dispositioned; do not emit a potential_side_effect that merely restates the same divergence. SAME must have zero findings; PARTIAL and DIFFERENT must have at least one finding. Flect owns job binding, serialization, evidence materialization, and persistence.";
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
    #[error("agent workspace must resolve outside the repository")]
    UnsafeWorkspace,
    #[error("invalid agent job identifier `{0}`")]
    InvalidJobId(String),
    #[error("agent job `{0}` does not exist")]
    JobNotFound(String),
    #[error("agent job `{0}` is not in the required lifecycle state")]
    InvalidJobState(String),
    #[error("agent submission job ID does not match `{0}`")]
    JobMismatch(String),
    #[error("submission file is not the designated file for agent job `{0}")]
    SubmissionFileMismatch(String),
    #[error("blind response references unavailable scope `{0}`")]
    UnavailableScope(String),
    #[error("agent state is invalid: {0}")]
    InvalidState(String),
    #[error("DIFFERENT requires a missing requirement or unrequested change finding")]
    DifferentWithoutObjectiveMismatch,
    #[error("agent workspace ownership could not be established for {0}")]
    UnsafeCleanup(String),
    #[error("submitted verdict failed trusted validation: {0}")]
    Evidence(#[from] EvidenceError),
}

pub struct AgentService {
    repository: GitRepository,
    workspace_root: PathBuf,
    cleanup_on_complete: bool,
}

/// Selection for explicit workspace cleanup.
#[derive(Debug, Clone, Copy, Default)]
pub struct CleanupOptions {
    pub dry_run: bool,
    pub include_all: bool,
    pub older_than_hours: Option<u64>,
}

/// Bounded cleanup result, suitable for CLI and MCP reporting.
#[derive(Debug, Serialize)]
pub struct CleanupReport {
    pub dry_run: bool,
    pub deleted: Vec<String>,
    pub retained: Vec<String>,
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
        let cleanup_on_complete = Config::load(&repository.root().join("flect.toml"))
            .map_err(|error| AgentWorkflowError::Configuration(error.to_string()))?
            .agent
            .cleanup_on_complete;
        Self::with_workspace_root_and_cleanup(
            repository,
            std::env::temp_dir().join("flect-agent-jobs"),
            cleanup_on_complete,
        )
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
        Self::with_workspace_root_and_cleanup(repository, workspace_root, true)
    }

    /// Creates a service with an explicit cleanup policy, primarily for embedding and tests.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace is unsafe or cannot be resolved.
    pub fn with_workspace_root_and_cleanup(
        repository: GitRepository,
        workspace_root: PathBuf,
        cleanup_on_complete: bool,
    ) -> Result<Self, AgentWorkflowError> {
        let repository_root = canonical_existing(repository.root())?;
        let workspace = resolve_with_missing(&workspace_root)?;
        if workspace.starts_with(&repository_root) {
            return Err(AgentWorkflowError::UnsafeWorkspace);
        }
        Ok(Self {
            repository,
            workspace_root,
            cleanup_on_complete,
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
        let draft_root = external_draft_root(&self.workspace_root, &job_id);
        let job = BlindAgentJob {
            version: 1,
            job_id: job_id.clone(),
            run_id: run.id,
            isolation: IsolationLevel::Structural,
            workspace: workspace.display().to_string(),
            instructions: format!(
                "{VERIFIER_INSTRUCTIONS} The assigned draft root is `{}`. The trusted parent commits only the bound job ID.",
                draft_root.display()
            ),
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
        ExternalVerifierService::new(&self.workspace_root)?.create(&job)?;
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
            .find(|scope| !allowed.contains(&scope.file.as_str()))
        {
            return Err(AgentWorkflowError::UnavailableScope(scope.file.clone()));
        }
        state.status = BlindStatus::EchoAccepted;
        state.echoed_spec = Some(submission.echoed_spec.clone());
        state.model = submission.model;
        state.model_selection = submission.model_selection;
        self.save_blind_state(&state)?;
        Ok(submission.echoed_spec)
    }

    /// Commits a completed external typed verifier draft into repository Flect state.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is not bound to this repository or its verifier lifecycle
    /// has not completed successfully.
    pub fn verifier_commit(&self, job_id: &str) -> Result<EchoedSpec, AgentWorkflowError> {
        let external = ExternalVerifierService::new(&self.workspace_root)?;
        let draft = if external_draft_root(&self.workspace_root, job_id)
            .join("submitted")
            .is_file()
        {
            external.completed_filesystem(job_id)?
        } else {
            external.completed(job_id)?
        };
        let state = self.load_blind_state(job_id)?;
        if state.job.run_id != draft.run_id || state.status != BlindStatus::Prepared {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        self.submit_echo(BlindAgentSubmission {
            job_id: job_id.to_owned(),
            echoed_spec: draft.echoed_spec,
            model: draft.model,
            model_selection: draft.model_selection,
        })
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
        let mut blind = blind;
        blind.status = BlindStatus::JudgePrepared;
        self.save_blind_state(&blind)?;
        let run = RunStore::new(self.repository.root())
            .load_run(Some(&blind.job.run_id))
            .map_err(|error| AgentWorkflowError::RunState(error.to_string()))?;
        let job_id = generate_id("judge", &blind.job.run_id)?;
        let evidence_ref_contract = evidence_ref_contract(&blind.job.bundle, &echoed_spec);
        let draft_root = external_draft_root(&self.workspace_root, &job_id);
        let job = ReconciliationAgentJob {
            version: 1,
            job_id: job_id.clone(),
            run_id: blind.job.run_id.clone(),
            blind_job_id: blind.job.job_id.clone(),
            instructions: format!(
                "{JUDGE_INSTRUCTIONS} The assigned draft root is `{}`. The trusted parent submits only the bound job ID.",
                draft_root.display()
            ),
            intended_spec: run.intended_spec,
            echoed_spec,
            evidence_ref_contract,
        };
        self.save_reconciliation_state(&ReconciliationState {
            version: 1,
            status: ReconciliationStatus::Prepared,
            job: job.clone(),
            bundle: blind.job.bundle,
            blind_model: blind.model,
            blind_model_selection: blind.model_selection,
            draft: None,
        })?;
        create_judge_draft(&external_draft_root(&self.workspace_root, &job_id))?;
        Ok(job)
    }

    /// Starts a Flect-owned typed judge submission. No judge-authored JSON is accepted.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is missing or not prepared.
    pub fn judge_begin(
        &self,
        job_id: &str,
        model: Option<String>,
        model_selection: AgentModelSelection,
    ) -> Result<(), AgentWorkflowError> {
        let mut state = self.load_reconciliation_state(job_id)?;
        if state.status != ReconciliationStatus::Prepared || state.job.job_id != job_id {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        state.status = ReconciliationStatus::Collecting;
        state.draft = Some(JudgeDraft {
            alignment: None,
            confidence: None,
            findings: Vec::new(),
            side_effect_dispositions: Vec::new(),
            model,
            model_selection,
        });
        self.save_reconciliation_state(&state)
    }

    /// Sets alignment on an active typed judge submission.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is missing or not collecting.
    pub fn judge_set_alignment(
        &self,
        job_id: &str,
        alignment: Alignment,
    ) -> Result<(), AgentWorkflowError> {
        self.with_judge_draft(job_id, |draft| draft.alignment = Some(alignment))
    }

    /// Adds one semantic finding to an active typed judge submission.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is missing or not collecting.
    pub fn judge_add_finding(
        &self,
        job_id: &str,
        kind: FindingCategory,
        text: String,
        evidence_ref: Option<String>,
    ) -> Result<(), AgentWorkflowError> {
        self.with_judge_draft(job_id, |draft| {
            draft.findings.push(JudgeFinding {
                kind,
                text,
                evidence_ref,
            });
        })
    }

    /// Sets confidence on an active typed judge submission.
    ///
    /// # Errors
    ///
    /// Returns an error when the job is missing or not collecting.
    pub fn judge_set_confidence(
        &self,
        job_id: &str,
        confidence: f64,
    ) -> Result<(), AgentWorkflowError> {
        self.with_judge_draft(job_id, |draft| draft.confidence = Some(confidence))
    }

    /// Adds a distinct verifier-reported side effect as a typed finding.
    ///
    /// # Errors
    ///
    /// Returns an error for an unavailable or already dispositioned candidate.
    pub fn judge_add_side_effect_finding(
        &self,
        job_id: &str,
        candidate: String,
        text: String,
        evidence_ref: String,
    ) -> Result<(), AgentWorkflowError> {
        self.with_judge_state(job_id, |echoed_spec, draft| {
            let side_effect_index = side_effect_candidate_index(echoed_spec, &candidate)?;
            if draft
                .side_effect_dispositions
                .iter()
                .any(|disposition| disposition.candidate == candidate)
            {
                return Err(AgentWorkflowError::InvalidState(format!(
                    "side effect candidate `{candidate}` was already dispositioned"
                )));
            }
            let finding_index = draft.findings.len();
            draft.findings.push(JudgeFinding {
                kind: FindingCategory::PotentialSideEffects,
                text,
                evidence_ref: Some(evidence_ref),
            });
            draft.side_effect_dispositions.push(SideEffectDisposition {
                candidate,
                side_effect_index,
                kind: SideEffectDispositionKind::Finding { finding_index },
            });
            Ok(())
        })
    }

    /// Marks a verifier-reported side effect as not a distinct finding.
    ///
    /// # Errors
    ///
    /// Returns an error for an unavailable candidate, duplicate disposition, or empty reason.
    pub fn judge_mark_side_effect_not_distinct(
        &self,
        job_id: &str,
        candidate: String,
        reason: String,
    ) -> Result<(), AgentWorkflowError> {
        self.with_judge_state(job_id, |echoed_spec, draft| {
            let side_effect_index = side_effect_candidate_index(echoed_spec, &candidate)?;
            if reason.trim().is_empty() {
                return Err(AgentWorkflowError::InvalidState(
                    "side effect non-distinct reason must not be empty".to_owned(),
                ));
            }
            if draft
                .side_effect_dispositions
                .iter()
                .any(|disposition| disposition.candidate == candidate)
            {
                return Err(AgentWorkflowError::InvalidState(format!(
                    "side effect candidate `{candidate}` was already dispositioned"
                )));
            }
            draft.side_effect_dispositions.push(SideEffectDisposition {
                candidate,
                side_effect_index,
                kind: SideEffectDispositionKind::NotDistinct { reason },
            });
            Ok(())
        })
    }

    /// Validates Flect-owned typed semantic state, constructs the domain verdict, and persists it.
    ///
    /// # Errors
    ///
    /// Returns an error for incomplete state, invalid semantics, invalid evidence, or persistence failure.
    pub fn judge_submit(&self, job_id: &str) -> Result<VerificationRecord, AgentWorkflowError> {
        let mut state = self.load_reconciliation_state(job_id)?;
        if state.job.job_id != job_id {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let filesystem_submitted =
            is_regular_file(&external_draft_root(&self.workspace_root, job_id).join("submitted"));
        if state.status == ReconciliationStatus::Prepared && filesystem_submitted {
            state.status = ReconciliationStatus::Collecting;
            state.draft = Some(JudgeDraft {
                alignment: None,
                confidence: None,
                findings: Vec::new(),
                side_effect_dispositions: Vec::new(),
                model: None,
                model_selection: AgentModelSelection::Unknown,
            });
        }
        if state.status != ReconciliationStatus::Collecting {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let draft = if filesystem_submitted {
            read_judge_filesystem(
                &external_draft_root(&self.workspace_root, job_id),
                &state.job.echoed_spec,
            )?
        } else {
            state
                .draft
                .clone()
                .ok_or_else(|| AgentWorkflowError::InvalidJobState(job_id.to_owned()))?
        };
        validate_side_effect_dispositions(&draft, &state.job.echoed_spec)?;
        let verdict = JudgeVerdict {
            alignment: draft.alignment.ok_or_else(|| {
                AgentWorkflowError::InvalidState("judge alignment was not set".to_owned())
            })?,
            findings: draft.findings,
            confidence: draft.confidence.ok_or_else(|| {
                AgentWorkflowError::InvalidState("judge confidence was not set".to_owned())
            })?,
        };
        validate_typed_judge_invariants(&verdict)?;
        let verdict = materialize_judge_verdict(verdict, &state.bundle)?;
        let record = VerificationRecord {
            version: 1,
            run_id: state.job.run_id.clone(),
            bundle: state.bundle.clone(),
            echoed_spec: state.job.echoed_spec.clone(),
            verdict,
            isolation: IsolationLevel::Structural,
            model_calls: vec![
                agent_call(
                    "backward",
                    state.blind_model.clone(),
                    state.blind_model_selection,
                ),
                agent_call("reconciliation", draft.model.clone(), draft.model_selection),
            ],
            verified_unix_ms: unix_millis()?,
        };
        RunStore::new(self.repository.root())
            .save_verification(&record)
            .map_err(|error| AgentWorkflowError::RunState(error.to_string()))?;
        state.status = ReconciliationStatus::Completed;
        self.save_reconciliation_state(&state)?;
        let mut blind = self.load_blind_state(&state.job.blind_job_id)?;
        blind.status = BlindStatus::Completed;
        self.save_blind_state(&blind)?;
        if self.cleanup_on_complete {
            self.remove_owned_workspace(&blind.job.job_id)?;
        }
        Ok(record)
    }

    fn with_judge_draft(
        &self,
        job_id: &str,
        update: impl FnOnce(&mut JudgeDraft),
    ) -> Result<(), AgentWorkflowError> {
        self.with_judge_state(job_id, |_, draft| {
            update(draft);
            Ok(())
        })
    }

    fn with_judge_state(
        &self,
        job_id: &str,
        update: impl FnOnce(&EchoedSpec, &mut JudgeDraft) -> Result<(), AgentWorkflowError>,
    ) -> Result<(), AgentWorkflowError> {
        let mut state = self.load_reconciliation_state(job_id)?;
        if state.status != ReconciliationStatus::Collecting || state.job.job_id != job_id {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let draft = state
            .draft
            .as_mut()
            .ok_or_else(|| AgentWorkflowError::InvalidJobState(job_id.to_owned()))?;
        let echoed_spec = state.job.echoed_spec.clone();
        update(&echoed_spec, draft)?;
        self.save_reconciliation_state(&state)
    }

    /// Removes only verified Flect-owned job directories.
    ///
    /// Completed jobs are eligible by default. `include_all` intentionally discards
    /// unfinished forensic state; `older_than_hours` selects stale workspaces.
    ///
    /// # Errors
    ///
    /// Returns an error when state cannot be read or a candidate fails ownership checks.
    pub fn cleanup(&self, options: CleanupOptions) -> Result<CleanupReport, AgentWorkflowError> {
        let state_root = self.repository.root().join(".flect/agent/blind");
        if !state_root.exists() {
            return Ok(CleanupReport {
                dry_run: options.dry_run,
                deleted: Vec::new(),
                retained: Vec::new(),
            });
        }
        let cutoff = options
            .older_than_hours
            .map(|hours| SystemTime::now() - Duration::from_secs(hours.saturating_mul(3600)));
        let mut report = CleanupReport {
            dry_run: options.dry_run,
            deleted: Vec::new(),
            retained: Vec::new(),
        };
        for entry in
            fs::read_dir(&state_root).map_err(|source| workspace_error(&state_root, source))?
        {
            let path = entry
                .map_err(|source| workspace_error(&state_root, source))?
                .path();
            let Some(job_id) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            if validate_job_id(job_id).is_err() {
                continue;
            }
            let state = self.load_blind_state(job_id)?;
            let stale_workspace = cutoff.is_some_and(|cutoff| {
                fs::metadata(&state.job.workspace)
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .is_some_and(|modified| modified < cutoff)
            });
            if state.status == BlindStatus::Completed || options.include_all || stale_workspace {
                let removed = options.dry_run || self.remove_owned_workspace(job_id)?;
                if removed {
                    report.deleted.push(job_id.to_owned());
                }
            } else {
                report.retained.push(job_id.to_owned());
            }
        }
        Ok(report)
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
        let repository_root = canonical_existing(self.repository.root())?;
        let workspace_root = canonical_existing(&self.workspace_root)?;
        if workspace_root.starts_with(repository_root) {
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

    fn remove_owned_workspace(&self, job_id: &str) -> Result<bool, AgentWorkflowError> {
        validate_job_id(job_id)?;
        let repository_root = canonical_existing(self.repository.root())?;
        let workspace_root = canonical_existing(&self.workspace_root)?;
        if workspace_root.starts_with(&repository_root) {
            return Err(AgentWorkflowError::UnsafeWorkspace);
        }
        let workspace = workspace_root.join(job_id);
        if !workspace.exists() {
            return Ok(false);
        }
        let canonical = canonical_existing(&workspace)?;
        if canonical != workspace
            || !canonical.starts_with(&workspace_root)
            || canonical.parent() != Some(workspace_root.as_path())
        {
            return Err(AgentWorkflowError::UnsafeCleanup(
                workspace.display().to_string(),
            ));
        }
        #[cfg(windows)]
        make_writable(&canonical)?;
        fs::remove_dir_all(&canonical).map_err(|source| workspace_error(&canonical, source))?;
        Ok(true)
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

fn evidence_ref_contract(bundle: &BlindBundle, echoed_spec: &EchoedSpec) -> Value {
    let mut hunk_index = 0_u32;
    let files = bundle.patch.files.iter().map(|file| {
        let hunks = file.patch.split("@@ ").skip(1).filter_map(|part| {
            let hunk = format!("@@ {part}");
            let header = hunk.lines().next()?;
            let plus = header.split_whitespace().find(|part| part.starts_with('+'))?;
            let (start, count) = plus[1..].split_once(',').unwrap_or((&plus[1..], "1"));
            let start = start.parse::<u32>().ok()?;
            let count = count.parse::<u32>().ok()?;
            let id = format!("hunk/{hunk_index}");
            hunk_index = hunk_index.saturating_add(1);
            Some(serde_json::json!({"hunk_id": id, "hunk": hunk, "line_start": start, "line_end": start.saturating_add(count.saturating_sub(1))}))
        }).collect::<Vec<_>>();
        serde_json::json!({"file": file.path, "hunks": hunks})
    }).collect::<Vec<_>>();
    serde_json::json!({
        "version": 4,
        "finding_fields": ["kind", "text", "evidence_ref"],
        "forbidden_finding_fields": [
            "evidence",
            "file",
            "line",
            "line_start",
            "line_end",
            "patch_hunk",
            "finding_id",
            "finding_ids"
        ],
        "allowed_alignments": ["SAME", "PARTIAL", "DIFFERENT", "UNCERTAIN"],
        "available_finding_kinds": [
            "missing_requirement",
            "unrequested_change",
            "violated_constraint",
            "potential_side_effect"
        ],
        "alignment_meanings": {
            "SAME": "The apparent behavior fulfills the intended objective and material requirements without divergence.",
            "PARTIAL": "The apparent behavior advances at least one requested objective or requirement but has a missing requirement, violated constraint, added behavior, or scope divergence.",
            "DIFFERENT": "The apparent behavior is materially unrelated to or contradictory with the requested work; do not use DIFFERENT solely because an otherwise goal-advancing patch violates a constraint.",
            "UNCERTAIN": "The supplied IntendedSpec and EchoedSpec are insufficient for a semantic judgment."
        },
        "finding_kind_guidance": {
            "missing_requirement": "A requested requirement or acceptance condition is absent or not met.",
            "unrequested_change": "The patch adds, broadens, or changes behavior outside the objective, requirements, expected scope, or non-goals, even when the requested behavior is also present.",
            "violated_constraint": "The patch conflicts with an explicit constraint or task boundary.",
            "potential_side_effect": "A distinct plausible externally observable impact of an added, broadened, or constraint-violating behavior. When a supported unrequested change or violated constraint has a separately described consequence in EchoedSpec behavior_after or side_effects, emit both findings with the same evidence reference; do not treat one category as a substitute for the other."
        },
        "rules": [
            "submission_schema is the only judge-output schema; do not return a verdict wrapper or another object.",
            "Each finding may contain only kind, text, and optional evidence_ref. evidence, file, line, patch_hunk, finding_id, and persisted evidence fields are forbidden.",
            "evidence_ref, when present, must equal one listed stable hunk ID.",
            "SAME requires findings to be empty; PARTIAL and DIFFERENT require at least one finding.",
            "Do not return SAME merely because a requested change is present; SAME requires no supported divergence from the full IntendedSpec.",
            "DIFFERENT requires at least one missing_requirement or unrequested_change finding; violated_constraint and potential_side_effect findings alone do not establish objective mismatch.",
            "Every side_effect candidate must be dispositioned before submit. Use a typed side-effect finding with valid evidence only for a downstream consequence beyond the base behavior itself, such as caller migration or newly exposed sensitive data. Logging request-derived identifiers, keys, or values is both an unrequested change and a potential disclosure/observability side effect. A changed API, added output, wrong-component behavior, or altered label itself remains only the base category. A downstream consequence may share a cause or hunk with the base divergence.",
            "confidence is required and must be a number from 0 through 1."
        ],
        "finding_example": files.iter().find_map(|file| file["hunks"].as_array().and_then(|hunks| hunks.first()).map(|hunk| serde_json::json!({"kind": "violated_constraint", "text": "The changed setting violates the constraint.", "evidence_ref": hunk["hunk_id"]}))),
        "files": files,
        "side_effect_candidates": echoed_spec.side_effects.iter().enumerate().map(|(index, text)| serde_json::json!({"id": format!("side_effect/{index}"), "text": text})).collect::<Vec<_>>()
    })
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

/// Repository-independent typed verifier commands for a prepared blind job.
pub struct ExternalVerifierService {
    workspace_root: PathBuf,
}

#[allow(clippy::missing_errors_doc)]
impl ExternalVerifierService {
    /// Opens Flect's external agent-workspace root without repository discovery.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace root cannot be safely resolved.
    pub fn discover() -> Result<Self, AgentWorkflowError> {
        Self::new(&std::env::temp_dir().join("flect-agent-jobs"))
    }

    /// Opens an explicit external verifier workspace root for embedding and tests.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace root cannot be safely resolved.
    pub fn new_for_tests(workspace_root: &Path) -> Result<Self, AgentWorkflowError> {
        Self::new(workspace_root)
    }

    fn new(workspace_root: &Path) -> Result<Self, AgentWorkflowError> {
        let workspace_root = resolve_with_missing(workspace_root)?;
        Ok(Self { workspace_root })
    }

    fn create(&self, job: &BlindAgentJob) -> Result<(), AgentWorkflowError> {
        let draft = external_draft_root(&self.workspace_root, &job.job_id);
        fs::create_dir_all(draft.join("behavior_before"))
            .map_err(|source| workspace_error(&draft, source))?;
        fs::create_dir_all(draft.join("behavior_after"))
            .map_err(|source| workspace_error(&draft, source))?;
        fs::create_dir_all(draft.join("affected_scope"))
            .map_err(|source| workspace_error(&draft, source))?;
        fs::create_dir_all(draft.join("side_effects"))
            .map_err(|source| workspace_error(&draft, source))?;
        fs::create_dir_all(draft.join("assumptions"))
            .map_err(|source| workspace_error(&draft, source))?;
        fs::create_dir_all(draft.join("uncertainties"))
            .map_err(|source| workspace_error(&draft, source))?;
        let state = ExternalVerifierState {
            version: 1,
            status: VerifierStatus::Prepared,
            job_id: job.job_id.clone(),
            run_id: job.run_id.clone(),
            allowed_paths: job
                .bundle
                .patch
                .files
                .iter()
                .map(|file| file.path.clone())
                .chain(job.bundle.context.iter().map(|file| file.path.clone()))
                .collect(),
            draft: None,
        };
        self.save(&state)
    }

    /// Begins Flect-owned verifier collection.
    pub fn begin(
        &self,
        job_id: &str,
        model: Option<String>,
        model_selection: AgentModelSelection,
    ) -> Result<(), AgentWorkflowError> {
        let mut state = self.load(job_id)?;
        if state.status != VerifierStatus::Prepared {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        state.status = VerifierStatus::Collecting;
        state.draft = Some(VerifierDraft {
            objective: None,
            before: Vec::new(),
            after: Vec::new(),
            scope: Vec::new(),
            side_effects: Vec::new(),
            assumptions: Vec::new(),
            uncertainties: Vec::new(),
            confidence: None,
            model,
            model_selection,
        });
        self.save(&state)
    }

    /// Sets the apparent objective from a verifier-owned semantic value.
    pub fn set_objective(&self, job_id: &str, text: String) -> Result<(), AgentWorkflowError> {
        self.update(job_id, |_state, draft| {
            draft.objective = Some(text);
            Ok(())
        })
    }

    /// Adds one typed text-list semantic value.
    pub fn add_text(
        &self,
        job_id: &str,
        field: VerifierTextField,
        text: String,
    ) -> Result<(), AgentWorkflowError> {
        self.update(job_id, |_state, draft| {
            match field {
                VerifierTextField::Before => draft.before.push(text),
                VerifierTextField::After => draft.after.push(text),
                VerifierTextField::SideEffect => draft.side_effects.push(text),
                VerifierTextField::Assumption => draft.assumptions.push(text),
                VerifierTextField::Uncertainty => draft.uncertainties.push(text),
            }
            Ok(())
        })
    }

    /// Adds an allowed affected scope at the typed boundary.
    pub fn add_scope(
        &self,
        job_id: &str,
        file: String,
        symbol: Option<String>,
    ) -> Result<(), AgentWorkflowError> {
        self.update(job_id, |state, draft| {
            if !state.allowed_paths.contains(&file) {
                return Err(AgentWorkflowError::UnavailableScope(file));
            }
            draft.scope.push(flect_core::AffectedScope { file, symbol });
            Ok(())
        })
    }

    /// Sets verifier confidence after finite range validation.
    pub fn set_confidence(&self, job_id: &str, confidence: f64) -> Result<(), AgentWorkflowError> {
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(AgentWorkflowError::InvalidState(
                "verifier confidence must be finite and between zero and one".to_owned(),
            ));
        }
        self.update(job_id, |_state, draft| {
            draft.confidence = Some(confidence);
            Ok(())
        })
    }

    /// Validates and seals one typed verifier draft without repository access.
    pub fn submit(&self, job_id: &str) -> Result<(), AgentWorkflowError> {
        let mut state = self.load(job_id)?;
        if state.status != VerifierStatus::Collecting {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let draft = state
            .draft
            .as_ref()
            .ok_or_else(|| AgentWorkflowError::InvalidJobState(job_id.to_owned()))?;
        if draft.objective.is_none() || draft.confidence.is_none() {
            return Err(AgentWorkflowError::InvalidState(
                "verifier objective and confidence must be set".to_owned(),
            ));
        }
        state.status = VerifierStatus::Submitted;
        self.save(&state)
    }

    fn completed(&self, job_id: &str) -> Result<CompletedVerifierDraft, AgentWorkflowError> {
        let state = self.load(job_id)?;
        if state.status != VerifierStatus::Submitted {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let draft = state
            .draft
            .ok_or_else(|| AgentWorkflowError::InvalidJobState(job_id.to_owned()))?;
        Ok(CompletedVerifierDraft {
            run_id: state.run_id,
            echoed_spec: EchoedSpec {
                apparent_objective: draft.objective.unwrap_or_default(),
                behavior_before: draft.before,
                behavior_after: draft.after,
                affected_scope: draft.scope,
                side_effects: draft.side_effects,
                assumptions: draft.assumptions,
                uncertainties: draft.uncertainties,
                confidence: draft.confidence.unwrap_or_default(),
            },
            model: draft.model,
            model_selection: draft.model_selection,
        })
    }

    fn completed_filesystem(
        &self,
        job_id: &str,
    ) -> Result<CompletedVerifierDraft, AgentWorkflowError> {
        let state = self.load(job_id)?;
        if state.status != VerifierStatus::Prepared {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let root = external_draft_root(&self.workspace_root, job_id);
        require_marker(&root.join("submitted"))?;
        let objective = read_required_text(&root.join("objective"))?;
        let confidence = read_confidence(&root.join("confidence"))?;
        let model = read_required_text(&root.join("model"))?;
        let model_selection = read_model_selection(&root.join("model_selection"))?;
        let before = read_numbered_texts(&root.join("behavior_before"))?;
        let after = read_numbered_texts(&root.join("behavior_after"))?;
        let side_effects = read_numbered_texts(&root.join("side_effects"))?;
        let assumptions = read_numbered_texts(&root.join("assumptions"))?;
        let uncertainties = read_numbered_texts(&root.join("uncertainties"))?;
        let scope = read_scopes(&root.join("affected_scope"), &state.allowed_paths)?;
        let allowed = [
            "objective",
            "confidence",
            "model",
            "model_selection",
            "submitted",
            "behavior_before",
            "behavior_after",
            "affected_scope",
            "side_effects",
            "assumptions",
            "uncertainties",
        ];
        reject_unexpected_entries(&root, &allowed)?;
        Ok(CompletedVerifierDraft {
            run_id: state.run_id,
            echoed_spec: EchoedSpec {
                apparent_objective: objective,
                behavior_before: before,
                behavior_after: after,
                affected_scope: scope,
                side_effects,
                assumptions,
                uncertainties,
                confidence,
            },
            model: Some(model),
            model_selection,
        })
    }

    fn update(
        &self,
        job_id: &str,
        update: impl FnOnce(
            &mut ExternalVerifierState,
            &mut VerifierDraft,
        ) -> Result<(), AgentWorkflowError>,
    ) -> Result<(), AgentWorkflowError> {
        let mut state = self.load(job_id)?;
        if state.status != VerifierStatus::Collecting {
            return Err(AgentWorkflowError::InvalidJobState(job_id.to_owned()));
        }
        let mut draft = state
            .draft
            .take()
            .ok_or_else(|| AgentWorkflowError::InvalidJobState(job_id.to_owned()))?;
        update(&mut state, &mut draft)?;
        state.draft = Some(draft);
        self.save(&state)
    }

    fn state_path(&self, job_id: &str) -> Result<PathBuf, AgentWorkflowError> {
        validate_job_id(job_id)?;
        Ok(self.workspace_root.join(format!("{job_id}.verifier.json")))
    }

    fn load(&self, job_id: &str) -> Result<ExternalVerifierState, AgentWorkflowError> {
        read_state(&self.state_path(job_id)?, job_id)
    }

    fn save(&self, state: &ExternalVerifierState) -> Result<(), AgentWorkflowError> {
        write_state(&self.state_path(&state.job_id)?, state)
    }
}

#[derive(Clone, Copy)]
pub enum VerifierTextField {
    Before,
    After,
    SideEffect,
    Assumption,
    Uncertainty,
}

#[derive(Serialize, Deserialize)]
struct ExternalVerifierState {
    version: u32,
    status: VerifierStatus,
    job_id: String,
    run_id: String,
    allowed_paths: Vec<String>,
    draft: Option<VerifierDraft>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VerifierStatus {
    Prepared,
    Collecting,
    Submitted,
}

#[derive(Serialize, Deserialize)]
struct VerifierDraft {
    objective: Option<String>,
    before: Vec<String>,
    after: Vec<String>,
    scope: Vec<flect_core::AffectedScope>,
    side_effects: Vec<String>,
    assumptions: Vec<String>,
    uncertainties: Vec<String>,
    confidence: Option<f64>,
    model: Option<String>,
    model_selection: AgentModelSelection,
}

struct CompletedVerifierDraft {
    run_id: String,
    echoed_spec: EchoedSpec,
    model: Option<String>,
    model_selection: AgentModelSelection,
}

fn external_draft_root(workspace_root: &Path, job_id: &str) -> PathBuf {
    workspace_root.join(job_id).join("draft")
}

fn read_required_text(path: &Path) -> Result<String, AgentWorkflowError> {
    if !is_regular_file(path) {
        return Err(AgentWorkflowError::InvalidState(format!(
            "missing draft file {}",
            path.display()
        )));
    }
    let text = fs::read_to_string(path).map_err(|source| workspace_error(path, source))?;
    if text.trim().is_empty() {
        return Err(AgentWorkflowError::InvalidState(format!(
            "empty draft file {}",
            path.display()
        )));
    }
    Ok(text)
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn require_marker(path: &Path) -> Result<(), AgentWorkflowError> {
    if !is_regular_file(path) {
        return Err(AgentWorkflowError::InvalidState(format!(
            "missing marker {}",
            path.display()
        )));
    }
    let contents = fs::read(path).map_err(|source| workspace_error(path, source))?;
    if !contents.is_empty() {
        return Err(AgentWorkflowError::InvalidState(format!(
            "marker must be empty {}",
            path.display()
        )));
    }
    Ok(())
}

fn read_confidence(path: &Path) -> Result<f64, AgentWorkflowError> {
    let text = read_required_text(path)?;
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| AgentWorkflowError::InvalidState("invalid confidence".to_owned()))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(AgentWorkflowError::InvalidState(
            "invalid confidence".to_owned(),
        ));
    }
    Ok(value)
}

fn read_model_selection(path: &Path) -> Result<AgentModelSelection, AgentWorkflowError> {
    match read_required_text(path)?.trim() {
        "explicit" => Ok(AgentModelSelection::Explicit),
        "inherited" => Ok(AgentModelSelection::Inherited),
        "unknown" => Ok(AgentModelSelection::Unknown),
        _ => Err(AgentWorkflowError::InvalidState(
            "invalid model selection".to_owned(),
        )),
    }
}

fn read_numbered_texts(dir: &Path) -> Result<Vec<String>, AgentWorkflowError> {
    if !is_real_dir(dir) {
        return Err(AgentWorkflowError::InvalidState(format!(
            "missing draft directory {}",
            dir.display()
        )));
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|source| workspace_error(dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| workspace_error(dir, source))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut values = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != format!("{index:06}.txt") {
            return Err(AgentWorkflowError::InvalidState(
                "invalid or gapped draft entry".to_owned(),
            ));
        }
        values.push(read_required_text(&entry.path())?);
    }
    Ok(values)
}

fn read_scopes(
    dir: &Path,
    allowed: &[String],
) -> Result<Vec<flect_core::AffectedScope>, AgentWorkflowError> {
    if !is_real_dir(dir) {
        return Err(AgentWorkflowError::InvalidState(format!(
            "missing draft directory {}",
            dir.display()
        )));
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|source| workspace_error(dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| workspace_error(dir, source))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut scopes = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        if entry.file_name().to_string_lossy() != format!("{index:06}")
            || !is_real_dir(&entry.path())
        {
            return Err(AgentWorkflowError::InvalidState(
                "invalid scope entry".to_owned(),
            ));
        }
        let scope = entry.path();
        let file = read_required_text(&scope.join("file"))?;
        if !allowed.contains(&file) {
            return Err(AgentWorkflowError::UnavailableScope(file));
        }
        let symbol_path = scope.join("symbol");
        let symbol = if symbol_path.exists() {
            Some(read_required_text(&symbol_path)?)
        } else {
            None
        };
        reject_unexpected_entries(&scope, &["file", "symbol"])?;
        scopes.push(flect_core::AffectedScope { file, symbol });
    }
    Ok(scopes)
}

fn reject_unexpected_entries(dir: &Path, allowed: &[&str]) -> Result<(), AgentWorkflowError> {
    for entry in fs::read_dir(dir).map_err(|source| workspace_error(dir, source))? {
        let entry = entry.map_err(|source| workspace_error(dir, source))?;
        if !allowed.contains(&entry.file_name().to_string_lossy().as_ref()) {
            return Err(AgentWorkflowError::InvalidState(format!(
                "unexpected draft entry {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn create_judge_draft(root: &Path) -> Result<(), AgentWorkflowError> {
    for dir in ["alignment", "findings", "side_effect_dispositions"] {
        fs::create_dir_all(root.join(dir)).map_err(|source| workspace_error(root, source))?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn read_judge_filesystem(
    root: &Path,
    echoed: &EchoedSpec,
) -> Result<JudgeDraft, AgentWorkflowError> {
    let submitted = root.join("submitted");
    require_marker(&submitted)?;
    reject_unexpected_entries(
        root,
        &[
            "alignment",
            "confidence",
            "model",
            "model_selection",
            "findings",
            "side_effect_dispositions",
            "submitted",
        ],
    )?;
    let align_dir = root.join("alignment");
    if !is_real_dir(&align_dir) {
        return Err(AgentWorkflowError::InvalidState(
            "missing alignment directory".to_owned(),
        ));
    }
    let markers = fs::read_dir(&align_dir)
        .map_err(|source| workspace_error(&align_dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| workspace_error(&align_dir, source))?;
    if markers.len() != 1 {
        return Err(AgentWorkflowError::InvalidState(
            "judge alignment must have one marker".to_owned(),
        ));
    }
    if !is_real_dir(&markers[0].path()) {
        return Err(AgentWorkflowError::InvalidState(
            "alignment marker must be a directory".to_owned(),
        ));
    }
    let alignment = match markers[0].file_name().to_string_lossy().as_ref() {
        "SAME" => Alignment::Same,
        "PARTIAL" => Alignment::Partial,
        "DIFFERENT" => Alignment::Different,
        "UNCERTAIN" => Alignment::Uncertain,
        _ => {
            return Err(AgentWorkflowError::InvalidState(
                "invalid judge alignment".to_owned(),
            ));
        }
    };
    reject_unexpected_entries(&align_dir, &["SAME", "PARTIAL", "DIFFERENT", "UNCERTAIN"])?;
    let confidence = read_confidence(&root.join("confidence"))?;
    let model = read_required_text(&root.join("model"))?;
    let model_selection = read_model_selection(&root.join("model_selection"))?;
    let findings_dir = root.join("findings");
    let mut findings = Vec::new();
    let mut entries = fs::read_dir(&findings_dir)
        .map_err(|source| workspace_error(&findings_dir, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| workspace_error(&findings_dir, source))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for (index, entry) in entries.into_iter().enumerate() {
        if entry.file_name().to_string_lossy() != format!("{index:06}")
            || !is_real_dir(&entry.path())
        {
            return Err(AgentWorkflowError::InvalidState(
                "invalid finding entry".to_owned(),
            ));
        }
        let dir = entry.path();
        let kind_dirs = fs::read_dir(&dir)
            .map_err(|source| workspace_error(&dir, source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| workspace_error(&dir, source))?
            .into_iter()
            .filter(|entry| is_real_dir(&entry.path()))
            .collect::<Vec<_>>();
        if kind_dirs.len() != 1 {
            return Err(AgentWorkflowError::InvalidState(
                "finding must have one kind marker".to_owned(),
            ));
        }
        let kind = match kind_dirs[0].file_name().to_string_lossy().as_ref() {
            "missing_requirement" => FindingCategory::MissingRequirements,
            "unrequested_change" => FindingCategory::UnrequestedChanges,
            "violated_constraint" => FindingCategory::ViolatedConstraints,
            "potential_side_effect" => FindingCategory::PotentialSideEffects,
            _ => {
                return Err(AgentWorkflowError::InvalidState(
                    "invalid finding kind".to_owned(),
                ));
            }
        };
        let text = read_required_text(&dir.join("text"))?;
        let evidence_path = dir.join("evidence_ref");
        let evidence_ref = if evidence_path.exists() {
            Some(read_required_text(&evidence_path)?)
        } else {
            None
        };
        reject_unexpected_entries(
            &dir,
            &[
                "missing_requirement",
                "unrequested_change",
                "violated_constraint",
                "potential_side_effect",
                "text",
                "evidence_ref",
            ],
        )?;
        findings.push(JudgeFinding {
            kind,
            text,
            evidence_ref,
        });
    }
    let mut side_effect_dispositions = Vec::new();
    for index in 0..echoed.side_effects.len() {
        let candidate = root
            .join("side_effect_dispositions")
            .join(format!("side_effect/{index}"));
        if !is_real_dir(&candidate) {
            return Err(AgentWorkflowError::InvalidState(format!(
                "missing disposition side_effect/{index}"
            )));
        }
        let branches = fs::read_dir(&candidate)
            .map_err(|source| workspace_error(&candidate, source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| workspace_error(&candidate, source))?;
        if branches.len() != 1 || !is_real_dir(&branches[0].path()) {
            return Err(AgentWorkflowError::InvalidState(
                "invalid side effect disposition".to_owned(),
            ));
        }
        let branch = branches[0].file_name().to_string_lossy().to_string();
        if branch == "finding" {
            let text = read_required_text(&candidate.join("finding/text"))?;
            let evidence = read_required_text(&candidate.join("finding/evidence_ref"))?;
            reject_unexpected_entries(&candidate.join("finding"), &["text", "evidence_ref"])?;
            let finding_index = findings.len();
            findings.push(JudgeFinding {
                kind: FindingCategory::PotentialSideEffects,
                text,
                evidence_ref: Some(evidence),
            });
            side_effect_dispositions.push(SideEffectDisposition {
                candidate: format!("side_effect/{index}"),
                side_effect_index: index,
                kind: SideEffectDispositionKind::Finding { finding_index },
            });
        } else if branch == "not-distinct" {
            let reason = read_required_text(&candidate.join("not-distinct/reason"))?;
            reject_unexpected_entries(&candidate.join("not-distinct"), &["reason"])?;
            side_effect_dispositions.push(SideEffectDisposition {
                candidate: format!("side_effect/{index}"),
                side_effect_index: index,
                kind: SideEffectDispositionKind::NotDistinct { reason },
            });
        } else {
            return Err(AgentWorkflowError::InvalidState(
                "invalid side effect disposition branch".to_owned(),
            ));
        }
    }
    let disposition_root = root.join("side_effect_dispositions");
    if !is_real_dir(&disposition_root) {
        return Err(AgentWorkflowError::InvalidState(
            "missing side effect disposition directory".to_owned(),
        ));
    }
    let side_effect_root = disposition_root.join("side_effect");
    if echoed.side_effects.is_empty() {
        if side_effect_root.exists() {
            return Err(AgentWorkflowError::InvalidState(
                "unexpected side effect disposition entries".to_owned(),
            ));
        }
        return Ok(JudgeDraft {
            alignment: Some(alignment),
            confidence: Some(confidence),
            findings,
            side_effect_dispositions,
            model: Some(model),
            model_selection,
        });
    }
    if !is_real_dir(&side_effect_root) {
        return Err(AgentWorkflowError::InvalidState(
            "missing side effect disposition entries".to_owned(),
        ));
    }
    let disposition_entries = fs::read_dir(&side_effect_root)
        .map_err(|source| workspace_error(&side_effect_root, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| workspace_error(&side_effect_root, source))?;
    if disposition_entries.len() != echoed.side_effects.len() {
        return Err(AgentWorkflowError::InvalidState(
            "unexpected side effect disposition".to_owned(),
        ));
    }
    reject_unexpected_entries(&disposition_root, &["side_effect"])?;
    Ok(JudgeDraft {
        alignment: Some(alignment),
        confidence: Some(confidence),
        findings,
        side_effect_dispositions,
        model: Some(model),
        model_selection,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlindStatus {
    Prepared,
    EchoAccepted,
    JudgePrepared,
    Completed,
    Failed,
    Abandoned,
}

#[derive(Serialize, Deserialize)]
struct ReconciliationState {
    version: u32,
    status: ReconciliationStatus,
    job: ReconciliationAgentJob,
    bundle: BlindBundle,
    blind_model: Option<String>,
    blind_model_selection: flect_core::AgentModelSelection,
    draft: Option<JudgeDraft>,
}

#[derive(Clone, Serialize, Deserialize)]
struct JudgeDraft {
    alignment: Option<Alignment>,
    confidence: Option<f64>,
    findings: Vec<JudgeFinding>,
    side_effect_dispositions: Vec<SideEffectDisposition>,
    model: Option<String>,
    model_selection: AgentModelSelection,
}

#[derive(Clone, Serialize, Deserialize)]
struct SideEffectDisposition {
    candidate: String,
    side_effect_index: usize,
    kind: SideEffectDispositionKind,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum SideEffectDispositionKind {
    Finding { finding_index: usize },
    NotDistinct { reason: String },
}

fn side_effect_candidate_index(
    echoed_spec: &EchoedSpec,
    candidate: &str,
) -> Result<usize, AgentWorkflowError> {
    let Some(index) = candidate
        .strip_prefix("side_effect/")
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return Err(AgentWorkflowError::InvalidState(format!(
            "invalid side effect candidate `{candidate}`"
        )));
    };
    if echoed_spec.side_effects.get(index).is_none() {
        return Err(AgentWorkflowError::InvalidState(format!(
            "unavailable side effect candidate `{candidate}`"
        )));
    }
    Ok(index)
}

fn validate_side_effect_dispositions(
    draft: &JudgeDraft,
    echoed_spec: &EchoedSpec,
) -> Result<(), AgentWorkflowError> {
    for index in 0..echoed_spec.side_effects.len() {
        let candidate = format!("side_effect/{index}");
        let disposition = draft
            .side_effect_dispositions
            .iter()
            .find(|disposition| disposition.candidate == candidate)
            .ok_or_else(|| {
                AgentWorkflowError::InvalidState(format!(
                    "verifier side effect candidate `{candidate}` has no disposition"
                ))
            })?;
        if disposition.side_effect_index != index {
            return Err(AgentWorkflowError::InvalidState(format!(
                "side effect candidate `{candidate}` has an invalid disposition"
            )));
        }
        if let SideEffectDispositionKind::Finding { finding_index } = disposition.kind {
            let finding = draft.findings.get(finding_index).ok_or_else(|| {
                AgentWorkflowError::InvalidState(format!(
                    "side effect candidate `{candidate}` has no linked finding"
                ))
            })?;
            if finding.kind != FindingCategory::PotentialSideEffects
                || finding.evidence_ref.as_deref().is_none_or(str::is_empty)
            {
                return Err(AgentWorkflowError::InvalidState(format!(
                    "side effect candidate `{candidate}` requires a potential side effect finding with evidence"
                )));
            }
        }
    }
    Ok(())
}

fn validate_typed_judge_invariants(verdict: &JudgeVerdict) -> Result<(), AgentWorkflowError> {
    if verdict.alignment == Alignment::Different
        && !verdict.findings.iter().any(|finding| {
            matches!(
                finding.kind,
                FindingCategory::MissingRequirements | FindingCategory::UnrequestedChanges
            )
        })
    {
        return Err(AgentWorkflowError::DifferentWithoutObjectiveMismatch);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReconciliationStatus {
    Prepared,
    Collecting,
    Completed,
    Failed,
    Abandoned,
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

#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn make_writable(path: &Path) -> Result<(), AgentWorkflowError> {
    for entry in fs::read_dir(path).map_err(|source| workspace_error(path, source))? {
        let path = entry
            .map_err(|source| workspace_error(path, source))?
            .path();
        if path.is_dir() {
            make_writable(&path)?;
        } else {
            let mut permissions = fs::metadata(&path)
                .map_err(|source| workspace_error(&path, source))?
                .permissions();
            permissions.set_readonly(false);
            fs::set_permissions(&path, permissions)
                .map_err(|source| workspace_error(&path, source))?;
        }
    }
    Ok(())
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

fn resolve_with_missing(path: &Path) -> Result<PathBuf, AgentWorkflowError> {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or(AgentWorkflowError::UnsafeWorkspace)?;
        missing.push(name.to_owned());
        existing = existing
            .parent()
            .ok_or(AgentWorkflowError::UnsafeWorkspace)?;
    }
    let mut resolved = canonical_existing(existing)?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
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
