//! Minimal stdio Model Context Protocol adapter for the existing CLI application.

use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use flect_app::AgentService;
use flect_core::{
    BlindAgentSubmission, ContextPolicy, EchoedSpec, GitRepository, JudgeVerdict,
    ReconciliationAgentSubmission, RunStore,
};
use miette::{IntoDiagnostic, Result, WrapErr};
use schemars::schema_for;
use serde_json::{Map, Value, json};

const PROTOCOL_VERSION: &str = "2025-11-25";
const INSTRUCTIONS: &str = "Use flect_start before implementation. The configured API route defaults to gpt-5.6-luna with one bounded gpt-5.6-terra fallback; both remain configurable. For Codex-native verification, call flect_prepare_blind, hand only its allowed resources to a fresh no-parent-context verifier, submit its EchoedSpec with flect_submit_echo, and prepare a separate judge with flect_prepare_reconciliation. The judge must submit directly through flect_submit_verdict when this tool is exposed to it; otherwise it must write the exact generated ReconciliationAgentSubmission file and invoke flect agent submit-verdict itself. Never parse or re-submit a judge chat response. Alternatively, flect_verify retains the configured automated API workflow. Use flect_get_result to retrieve the persisted verdict.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Lifecycle {
    #[default]
    Uninitialized,
    Initializing,
    Ready,
}

pub fn run() -> Result<()> {
    let executable = std::env::current_exe()
        .into_diagnostic()
        .wrap_err("could not locate the Flect executable")?;
    let working_directory = repository_context()?;
    serve(
        io::stdin().lock(),
        io::stdout().lock(),
        &executable,
        &working_directory,
    )
}

fn repository_context() -> Result<PathBuf> {
    let configured = std::env::var_os("FLECT_MCP_REPOSITORY_ROOT").map_or(
        std::env::current_dir()
            .into_diagnostic()
            .wrap_err("could not determine the MCP working directory")?,
        PathBuf::from,
    );
    let canonical = configured
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "MCP repository context does not exist: {}",
                configured.display()
            )
        })?;
    let repository = GitRepository::discover(&canonical)
        .into_diagnostic()
        .wrap_err("MCP repository context must be a Git worktree")?;
    repository
        .root()
        .canonicalize()
        .into_diagnostic()
        .wrap_err("could not canonicalize MCP repository root")
}

fn serve(
    input: impl BufRead,
    mut output: impl Write,
    executable: &Path,
    working_directory: &Path,
) -> Result<()> {
    let mut lifecycle = Lifecycle::default();
    for line in input.lines() {
        let line = line
            .into_diagnostic()
            .wrap_err("could not read MCP input")?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle_request(&request, executable, working_directory, &mut lifecycle),
            Err(_) => Some(error_response(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response).into_diagnostic()?;
            writeln!(output).into_diagnostic()?;
            output.flush().into_diagnostic()?;
        }
    }
    Ok(())
}

fn handle_request(
    request: &Value,
    executable: &Path,
    working_directory: &Path,
    lifecycle: &mut Lifecycle,
) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(error_response(Value::Null, -32600, "Invalid Request"));
    };
    let id = object.get("id").cloned();
    if object.get("jsonrpc") != Some(&Value::String("2.0".to_owned())) {
        return Some(error_response(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(error_response(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    };
    let Some(id) = id else {
        if method == "notifications/initialized" && *lifecycle == Lifecycle::Initializing {
            *lifecycle = Lifecycle::Ready;
        }
        return None;
    };
    if method == "initialize" && *lifecycle != Lifecycle::Uninitialized {
        return Some(error_response(id, -32600, "Server already initialized"));
    }
    if matches!(method, "tools/list" | "tools/call") && *lifecycle != Lifecycle::Ready {
        return Some(error_response(id, -32002, "Server not initialized"));
    }
    let result = match method {
        "initialize" => initialize(object.get("params")).inspect(|_| {
            *lifecycle = Lifecycle::Initializing;
        }),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call_tool(object.get("params"), executable, working_directory),
        _ => return Some(error_response(id, -32601, "Method not found")),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(message) => error_response(id, -32602, &message),
    })
}

fn initialize(params: Option<&Value>) -> std::result::Result<Value, String> {
    if let Some(params) = params {
        let object = params
            .as_object()
            .ok_or_else(|| "initialize params must be an object".to_owned())?;
        if object
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_none()
        {
            return Err("initialize requires protocolVersion".to_owned());
        }
    }
    Ok(json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "flect", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    }))
}

fn call_tool(
    params: Option<&Value>,
    executable: &Path,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    let params = params
        .and_then(Value::as_object)
        .ok_or_else(|| "tools/call params must be an object".to_owned())?;
    reject_unknown(params, &["name", "arguments"])?;
    let name = required_string(params, "name")?;
    let empty = Map::new();
    let arguments = params
        .get("arguments")
        .map(|value| {
            value
                .as_object()
                .ok_or_else(|| "arguments must be an object".to_owned())
        })
        .transpose()?
        .unwrap_or(&empty);
    validate_arguments(&name, arguments)?;

    let result = match name.as_str() {
        "flect_start" => run_start(arguments, executable, working_directory),
        "flect_inspect" => run_inspect(arguments, executable, working_directory),
        "flect_echo" => run_echo(arguments, executable, working_directory),
        "flect_verify" => run_verify(arguments, executable, working_directory),
        "flect_prepare_blind" => prepare_blind(arguments, working_directory),
        "flect_submit_echo" => submit_echo(arguments, working_directory),
        "flect_prepare_reconciliation" => prepare_reconciliation(arguments, working_directory),
        "flect_submit_verdict" => submit_verdict(arguments, working_directory),
        "flect_get_result" => get_result(arguments, working_directory),
        _ => return Err(format!("unknown Flect tool `{name}`")),
    };
    Ok(match result {
        Ok(value) => tool_result(value, false),
        Err(message) => tool_result(json!({"error": message}), true),
    })
}

fn validate_arguments(
    name: &str,
    arguments: &Map<String, Value>,
) -> std::result::Result<(), String> {
    match name {
        "flect_start" => {
            reject_unknown(arguments, &["task", "spec_file"])?;
            if required_string(arguments, "task")?.trim().is_empty() {
                return Err("task cannot be empty".to_owned());
            }
            optional_string(arguments, "spec_file")?;
        }
        "flect_inspect" | "flect_prepare_blind" => {
            reject_unknown(arguments, &["run", "context"])?;
            optional_string(arguments, "run")?;
            context(arguments)?;
        }
        "flect_echo" => {
            reject_unknown(arguments, &["revision", "echoed_spec", "context"])?;
            optional_string(arguments, "revision")?;
            optional_string(arguments, "echoed_spec")?;
            context(arguments)?;
        }
        "flect_verify" => {
            reject_unknown(arguments, &["run", "echoed_spec", "context", "dry_run"])?;
            optional_string(arguments, "run")?;
            optional_string(arguments, "echoed_spec")?;
            context(arguments)?;
            optional_bool(arguments, "dry_run")?;
        }
        "flect_submit_echo" => {
            reject_unknown(
                arguments,
                &["job_id", "echoed_spec", "model", "model_selection"],
            )?;
            required_string(arguments, "job_id")?;
            required_object(arguments, "echoed_spec")?;
            optional_string(arguments, "model")?;
            model_selection(arguments)?;
        }
        "flect_prepare_reconciliation" => {
            reject_unknown(arguments, &["blind_job_id"])?;
            required_string(arguments, "blind_job_id")?;
        }
        "flect_submit_verdict" => {
            reject_unknown(
                arguments,
                &["job_id", "verdict", "model", "model_selection"],
            )?;
            required_string(arguments, "job_id")?;
            required_object(arguments, "verdict")?;
            optional_string(arguments, "model")?;
            model_selection(arguments)?;
        }
        "flect_get_result" => {
            reject_unknown(arguments, &["run"])?;
            optional_string(arguments, "run")?;
        }
        _ => return Err(format!("unknown Flect tool `{name}`")),
    }
    Ok(())
}

fn run_start(
    arguments: &Map<String, Value>,
    executable: &Path,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    reject_unknown(arguments, &["task", "spec_file"])?;
    let task = required_string(arguments, "task")?;
    if task.trim().is_empty() {
        return Err("task cannot be empty".to_owned());
    }
    let mut args = vec![
        "--json".to_owned(),
        "start".to_owned(),
        "--task".to_owned(),
        task,
    ];
    push_option(
        &mut args,
        "--spec-file",
        optional_string(arguments, "spec_file")?,
    );
    run_cli(executable, working_directory, &args)
}

fn run_inspect(
    arguments: &Map<String, Value>,
    executable: &Path,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    reject_unknown(arguments, &["run", "context"])?;
    let mut args = vec!["--json".to_owned(), "inspect".to_owned()];
    push_option(&mut args, "--run", optional_string(arguments, "run")?);
    push_option(&mut args, "--context", context(arguments)?);
    run_cli(executable, working_directory, &args)
}

fn run_echo(
    arguments: &Map<String, Value>,
    executable: &Path,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    reject_unknown(arguments, &["revision", "echoed_spec", "context"])?;
    let mut args = vec!["--json".to_owned(), "echo".to_owned()];
    if let Some(revision) = optional_string(arguments, "revision")? {
        args.push(revision);
    }
    push_option(
        &mut args,
        "--echoed-spec",
        optional_string(arguments, "echoed_spec")?,
    );
    push_option(&mut args, "--context", context(arguments)?);
    run_cli(executable, working_directory, &args)
}

fn run_verify(
    arguments: &Map<String, Value>,
    executable: &Path,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    reject_unknown(arguments, &["run", "echoed_spec", "context", "dry_run"])?;
    let mut args = vec!["--json".to_owned(), "verify".to_owned()];
    push_option(&mut args, "--run", optional_string(arguments, "run")?);
    push_option(
        &mut args,
        "--echoed-spec",
        optional_string(arguments, "echoed_spec")?,
    );
    push_option(&mut args, "--context", context(arguments)?);
    if optional_bool(arguments, "dry_run")?.unwrap_or(false) {
        args.push("--dry-run".to_owned());
    }
    run_cli(executable, working_directory, &args)
}

fn get_result(
    arguments: &Map<String, Value>,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    reject_unknown(arguments, &["run"])?;
    let run = optional_string(arguments, "run")?;
    let repository =
        GitRepository::discover(working_directory).map_err(|error| error.to_string())?;
    let record = RunStore::new(repository.root())
        .load_verification(run.as_deref())
        .map_err(|error| error.to_string())?;
    serde_json::to_value(record).map_err(|error| error.to_string())
}

fn prepare_blind(
    arguments: &Map<String, Value>,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    let service = AgentService::discover(working_directory).map_err(|error| error.to_string())?;
    let run = optional_string(arguments, "run")?;
    let context = context(arguments)?
        .map(|value| value.parse::<ContextPolicy>().map_err(str::to_owned))
        .transpose()?;
    let job = service
        .prepare_blind(run.as_deref(), context)
        .map_err(|error| error.to_string())?;
    serde_json::to_value(job).map_err(|error| error.to_string())
}

fn submit_echo(
    arguments: &Map<String, Value>,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    let submission =
        serde_json::from_value::<BlindAgentSubmission>(Value::Object(arguments.clone()))
            .map_err(|error| format!("invalid blind submission: {error}"))?;
    let service = AgentService::discover(working_directory).map_err(|error| error.to_string())?;
    let echo = service
        .submit_echo(submission)
        .map_err(|error| error.to_string())?;
    serde_json::to_value(echo).map_err(|error| error.to_string())
}

fn prepare_reconciliation(
    arguments: &Map<String, Value>,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    let blind_job_id = required_string(arguments, "blind_job_id")?;
    let service = AgentService::discover(working_directory).map_err(|error| error.to_string())?;
    let job = service
        .prepare_reconciliation(&blind_job_id)
        .map_err(|error| error.to_string())?;
    serde_json::to_value(job).map_err(|error| error.to_string())
}

fn submit_verdict(
    arguments: &Map<String, Value>,
    working_directory: &Path,
) -> std::result::Result<Value, String> {
    let submission =
        serde_json::from_value::<ReconciliationAgentSubmission>(Value::Object(arguments.clone()))
            .map_err(|error| format!("invalid reconciliation submission: {error}"))?;
    let service = AgentService::discover(working_directory).map_err(|error| error.to_string())?;
    let record = service
        .submit_verdict(submission)
        .map_err(|error| error.to_string())?;
    serde_json::to_value(record).map_err(|error| error.to_string())
}

fn run_cli(
    executable: &Path,
    working_directory: &Path,
    arguments: &[String],
) -> std::result::Result<Value, String> {
    let output = Command::new(executable)
        .current_dir(working_directory)
        .args(arguments)
        .output()
        .map_err(|error| format!("could not launch Flect: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(truncate(message.trim(), 4096));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Flect returned invalid structured output: {error}"))
}

fn push_option(arguments: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        arguments.push(flag.to_owned());
        arguments.push(value);
    }
}

fn required_string(object: &Map<String, Value>, key: &str) -> std::result::Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("`{key}` must be a string"))
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<String>, String> {
    object
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{key}` must be a string"))
        })
        .transpose()
}

fn optional_bool(
    object: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<bool>, String> {
    object
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("`{key}` must be a boolean"))
        })
        .transpose()
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> std::result::Result<&'a Map<String, Value>, String> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("`{key}` must be an object"))
}

fn model_selection(object: &Map<String, Value>) -> std::result::Result<Option<String>, String> {
    let value = optional_string(object, "model_selection")?;
    if value
        .as_deref()
        .is_some_and(|value| !matches!(value, "explicit" | "inherited" | "unknown"))
    {
        return Err("`model_selection` must be explicit, inherited, or unknown".to_owned());
    }
    Ok(value)
}

fn context(object: &Map<String, Value>) -> std::result::Result<Option<String>, String> {
    let value = optional_string(object, "context")?;
    if value
        .as_deref()
        .is_some_and(|value| !matches!(value, "patch" | "focused" | "repo"))
    {
        return Err("`context` must be patch, focused, or repo".to_owned());
    }
    Ok(value)
}

fn reject_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> std::result::Result<(), String> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    // Codex may attach transport metadata to otherwise valid MCP calls. It is
    // deliberately not part of any Flect contract and must not affect input
    // validation or persisted state.
    if let Some(key) = object
        .keys()
        .find(|key| key.as_str() != "_meta" && !allowed.contains(key.as_str()))
    {
        return Err(format!("unknown parameter `{key}`"));
    }
    Ok(())
}

fn tool_result(value: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "null".to_owned());
    let response = json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": value,
        "isError": is_error,
    });
    drop(value);
    response
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
    let response = json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}});
    drop(id);
    response
}

fn truncate(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.to_owned();
    }
    value.chars().take(maximum).collect::<String>() + "…"
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "flect_start",
            "Capture the original task and immutable base revision before implementation.",
            json!({
                "type": "object",
                "properties": {
                    "task": {"type": "string", "minLength": 1},
                    "spec_file": {"type": "string", "description": "Optional IntendedSpec JSON path."}
                },
                "required": ["task"], "additionalProperties": false
            }),
            false,
        ),
        tool(
            "flect_inspect",
            "Return the exact strict blind bundle that a backward verifier would receive.",
            run_schema(false, false),
            true,
        ),
        tool(
            "flect_echo",
            "Reconstruct the apparent intent of a patch without the original task.",
            json!({
                "type": "object",
                "properties": {
                    "revision": {"type": "string"},
                    "echoed_spec": {"type": "string", "description": "Optional EchoedSpec JSON path."},
                    "context": context_schema()
                }, "additionalProperties": false
            }),
            true,
        ),
        tool(
            "flect_verify",
            "Blindly reconstruct patch intent, reconcile it with the captured spec, and persist a verdict.",
            run_schema(true, true),
            false,
        ),
        tool(
            "flect_prepare_blind",
            "Prepare sanitized read-only resources and a typed job for a fresh blind verifier.",
            run_schema(false, false),
            false,
        ),
        tool(
            "flect_submit_echo",
            "Validate and accept one EchoedSpec from the prepared blind verifier job.",
            agent_submission_schema("echoed_spec", json!(schema_for!(EchoedSpec))),
            false,
        ),
        tool(
            "flect_prepare_reconciliation",
            "Prepare a typed reconciliation job for a distinct fresh judge.",
            json!({
                "type": "object",
                "properties": {"blind_job_id": {"type": "string", "minLength": 1}},
                "required": ["blind_job_id"], "additionalProperties": false
            }),
            false,
        ),
        tool(
            "flect_submit_verdict",
            "Validate a judge Verdict against available evidence and persist the final result.",
            agent_submission_schema("verdict", json!(schema_for!(JudgeVerdict))),
            false,
        ),
        tool(
            "flect_get_result",
            "Retrieve a persisted structured verification result for a run.",
            json!({
                "type": "object", "properties": {"run": {"type": "string"}},
                "additionalProperties": false
            }),
            true,
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value, read_only: bool) -> Value {
    let definition = json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only,
            "openWorldHint": matches!(name, "flect_echo" | "flect_verify" | "flect_start")
        }
    });
    drop(input_schema);
    definition
}

fn run_schema(include_echoed: bool, include_dry_run: bool) -> Value {
    let mut properties = Map::from_iter([
        ("run".to_owned(), json!({"type": "string"})),
        ("context".to_owned(), context_schema()),
    ]);
    if include_echoed {
        properties.insert(
            "echoed_spec".to_owned(),
            json!({"type": "string", "description": "Optional EchoedSpec JSON path."}),
        );
    }
    if include_dry_run {
        properties.insert("dry_run".to_owned(), json!({"type": "boolean"}));
    }
    json!({"type": "object", "properties": properties, "additionalProperties": false})
}

fn context_schema() -> Value {
    json!({"type": "string", "enum": ["patch", "focused", "repo"]})
}

fn agent_submission_schema(payload: &str, payload_schema: Value) -> Value {
    let properties = Map::from_iter([
        (
            "job_id".to_owned(),
            json!({"type": "string", "minLength": 1}),
        ),
        (payload.to_owned(), payload_schema),
        ("model".to_owned(), json!({"type": "string"})),
        (
            "model_selection".to_owned(),
            json!({"type": "string", "enum": ["explicit", "inherited", "unknown"]}),
        ),
    ]);
    let required = vec![
        Value::String("job_id".to_owned()),
        Value::String(payload.to_owned()),
    ];
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_lists_exactly_the_public_tools() {
        let names = tools()
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "flect_start",
                "flect_inspect",
                "flect_echo",
                "flect_verify",
                "flect_prepare_blind",
                "flect_submit_echo",
                "flect_prepare_reconciliation",
                "flect_submit_verdict",
                "flect_get_result"
            ]
        );
    }

    #[test]
    fn invalid_protocol_input_returns_standard_errors() {
        let mut lifecycle = Lifecycle::Ready;
        let request = json!({"jsonrpc": "2.0", "id": 7, "method": "unknown"});
        let response =
            handle_request(&request, Path::new("flect"), Path::new("."), &mut lifecycle).unwrap();
        assert_eq!(response["error"]["code"], -32601);

        let request = json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": {"name": "flect_start", "arguments": {"task": 42}}
        });
        let response =
            handle_request(&request, Path::new("flect"), Path::new("."), &mut lifecycle).unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn lifecycle_requires_initialize_then_notification() {
        let mut lifecycle = Lifecycle::Uninitialized;
        let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let response =
            handle_request(&list, Path::new("flect"), Path::new("."), &mut lifecycle).unwrap();
        assert_eq!(response["error"]["code"], -32002);

        let initialize = json!({
            "jsonrpc": "2.0", "id": 2, "method": "initialize",
            "params": {"protocolVersion": PROTOCOL_VERSION}
        });
        let response = handle_request(
            &initialize,
            Path::new("flect"),
            Path::new("."),
            &mut lifecycle,
        )
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(lifecycle, Lifecycle::Initializing);

        let response =
            handle_request(&list, Path::new("flect"), Path::new("."), &mut lifecycle).unwrap();
        assert_eq!(response["error"]["code"], -32002);
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(
            handle_request(
                &initialized,
                Path::new("flect"),
                Path::new("."),
                &mut lifecycle
            )
            .is_none()
        );
        assert_eq!(lifecycle, Lifecycle::Ready);
        let response =
            handle_request(&list, Path::new("flect"), Path::new("."), &mut lifecycle).unwrap();
        assert!(response["result"]["tools"].is_array());

        let duplicate = handle_request(
            &initialize,
            Path::new("flect"),
            Path::new("."),
            &mut lifecycle,
        )
        .unwrap();
        assert_eq!(duplicate["error"]["code"], -32600);
    }

    #[test]
    fn agent_submission_schemas_include_typed_payloads() {
        let tools = tools();
        let echo = tools
            .iter()
            .find(|tool| tool["name"] == "flect_submit_echo")
            .unwrap();
        assert_eq!(
            echo["inputSchema"]["properties"]["echoed_spec"]["additionalProperties"],
            false
        );
        let verdict = tools
            .iter()
            .find(|tool| tool["name"] == "flect_submit_verdict")
            .unwrap();
        assert_eq!(
            verdict["inputSchema"]["properties"]["verdict"]["additionalProperties"],
            false
        );
    }
}
