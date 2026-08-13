//! Versioned, project-local run persistence.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::domain::{RunRecord, VerificationRecord};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Storage rooted at `.flect/` in a project.
#[derive(Debug, Clone)]
pub struct RunStore {
    root: PathBuf,
}

/// Run state could not be loaded or persisted.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("could not create Flect state directory {path}: {source}")]
    CreateDirectory {
        path: String,
        source: std::io::Error,
    },
    #[error("could not read Flect state at {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("could not write Flect state at {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("Flect state at {path} is invalid: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
    #[error("no Flect runs exist in this repository; run `flect start --task ...` first")]
    NoRuns,
    #[error("Flect run `{0}` does not exist in this repository")]
    RunNotFound(String),
    #[error("verification result for Flect run `{0}` does not exist; run `flect verify` first")]
    VerificationNotFound(String),
    #[error("Flect state uses unsupported version {0}; this release supports version 1")]
    UnsupportedVersion(u32),
    #[error("invalid Flect run identifier `{0}`")]
    InvalidRunId(String),
}

impl RunStore {
    /// Creates a store handle without touching the filesystem.
    pub fn new(repository_root: &Path) -> Self {
        Self {
            root: repository_root.join(".flect"),
        }
    }

    /// Persists a run and marks it as the latest run.
    ///
    /// # Errors
    ///
    /// Returns [`StateError`] when state directories or files cannot be written.
    pub fn save_run(&self, run: &RunRecord) -> Result<(), StateError> {
        validate_run_id(&run.id)?;
        let runs = self.root.join("runs");
        fs::create_dir_all(&runs).map_err(|source| StateError::CreateDirectory {
            path: runs.display().to_string(),
            source,
        })?;
        write_json(&runs.join(format!("{}.json", run.id)), run)?;
        write_text(&self.root.join("latest"), &format!("{}\n", run.id))
    }

    /// Loads a run by ID or the latest run when ID is omitted.
    ///
    /// # Errors
    ///
    /// Returns [`StateError`] when no matching run exists or its JSON is invalid.
    pub fn load_run(&self, id: Option<&str>) -> Result<RunRecord, StateError> {
        let id = match id {
            Some(id) => id.to_owned(),
            None => self.latest_id()?,
        };
        validate_run_id(&id)?;
        let path = self.root.join("runs").join(format!("{id}.json"));
        if !path.exists() {
            return Err(StateError::RunNotFound(id));
        }
        let run: RunRecord = read_json(&path)?;
        if run.version != 1 {
            return Err(StateError::UnsupportedVersion(run.version));
        }
        if run.id != id {
            return Err(StateError::InvalidRunId(run.id));
        }
        Ok(run)
    }

    /// Persists the result separately from immutable run input.
    ///
    /// # Errors
    ///
    /// Returns [`StateError`] when result serialization or writing fails.
    pub fn save_verification(&self, result: &VerificationRecord) -> Result<(), StateError> {
        validate_run_id(&result.run_id)?;
        let directory = self.root.join("results");
        fs::create_dir_all(&directory).map_err(|source| StateError::CreateDirectory {
            path: directory.display().to_string(),
            source,
        })?;
        write_json(&directory.join(format!("{}.json", result.run_id)), result)
    }

    /// Loads a verification result by run ID, or for the latest run when omitted.
    ///
    /// # Errors
    ///
    /// Returns [`StateError`] when no matching result exists or its JSON is invalid.
    pub fn load_verification(&self, id: Option<&str>) -> Result<VerificationRecord, StateError> {
        let id = match id {
            Some(id) => id.to_owned(),
            None => self.latest_id()?,
        };
        validate_run_id(&id)?;
        let path = self.root.join("results").join(format!("{id}.json"));
        if !path.exists() {
            return Err(StateError::VerificationNotFound(id));
        }
        let result: VerificationRecord = read_json(&path)?;
        if result.version != 1 {
            return Err(StateError::UnsupportedVersion(result.version));
        }
        Ok(result)
    }

    fn latest_id(&self) -> Result<String, StateError> {
        let path = self.root.join("latest");
        if !path.exists() {
            return Err(StateError::NoRuns);
        }
        let id = fs::read_to_string(&path).map_err(|source| StateError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let id = id.trim();
        if id.is_empty() {
            return Err(StateError::NoRuns);
        }
        validate_run_id(id)?;
        Ok(id.to_owned())
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, StateError> {
    let bytes = fs::read(path).map_err(|source| StateError::Read {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| StateError::Parse {
        path: path.display().to_string(),
        source,
    })
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), StateError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| StateError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    write_bytes(path, &bytes)
}

fn write_text(path: &Path, text: &str) -> Result<(), StateError> {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), StateError> {
    let parent = path.parent().ok_or_else(|| StateError::Write {
        path: path.display().to_string(),
        source: std::io::Error::other("state path has no parent directory"),
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|source| StateError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// Validates the single grammar used for persisted and user-supplied run IDs.
///
/// # Errors
///
/// Returns [`StateError::InvalidRunId`] unless `id` is `fl_` followed by
/// exactly sixteen lowercase hexadecimal characters.
pub fn validate_run_id(id: &str) -> Result<(), StateError> {
    let valid = id.strip_prefix("fl_").is_some_and(|suffix| {
        suffix.len() == 16
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if valid {
        Ok(())
    } else {
        Err(StateError::InvalidRunId(id.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{IntendedSpec, TaskInput};

    #[test]
    fn run_round_trip() {
        let temporary = tempfile::tempdir().unwrap();
        let store = RunStore::new(temporary.path());
        let run = RunRecord {
            version: 1,
            id: "fl_0123456789abcdef".to_owned(),
            repository_root: temporary.path().display().to_string(),
            base_revision: "abc".to_owned(),
            task: TaskInput {
                text: "Fix it".to_owned(),
            },
            intended_spec: IntendedSpec::default(),
            model_calls: Vec::new(),
            created_unix_ms: 0,
        };
        store.save_run(&run).unwrap();
        assert_eq!(store.load_run(None).unwrap(), run);
    }

    #[test]
    fn rejects_future_run_versions() {
        let temporary = tempfile::tempdir().unwrap();
        let store = RunStore::new(temporary.path());
        let run = RunRecord {
            version: 2,
            id: "fl_fedcba9876543210".to_owned(),
            repository_root: temporary.path().display().to_string(),
            base_revision: "abc".to_owned(),
            task: TaskInput {
                text: "Fix it".to_owned(),
            },
            intended_spec: IntendedSpec::default(),
            model_calls: Vec::new(),
            created_unix_ms: 0,
        };
        store.save_run(&run).unwrap();
        assert!(matches!(
            store.load_run(None),
            Err(StateError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn rejects_path_like_run_ids() {
        let temporary = tempfile::tempdir().unwrap();
        let store = RunStore::new(temporary.path());
        for id in [
            "../fl_0123456789abcdef",
            "/tmp/fl_0123456789abcdef",
            r"..\fl_0123456789abcdef",
            r"C:\fl_0123456789abcdef",
            "fl_0123456789ABCDEf",
            "fl_short",
        ] {
            assert!(matches!(
                store.load_run(Some(id)),
                Err(StateError::InvalidRunId(_))
            ));
        }
    }

    #[test]
    fn atomically_replaces_existing_run() {
        let temporary = tempfile::tempdir().unwrap();
        let store = RunStore::new(temporary.path());
        let mut run = RunRecord {
            version: 1,
            id: "fl_0123456789abcdef".to_owned(),
            repository_root: temporary.path().display().to_string(),
            base_revision: "first".to_owned(),
            task: TaskInput {
                text: "Fix it".to_owned(),
            },
            intended_spec: IntendedSpec::default(),
            model_calls: Vec::new(),
            created_unix_ms: 0,
        };
        store.save_run(&run).unwrap();
        run.base_revision = "second".to_owned();
        store.save_run(&run).unwrap();
        assert_eq!(store.load_run(None).unwrap(), run);
        assert_eq!(
            fs::read_dir(temporary.path().join(".flect/runs"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn malformed_latest_identifier_fails_closed() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join(".flect");
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join("latest"), "../outside\n").unwrap();
        assert!(matches!(
            RunStore::new(temporary.path()).load_run(None),
            Err(StateError::InvalidRunId(_))
        ));
    }
}
