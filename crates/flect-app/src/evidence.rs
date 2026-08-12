use flect_core::{Alignment, BlindBundle, Evidence, RecommendedAction, Verdict};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("evidence references unavailable file `{0}`")]
    UnknownFile(String),
    #[error("evidence hunk is not present in patch for `{0}`")]
    UnknownHunk(String),
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
