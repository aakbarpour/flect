//! Deterministic context selection and privacy filtering.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use thiserror::Error;

use crate::blind::BundleContext;
use crate::config::Config;
use crate::domain::{ContextFile, ContextPolicy, ExcludedPath, FileStatus, PatchSet};

const DEFAULT_EXCLUSIONS: &[&str] = &[
    ".env",
    ".env.*",
    "**/.env",
    "**/.env.*",
    "*.pem",
    "**/*.pem",
    "*.key",
    "**/*.key",
    "id_rsa",
    "**/id_rsa",
    "id_ed25519",
    "**/id_ed25519",
    "credentials",
    "credentials.*",
    "**/credentials",
    "**/credentials.*",
    "*secret*",
    "**/*secret*",
    ".git",
    ".git/**",
    ".flect",
    ".flect/**",
    "target/**",
    "dist/**",
    "node_modules/**",
    "vendor/**",
];

const ROOT_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
];

/// Builds the non-sensitive inputs from which `BlindGuard` creates a bundle.
pub struct ContextBuilder {
    root: PathBuf,
    policy: ContextPolicy,
    matcher: GlobSet,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

/// Context selection failed before a verifier was invoked.
#[derive(Debug, Error)]
pub enum ContextError {
    #[error("invalid ignore pattern `{pattern}`: {source}")]
    InvalidPattern {
        pattern: String,
        source: globset::Error,
    },
    #[error(
        "repository context mode is reserved for a later Flect milestone; use `patch` or `focused`"
    )]
    RepoModeUnavailable,
    #[error("could not inspect context file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
}

impl ContextBuilder {
    /// Creates a deterministic selector from project configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ContextError`] when an ignore pattern cannot be compiled.
    pub fn new(root: &Path, config: &Config) -> Result<Self, ContextError> {
        let mut builder = GlobSetBuilder::new();
        for pattern in DEFAULT_EXCLUSIONS
            .iter()
            .copied()
            .chain(config.ignore.patterns.iter().map(String::as_str))
        {
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|source| ContextError::InvalidPattern {
                    pattern: pattern.to_owned(),
                    source,
                })?;
            builder.add(glob);
        }
        let matcher = builder
            .build()
            .map_err(|source| ContextError::InvalidPattern {
                pattern: "combined pattern set".to_owned(),
                source,
            })?;
        Ok(Self {
            root: root.to_owned(),
            policy: config.verification.context,
            matcher,
            max_file_bytes: config.verification.max_context_file_bytes,
            max_total_bytes: config.verification.max_context_bytes,
        })
    }

    /// Filters the patch and chooses any focused context files.
    ///
    /// # Errors
    ///
    /// Returns [`ContextError`] for unsupported context modes or unreadable files.
    pub fn build(&self, mut patch: PatchSet) -> Result<BundleContext, ContextError> {
        if self.policy == ContextPolicy::Repo {
            return Err(ContextError::RepoModeUnavailable);
        }

        let mut excluded = Vec::new();
        patch.files.retain(|file| {
            if self.matcher.is_match(&file.path) {
                excluded.push(ExcludedPath {
                    path: file.path.clone(),
                    reason: "matched a privacy or ignore pattern".to_owned(),
                });
                false
            } else if file.binary {
                excluded.push(ExcludedPath {
                    path: file.path.clone(),
                    reason: "binary files are not sent to verifiers".to_owned(),
                });
                false
            } else {
                true
            }
        });
        recalculate_summary(&mut patch);

        let context = if self.policy == ContextPolicy::Focused {
            self.focused_context(&patch, &mut excluded)?
        } else {
            Vec::new()
        };

        Ok(BundleContext {
            patch,
            context,
            excluded_paths: excluded,
            policy: self.policy,
        })
    }

    fn focused_context(
        &self,
        patch: &PatchSet,
        excluded: &mut Vec<ExcludedPath>,
    ) -> Result<Vec<ContextFile>, ContextError> {
        let mut candidates = BTreeSet::new();
        for file in &patch.files {
            if file.status != FileStatus::Deleted {
                candidates.insert(file.path.clone());
            }
        }
        for manifest in ROOT_MANIFESTS {
            if self.root.join(manifest).is_file() {
                candidates.insert((*manifest).to_owned());
            }
        }

        let mut context = Vec::new();
        let mut total = 0_u64;
        for path in candidates {
            if self.matcher.is_match(&path) {
                excluded.push(ExcludedPath {
                    path,
                    reason: "matched a privacy or ignore pattern".to_owned(),
                });
                continue;
            }
            let absolute = self.root.join(&path);
            let metadata = match fs::symlink_metadata(&absolute) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(ContextError::Read {
                        path: absolute.display().to_string(),
                        source,
                    });
                }
            };
            if metadata.file_type().is_symlink() {
                excluded.push(ExcludedPath {
                    path,
                    reason: "symbolic links are not followed into verifier context".to_owned(),
                });
                continue;
            }
            if metadata.len() > self.max_file_bytes {
                excluded.push(ExcludedPath {
                    path,
                    reason: format!(
                        "file exceeds the {}-byte context limit",
                        self.max_file_bytes
                    ),
                });
                continue;
            }
            if total.saturating_add(metadata.len()) > self.max_total_bytes {
                excluded.push(ExcludedPath {
                    path,
                    reason: format!(
                        "including the file would exceed the {}-byte total context limit",
                        self.max_total_bytes
                    ),
                });
                continue;
            }
            let bytes = fs::read(&absolute).map_err(|source| ContextError::Read {
                path: absolute.display().to_string(),
                source,
            })?;
            if bytes.contains(&0) {
                excluded.push(ExcludedPath {
                    path,
                    reason: "file appears to be binary".to_owned(),
                });
                continue;
            }
            let Ok(file_text) = String::from_utf8(bytes) else {
                excluded.push(ExcludedPath {
                    path,
                    reason: "file is not valid UTF-8".to_owned(),
                });
                continue;
            };
            total = total.saturating_add(file_text.len() as u64);
            context.push(ContextFile {
                path,
                content: file_text,
            });
        }
        Ok(context)
    }
}

fn recalculate_summary(patch: &mut PatchSet) {
    patch.insertions = patch.files.iter().map(|file| file.insertions).sum();
    patch.deletions = patch.files.iter().map(|file| file.deletions).sum();
    patch.renames = patch
        .files
        .iter()
        .filter(|file| file.status == FileStatus::Renamed)
        .count() as u64;
    patch.binary_files = patch
        .files
        .iter()
        .filter(|file| file.binary)
        .map(|file| file.path.clone())
        .collect();
    patch.untracked_files = patch
        .files
        .iter()
        .filter(|file| file.status == FileStatus::Untracked)
        .map(|file| file.path.clone())
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ChangedFile, FileStatus};

    fn patch(files: &[(&str, bool)]) -> PatchSet {
        PatchSet {
            base_revision: "abc".to_owned(),
            files: files
                .iter()
                .map(|(path, binary)| ChangedFile {
                    path: (*path).to_owned(),
                    status: FileStatus::Modified,
                    patch: "patch".to_owned(),
                    old_path: None,
                    insertions: 1,
                    deletions: 0,
                    binary: *binary,
                })
                .collect(),
            renames: 0,
            insertions: files.len() as u64,
            deletions: 0,
            binary_files: Vec::new(),
            untracked_files: Vec::new(),
        }
    }

    #[test]
    fn excludes_secret_paths_from_patch() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::default();
        let result = ContextBuilder::new(temp.path(), &config)
            .unwrap()
            .build(patch(&[("src/lib.rs", false), (".env", false)]))
            .unwrap();
        assert_eq!(result.patch.files.len(), 1);
        assert_eq!(result.excluded_paths[0].path, ".env");
    }

    #[test]
    fn secret_matching_is_case_insensitive() {
        let temp = tempfile::tempdir().unwrap();
        let result = ContextBuilder::new(temp.path(), &Config::default())
            .unwrap()
            .build(patch(&[("config/.ENV.PRODUCTION", false)]))
            .unwrap();
        assert!(result.patch.files.is_empty());
    }

    #[test]
    fn excludes_binary_patch_files() {
        let temp = tempfile::tempdir().unwrap();
        let result = ContextBuilder::new(temp.path(), &Config::default())
            .unwrap()
            .build(patch(&[("image.png", true)]))
            .unwrap();
        assert!(result.patch.files.is_empty());
    }

    #[test]
    fn excludes_flect_and_git_state_even_without_gitignore() {
        let temp = tempfile::tempdir().unwrap();
        let result = ContextBuilder::new(temp.path(), &Config::default())
            .unwrap()
            .build(patch(&[
                (".flect/runs/fl_secret.json", false),
                (".git/config", false),
                ("src/lib.rs", false),
            ]))
            .unwrap();
        assert_eq!(result.patch.files.len(), 1);
        assert_eq!(result.patch.files[0].path, "src/lib.rs");
    }
}
