//! Minimal stdio Model Context Protocol adapter for the existing CLI application.

use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

use flect_core::{GitRepository, RunStore};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde_json::{Map, Value, json};

const PROTOCOL_VERSION: &str = "2025-11-25";
const INSTRUCTIONS: &str = "Use flect_start before implementation to preserve the original task and base revision. After edits, use flect_inspect to review the strict blind bundle, then flect_verify. The backward verifier receives the patch bundle but never the original task. Use flect_get_result to retrieve the persisted structured verdict.";

pub fn run() -> Result<()> {
    let executable = std::env::current_exe()
        .into_diagnostic()
        .wrap_err("could not locate the Flect executable")?;
    let working_directory = std::env::current_dir()
        .into_diagnostic()
        .wrap_err("could not determine the MCP working directory")?;
    serve(
        io::stdin().lock(),
        io::stdout().lock(),
        &executable,
        &working_directory,
    )
}

fn serve(
    input: impl BufRead,
    mut output: impl Write,
    executable: &Path,
    working_directory: &Path,
) -> Result<()> {
    for line in input.lines() {
        let line = line
            .into_diagnostic()
            .wrap_err("could not read MCP input")?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle_request(&request, executable, working_directory),
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

fn handle_request(request: &Value, executable: &Path, working_directory: &Path) -> Option<Value> {
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
    let id = id?;
    let result = match method {
        "initialize" => initialize(object.get("params")),
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
        "flect_inspect" => {
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

fn context(object: &Map<String, Value>) -> std::result::Result<Option<String>, String> {
    let value = optional_string(object, "context")?;
    if value
        .as_deref()
        .is_some_and(|value| !matches!(value, "patch" | "focused" | "repository"))
    {
        return Err("`context` must be patch, focused, or repository".to_owned());
    }
    Ok(value)
}

fn reject_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> std::result::Result<(), String> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    if let Some(key) = object.keys().find(|key| !allowed.contains(key.as_str())) {
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
    json!({"type": "string", "enum": ["patch", "focused", "repository"]})
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
                "flect_get_result"
            ]
        );
    }

    #[test]
    fn invalid_protocol_input_returns_standard_errors() {
        let request = json!({"jsonrpc": "2.0", "id": 7, "method": "unknown"});
        let response = handle_request(&request, Path::new("flect"), Path::new(".")).unwrap();
        assert_eq!(response["error"]["code"], -32601);

        let request = json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": {"name": "flect_start", "arguments": {"task": 42}}
        });
        let response = handle_request(&request, Path::new("flect"), Path::new(".")).unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }
}
