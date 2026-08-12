//! Structural isolation boundary for blind verification.

use thiserror::Error;

use crate::config::BlindConfig;
use crate::domain::{
    BlindBundle, BlindnessReport, BundleManifest, ContextFile, ContextPolicy, ExcludedPath,
    IsolationEntry, IsolationKind, PatchSet,
};

/// Sanitized inputs accepted by `BlindGuard`.
#[derive(Debug, Clone)]
pub struct BundleContext {
    pub patch: PatchSet,
    pub context: Vec<ContextFile>,
    pub excluded_paths: Vec<ExcludedPath>,
    pub policy: ContextPolicy,
}

/// Enforces that restricted metadata is not enabled for verifier input.
pub struct BlindGuard;

/// `BlindGuard` refused to construct a verifier bundle.
#[derive(Debug, Error)]
pub enum BlindGuardError {
    #[error("BlindGuard refused to build a bundle because `{0}` is configured for inclusion")]
    RestrictedSource(&'static str),
}

impl BlindGuard {
    /// Converts already privacy-filtered content into the sole verifier payload type.
    ///
    /// # Errors
    ///
    /// Returns [`BlindGuardError`] if any restricted metadata source is enabled.
    pub fn build(
        input: BundleContext,
        config: &BlindConfig,
    ) -> Result<BlindBundle, BlindGuardError> {
        if !config.strip_git_metadata {
            return Err(BlindGuardError::RestrictedSource("Git metadata"));
        }
        if !config.strip_branch_name {
            return Err(BlindGuardError::RestrictedSource("branch name"));
        }
        if !config.strip_commit_messages {
            return Err(BlindGuardError::RestrictedSource("commit messages"));
        }

        let patch_files = input
            .patch
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let context_files = input.context.iter().map(|file| file.path.clone()).collect();
        let total_bytes = input
            .patch
            .files
            .iter()
            .map(|file| file.patch.len() as u64)
            .chain(input.context.iter().map(|file| file.content.len() as u64))
            .sum();
        let manifest = BundleManifest {
            context_policy: input.policy,
            patch_files,
            context_files,
            excluded_paths: input.excluded_paths,
            total_bytes,
        };
        let blindness_report = BlindnessReport {
            isolation: "strict".to_owned(),
            entries: vec![
                excluded("Original task"),
                excluded("Conversation"),
                excluded("Forward spec"),
                excluded("Branch metadata"),
                excluded("Commit metadata"),
                IsolationEntry {
                    source: "Issue metadata".to_owned(),
                    status: "absent".to_owned(),
                    assurance: IsolationKind::StructurallyExcluded,
                },
                IsolationEntry {
                    source: "Patch text leakage".to_owned(),
                    status: "not semantically provable".to_owned(),
                    assurance: IsolationKind::Unknown,
                },
            ],
            limitations: vec![
                "BlindGuard structurally excludes known metadata sources; it cannot prove that code or comments do not reveal task semantics.".to_owned(),
            ],
        };

        Ok(BlindBundle {
            patch: input.patch,
            context: input.context,
            manifest,
            blindness_report,
        })
    }
}

fn excluded(source: &str) -> IsolationEntry {
    IsolationEntry {
        source: source.to_owned(),
        status: "hidden".to_owned(),
        assurance: IsolationKind::StructurallyExcluded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fails_closed_when_commit_metadata_is_allowed() {
        let input = BundleContext {
            patch: PatchSet {
                base_revision: "abc".to_owned(),
                files: Vec::new(),
                renames: 0,
                insertions: 0,
                deletions: 0,
                binary_files: Vec::new(),
                untracked_files: Vec::new(),
            },
            context: Vec::new(),
            excluded_paths: Vec::new(),
            policy: ContextPolicy::Patch,
        };
        let config = BlindConfig {
            strip_commit_messages: false,
            ..BlindConfig::default()
        };
        assert!(matches!(
            BlindGuard::build(input, &config),
            Err(BlindGuardError::RestrictedSource("commit messages"))
        ));
    }
}
