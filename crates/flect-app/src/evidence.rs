use flect_core::{
    Alignment, BlindBundle, Evidence, FindingCategory, JudgeVerdict, RecommendedAction, Verdict,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("evidence references unavailable file `{0}`")]
    UnknownFile(String),
    #[error("evidence hunk is not present in patch for `{0}`")]
    UnknownHunk(String),
    #[error("evidence references unavailable stable hunk ID `{0}`")]
    UnknownHunkId(String),
    #[error("evidence line range is invalid for the supplied patch hunk in `{0}`")]
    InvalidLineRange(String),
    #[error("negative finding `{0}` has no corresponding evidence association")]
    MissingFindingEvidence(String),
    #[error("evidence references unavailable finding ID `{0}`")]
    UnknownFinding(String),
    #[error("verdict alignment {alignment} is inconsistent with recommended action {action}")]
    InconsistentAction {
        alignment: Alignment,
        action: RecommendedAction,
    },
    #[error("evidence category `{0}` has no emitted negative findings")]
    EmptyFindingCategory(String),
    #[error("judge alignment {0} is incompatible with its findings")]
    IncompatibleJudgeOutput(Alignment),
    #[error("judge confidence must be finite and between zero and one")]
    InvalidConfidence,
    #[error("judge findings must contain non-empty text")]
    EmptyFindingText,
    #[error("potential side effect duplicates a base divergence finding")]
    DuplicateSideEffect,
}

/// Converts the compact judge payload into Flect's persisted verdict.
///
/// This resolves only job-provided hunk IDs to immutable patch locations and
/// derives action and stable finding IDs. It does not repair semantic output.
///
/// # Errors
///
/// Returns [`EvidenceError`] when an evidence category, hunk ID, or resulting
/// trusted verdict fails validation.
pub fn materialize_judge_verdict(
    judge: JudgeVerdict,
    bundle: &BlindBundle,
) -> Result<Verdict, EvidenceError> {
    validate_judge_semantics(&judge)?;
    let mut verdict = Verdict {
        alignment: judge.alignment,
        agreements: Vec::new(),
        missing_requirements: findings_for(&judge, FindingCategory::MissingRequirements),
        unrequested_changes: findings_for(&judge, FindingCategory::UnrequestedChanges),
        violated_constraints: findings_for(&judge, FindingCategory::ViolatedConstraints),
        potential_side_effects: findings_for(&judge, FindingCategory::PotentialSideEffects),
        uncertainties: Vec::new(),
        evidence: Vec::new(),
        confidence: judge.confidence,
        recommended_action: action_for(judge.alignment),
    };
    let ids = finding_ids_by_category(&verdict);
    for finding in judge.findings {
        let finding_ids = ids
            .get(&finding.kind)
            .filter(|ids| !ids.is_empty())
            .ok_or_else(|| EvidenceError::EmptyFindingCategory(category_name(finding.kind)))?
            .clone();
        let (file, patch_hunk, line_start, line_end) = match finding.evidence_ref {
            Some(id) => resolve_hunk(bundle, &id)?,
            None => (None, None, None, None),
        };
        verdict.evidence.push(Evidence {
            file,
            line_start,
            line_end,
            patch_hunk,
            finding_ids,
            description: finding.text,
            confidence: verdict.confidence,
        });
    }
    validate_verdict_evidence(&verdict, bundle)?;
    Ok(verdict)
}

fn validate_judge_semantics(judge: &JudgeVerdict) -> Result<(), EvidenceError> {
    if !judge.confidence.is_finite() || !(0.0..=1.0).contains(&judge.confidence) {
        return Err(EvidenceError::InvalidConfidence);
    }
    if judge
        .findings
        .iter()
        .any(|finding| finding.text.trim().is_empty())
    {
        return Err(EvidenceError::EmptyFindingText);
    }
    let base_findings = judge
        .findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.kind,
                FindingCategory::UnrequestedChanges | FindingCategory::ViolatedConstraints
            )
        })
        .map(|finding| normalized_finding_text(&finding.text))
        .collect::<Vec<_>>();
    if judge
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingCategory::PotentialSideEffects)
        .map(|finding| normalized_finding_text(&finding.text))
        .any(|side_effect| base_findings.contains(&side_effect))
    {
        return Err(EvidenceError::DuplicateSideEffect);
    }
    let compatible = match judge.alignment {
        Alignment::Same => judge.findings.is_empty(),
        Alignment::Partial | Alignment::Different => !judge.findings.is_empty(),
        Alignment::Uncertain => true,
    };
    compatible
        .then_some(())
        .ok_or(EvidenceError::IncompatibleJudgeOutput(judge.alignment))
}

fn normalized_finding_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn findings_for(judge: &JudgeVerdict, category: FindingCategory) -> Vec<String> {
    judge
        .findings
        .iter()
        .filter(|finding| finding.kind == category)
        .map(|finding| finding.text.clone())
        .collect()
}

/// Rejects fabricated evidence and inconsistent verdict/action combinations.
///
/// # Errors
///
/// Returns [`EvidenceError`] when a finding, location, hunk, line range, or action is invalid.
pub fn validate_verdict_evidence(
    verdict: &Verdict,
    bundle: &BlindBundle,
) -> Result<(), EvidenceError> {
    validate_action(verdict)?;
    for evidence in &verdict.evidence {
        let Some(file) = evidence.file.as_deref() else {
            if evidence.line_start.is_some()
                || evidence.line_end.is_some()
                || evidence.patch_hunk.is_some()
            {
                return Err(EvidenceError::UnknownFile("<missing>".to_owned()));
            }
            continue;
        };
        let changed = bundle
            .patch
            .files
            .iter()
            .find(|changed| changed.path == file)
            .ok_or_else(|| EvidenceError::UnknownFile(file.to_owned()))?;
        if let Some(hunk) = evidence.patch_hunk.as_deref() {
            if !changed.patch.contains(hunk) {
                return Err(EvidenceError::UnknownHunk(file.to_owned()));
            }
            let valid = new_line_range(hunk).is_some_and(|(first, last)| {
                matches!(
                    (evidence.line_start, evidence.line_end),
                    (Some(start), Some(end)) if start <= end && start >= first && end <= last
                )
            });
            if !valid {
                return Err(EvidenceError::InvalidLineRange(file.to_owned()));
            }
        } else if evidence.line_start.is_some() || evidence.line_end.is_some() {
            return Err(EvidenceError::InvalidLineRange(file.to_owned()));
        }
    }
    let expected_findings = finding_ids(verdict);
    for evidence in &verdict.evidence {
        if let Some(unknown) = evidence
            .finding_ids
            .iter()
            .find(|finding| !expected_findings.contains(finding))
        {
            return Err(EvidenceError::UnknownFinding(unknown.clone()));
        }
    }
    for finding in expected_findings {
        if !verdict
            .evidence
            .iter()
            .any(|evidence| evidence.finding_ids.contains(&finding))
        {
            return Err(EvidenceError::MissingFindingEvidence(finding));
        }
    }
    Ok(())
}

/// Removes unsupported locations and adds unlocated evidence for uncovered findings.
pub fn sanitize_verdict_evidence(verdict: &mut Verdict, bundle: &BlindBundle) {
    for evidence in &mut verdict.evidence {
        sanitize_location(evidence, bundle);
    }
    for finding in finding_ids(verdict) {
        if !verdict
            .evidence
            .iter()
            .any(|evidence| evidence.finding_ids.contains(&finding))
        {
            verdict.evidence.push(Evidence {
                file: None,
                line_start: None,
                line_end: None,
                patch_hunk: None,
                description: finding.clone(),
                finding_ids: vec![finding],
                confidence: verdict.confidence,
            });
        }
    }
}

/// Stable IDs exposed in the judge contract for every negative finding.
pub fn finding_ids(verdict: &Verdict) -> Vec<String> {
    [
        ("missing_requirements", &verdict.missing_requirements),
        ("unrequested_changes", &verdict.unrequested_changes),
        ("violated_constraints", &verdict.violated_constraints),
        ("potential_side_effects", &verdict.potential_side_effects),
    ]
    .into_iter()
    .flat_map(|(kind, findings)| {
        findings
            .iter()
            .enumerate()
            .map(move |(index, _)| format!("{kind}/{index}"))
    })
    .collect()
}

fn sanitize_location(evidence: &mut Evidence, bundle: &BlindBundle) {
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

fn validate_action(verdict: &Verdict) -> Result<(), EvidenceError> {
    let valid = match verdict.alignment {
        Alignment::Same => verdict.recommended_action == RecommendedAction::Ship,
        Alignment::Partial => matches!(
            verdict.recommended_action,
            RecommendedAction::RevisePatch | RecommendedAction::ReviseBoth
        ),
        Alignment::Different => matches!(
            verdict.recommended_action,
            RecommendedAction::RevisitReasoning | RecommendedAction::ReviseBoth
        ),
        Alignment::Uncertain => verdict.recommended_action == RecommendedAction::RequestMoreContext,
    };
    if valid {
        Ok(())
    } else {
        Err(EvidenceError::InconsistentAction {
            alignment: verdict.alignment,
            action: verdict.recommended_action,
        })
    }
}

fn action_for(alignment: Alignment) -> RecommendedAction {
    match alignment {
        Alignment::Same => RecommendedAction::Ship,
        Alignment::Partial => RecommendedAction::RevisePatch,
        Alignment::Different => RecommendedAction::RevisitReasoning,
        Alignment::Uncertain => RecommendedAction::RequestMoreContext,
    }
}

fn finding_ids_by_category(
    verdict: &Verdict,
) -> std::collections::BTreeMap<FindingCategory, Vec<String>> {
    [
        (
            FindingCategory::MissingRequirements,
            "missing_requirements",
            &verdict.missing_requirements,
        ),
        (
            FindingCategory::UnrequestedChanges,
            "unrequested_changes",
            &verdict.unrequested_changes,
        ),
        (
            FindingCategory::ViolatedConstraints,
            "violated_constraints",
            &verdict.violated_constraints,
        ),
        (
            FindingCategory::PotentialSideEffects,
            "potential_side_effects",
            &verdict.potential_side_effects,
        ),
    ]
    .into_iter()
    .map(|(category, name, findings)| {
        (
            category,
            findings
                .iter()
                .enumerate()
                .map(|(index, _)| format!("{name}/{index}"))
                .collect(),
        )
    })
    .collect()
}

fn category_name(category: FindingCategory) -> String {
    match category {
        FindingCategory::MissingRequirements => "missing_requirements",
        FindingCategory::UnrequestedChanges => "unrequested_changes",
        FindingCategory::ViolatedConstraints => "violated_constraints",
        FindingCategory::PotentialSideEffects => "potential_side_effects",
    }
    .to_owned()
}

type TrustedLocation = (Option<String>, Option<String>, Option<u32>, Option<u32>);

fn resolve_hunk(bundle: &BlindBundle, requested: &str) -> Result<TrustedLocation, EvidenceError> {
    let mut index = 0_u32;
    for changed in &bundle.patch.files {
        for part in changed.patch.split("@@ ").skip(1) {
            let hunk = format!("@@ {part}");
            let id = format!("hunk/{index}");
            index = index.saturating_add(1);
            if id != requested {
                continue;
            }
            let (line_start, line_end) = new_line_range(&hunk)
                .ok_or_else(|| EvidenceError::UnknownHunkId(requested.to_owned()))?;
            return Ok((
                Some(changed.path.clone()),
                Some(hunk),
                Some(line_start),
                Some(line_end),
            ));
        }
    }
    Err(EvidenceError::UnknownHunkId(requested.to_owned()))
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
