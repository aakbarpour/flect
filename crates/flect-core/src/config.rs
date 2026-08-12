//! Strict, versioned project configuration.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::ContextPolicy;

/// Configured runner implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    #[default]
    Mock,
    Api,
}

impl std::fmt::Display for RunnerKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mock => formatter.write_str("mock"),
            Self::Api => formatter.write_str("api"),
        }
    }
}

/// Wire protocol used by an API runner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerProtocol {
    #[default]
    Responses,
}

/// Top-level contents of `flect.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub verification: VerificationConfig,
    pub runner: RunnerConfig,
    pub blind: BlindConfig,
    pub privacy: PrivacyConfig,
    pub ignore: IgnoreConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            verification: VerificationConfig::default(),
            runner: RunnerConfig::default(),
            blind: BlindConfig::default(),
            privacy: PrivacyConfig::default(),
            ignore: IgnoreConfig::default(),
        }
    }
}

/// Verification pipeline limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerificationConfig {
    pub context: ContextPolicy,
    pub max_iterations: u8,
    pub include_untracked: bool,
    pub max_patch_bytes: u64,
    pub max_context_file_bytes: u64,
    pub max_context_bytes: u64,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            context: ContextPolicy::Focused,
            max_iterations: 2,
            include_untracked: true,
            max_patch_bytes: 1_000_000,
            max_context_file_bytes: 128_000,
            max_context_bytes: 512_000,
        }
    }
}

/// Provider-neutral runner configuration. Credentials are referenced by environment name only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunnerConfig {
    #[serde(alias = "provider")]
    pub kind: RunnerKind,
    pub protocol: RunnerProtocol,
    pub base_url: String,
    pub api_key_env: String,
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    pub reasoning_effort: String,
    pub timeout_seconds: u64,
    pub escalate_on_uncertain: bool,
    pub confidence_threshold: f64,
    pub complexity_file_threshold: usize,
    pub complexity_byte_threshold: u64,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            kind: RunnerKind::Mock,
            protocol: RunnerProtocol::Responses,
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key_env: "OPENAI_API_KEY".to_owned(),
            model: None,
            fallback_model: None,
            reasoning_effort: "medium".to_owned(),
            timeout_seconds: 120,
            escalate_on_uncertain: true,
            confidence_threshold: 0.65,
            complexity_file_threshold: 12,
            complexity_byte_threshold: 200_000,
        }
    }
}

/// Metadata excluded from blind verifier requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BlindConfig {
    pub strip_git_metadata: bool,
    pub strip_branch_name: bool,
    pub strip_commit_messages: bool,
}

impl Default for BlindConfig {
    fn default() -> Self {
        Self {
            strip_git_metadata: true,
            strip_branch_name: true,
            strip_commit_messages: true,
        }
    }
}

/// Privacy controls applied before constructing verifier context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrivacyConfig {
    pub respect_gitignore: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            respect_gitignore: true,
        }
    }
}

/// Additional project-specific path patterns to exclude.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IgnoreConfig {
    pub patterns: Vec<String>,
}

/// Configuration loading or validation failed.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read Flect configuration at {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid Flect configuration at {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("unsupported configuration version {0}; this Flect release supports version 1")]
    UnsupportedVersion(u32),
    #[error("configuration value `{field}` is invalid: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
}

impl Config {
    /// Loads `flect.toml`, using defaults if the file is absent.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the file cannot be read, parsed, or validated.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let config: Self = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Serializes the default configuration installed by `flect init`.
    pub fn default_document() -> &'static str {
        r#"version = 1

[verification]
context = "focused"
max_iterations = 2
include_untracked = true
max_patch_bytes = 1000000
max_context_file_bytes = 128000
max_context_bytes = 512000

[runner]
kind = "mock"
protocol = "responses"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
reasoning_effort = "medium"
timeout_seconds = 120
escalate_on_uncertain = true
confidence_threshold = 0.65
complexity_file_threshold = 12
complexity_byte_threshold = 200000

[blind]
strip_git_metadata = true
strip_branch_name = true
strip_commit_messages = true

[privacy]
respect_gitignore = true

[ignore]
patterns = []
"#
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::UnsupportedVersion(self.version));
        }
        if self.verification.max_iterations == 0 {
            return Err(ConfigError::Invalid {
                field: "verification.max_iterations",
                message: "must be at least 1".to_owned(),
            });
        }
        if !(0.0..=1.0).contains(&self.runner.confidence_threshold) {
            return Err(ConfigError::Invalid {
                field: "runner.confidence_threshold",
                message: "must be between 0 and 1".to_owned(),
            });
        }
        if self.runner.timeout_seconds == 0 {
            return Err(ConfigError::Invalid {
                field: "runner.timeout_seconds",
                message: "must be at least 1".to_owned(),
            });
        }
        if self.runner.complexity_file_threshold == 0 {
            return Err(ConfigError::Invalid {
                field: "runner.complexity_file_threshold",
                message: "must be at least 1".to_owned(),
            });
        }
        if self.runner.complexity_byte_threshold == 0 {
            return Err(ConfigError::Invalid {
                field: "runner.complexity_byte_threshold",
                message: "must be at least 1".to_owned(),
            });
        }
        if self.runner.api_key_env.trim().is_empty() {
            return Err(ConfigError::Invalid {
                field: "runner.api_key_env",
                message: "cannot be empty".to_owned(),
            });
        }
        if self.runner.kind == RunnerKind::Api
            && self
                .runner
                .model
                .as_deref()
                .is_none_or(|model| model.trim().is_empty())
        {
            return Err(ConfigError::Invalid {
                field: "runner.model",
                message: "is required when runner.kind is `api`".to_owned(),
            });
        }
        if self.verification.max_context_file_bytes > self.verification.max_context_bytes {
            return Err(ConfigError::Invalid {
                field: "verification.max_context_file_bytes",
                message: "cannot exceed verification.max_context_bytes".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let parsed: Config = toml::from_str(Config::default_document()).unwrap();
        assert_eq!(parsed, Config::default());
        parsed.validate().unwrap();
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = toml::from_str::<Config>("version = 1\nmagic = true").unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn validates_threshold() {
        let mut config = Config::default();
        config.runner.confidence_threshold = 2.0;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::Invalid { .. })
        ));
    }

    #[test]
    fn api_runner_requires_model() {
        let mut config = Config::default();
        config.runner.kind = RunnerKind::Api;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "runner.model",
                ..
            })
        ));
    }

    #[test]
    fn accepts_legacy_provider_field() {
        let parsed: Config = toml::from_str("version = 1\n[runner]\nprovider = 'mock'").unwrap();
        assert_eq!(parsed.runner.kind, RunnerKind::Mock);
    }
}
