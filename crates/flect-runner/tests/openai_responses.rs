use std::fmt::Write as _;
use std::time::Duration;

use flect_runner::{
    AgentRequest, AgentRunner, OpenAiResponsesConfig, OpenAiResponsesRunner, RequestPurpose,
    RunnerError,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[tokio::test]
async fn sends_responses_structured_output_request_and_records_usage() {
    let response = json!({
        "status": "completed",
        "model": "test-model-2026",
        "output": [{
            "type": "message",
            "content": [{"type": "output_text", "text": "{\"answer\":\"yes\"}"}]
        }],
        "usage": {
            "input_tokens": 42,
            "output_tokens": 7,
            "input_tokens_details": {"cached_tokens": 12}
        }
    });
    let (base_url, request_rx) = server(200, &response.to_string(), &[]).await;
    let runner = runner(&base_url, "secret-test-key");

    let output = runner
        .generate_structured(&request(), &schema())
        .await
        .unwrap();

    assert_eq!(output.value, json!({"answer": "yes"}));
    assert_eq!(output.metadata.model, "test-model-2026");
    assert_eq!(output.metadata.usage.input_tokens, Some(42));
    assert_eq!(output.metadata.usage.cached_input_tokens, Some(12));
    assert_eq!(output.metadata.usage.output_tokens, Some(7));

    let raw_request = request_rx.await.unwrap();
    assert!(raw_request.starts_with("POST /v1/responses HTTP/1.1"));
    assert!(raw_request.contains("authorization: Bearer secret-test-key"));
    let body = request_body(&raw_request);
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["store"], false);
    assert_eq!(body["reasoning"]["effort"], "medium");
    assert_eq!(body["text"]["format"]["type"], "json_schema");
    assert_eq!(body["text"]["format"]["strict"], true);
    assert_eq!(body["text"]["format"]["name"], "echoed_spec");
}

#[tokio::test]
async fn maps_authentication_without_exposing_the_key() {
    let (base_url, _request_rx) =
        server(401, r#"{"error":{"message":"invalid credential"}}"#, &[]).await;
    let runner = runner(&base_url, "must-never-appear");

    let error = runner
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();

    assert!(matches!(error, RunnerError::Authentication { .. }));
    assert!(!error.to_string().contains("must-never-appear"));
    assert!(error.to_string().contains("TEST_API_KEY"));
}

#[tokio::test]
async fn maps_rate_limit_with_retry_hint() {
    let (base_url, _request_rx) = server(
        429,
        r#"{"error":{"message":"slow down"}}"#,
        &[("Retry-After", "17")],
    )
    .await;
    let error = runner(&base_url, "key")
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::RateLimited { .. }));
    assert!(error.to_string().contains("17"));
}

#[tokio::test]
async fn maps_unsupported_request_details() {
    let (base_url, _request_rx) = server(
        400,
        r#"{"error":{"message":"reasoning effort is unsupported"}}"#,
        &[],
    )
    .await;
    let error = runner(&base_url, "key")
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::UnsupportedRequest { .. }));
    assert!(
        error
            .to_string()
            .contains("reasoning effort is unsupported")
    );
}

#[tokio::test]
async fn maps_provider_failures() {
    let (base_url, _request_rx) =
        server(500, r#"{"error":{"message":"provider unavailable"}}"#, &[]).await;
    let error = runner(&base_url, "key")
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::Provider { status: 500, .. }));
    assert!(error.to_string().contains("provider unavailable"));
}

#[tokio::test]
async fn maps_timeouts() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let runner = OpenAiResponsesRunner::new(
        OpenAiResponsesConfig {
            base_url: format!("http://{address}/v1"),
            api_key_env: "TEST_API_KEY".to_owned(),
            model: "test-model".to_owned(),
            reasoning_effort: "medium".to_owned(),
            timeout: Duration::from_millis(20),
        },
        "key".to_owned(),
    )
    .unwrap();
    let error = runner
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::Timeout { .. }));
}

#[tokio::test]
async fn maps_network_failures_without_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
    });
    let error = runner(&format!("http://{address}/v1"), "never-print-this")
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();
    assert!(
        matches!(error, RunnerError::Network { .. }),
        "unexpected error: {error:?}"
    );
    assert!(!error.to_string().contains("never-print-this"));
}

#[tokio::test]
async fn rejects_output_that_does_not_match_schema() {
    let response = json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "content": [{"type": "output_text", "text": "{\"answer\":12}"}]
        }]
    });
    let (base_url, _request_rx) = server(200, &response.to_string(), &[]).await;
    let error = runner(&base_url, "key")
        .generate_structured(&request(), &schema())
        .await
        .unwrap_err();
    assert!(matches!(error, RunnerError::SchemaValidation(_)));
}

#[test]
fn rejects_missing_environment_credential() {
    let variable = "FLECT_TEST_CREDENTIAL_2F30E7B1_67E8_49C0_A28B_7FD2645D013A";
    let error = OpenAiResponsesRunner::from_env(OpenAiResponsesConfig {
        base_url: "https://api.openai.com/v1".to_owned(),
        api_key_env: variable.to_owned(),
        model: "test-model".to_owned(),
        reasoning_effort: "medium".to_owned(),
        timeout: Duration::from_secs(1),
    })
    .unwrap_err();
    assert!(matches!(error, RunnerError::MissingCredential { .. }));
}

fn runner(base_url: &str, key: &str) -> OpenAiResponsesRunner {
    OpenAiResponsesRunner::new(
        OpenAiResponsesConfig {
            base_url: base_url.to_owned(),
            api_key_env: "TEST_API_KEY".to_owned(),
            model: "test-model".to_owned(),
            reasoning_effort: "medium".to_owned(),
            timeout: Duration::from_secs(2),
        },
        key.to_owned(),
    )
    .unwrap()
}

fn request() -> AgentRequest {
    AgentRequest {
        purpose: RequestPurpose::ReconstructPatchIntent,
        input: json!({"patch": "sanitized"}),
    }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
        "required": ["answer"],
        "additionalProperties": false
    })
}

fn request_body(raw_request: &str) -> Value {
    let (_, body) = raw_request.split_once("\r\n\r\n").unwrap();
    serde_json::from_str(body).unwrap()
}

async fn server(
    status: u16,
    body: &str,
    headers: &[(&str, &str)],
) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_owned();
    let headers = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<Vec<_>>();
    let (request_tx, request_rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut stream).await;
        let reason = match status {
            200 => "OK",
            400 => "Bad Request",
            401 => "Unauthorized",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "Error",
        };
        let extra_headers = headers
            .iter()
            .fold(String::new(), |mut output, (name, value)| {
                write!(output, "{name}: {value}\r\n").unwrap();
                output
            });
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        request_tx.send(request).ok();
    });
    (format!("http://{address}/v1"), request_rx)
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
