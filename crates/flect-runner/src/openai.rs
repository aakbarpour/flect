//! OpenAI-compatible Responses API transport.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AgentRequest, AgentRunner, JsonSchema, RunnerError, RunnerMetadata, RunnerOutput, TokenUsage,
    validate_output,
};

/// Configuration for a Responses-compatible HTTP endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiResponsesConfig {
    pub base_url: String,
    pub api_key_env: String,
    pub model: String,
    pub reasoning_effort: String,
    pub timeout: Duration,
}

/// Structured-output runner for `OpenAI` and Responses-compatible providers.
#[derive(Debug, Clone)]
pub struct OpenAiResponsesRunner {
    client: reqwest::Client,
    endpoint: Url,
    api_key: String,
    api_key_env: String,
    model: String,
    reasoning_effort: String,
    timeout_seconds: u64,
}

impl OpenAiResponsesRunner {
    /// Builds a runner and reads its credential from the configured environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`RunnerError`] for a missing credential, invalid URL, or HTTP client failure.
    pub fn from_env(config: OpenAiResponsesConfig) -> Result<Self, RunnerError> {
        let api_key = std::env::var(&config.api_key_env)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| RunnerError::MissingCredential {
                variable: config.api_key_env.clone(),
            })?;
        Self::new(config, api_key)
    }

    /// Builds a runner with an explicit credential, primarily for controlled embedding and tests.
    ///
    /// # Errors
    ///
    /// Returns [`RunnerError`] for an empty credential, invalid URL, or HTTP client failure.
    pub fn new(config: OpenAiResponsesConfig, api_key: String) -> Result<Self, RunnerError> {
        if api_key.trim().is_empty() {
            return Err(RunnerError::MissingCredential {
                variable: config.api_key_env,
            });
        }
        let base_url = ensure_trailing_slash(&config.base_url);
        let base = Url::parse(&base_url).map_err(|error| RunnerError::InvalidBaseUrl {
            url: config.base_url.clone(),
            details: error.to_string(),
        })?;
        if base.scheme() != "http" && base.scheme() != "https" {
            return Err(RunnerError::InvalidBaseUrl {
                url: config.base_url,
                details: "scheme must be http or https".to_owned(),
            });
        }
        let endpoint = base
            .join("responses")
            .map_err(|error| RunnerError::InvalidBaseUrl {
                url: base.to_string(),
                details: error.to_string(),
            })?;
        let timeout_seconds = config.timeout.as_secs();
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(concat!("flect/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| RunnerError::ClientInitialization(error.to_string()))?;
        Ok(Self {
            client,
            endpoint,
            api_key,
            api_key_env: config.api_key_env,
            model: config.model,
            reasoning_effort: config.reasoning_effort,
            timeout_seconds,
        })
    }

    /// Configured model identifier.
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl AgentRunner for OpenAiResponsesRunner {
    async fn generate_structured(
        &self,
        request: &AgentRequest,
        schema: &JsonSchema,
    ) -> Result<RunnerOutput, RunnerError> {
        let body = json!({
            "model": self.model,
            "store": false,
            "reasoning": {"effort": self.reasoning_effort},
            "input": [
                {"role": "system", "content": request.purpose.instruction()},
                {"role": "user", "content": serde_json::to_string(&request.input).map_err(|error| RunnerError::InvalidJson(error.to_string()))?}
            ],
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": request.purpose.schema_name(),
                    "strict": true,
                    "schema": schema
                }
            }
        });

        let started = Instant::now();
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| self.transport_error(&error))?;
        let latency_ms = elapsed_millis(started);
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .bytes()
            .await
            .map_err(|error| RunnerError::Network {
                url: redacted_endpoint(&self.endpoint),
                details: error.to_string(),
            })?;

        if !status.is_success() {
            return Err(http_error(
                status,
                &bytes,
                &self.api_key_env,
                retry_after.as_deref(),
            ));
        }

        let envelope: ResponsesEnvelope = serde_json::from_slice(&bytes)
            .map_err(|error| RunnerError::InvalidJson(error.to_string()))?;
        if envelope.status.as_deref() == Some("incomplete") {
            return Err(RunnerError::Incomplete(
                envelope
                    .incomplete_details
                    .and_then(|details| details.reason)
                    .unwrap_or_else(|| "provider did not report a reason".to_owned()),
            ));
        }
        let text = extract_output_text(&envelope.output)?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| RunnerError::InvalidJson(error.to_string()))?;
        validate_output(schema, &value)?;

        Ok(RunnerOutput {
            value,
            metadata: RunnerMetadata {
                provider: "openai-compatible".to_owned(),
                model: envelope.model.unwrap_or_else(|| self.model.clone()),
                latency_ms,
                usage: envelope.usage.map_or_else(TokenUsage::default, Into::into),
            },
        })
    }
}

impl OpenAiResponsesRunner {
    fn transport_error(&self, error: &reqwest::Error) -> RunnerError {
        if error.is_timeout() {
            RunnerError::Timeout {
                seconds: self.timeout_seconds,
            }
        } else {
            RunnerError::Network {
                url: redacted_endpoint(&self.endpoint),
                details: error.to_string(),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResponsesEnvelope {
    status: Option<String>,
    model: Option<String>,
    #[serde(default)]
    output: Vec<OutputItem>,
    incomplete_details: Option<IncompleteDetails>,
    usage: Option<ResponseUsage>,
}

#[derive(Debug, Deserialize)]
struct IncompleteDetails {
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    #[serde(default)]
    content: Vec<ContentItem>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentItem {
    OutputText {
        text: String,
    },
    Refusal {
        refusal: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ResponseUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    input_tokens_details: Option<InputTokenDetails>,
}

#[derive(Debug, Deserialize)]
struct InputTokenDetails {
    cached_tokens: Option<u64>,
}

impl From<ResponseUsage> for TokenUsage {
    fn from(usage: ResponseUsage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage
                .input_tokens_details
                .and_then(|details| details.cached_tokens),
            output_tokens: usage.output_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: Option<ProviderError>,
}

#[derive(Debug, Deserialize)]
struct ProviderError {
    message: Option<String>,
}

fn extract_output_text(output: &[OutputItem]) -> Result<String, RunnerError> {
    for item in output {
        for content in &item.content {
            match content {
                ContentItem::OutputText { text } => return Ok(text.clone()),
                ContentItem::Refusal { refusal } => {
                    return Err(RunnerError::Refusal(truncate(refusal, 500)));
                }
                ContentItem::Other => {}
            }
        }
    }
    Err(RunnerError::MissingOutput)
}

fn http_error(
    status: StatusCode,
    body: &[u8],
    api_key_env: &str,
    retry_after: Option<&str>,
) -> RunnerError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return RunnerError::Authentication {
            variable: api_key_env.to_owned(),
        };
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry = retry_after.map_or_else(String::new, |value| format!("; retry after {value}"));
        return RunnerError::RateLimited { retry };
    }
    let message = serde_json::from_slice::<ErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| {
            status
                .canonical_reason()
                .unwrap_or("provider error")
                .to_owned()
        });
    let message = truncate(&message, 500);
    if status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY {
        RunnerError::UnsupportedRequest { message }
    } else {
        RunnerError::Provider {
            status: status.as_u16(),
            message,
        }
    }
}

fn ensure_trailing_slash(base_url: &str) -> String {
    format!("{}/", base_url.trim_end_matches('/'))
}

fn redacted_endpoint(endpoint: &Url) -> String {
    format!(
        "{}://{}{}",
        endpoint.scheme(),
        endpoint.host_str().unwrap_or("unknown-host"),
        endpoint.path()
    )
}

fn truncate(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
