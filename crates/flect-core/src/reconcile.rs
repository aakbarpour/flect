//! Conservative deterministic reconciliation for Milestone 1.

use std::collections::BTreeSet;

use crate::domain::{
    AffectedScope, Alignment, EchoedSpec, Evidence, IntendedSpec, RecommendedAction, Verdict,
};

/// Compares intended and reconstructed behavior without invoking another model.
///
/// This intentionally prefers `Uncertain` over confident guesses. Model-assisted
/// reconciliation belongs to a later milestone.
pub fn reconcile(intended: &IntendedSpec, echoed: &EchoedSpec) -> Verdict {
    let reconstructed = reconstructed_text(echoed);
    let objective_score = coverage(&intended.objective, &reconstructed);

    let expected: Vec<&String> = intended
        .requirements
        .iter()
        .chain(intended.acceptance_criteria.iter())
        .collect();
    let mut agreements = Vec::new();
    let mut missing_requirements = Vec::new();
    for requirement in expected {
        if coverage(requirement, &reconstructed) >= 0.55 {
            agreements.push(requirement.clone());
        } else {
            missing_requirements.push(requirement.clone());
        }
    }
    if objective_score >= 0.45 {
        agreements.insert(0, intended.objective.clone());
    }

    let violated_constraints: Vec<String> = intended
        .constraints
        .iter()
        .filter(|constraint| constraint_is_violated(constraint, echoed))
        .cloned()
        .collect();
    let mut unrequested_changes = echoed.side_effects.clone();
    if !intended.expected_scope.is_empty() {
        for affected in &echoed.affected_scope {
            if !intended
                .expected_scope
                .iter()
                .any(|expected| coverage(expected, &affected.file) >= 0.5)
            {
                unrequested_changes.push(format!(
                    "Affected scope outside the expected boundary: {}",
                    affected.file
                ));
            }
        }
    }
    for non_goal in &intended.non_goals {
        if coverage(non_goal, &reconstructed) >= 0.55 {
            unrequested_changes.push(format!(
                "Implementation appears to include non-goal: {non_goal}"
            ));
        }
    }
    let potential_side_effects = unrequested_changes.clone();
    let mut uncertainties = echoed.uncertainties.clone();
    uncertainties.extend(intended.uncertainties.iter().cloned());

    let all_expected_matched = missing_requirements.is_empty();
    let alignment = if echoed.confidence < 0.5 || echoed.apparent_objective.trim().is_empty() {
        Alignment::Uncertain
    } else if objective_score < 0.2
        && agreements.is_empty()
        && echoed.confidence >= 0.65
        && !intended.objective.trim().is_empty()
    {
        Alignment::Different
    } else if objective_score >= 0.45
        && all_expected_matched
        && violated_constraints.is_empty()
        && unrequested_changes.is_empty()
        && uncertainties.is_empty()
    {
        Alignment::Same
    } else {
        Alignment::Partial
    };

    let evidence = build_evidence(
        intended,
        echoed,
        &missing_requirements,
        &violated_constraints,
        &unrequested_changes,
        alignment,
    );

    let recommended_action = match alignment {
        Alignment::Same => RecommendedAction::Ship,
        Alignment::Partial => RecommendedAction::RevisePatch,
        Alignment::Different => RecommendedAction::RevisitReasoning,
        Alignment::Uncertain => RecommendedAction::RequestMoreContext,
    };

    Verdict {
        alignment,
        agreements,
        missing_requirements,
        unrequested_changes,
        violated_constraints,
        potential_side_effects,
        uncertainties,
        evidence,
        confidence: echoed.confidence,
        recommended_action,
    }
}

fn build_evidence(
    intended: &IntendedSpec,
    echoed: &EchoedSpec,
    missing_requirements: &[String],
    violated_constraints: &[String],
    unrequested_changes: &[String],
    alignment: Alignment,
) -> Vec<Evidence> {
    let mut evidence = Vec::new();
    evidence.extend(missing_requirements.iter().map(|requirement| Evidence {
        file: None,
        line_start: None,
        line_end: None,
        patch_hunk: None,
        finding_ids: Vec::new(),
        description: format!("No reconstructed behavior matched requirement: {requirement}"),
        confidence: echoed.confidence,
    }));
    evidence.extend(violated_constraints.iter().map(|constraint| Evidence {
        file: None,
        line_start: None,
        line_end: None,
        patch_hunk: None,
        finding_ids: Vec::new(),
        description: format!("Reconstructed behavior appears to conflict with: {constraint}"),
        confidence: echoed.confidence,
    }));
    evidence.extend(unrequested_changes.iter().map(|change| Evidence {
        file: scope_file_for(change, &echoed.affected_scope),
        line_start: None,
        line_end: None,
        patch_hunk: None,
        finding_ids: Vec::new(),
        description: change.clone(),
        confidence: echoed.confidence,
    }));
    if alignment == Alignment::Different {
        evidence.push(Evidence {
            file: None,
            line_start: None,
            line_end: None,
            patch_hunk: None,
            finding_ids: Vec::new(),
            description: format!(
                "Requested objective `{}` has little lexical overlap with reconstructed objective `{}`",
                intended.objective, echoed.apparent_objective
            ),
            confidence: echoed.confidence,
        });
    }
    evidence
}

fn reconstructed_text(echoed: &EchoedSpec) -> String {
    std::iter::once(echoed.apparent_objective.as_str())
        .chain(echoed.behavior_before.iter().map(String::as_str))
        .chain(echoed.behavior_after.iter().map(String::as_str))
        .chain(
            echoed
                .affected_scope
                .iter()
                .map(|scope| scope.file.as_str()),
        )
        .chain(echoed.side_effects.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn constraint_is_violated(constraint: &str, echoed: &EchoedSpec) -> bool {
    let normalized = constraint.to_ascii_lowercase();
    if !(normalized.contains("must not")
        || normalized.contains("do not")
        || normalized.contains("without"))
    {
        return false;
    }
    echoed
        .behavior_after
        .iter()
        .chain(echoed.side_effects.iter())
        .any(|behavior| coverage(constraint, behavior) >= 0.5)
}

fn coverage(expected: &str, actual: &str) -> f64 {
    let expected = terms(expected);
    if expected.is_empty() {
        return 0.0;
    }
    let actual = terms(actual);
    let matches = u32::try_from(expected.intersection(&actual).count()).unwrap_or(u32::MAX);
    let expected_count = u32::try_from(expected.len()).unwrap_or(u32::MAX);
    f64::from(matches) / f64::from(expected_count)
}

fn terms(text: &str) -> BTreeSet<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "as", "be", "by", "do", "for", "from", "in", "is", "it", "must", "not",
        "of", "on", "or", "should", "the", "to", "with", "without",
    ];
    text.split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_ascii_lowercase)
        .filter(|word| word.len() > 1 && !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

fn scope_file_for(description: &str, scope: &[AffectedScope]) -> Option<String> {
    scope
        .iter()
        .find(|item| item.file.contains('/') && description.contains(item.file.as_str()))
        .map(|item| item.file.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intended() -> IntendedSpec {
        IntendedSpec {
            objective: "Reject expired refresh tokens and rotate valid tokens".to_owned(),
            requirements: vec![
                "Reject expired refresh tokens".to_owned(),
                "Rotate tokens after refresh".to_owned(),
            ],
            constraints: vec!["Do not remove the legacy fallback".to_owned()],
            ..IntendedSpec::default()
        }
    }

    #[test]
    fn same_requires_all_expected_behavior() {
        let echoed = EchoedSpec {
            apparent_objective: "Reject expired refresh tokens and rotate valid tokens".to_owned(),
            behavior_after: vec![
                "Expired refresh tokens are rejected".to_owned(),
                "Tokens rotate after refresh".to_owned(),
            ],
            confidence: 0.9,
            ..EchoedSpec::default()
        };
        assert_eq!(reconcile(&intended(), &echoed).alignment, Alignment::Same);
    }

    #[test]
    fn missing_requirement_is_partial() {
        let echoed = EchoedSpec {
            apparent_objective: "Reject expired refresh tokens".to_owned(),
            behavior_after: vec!["Expired refresh tokens are rejected".to_owned()],
            confidence: 0.9,
            ..EchoedSpec::default()
        };
        let verdict = reconcile(&intended(), &echoed);
        assert_eq!(verdict.alignment, Alignment::Partial);
        assert_eq!(verdict.missing_requirements.len(), 1);
    }

    #[test]
    fn unrelated_objective_is_different() {
        let echoed = EchoedSpec {
            apparent_objective: "Reformat database migration files".to_owned(),
            behavior_after: vec!["SQL indentation changes".to_owned()],
            confidence: 0.9,
            ..EchoedSpec::default()
        };
        assert_eq!(
            reconcile(&intended(), &echoed).alignment,
            Alignment::Different
        );
    }

    #[test]
    fn low_confidence_is_uncertain() {
        let echoed = EchoedSpec {
            apparent_objective: "Maybe token changes".to_owned(),
            confidence: 0.2,
            ..EchoedSpec::default()
        };
        assert_eq!(
            reconcile(&intended(), &echoed).alignment,
            Alignment::Uncertain
        );
    }

    #[test]
    fn reports_scope_outside_the_expected_boundary() {
        let mut intended = intended();
        intended.expected_scope = vec!["auth".to_owned()];
        let echoed = EchoedSpec {
            apparent_objective: "Reject expired refresh tokens and rotate valid tokens".to_owned(),
            behavior_after: vec![
                "Expired refresh tokens are rejected".to_owned(),
                "Tokens rotate after refresh".to_owned(),
            ],
            affected_scope: vec![
                AffectedScope {
                    file: "auth".to_owned(),
                    symbol: None,
                },
                AffectedScope {
                    file: "billing".to_owned(),
                    symbol: None,
                },
            ],
            confidence: 0.9,
            ..EchoedSpec::default()
        };
        let verdict = reconcile(&intended, &echoed);
        assert_eq!(verdict.alignment, Alignment::Partial);
        assert!(
            verdict
                .unrequested_changes
                .iter()
                .any(|finding| finding.contains("billing"))
        );
    }
}
