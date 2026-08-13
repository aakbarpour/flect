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
    assert!(judge.instructions.contains("judge-begin"));
    assert!(!judge.instructions.contains("ReconciliationAgentSubmission"));
    assert!(
        RunStore::new(repository.path())
            .load_verification(None)
            .is_err()
    );
    service
        .judge_begin(
            &judge.job_id,
            Some("runtime-inherited".to_owned()),
            AgentModelSelection::Inherited,
        )
        .unwrap();
    service
        .judge_set_alignment(&judge.job_id, Alignment::Same)
        .unwrap();
    service.judge_set_confidence(&judge.job_id, 0.9).unwrap();
    let record = service.judge_submit(&judge.job_id).unwrap();
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
    assert!(service.judge_submit(&judge.job_id).is_err());
}

#[test]
fn filesystem_draft_protocol_validates_and_persists_verifier_and_judge() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let jobs = workspace.path().join("jobs");
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        jobs.clone(),
    )
    .unwrap();

    let blind = service.prepare_blind(None, None).unwrap();
    let draft = jobs.join(&blind.job_id).join("draft");
    fs::write(draft.join("objective"), "Reject disabled accounts").unwrap();
    fs::write(draft.join("confidence"), "0.9").unwrap();
    fs::write(draft.join("submitted"), []).unwrap();
    fs::create_dir_all(draft.join("affected_scope/000000")).unwrap();
    fs::write(draft.join("affected_scope/000000/file"), "app.txt").unwrap();
    let echoed = service.verifier_commit(&blind.job_id).unwrap();
    assert_eq!(echoed.apparent_objective, "Reject disabled accounts");
    assert!(service.verifier_commit(&blind.job_id).is_err());

    let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
    let judge_draft = jobs.join(&judge.job_id).join("draft");
    fs::create_dir_all(judge_draft.join("alignment/SAME")).unwrap();
    fs::write(judge_draft.join("confidence"), "0.8").unwrap();
    fs::write(judge_draft.join("submitted"), []).unwrap();
    let record = service.judge_submit(&judge.job_id).unwrap();
    assert_eq!(record.verdict.alignment, Alignment::Same);
    assert!(service.judge_submit(&judge.job_id).is_err());
}

#[test]
fn filesystem_draft_protocol_rejects_unknown_entries_and_invalid_values() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let jobs = workspace.path().join("jobs");
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        jobs.clone(),
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();
    let draft = jobs.join(&blind.job_id).join("draft");
    fs::write(draft.join("objective"), "objective").unwrap();
    fs::write(draft.join("confidence"), "2.0").unwrap();
    fs::write(draft.join("submitted"), []).unwrap();
    fs::write(draft.join("unexpected"), "reject").unwrap();
    assert!(service.verifier_commit(&blind.job_id).is_err());
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
    assert_eq!(judge.evidence_ref_contract["version"], 4);
    assert_eq!(
        judge.evidence_ref_contract["finding_fields"],
        serde_json::json!(["kind", "text", "evidence_ref"])
    );
    assert!(
        judge.evidence_ref_contract["forbidden_finding_fields"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("evidence"))
    );
    assert!(
        judge.evidence_ref_contract["finding_kind_guidance"]["unrequested_change"]
            .as_str()
            .unwrap()
            .contains("even when the requested behavior is also present")
    );
    assert!(
        judge.evidence_ref_contract["finding_kind_guidance"]["potential_side_effect"]
            .as_str()
            .unwrap()
            .contains("separately described consequence")
    );
    assert!(
        judge.evidence_ref_contract["alignment_meanings"]["DIFFERENT"]
            .as_str()
            .unwrap()
            .contains("do not use DIFFERENT solely")
    );
    assert!(
        judge.evidence_ref_contract["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().unwrap().contains("evidence_ref"))
    );
    assert!(
        judge
            .instructions
            .contains("Each verifier-reported side effect must be explicitly dispositioned")
    );
    assert_eq!(judge.evidence_ref_contract["files"][0]["file"], "app.txt");
    assert!(
        judge.evidence_ref_contract["files"][0]["hunks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hunk| hunk["hunk"].as_str().unwrap().starts_with("@@ "))
    );
    assert_eq!(
        judge.evidence_ref_contract["files"][0]["hunks"][0]["hunk_id"],
        "hunk/0"
    );
    let serialized = serde_json::to_value(&judge).unwrap();
    assert!(serialized.get("submission_schema").is_none());
    assert!(serialized.get("submission_file").is_none());
}

#[test]
#[allow(clippy::too_many_lines)]
fn external_verifier_submission_is_path_only_strict_and_single_use() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();
    let blind = service.prepare_blind(None, None).unwrap();
    let verifier =
        flect_app::ExternalVerifierService::new_for_tests(&workspace.path().join("jobs")).unwrap();

    // The child-facing service is external and has neither repository discovery nor JSON input.
    assert!(!Path::new(&blind.workspace).starts_with(repository.path()));
    assert!(
        serde_json::to_value(&blind)
            .unwrap()
            .get("submission_file")
            .is_none()
    );
    assert!(matches!(
        verifier.begin("blind_0000000000000000", None, AgentModelSelection::Unknown),
        Err(AgentWorkflowError::JobNotFound(_))
    ));
    verifier
        .begin(
            &blind.job_id,
            Some("gpt-5.6-terra".to_owned()),
            AgentModelSelection::Explicit,
        )
        .unwrap();
    assert!(matches!(
        verifier.add_scope(&blind.job_id, "invented.rs".to_owned(), None),
        Err(AgentWorkflowError::UnavailableScope(_))
    ));
    assert!(verifier.set_confidence(&blind.job_id, f64::NAN).is_err());
    verifier
        .set_objective(&blind.job_id, "App behavior changes.".to_owned())
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::Before,
            "Old behavior.".to_owned(),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::Before,
            "Earlier callers saw the original result.".to_owned(),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::After,
            "New behavior.".to_owned(),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::After,
            "Later callers see the changed result.".to_owned(),
        )
        .unwrap();
    verifier
        .add_scope(&blind.job_id, "app.txt".to_owned(), Some("run".to_owned()))
        .unwrap();
    verifier
        .add_scope(
            &blind.job_id,
            "app.txt".to_owned(),
            Some("result".to_owned()),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::SideEffect,
            "Callers observe new behavior.".to_owned(),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::SideEffect,
            "Observers receive the updated result.".to_owned(),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::Assumption,
            "The file is representative.".to_owned(),
        )
        .unwrap();
    verifier
        .add_text(
            &blind.job_id,
            flect_app::VerifierTextField::Uncertainty,
            "Callers are not shown.".to_owned(),
        )
        .unwrap();
    verifier.set_confidence(&blind.job_id, 0.9).unwrap();
    verifier.submit(&blind.job_id).unwrap();
    assert!(matches!(
        verifier.submit(&blind.job_id),
        Err(AgentWorkflowError::InvalidJobState(_))
    ));
    // The parent supplies only the job ID; it never reads the typed semantic values.
    let echoed = service.verifier_commit(&blind.job_id).unwrap();
    assert_eq!(echoed.behavior_before.len(), 2);
    assert_eq!(echoed.behavior_after.len(), 2);
    assert_eq!(echoed.affected_scope.len(), 2);
    assert_eq!(echoed.side_effects.len(), 2);
}

#[test]
#[allow(clippy::too_many_lines)]
fn side_effect_guidance_requires_a_distinct_consequence_without_duplication() {
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
            echoed_spec: EchoedSpec::default(),
            model: None,
            model_selection: AgentModelSelection::Unknown,
        })
        .unwrap();
    let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
    let guidance = judge.evidence_ref_contract["finding_kind_guidance"]["potential_side_effect"]
        .as_str()
        .unwrap();
    let rule = judge.evidence_ref_contract["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .find(|value| value.contains("Every side_effect candidate"))
        .unwrap();

    // A base divergence alone remains only its base category.
    assert!(guidance.contains("distinct plausible externally observable impact"));
    // Verifier-reported consequences must receive a typed disposition.
    assert!(rule.contains("dispositioned"));
    // Equivalent wording is not a separate downstream consequence.
    assert!(guidance.contains("do not treat one category as a substitute"));
    assert!(
        judge
            .instructions
            .contains("do not emit a potential_side_effect that merely restates")
    );

    let bundle = service.prepare_blind(None, None).unwrap().bundle;
    let unrequested = JudgeFinding {
        kind: FindingCategory::UnrequestedChanges,
        text: "Add request logging.".to_owned(),
        evidence_ref: None,
    };
    let constraint = JudgeFinding {
        kind: FindingCategory::ViolatedConstraints,
        text: "Changes the public input type.".to_owned(),
        evidence_ref: None,
    };
    let downstream = JudgeFinding {
        kind: FindingCategory::PotentialSideEffects,
        text: "Callers must migrate to the new input type.".to_owned(),
        evidence_ref: None,
    };

    // A base divergence with no distinct consequence has only its base category.
    let base_only = flect_app::materialize_judge_verdict(
        JudgeVerdict {
            alignment: Alignment::Partial,
            findings: vec![unrequested.clone()],
            confidence: 0.9,
        },
        &bundle,
    )
    .unwrap();
    assert_eq!(
        base_only.unrequested_changes,
        vec![unrequested.text.clone()]
    );
    assert!(base_only.potential_side_effects.is_empty());

    // Either generic base divergence may be paired with a distinct consequence.
    for base in [unrequested, constraint] {
        let verdict = flect_app::materialize_judge_verdict(
            JudgeVerdict {
                alignment: Alignment::Partial,
                findings: vec![base, downstream.clone()],
                confidence: 0.9,
            },
            &bundle,
        )
        .unwrap();
        assert_eq!(
            verdict.potential_side_effects,
            vec![downstream.text.clone()]
        );
    }

    // Equivalent wording is a duplicate restatement, not a downstream consequence.
    assert!(
        flect_app::materialize_judge_verdict(
            JudgeVerdict {
                alignment: Alignment::Partial,
                findings: vec![
                    JudgeFinding {
                        kind: FindingCategory::UnrequestedChanges,
                        text: "Logs each request key".to_owned(),
                        evidence_ref: None,
                    },
                    JudgeFinding {
                        kind: FindingCategory::PotentialSideEffects,
                        text: " logs   each REQUEST key ".to_owned(),
                        evidence_ref: None,
                    },
                ],
                confidence: 0.9,
            },
            &bundle,
        )
        .is_err()
    );
}

#[test]
fn different_requires_an_objective_mismatch_finding() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();
    let constraint = JudgeFinding {
        kind: FindingCategory::ViolatedConstraints,
        text: "Changes the public input type.".to_owned(),
        evidence_ref: None,
    };
    let side_effect = JudgeFinding {
        kind: FindingCategory::PotentialSideEffects,
        text: "Callers must migrate to the new input type.".to_owned(),
        evidence_ref: None,
    };

    let submit = |alignment: Alignment, finding: JudgeFinding| {
        let blind = service.prepare_blind(None, None).unwrap();
        service
            .submit_echo(BlindAgentSubmission {
                job_id: blind.job_id.clone(),
                echoed_spec: EchoedSpec::default(),
                model: None,
                model_selection: AgentModelSelection::Unknown,
            })
            .unwrap();
        let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
        service
            .judge_begin(&judge.job_id, None, AgentModelSelection::Unknown)
            .unwrap();
        service
            .judge_set_alignment(&judge.job_id, alignment)
            .unwrap();
        service
            .judge_add_finding(
                &judge.job_id,
                finding.kind,
                finding.text,
                finding.evidence_ref,
            )
            .unwrap();
        service.judge_set_confidence(&judge.job_id, 0.9).unwrap();
        service.judge_submit(&judge.job_id)
    };

    // An objective-advancing implementation with a constraint break is PARTIAL.
    assert!(submit(Alignment::Partial, constraint.clone()).is_ok());

    // A constraint break or downstream effect alone does not establish objective mismatch.
    for findings in [vec![constraint], vec![side_effect]] {
        assert!(submit(Alignment::Different, findings.into_iter().next().unwrap()).is_err());
    }

    // Genuinely unrelated work is represented by an objective-mismatch category.
    assert!(
        submit(
            Alignment::Different,
            JudgeFinding {
                kind: FindingCategory::UnrequestedChanges,
                text: "Adds unrelated request logging instead of the requested behavior."
                    .to_owned(),
                evidence_ref: None,
            },
        )
        .is_ok()
    );
}

#[test]
fn verifier_side_effects_require_a_typed_disposition() {
    let repository = fixture_repository();
    let workspace = tempfile::tempdir().unwrap();
    let service = AgentService::with_workspace_root(
        GitRepository::discover(repository.path()).unwrap(),
        workspace.path().join("jobs"),
    )
    .unwrap();

    let prepare = |service: &AgentService| {
        let blind = service.prepare_blind(None, None).unwrap();
        service
            .submit_echo(BlindAgentSubmission {
                job_id: blind.job_id.clone(),
                echoed_spec: EchoedSpec {
                    side_effects: vec!["Callers must migrate to the new input type.".to_owned()],
                    ..EchoedSpec::default()
                },
                model: None,
                model_selection: AgentModelSelection::Unknown,
            })
            .unwrap();
        service.prepare_reconciliation(&blind.job_id).unwrap()
    };

    let undispositioned = prepare(&service);
    assert_eq!(
        undispositioned.evidence_ref_contract["side_effect_candidates"][0]["id"],
        "side_effect/0"
    );
    service
        .judge_begin(&undispositioned.job_id, None, AgentModelSelection::Unknown)
        .unwrap();
    service
        .judge_set_alignment(&undispositioned.job_id, Alignment::Partial)
        .unwrap();
    service
        .judge_add_finding(
            &undispositioned.job_id,
            FindingCategory::ViolatedConstraints,
            "Changes the public input type.".to_owned(),
            Some("hunk/0".to_owned()),
        )
        .unwrap();
    service
        .judge_set_confidence(&undispositioned.job_id, 0.9)
        .unwrap();
    assert!(service.judge_submit(&undispositioned.job_id).is_err());

    let distinct = prepare(&service);
    service
        .judge_begin(&distinct.job_id, None, AgentModelSelection::Unknown)
        .unwrap();
    service
        .judge_set_alignment(&distinct.job_id, Alignment::Partial)
        .unwrap();
    service
        .judge_add_finding(
            &distinct.job_id,
            FindingCategory::ViolatedConstraints,
            "Changes the public input type.".to_owned(),
            Some("hunk/0".to_owned()),
        )
        .unwrap();
    service
        .judge_add_side_effect_finding(
            &distinct.job_id,
            "side_effect/0".to_owned(),
            "Callers must migrate to the new input type.".to_owned(),
            "hunk/0".to_owned(),
        )
        .unwrap();
    service.judge_set_confidence(&distinct.job_id, 0.9).unwrap();
    assert!(service.judge_submit(&distinct.job_id).is_ok());

    let non_distinct = prepare(&service);
    service
        .judge_begin(&non_distinct.job_id, None, AgentModelSelection::Unknown)
        .unwrap();
    service
        .judge_set_alignment(&non_distinct.job_id, Alignment::Partial)
        .unwrap();
    service
        .judge_add_finding(
            &non_distinct.job_id,
            FindingCategory::ViolatedConstraints,
            "Changes the public input type.".to_owned(),
            Some("hunk/0".to_owned()),
        )
        .unwrap();
    service
        .judge_mark_side_effect_not_distinct(
            &non_distinct.job_id,
            "side_effect/0".to_owned(),
            "This only restates the public API change.".to_owned(),
        )
        .unwrap();
    service
        .judge_set_confidence(&non_distinct.job_id, 0.9)
        .unwrap();
    assert!(service.judge_submit(&non_distinct.job_id).is_ok());
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
        serde_json::json!({"kind": "missing_requirement", "text": "x", "evidence": {"hunk": 0}}),
        serde_json::json!({"kind": "missing_requirement", "text": "x", "file": "invented.rs"}),
        serde_json::json!({"kind": "missing_requirement", "text": "x", "line_start": 99}),
        serde_json::json!({"finding_ids": ["missing_requirements/99"], "description": "x"}),
    ] {
        assert!(serde_json::from_value::<JudgeFinding>(evidence).is_err());
    }
}

#[test]
fn reconciliation_submission_rejects_observed_evidence_shape_and_wrappers() {
    let observed = serde_json::json!({
        "job_id": "judge_observed",
        "model": "gpt-5.6-terra",
        "model_selection": "explicit",
        "verdict": {
            "alignment": "PARTIAL",
            "confidence": 0.9,
            "findings": [{
                "kind": "missing_requirement",
                "text": "Missing behavior",
                "evidence": {"hunk": 0}
            }]
        }
    });
    assert!(serde_json::from_value::<ReconciliationAgentSubmission>(observed).is_err());

    for wrapper in [
        serde_json::json!({"submission": {}}),
        serde_json::json!({"job_id": "judge", "verdict": {}}),
    ] {
        assert!(serde_json::from_value::<ReconciliationAgentSubmission>(wrapper).is_err());
    }
}

#[test]
fn valid_evidence_ref_submission_persists_without_evidence_repair() {
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
            echoed_spec: EchoedSpec::default(),
            model: Some("gpt-5.6-terra".to_owned()),
            model_selection: AgentModelSelection::Explicit,
        })
        .unwrap();
    let judge = service.prepare_reconciliation(&blind.job_id).unwrap();
    service
        .judge_begin(
            &judge.job_id,
            Some("gpt-5.6-terra".to_owned()),
            AgentModelSelection::Explicit,
        )
        .unwrap();
    service
        .judge_set_alignment(&judge.job_id, Alignment::Partial)
        .unwrap();
    service
        .judge_add_finding(
            &judge.job_id,
            FindingCategory::MissingRequirements,
            "The requested behavior is absent.".to_owned(),
            Some("hunk/0".to_owned()),
        )
        .unwrap();
    service.judge_set_confidence(&judge.job_id, 0.9).unwrap();
    let record = service.judge_submit(&judge.job_id).unwrap();
    assert_eq!(record.verdict.alignment, Alignment::Partial);
    assert_eq!(
        record.verdict.evidence[0].finding_ids,
        vec!["missing_requirements/0"]
    );
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
    service
        .judge_begin(&judge.job_id, None, AgentModelSelection::Unknown)
        .unwrap();
    service
        .judge_set_alignment(&judge.job_id, Alignment::Partial)
        .unwrap();
    service
        .judge_add_finding(
            &judge.job_id,
            FindingCategory::MissingRequirements,
            "The requested behavior is missing".to_owned(),
            Some("hunk/999".to_owned()),
        )
        .unwrap();
    service.judge_set_confidence(&judge.job_id, 0.8).unwrap();
    let result = service.judge_submit(&judge.job_id);
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
