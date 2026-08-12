use std::fs;
use std::path::Path;
use std::process::Command;

use flect_app::{AgentService, AgentWorkflowError, CleanupOptions};
use flect_core::{
    AffectedScope, AgentModelSelection, Alignment, BlindAgentSubmission, EchoedSpec,
    FindingCategory, GitRepository, IntendedSpec, JudgeFinding, JudgeVerdict,
    ReconciliationAgentSubmission, RunRecord, RunStore, TaskInput,
};

const TASK_SENTINEL: &str = "ORIGINAL_TASK_SECRET_7F91";
const FORWARD_SENTINEL: &str = "FORWARD_SPEC_SECRET_29AB";
const COMMIT_SENTINEL: &str = "COMMIT_SECRET_8821";
const CONVERSATION_SENTINEL: &str = "CONVERSATION_SECRET_D401";

#[test]
fn complete_agent_handoff_is_blind_validated_and_persisted() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();

    let blind = service.prepare_blind(None, None).unwrap();
    let blind_workspace = blind.workspace.clone();
    let serialized = serde_json::to_string(&blind).unwrap();
    for forbidden in [
        TASK_SENTINEL,
        FORWARD_SENTINEL,
        COMMIT_SENTINEL,
        CONVERSATION_SENTINEL,
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
    assert_eq!(blind.isolation, flect_core::IsolationLevel::Structural);
    assert!(
        blind
            .bundle
            .patch
            .files
            .iter()
            .all(|file| { !file.path.starts_with(".git") && !file.path.starts_with(".flect") })
    );
    assert_workspace_is_sanitized(Path::new(&blind.workspace));

    let echoed = EchoedSpec {
        apparent_objective: "Reject disabled accounts".to_owned(),
        behavior_after: vec!["Disabled accounts are rejected".to_owned()],
        affected_scope: vec![AffectedScope {
            file: "app.txt".to_owned(),
            symbol: None,
        }],
        confidence: 0.9,
        ..EchoedSpec::default()
    };
    service
        .submit_echo(BlindAgentSubmission {
            job_id: blind.job_id.clone(),
            echoed_spec: echoed.clone(),
            model: None,
            model_selection: AgentModelSelection::Inherited,
        })
        .unwrap();
    let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
    assert_ne!(judge.job_id, blind.job_id);
    assert_eq!(judge.echoed_spec, echoed);
    assert!(judge.intended_spec.objective.contains(FORWARD_SENTINEL));

    let record = service
        .submit_verdict(ReconciliationAgentSubmission {
            job_id: judge.job_id,
            verdict: JudgeVerdict {
                alignment: Alignment::Same,
                findings: Vec::new(),
                confidence: 0.9,
            },
            model: Some("runtime-inherited".to_owned()),
            model_selection: AgentModelSelection::Inherited,
        })
        .unwrap();
    assert_eq!(record.isolation, flect_core::IsolationLevel::Structural);
    assert_eq!(record.model_calls.len(), 2);
    assert_eq!(record.model_calls[0].provider, "codex-native");
    assert_eq!(
        RunStore::new(repository.path())
            .load_verification(None)
            .unwrap(),
        record
    );
    assert!(!Path::new(&blind_workspace).exists());
}

#[test]
fn accepts_structured_scope_and_exposes_judge_evidence_contract() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();
    service
        .submit_echo(BlindAgentSubmission {
            job_id: blind.job_id.clone(),
            echoed_spec: EchoedSpec {
                affected_scope: vec![AffectedScope {
                    file: "app.txt".to_owned(),
                    symbol: Some("normalize_name".to_owned()),
                }],
                confidence: 0.9,
                ..EchoedSpec::default()
            },
            model: None,
            model_selection: AgentModelSelection::Explicit,
        })
        .unwrap();
    let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
    assert_eq!(judge.evidence_contract["version"], 2);
    assert!(
        judge.evidence_contract["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().unwrap().contains("finding ID"))
    );
    assert_eq!(judge.evidence_contract["files"][0]["file"], "app.txt");
    assert!(
        judge.evidence_contract["files"][0]["hunks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hunk| hunk["hunk"].as_str().unwrap().starts_with("@@ "))
    );
    assert_eq!(
        judge.evidence_contract["files"][0]["hunks"][0]["hunk_id"],
        "hunk/0"
    );
}

#[test]
fn rejects_the_four_observed_judge_wrapper_shapes() {
    let raw = [
        serde_json::json!({"verdict": "PARTIAL", "evidence": {"finding_ids": ["missing_requirements/0"]}}),
        serde_json::json!({"verdict": "PARTIAL", "findings": []}),
        serde_json::json!({"verdict": "PARTIAL", "summary": "scope creep", "evidence": {}}),
        serde_json::json!({"verdict": "DIFFERENT", "summary": "semantic mismatch", "evidence": {}}),
    ];
    for payload in raw {
        assert!(serde_json::from_value::<JudgeVerdict>(payload).is_err());
    }
}

#[test]
fn rejects_observed_malformed_compact_judge_shapes() {
    let raw = [
        serde_json::json!({"alignment": "PARTIAL", "findings": []}),
        serde_json::json!({"alignment": "PARTIAL", "confidence": 0.9}),
        serde_json::json!({"alignment": "PARTIAL", "findings": [], "confidence": 0.9, "summary": "extra"}),
        serde_json::json!({"alignment": "PARTIAL", "findings": [{"kind": "missing_requirements", "text": "x"}], "confidence": 0.9}),
        serde_json::json!({"alignment": "PARTIAL", "findings": [{"kind": "missing_requirement", "text": "x"}], "confidence": "high"}),
    ];
    for payload in raw {
        assert!(serde_json::from_value::<JudgeVerdict>(payload).is_err());
    }
}

#[test]
fn rejects_semantically_incompatible_compact_judge_output() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();
    let bundle = service.prepare_blind(None, None).unwrap().bundle;

    for verdict in [
        JudgeVerdict {
            alignment: Alignment::Same,
            findings: vec![JudgeFinding {
                kind: FindingCategory::UnrequestedChanges,
                text: "unexpected change".to_owned(),
                evidence_ref: None,
            }],
            confidence: 0.9,
        },
        JudgeVerdict {
            alignment: Alignment::Partial,
            findings: Vec::new(),
            confidence: 0.9,
        },
        JudgeVerdict {
            alignment: Alignment::Different,
            findings: Vec::new(),
            confidence: 1.1,
        },
        JudgeVerdict {
            alignment: Alignment::Partial,
            findings: vec![JudgeFinding {
                kind: FindingCategory::MissingRequirements,
                text: "  ".to_owned(),
                evidence_ref: None,
            }],
            confidence: 0.9,
        },
    ] {
        assert!(flect_app::materialize_judge_verdict(verdict, &bundle).is_err());
    }
}

#[test]
fn rejects_fabricated_agent_facing_evidence_fields() {
    for evidence in [
        serde_json::json!({"kind": "missing_requirement", "text": "x", "file": "invented.rs"}),
        serde_json::json!({"kind": "missing_requirement", "text": "x", "line_start": 99}),
        serde_json::json!({"finding_ids": ["missing_requirements/99"], "description": "x"}),
    ] {
        assert!(serde_json::from_value::<JudgeFinding>(evidence).is_err());
    }
}

#[test]
fn cleanup_retains_unfinished_jobs_and_removes_only_owned_completed_jobs() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let jobs = workspace.path().join("jobs");
    let sibling = workspace.path().join("unrelated");
    fs::create_dir(&sibling).unwrap();
    fs::write(sibling.join("keep.txt"), "keep").unwrap();
    let service = AgentService::with_workspace_root_and_cleanup(
        GitRepository::discover(repository.path()).unwrap(),
        jobs,
        false,
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();

    let report = service.cleanup(CleanupOptions::default()).unwrap();
    assert!(report.deleted.is_empty());
    assert_eq!(report.retained, vec![blind.job_id.clone()]);
    assert!(Path::new(&blind.workspace).exists());

    let dry_run = service
        .cleanup(flect_app::CleanupOptions {
            dry_run: true,
            include_all: true,
            older_than_hours: None,
        })
        .unwrap();
    assert_eq!(dry_run.deleted, vec![blind.job_id.clone()]);
    assert!(Path::new(&blind.workspace).exists());

    let deleted = service
        .cleanup(flect_app::CleanupOptions {
            dry_run: false,
            include_all: true,
            older_than_hours: None,
        })
        .unwrap();
    assert_eq!(deleted.deleted, vec![blind.job_id.clone()]);
    assert!(!Path::new(&blind.workspace).exists());
    assert!(sibling.join("keep.txt").exists());
    assert!(
        service
            .cleanup(flect_app::CleanupOptions {
                dry_run: false,
                include_all: true,
                older_than_hours: None,
            })
            .unwrap()
            .deleted
            .is_empty()
    );
}

#[cfg(windows)]
#[test]
fn cleanup_rejects_workspace_symlink_escape() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let jobs = workspace.path().join("jobs");
    let service = AgentService::with_workspace_root_and_cleanup(
        GitRepository::discover(repository.path()).unwrap(),
        jobs.clone(),
        false,
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();
    fs::remove_dir_all(&blind.workspace).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let result = std::os::windows::fs::symlink_dir(outside.path(), jobs.join(&blind.job_id));
    if result.is_err() {
        return;
    }
    assert!(matches!(
        service.cleanup(flect_app::CleanupOptions {
            dry_run: false,
            include_all: true,
            older_than_hours: None,
        }),
        Err(AgentWorkflowError::UnsafeCleanup(_))
    ));
    assert!(outside.path().exists());
}

#[test]
fn rejects_fabricated_scope_evidence_reuse_and_unsafe_workspace() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();
    let error = service
        .submit_echo(BlindAgentSubmission {
            job_id: blind.job_id.clone(),
            echoed_spec: EchoedSpec {
                affected_scope: vec![AffectedScope {
                    file: "invented.rs".to_owned(),
                    symbol: None,
                }],
                ..EchoedSpec::default()
            },
            model: None,
            model_selection: AgentModelSelection::Unknown,
        })
        .unwrap_err();
    assert!(matches!(error, AgentWorkflowError::UnavailableScope(_)));

    let echoed = EchoedSpec {
        affected_scope: vec![AffectedScope {
            file: "app.txt".to_owned(),
            symbol: None,
        }],
        confidence: 0.8,
        ..EchoedSpec::default()
    };
    service
        .submit_echo(BlindAgentSubmission {
            job_id: blind.job_id.clone(),
            echoed_spec: echoed,
            model: None,
            model_selection: AgentModelSelection::Unknown,
        })
        .unwrap();
    assert!(matches!(
        service.submit_echo(BlindAgentSubmission {
            job_id: blind.job_id,
            echoed_spec: EchoedSpec::default(),
            model: None,
            model_selection: AgentModelSelection::Unknown,
        }),
        Err(AgentWorkflowError::InvalidJobState(_))
    ));

    assert!(matches!(
        AgentService::with_workspace_root(
            GitRepository::discover(repository.path()).unwrap(),
            repository.path().join(".flect/blind"),
        ),
        Err(AgentWorkflowError::UnsafeWorkspace)
    ));
    assert!(matches!(
        service.submit_echo(BlindAgentSubmission {
            job_id: "blind_../../escape".to_owned(),
            echoed_spec: EchoedSpec::default(),
            model: None,
            model_selection: AgentModelSelection::Unknown,
        }),
        Err(AgentWorkflowError::InvalidJobId(_))
    ));
}

#[test]
fn rejects_fabricated_verdict_evidence() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();
    service
        .submit_echo(BlindAgentSubmission {
            job_id: blind.job_id.clone(),
            echoed_spec: EchoedSpec {
                affected_scope: vec![AffectedScope {
                    file: "app.txt".to_owned(),
                    symbol: None,
                }],
                confidence: 0.8,
                ..EchoedSpec::default()
            },
            model: None,
            model_selection: AgentModelSelection::Unknown,
        })
        .unwrap();
    let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
    let finding = "The requested behavior is missing".to_owned();
    let result = service.submit_verdict(ReconciliationAgentSubmission {
        job_id: judge.job_id,
        verdict: JudgeVerdict {
            alignment: Alignment::Partial,
            findings: vec![JudgeFinding {
                kind: FindingCategory::MissingRequirements,
                text: finding,
                evidence_ref: Some("hunk/999".to_owned()),
            }],
            confidence: 0.8,
        },
        model: None,
        model_selection: AgentModelSelection::Unknown,
    });
    assert!(matches!(result, Err(AgentWorkflowError::Evidence(_))));
}

fn fixture_repository() -> tempfile::TempDir {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "-b", "task-secret-branch"]);
    git(
        repository.path(),
        ["config", "user.email", "tests@flect.local"],
    );
    git(repository.path(), ["config", "user.name", "Flect Tests"]);
    fs::write(repository.path().join("app.txt"), "old\n").unwrap();
    git(repository.path(), ["add", "app.txt"]);
    git(repository.path(), ["commit", "-m", COMMIT_SENTINEL]);
    let base = git_output(repository.path(), ["rev-parse", "HEAD"]);
    fs::write(
        repository.path().join("flect.toml"),
        flect_core::Config::default_document(),
    )
    .unwrap();
    let run = RunRecord {
        version: 1,
        id: "fl_a9e17e57a9e17e57".to_owned(),
        repository_root: repository.path().display().to_string(),
        base_revision: base.trim().to_owned(),
        task: TaskInput {
            text: format!("{TASK_SENTINEL} {CONVERSATION_SENTINEL}"),
        },
        intended_spec: IntendedSpec {
            objective: format!("Reject disabled accounts {FORWARD_SENTINEL}"),
            requirements: vec![FORWARD_SENTINEL.to_owned()],
            ..IntendedSpec::default()
        },
        model_calls: Vec::new(),
        created_unix_ms: 0,
    };
    RunStore::new(repository.path()).save_run(&run).unwrap();
    fs::write(repository.path().join("app.txt"), "new\n").unwrap();
    repository
}

fn assert_workspace_is_sanitized(workspace: &Path) {
    let mut pending = vec![workspace.to_path_buf()];
    let mut names = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            names.push(path.file_name().unwrap().to_string_lossy().to_string());
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                for forbidden in [
                    TASK_SENTINEL,
                    FORWARD_SENTINEL,
                    COMMIT_SENTINEL,
                    CONVERSATION_SENTINEL,
                ] {
                    assert!(
                        !text.contains(forbidden),
                        "{} leaked {forbidden}",
                        path.display()
                    );
                }
                assert!(fs::metadata(path).unwrap().permissions().readonly());
            }
        }
    }
    assert!(
        !names
            .iter()
            .any(|name| matches!(name.as_str(), ".git" | ".flect"))
    );
}

fn git<const N: usize>(directory: &Path, arguments: [&str; N]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output<const N: usize>(directory: &Path, arguments: [&str; N]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}
