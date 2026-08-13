//! Read-only interaction with the user's installed Git executable.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use thiserror::Error;

use crate::domain::{ChangedFile, FileStatus, PatchSet};

/// A discovered Git worktree.
#[derive(Debug, Clone)]
pub struct GitRepository {
    root: PathBuf,
}

/// Git discovery or patch capture failed.
#[derive(Debug, Error)]
pub enum GitError {
    #[error(
        "Flect could not find the `git` executable. Git is required to capture repository changes. Install Git and run `flect doctor`."
    )]
    MissingExecutable,
    #[error(
        "Flect could not find a Git repository from {0}. Run this command inside a Git worktree."
    )]
    NotRepository(String),
    #[error(
        "the repository has no base revision. Create an initial commit before starting a Flect run"
    )]
    MissingBaseRevision,
    #[error("Git rejected revision `{revision}`: {details}")]
    InvalidRevision { revision: String, details: String },
    #[error("Git command `{command}` failed: {details}")]
    CommandFailed { command: String, details: String },
    #[error("Git returned non-UTF-8 output for `{0}`; Flect v0.1 requires UTF-8 repository paths")]
    NonUtf8Output(String),
    #[error("could not read changed file {path}: {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },
    #[error("refusing to capture untracked path {path}: {reason}")]
    UnsafeUntrackedPath { path: String, reason: String },
    #[error(
        "captured patch is {actual} bytes, exceeding the configured {limit}-byte limit; narrow the patch or raise `verification.max_patch_bytes`"
    )]
    PatchTooLarge { actual: u64, limit: u64 },
}

impl GitRepository {
    /// Finds the containing repository, including when called from a subdirectory.
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] when Git is missing or `start` is outside a worktree.
    pub fn discover(start: &Path) -> Result<Self, GitError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|error| map_spawn_error(&error))?;
        if !output.status.success() {
            return Err(GitError::NotRepository(start.display().to_string()));
        }
        let root = output_text(output, "git rev-parse --show-toplevel")?;
        Ok(Self {
            root: PathBuf::from(root.trim()),
        })
    }

    /// Repository root returned by Git.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves `HEAD` to the immutable base revision stored by a run.
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] when the repository has no commit or Git fails.
    pub fn head_revision(&self) -> Result<String, GitError> {
        let output = self.git(["rev-parse", "--verify", "HEAD"])?;
        if !output.status.success() {
            return Err(GitError::MissingBaseRevision);
        }
        Ok(output_text(output, "git rev-parse --verify HEAD")?
            .trim()
            .to_owned())
    }

    /// Resolves an arbitrary revision to a commit identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] when `revision` does not resolve to a commit.
    pub fn resolve_revision(&self, revision: &str) -> Result<String, GitError> {
        let output = self.git(["rev-parse", "--verify", &format!("{revision}^{{commit}}")])?;
        if !output.status.success() {
            return Err(GitError::InvalidRevision {
                revision: revision.to_owned(),
                details: stderr_text(&output),
            });
        }
        Ok(output_text(output, "git rev-parse --verify")?
            .trim()
            .to_owned())
    }

    /// Captures the worktree relative to `base_revision` without mutating Git state.
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] for invalid revisions, Git failures, unreadable files,
    /// non-UTF-8 text, or a patch larger than `max_patch_bytes`.
    pub fn capture_patch(
        &self,
        base_revision: &str,
        include_untracked: bool,
        respect_gitignore: bool,
        max_patch_bytes: u64,
    ) -> Result<PatchSet, GitError> {
        self.resolve_revision(base_revision)?;
        let output = self.git([
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "--no-ext-diff",
            base_revision,
        ])?;
        ensure_success(&output, "git diff --name-status")?;
        let status_bytes = output.stdout;
        let mut changes = parse_name_status(&status_bytes)?;

        if include_untracked {
            let arguments = if respect_gitignore {
                vec!["ls-files", "--others", "--exclude-standard", "-z"]
            } else {
                vec!["ls-files", "--others", "-z"]
            };
            let output = self.git(arguments)?;
            ensure_success(&output, "git ls-files --others")?;
            for path in split_nul_utf8(&output.stdout, "git ls-files --others")? {
                changes.push((FileStatus::Untracked, None, path));
            }
        }

        let mut files = Vec::with_capacity(changes.len());
        let mut total_bytes = 0_u64;
        for (status, old_path, path) in changes {
            let changed_file = if status == FileStatus::Untracked {
                self.capture_untracked(&path, max_patch_bytes.saturating_sub(total_bytes))?
            } else {
                self.capture_tracked(base_revision, status, old_path, path)?
            };
            total_bytes = total_bytes.saturating_add(changed_file.patch.len() as u64);
            if total_bytes > max_patch_bytes {
                return Err(GitError::PatchTooLarge {
                    actual: total_bytes,
                    limit: max_patch_bytes,
                });
            }
            files.push(changed_file);
        }

        files.sort_by(|left, right| left.path.cmp(&right.path));
        let insertions = files.iter().map(|file| file.insertions).sum();
        let deletions = files.iter().map(|file| file.deletions).sum();
        let renames = files
            .iter()
            .filter(|file| file.status == FileStatus::Renamed)
            .count() as u64;
        let binary_files = files
            .iter()
            .filter(|file| file.binary)
            .map(|file| file.path.clone())
            .collect();
        let untracked_files = files
            .iter()
            .filter(|file| file.status == FileStatus::Untracked)
            .map(|file| file.path.clone())
            .collect();

        Ok(PatchSet {
            base_revision: base_revision.to_owned(),
            files,
            renames,
            insertions,
            deletions,
            binary_files,
            untracked_files,
        })
    }

    fn capture_tracked(
        &self,
        base: &str,
        status: FileStatus,
        old_path: Option<String>,
        path: String,
    ) -> Result<ChangedFile, GitError> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.root).args([
            "diff",
            "--binary",
            "--find-renames",
            "--no-ext-diff",
            base,
            "--",
        ]);
        if let Some(old) = &old_path {
            command.arg(old);
        }
        command.arg(&path);
        let output = command.output().map_err(|error| map_spawn_error(&error))?;
        ensure_success(&output, "git diff")?;
        let diff_text = String::from_utf8(output.stdout)
            .map_err(|_| GitError::NonUtf8Output("git diff".to_owned()))?;
        let binary = diff_text.contains("GIT binary patch") || diff_text.contains("Binary files ");
        let (insertions, deletions) = if binary {
            (0, 0)
        } else {
            count_patch_lines(&diff_text)
        };
        Ok(ChangedFile {
            path,
            status,
            patch: diff_text,
            old_path,
            insertions,
            deletions,
            binary,
        })
    }

    fn capture_untracked(&self, path: &str, remaining_bytes: u64) -> Result<ChangedFile, GitError> {
        if Path::new(path)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(GitError::UnsafeUntrackedPath {
                path: path.to_owned(),
                reason: "path is not a repository-relative normal path".to_owned(),
            });
        }
        let absolute = self.root.join(path);
        let metadata = fs::symlink_metadata(&absolute).map_err(|source| GitError::ReadFile {
            path: absolute.display().to_string(),
            source,
        })?;
        if !metadata.file_type().is_file() {
            return Err(GitError::UnsafeUntrackedPath {
                path: path.to_owned(),
                reason: "symlinks and non-regular files are not captured".to_owned(),
            });
        }
        let canonical_root = fs::canonicalize(&self.root).map_err(|source| GitError::ReadFile {
            path: self.root.display().to_string(),
            source,
        })?;
        let canonical_file = fs::canonicalize(&absolute).map_err(|source| GitError::ReadFile {
            path: absolute.display().to_string(),
            source,
        })?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err(GitError::UnsafeUntrackedPath {
                path: path.to_owned(),
                reason: "resolved path is outside the repository".to_owned(),
            });
        }
        if metadata.len() > remaining_bytes {
            return Err(GitError::PatchTooLarge {
                actual: metadata.len(),
                limit: remaining_bytes,
            });
        }
        let bytes = read_bounded_regular_file(&absolute, path, remaining_bytes)?;
        let text = String::from_utf8(bytes).ok();
        let binary = text
            .as_ref()
            .is_none_or(|content| content.as_bytes().contains(&0));
        let (diff_text, insertions) = match text {
            Some(content) if !binary => {
                let added = content.lines().count() as u64;
                let mut untracked_diff = format!(
                    "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{added} @@\n"
                );
                for line in content.lines() {
                    untracked_diff.push('+');
                    untracked_diff.push_str(line);
                    untracked_diff.push('\n');
                }
                (untracked_diff, added)
            }
            _ => (
                format!(
                    "diff --git a/{path} b/{path}\nnew file mode 100644\nBinary file omitted\n"
                ),
                0,
            ),
        };
        Ok(ChangedFile {
            path: path.to_owned(),
            status: FileStatus::Untracked,
            patch: diff_text,
            old_path: None,
            insertions,
            deletions: 0,
            binary,
        })
    }

    fn git<I, S>(&self, args: I) -> Result<Output, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .map_err(|error| map_spawn_error(&error))
    }
}

fn read_bounded_regular_file(
    absolute: &Path,
    repository_path: &str,
    limit: u64,
) -> Result<Vec<u8>, GitError> {
    let file = fs::File::open(absolute).map_err(|source| GitError::ReadFile {
        path: absolute.display().to_string(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| GitError::ReadFile {
        path: absolute.display().to_string(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(GitError::UnsafeUntrackedPath {
            path: repository_path.to_owned(),
            reason: "opened object is not a regular file".to_owned(),
        });
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| GitError::ReadFile {
            path: absolute.display().to_string(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(GitError::PatchTooLarge {
            actual: bytes.len() as u64,
            limit,
        });
    }
    Ok(bytes)
}

fn parse_name_status(bytes: &[u8]) -> Result<Vec<(FileStatus, Option<String>, String)>, GitError> {
    let fields = split_nul_utf8(bytes, "git diff --name-status")?;
    let mut changes = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = &fields[index];
        let (status_text, inline_path) = field
            .split_once('\t')
            .map_or((field.as_str(), None), |(status, path)| {
                (status, Some(path))
            });
        let kind = status_text.chars().next().unwrap_or(' ');
        let status = match kind {
            'A' => FileStatus::Added,
            'M' | 'T' => FileStatus::Modified,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            other => {
                return Err(GitError::CommandFailed {
                    command: "git diff --name-status".to_owned(),
                    details: format!("unsupported file status `{other}`"),
                });
            }
        };
        index += 1;
        let first_path = if let Some(path) = inline_path {
            path.to_owned()
        } else {
            take_field(&fields, &mut index, "path")?
        };
        if status == FileStatus::Renamed {
            let new_path = take_field(&fields, &mut index, "rename destination")?;
            changes.push((status, Some(first_path), new_path));
        } else {
            changes.push((status, None, first_path));
        }
    }
    Ok(changes)
}

fn take_field(fields: &[String], index: &mut usize, name: &str) -> Result<String, GitError> {
    let value = fields
        .get(*index)
        .cloned()
        .ok_or_else(|| GitError::CommandFailed {
            command: "git diff --name-status".to_owned(),
            details: format!("missing {name} in Git output"),
        })?;
    *index += 1;
    Ok(value)
}

fn split_nul_utf8(bytes: &[u8], command: &str) -> Result<Vec<String>, GitError> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| {
            std::str::from_utf8(part)
                .map(str::to_owned)
                .map_err(|_| GitError::NonUtf8Output(command.to_owned()))
        })
        .collect()
}

fn count_patch_lines(patch: &str) -> (u64, u64) {
    let mut insertions = 0;
    let mut deletions = 0;
    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            insertions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }
    (insertions, deletions)
}

fn ensure_success(output: &Output, command: &str) -> Result<(), GitError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: command.to_owned(),
            details: stderr_text(output),
        })
    }
}

fn output_text(output: Output, command: &str) -> Result<String, GitError> {
    ensure_success(&output, command)?;
    String::from_utf8(output.stdout).map_err(|_| GitError::NonUtf8Output(command.to_owned()))
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

fn map_spawn_error(error: &std::io::Error) -> GitError {
    if error.kind() == std::io::ErrorKind::NotFound {
        GitError::MissingExecutable
    } else {
        GitError::CommandFailed {
            command: "git".to_owned(),
            details: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_hunk_lines() {
        let patch = "--- a/x\n+++ b/x\n@@\n-old\n+new\n context\n";
        assert_eq!(count_patch_lines(patch), (1, 1));
    }

    #[test]
    fn parses_nul_separated_statuses() {
        let parsed = parse_name_status(b"M\0src/a.rs\0R100\0old.rs\0new.rs\0").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].1.as_deref(), Some("old.rs"));
        assert_eq!(parsed[1].2, "new.rs");
    }
}
