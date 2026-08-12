//! Provider-neutral execution boundary for structured agent calls.
//!
//! Milestone 1 ships only [`MockRunner`]. Network providers are intentionally
//! deferred until the deterministic pipeline is stable.

use std::collections::VecDeque;
use std::sync::Mutex;

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

/// JSON Schema supplied to providers that support structured output.
pub type JsonSchema = Value;

/// A deliberately non-generic boundary that remains object-safe for providers.
pub trait AgentRunner: Send + Sync {
    /// Produces JSON matching `schema`, leaving typed deserialization to callers.
    ///
    /// # Errors
    ///
    /// Returns [`RunnerError`] when the provider cannot produce structured output.
    fn generate_structured(
        &self,
        request: &AgentRequest,
        schema: &JsonSchema,
    ) -> Result<Value, RunnerError>;
}

/// Provider invocation failed.
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("mock runner has no response configured for this request")]
    MissingMockResponse,
    #[error("mock runner state is unavailable because a previous caller panicked")]
    PoisonedMock,
    #[error("provider returned invalid structured output: {0}")]
    InvalidStructuredOutput(String),
    #[error("runner provider `{0}` is not available in this Flect milestone")]
    ProviderUnavailable(String),
}

/// Deterministic, in-memory runner used by tests and local dry workflows.
#[derive(Debug)]
pub struct MockRunner {
    responses: Mutex<VecDeque<Value>>,
}

impl MockRunner {
    /// Creates a runner that returns the supplied responses in order.
    pub fn new(responses: impl IntoIterator<Item = Value>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    /// Creates a runner with one serializable response.
    ///
    /// # Errors
    ///
    /// Returns [`RunnerError::InvalidStructuredOutput`] when serialization fails.
    pub fn with_response<T: Serialize>(response: &T) -> Result<Self, RunnerError> {
        let value = serde_json::to_value(response)
            .map_err(|error| RunnerError::InvalidStructuredOutput(error.to_string()))?;
        Ok(Self::new([value]))
    }
}

impl AgentRunner for MockRunner {
    fn generate_structured(
        &self,
        _request: &AgentRequest,
        _schema: &JsonSchema,
    ) -> Result<Value, RunnerError> {
        self.responses
            .lock()
            .map_err(|_| RunnerError::PoisonedMock)?
            .pop_front()
            .ok_or(RunnerError::MissingMockResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_responses_in_order() {
        let runner = MockRunner::new([Value::from(1), Value::from(2)]);
        let request = AgentRequest {
            purpose: RequestPurpose::ReconstructPatchIntent,
            input: Value::Null,
        };
        assert_eq!(
            runner.generate_structured(&request, &Value::Null).unwrap(),
            Value::from(1)
        );
        assert_eq!(
            runner.generate_structured(&request, &Value::Null).unwrap(),
            Value::from(2)
        );
    }
}
