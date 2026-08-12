//! Provider-neutral execution boundary for structured agent calls.

mod openai;

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
pub use openai::{OpenAiResponsesConfig, OpenAiResponsesRunner};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// A provider-neutral request containing only a purpose and sanitized input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub purpose: RequestPurpose,
    pub input: Value,
}

/// Narrow task the provider is asked to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPurpose {
    ReconstructPatchIntent,
    AnalyzeForwardIntent,
    ReconcileIntent,
}

impl RequestPurpose {
    fn schema_name(self) -> &'static str {
        match self {
            Self::ReconstructPatchIntent => "echoed_spec",
            Self::AnalyzeForwardIntent => "intended_spec",
            Self::ReconcileIntent => "verdict",
        }
    }

    fn instruction(self) -> &'static str {
        match self {
            Self::ReconstructPatchIntent => {
                "You are an independent blind patch verifier. Reconstruct only what the supplied patch and sanitized context demonstrate. You have no access to the original task, forward specification, conversation, branch name, commit messages, or issue metadata. Never infer or claim access to them. Distinguish behavior before from behavior after, identify affected scope and side effects, and preserve uncertainty. Do not invent file names, line numbers, or evidence."
            }
            Self::AnalyzeForwardIntent => {
                "Convert only the supplied original task into a faithful implementation specification. Preserve explicit requirements, constraints, non-goals, acceptance criteria, expected scope, and ambiguities. Do not inspect implementation evidence and do not invent requirements."
            }
            Self::ReconcileIntent => {
                "Compare the intended specification with the independently reconstructed specification. Classify alignment conservatively as SAME, PARTIAL, DIFFERENT, or UNCERTAIN. Every negative finding must have a corresponding evidence entry. Use a file, line range, or patch hunk only when it appears verbatim in available_evidence; otherwise leave location fields null and explain the evidentiary limitation. Never fabricate evidence."
            }
        }
    }
}

/// JSON Schema supplied to providers that support structured output.
pub type JsonSchema = Value;

/// Token accounting reported by a provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Observable metadata from one runner request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerMetadata {
    pub provider: String,
    pub model: String,
    pub latency_ms: u64,
    pub usage: TokenUsage,
}

/// Structured provider output plus request metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerOutput {
    pub value: Value,
    pub metadata: RunnerMetadata,
}

/// Object-safe provider boundary for structured model calls.
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Produces and validates JSON matching `schema`.
    ///
    /// # Errors
    ///
    /// Returns [`RunnerError`] when the provider cannot produce valid structured output.
    async fn generate_structured(
        &self,
        request: &AgentRequest,
        schema: &JsonSchema,
    ) -> Result<RunnerOutput, RunnerError>;
}

/// Provider invocation failed.
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("mock runner has no response configured for this request")]
    MissingMockResponse,
    #[error("mock runner state is unavailable because a previous caller panicked")]
    PoisonedMock,
    #[error("API credential environment variable `{variable}` is not set or is empty")]
    MissingCredential { variable: String },
    #[error("runner base URL `{url}` is invalid: {details}")]
    InvalidBaseUrl { url: String, details: String },
    #[error("could not initialize the HTTP client: {0}")]
    ClientInitialization(String),
    #[error("the provider rejected authentication; check `{variable}` and the configured base URL")]
    Authentication { variable: String },
    #[error("the provider rate limit was reached{retry}")]
    RateLimited { retry: String },
    #[error("the provider request timed out after {seconds} seconds")]
    Timeout { seconds: u64 },
    #[error("could not reach the provider at {url}: {details}")]
    Network { url: String, details: String },
    #[error("the provider rejected this request as unsupported: {message}")]
    UnsupportedRequest { message: String },
    #[error("provider request failed with HTTP {status}: {message}")]
    Provider { status: u16, message: String },
    #[error("the provider refused the structured request: {0}")]
    Refusal(String),
    #[error("the provider returned an incomplete response: {0}")]
    Incomplete(String),
    #[error("the provider response did not contain structured output text")]
    MissingOutput,
    #[error("provider returned invalid JSON: {0}")]
    InvalidJson(String),
    #[error("provider output did not satisfy the requested schema: {0}")]
    SchemaValidation(String),
}

/// Deterministic, in-memory runner used by tests and offline workflows.
#[derive(Debug)]
pub struct MockRunner {
    responses: Mutex<VecDeque<Value>>,
    model: String,
}

impl MockRunner {
    /// Creates a runner that returns the supplied responses in order.
    pub fn new(responses: impl IntoIterator<Item = Value>) -> Self {
        Self::named("mock", responses)
    }

    /// Creates a deterministic runner that reports a chosen model name.
    pub fn named(model: impl Into<String>, responses: impl IntoIterator<Item = Value>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            model: model.into(),
        }
    }

    /// Creates a runner with one serializable response.
    ///
    /// # Errors
    ///
    /// Returns [`RunnerError::InvalidJson`] when serialization fails.
    pub fn with_response<T: Serialize>(response: &T) -> Result<Self, RunnerError> {
        let value = serde_json::to_value(response)
            .map_err(|error| RunnerError::InvalidJson(error.to_string()))?;
        Ok(Self::new([value]))
    }
}

#[async_trait]
impl AgentRunner for MockRunner {
    async fn generate_structured(
        &self,
        _request: &AgentRequest,
        schema: &JsonSchema,
    ) -> Result<RunnerOutput, RunnerError> {
        let value = self
            .responses
            .lock()
            .map_err(|_| RunnerError::PoisonedMock)?
            .pop_front()
            .ok_or(RunnerError::MissingMockResponse)?;
        validate_output(schema, &value)?;
        Ok(RunnerOutput {
            value,
            metadata: RunnerMetadata {
                provider: "mock".to_owned(),
                model: self.model.clone(),
                latency_ms: 0,
                usage: TokenUsage::default(),
            },
        })
    }
}

fn validate_output(schema: &JsonSchema, value: &Value) -> Result<(), RunnerError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| RunnerError::SchemaValidation(error.to_string()))?;
    let errors = validator
        .iter_errors(value)
        .take(5)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RunnerError::SchemaValidation(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request() -> AgentRequest {
        AgentRequest {
            purpose: RequestPurpose::ReconstructPatchIntent,
            input: Value::Null,
        }
    }

    #[tokio::test]
    async fn returns_responses_in_order() {
        let runner = MockRunner::new([json!({"ok": 1}), json!({"ok": 2})]);
        let schema = json!({
            "type": "object",
            "properties": {"ok": {"type": "integer"}},
            "required": ["ok"]
        });
        assert_eq!(
            runner
                .generate_structured(&request(), &schema)
                .await
                .unwrap()
                .value,
            json!({"ok": 1})
        );
        assert_eq!(
            runner
                .generate_structured(&request(), &schema)
                .await
                .unwrap()
                .value,
            json!({"ok": 2})
        );
    }

    #[tokio::test]
    async fn mock_validates_schema() {
        let runner = MockRunner::new([json!({"ok": "not an integer"})]);
        let schema = json!({
            "type": "object",
            "properties": {"ok": {"type": "integer"}},
            "required": ["ok"]
        });
        assert!(matches!(
            runner.generate_structured(&request(), &schema).await,
            Err(RunnerError::SchemaValidation(_))
        ));
    }
}
