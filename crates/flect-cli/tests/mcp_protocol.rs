use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

const SENTINEL: &str = "SECRET_ORIGINAL_TASK_MCP_SENTINEL";

#[test]
fn stdio_session_discovers_tools_and_persists_strict_blind_results() {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "-b", "main"]);
    git(
        repository.path(),
        ["config", "user.email", "tests@flect.local"],
    );
    git(repository.path(), ["config", "user.name", "Flect Tests"]);
    fs::write(repository.path().join("app.txt"), "old behavior\n").unwrap();
    git(repository.path(), ["add", "app.txt"]);
    git(repository.path(), ["commit", "-m", "base"]);
    assert!(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    git(repository.path(), ["add", ".gitignore", "flect.toml"]);
    git(repository.path(), ["commit", "-m", "configure flect"]);

    let mut client = McpClient::spawn(repository.path());
    let before_initialize =
        client.request(&json!({"jsonrpc": "2.0", "id": 0, "method": "tools/list"}));
    assert_eq!(before_initialize["error"]["code"], -32002);
    let initialized = client.request(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "flect-tests", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "flect");
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    client.notify(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let duplicate_initialize = client.request(&json!({
        "jsonrpc": "2.0", "id": 99, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25"}
    }));
    assert_eq!(duplicate_initialize["error"]["code"], -32600);

    let discovery =
        client.request(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}));
    let names = discovery["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
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

    let invalid = client.request(&json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "flect_start", "arguments": {"task": 42}}
    }));
    assert_eq!(invalid["error"]["code"], -32602);

    let started = client.call(4, "flect_start", &start_arguments());
    assert_eq!(started["result"]["isError"], false);
    fs::write(repository.path().join("app.txt"), "new behavior\n").unwrap();

    let inspected = client.call(5, "flect_inspect", &json!({}));
    assert_eq!(inspected["result"]["isError"], false);
    assert!(
        !serde_json::to_string(&inspected["result"])
            .unwrap()
            .contains(SENTINEL)
    );

    let verified = client.call(6, "flect_verify", &json!({}));
    assert_eq!(verified["result"]["isError"], false);
    assert!(
        !serde_json::to_string(&verified["result"])
            .unwrap()
            .contains(SENTINEL)
    );
    assert_eq!(
        verified["result"]["structuredContent"]["run_id"],
        started["result"]["structuredContent"]["id"]
    );

    let judged = exercise_agent_handoff(&mut client);

    let stored = client.call(12, "flect_get_result", &json!({}));
    assert_eq!(stored["result"]["isError"], false);
    assert_eq!(
        stored["result"]["structuredContent"],
        judged["result"]["structuredContent"]
    );
    assert!(
        !serde_json::to_string(&stored["result"])
            .unwrap()
            .contains(SENTINEL)
    );
    client.finish();
}

#[test]
fn configured_repository_context_overrides_mcp_process_directory() {
    let repository = initialized_fixture();
    let process_directory = tempfile::tempdir().unwrap();
    let direct_root = git_output(repository.path(), ["rev-parse", "--show-toplevel"]);

    let mut client =
        McpClient::spawn_with_repository_context(process_directory.path(), repository.path());
    initialize(&mut client);

    let started = client.call(
        1,
        "flect_start",
        &json!({"task": "discover the configured repository"}),
    );
    assert_eq!(started["result"]["isError"], false);
    let mcp_root = started["result"]["structuredContent"]["repository_root"]
        .as_str()
        .unwrap();
    assert_eq!(Path::new(mcp_root).canonicalize().unwrap(), direct_root);
    assert!(repository.path().join(".flect").is_dir());
    assert!(!process_directory.path().join(".flect").exists());
    client.finish();
}

fn initialized_fixture() -> tempfile::TempDir {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "-b", "main"]);
    git(
        repository.path(),
        ["config", "user.email", "tests@flect.local"],
    );
    git(repository.path(), ["config", "user.name", "Flect Tests"]);
    fs::write(repository.path().join("app.txt"), "old behavior\n").unwrap();
    git(repository.path(), ["add", "app.txt"]);
    git(repository.path(), ["commit", "-m", "base"]);
    assert!(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    git(repository.path(), ["add", ".gitignore", "flect.toml"]);
    git(repository.path(), ["commit", "-m", "configure flect"]);
    repository
}

fn initialize(client: &mut McpClient) {
    let initialized = client.request(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25"}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "flect");
    client.notify(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
}

fn start_arguments() -> Value {
    json!({"task": SENTINEL, "_meta": {"progressToken": "codex"}})
}

fn exercise_agent_handoff(client: &mut McpClient) -> Value {
    let semantic_error = client.call(
        7,
        "flect_prepare_reconciliation",
        &json!({
            "blind_job_id": "blind_missing"
        }),
    );
    assert_eq!(semantic_error["result"]["isError"], true);

    let blind = client.call(8, "flect_prepare_blind", &json!({"context": "patch"}));
    assert_eq!(blind["result"]["isError"], false);
    let blind_job = &blind["result"]["structuredContent"];
    assert_eq!(blind_job["isolation"], "structural");
    assert!(!serde_json::to_string(blind_job).unwrap().contains(SENTINEL));
    let blind_job_id = blind_job["job_id"].as_str().unwrap();

    let echoed = client.call(
        9,
        "flect_submit_echo",
        &json!({
            "job_id": blind_job_id,
            "echoed_spec": {
                "apparent_objective": "Change app behavior",
                "behavior_before": ["The app used old behavior"],
                "behavior_after": ["The app uses new behavior"],
                "affected_scope": [{"file": "app.txt", "symbol": null}],
                "side_effects": [], "assumptions": [], "uncertainties": [], "confidence": 0.9
            },
            "model": "test-verifier", "model_selection": "explicit"
        }),
    );
    assert_eq!(echoed["result"]["isError"], false);

    let judge = client.call(
        10,
        "flect_prepare_reconciliation",
        &json!({"blind_job_id": blind_job_id}),
    );
    assert_eq!(judge["result"]["isError"], false);
    let judge_job_id = judge["result"]["structuredContent"]["job_id"]
        .as_str()
        .unwrap();
    assert_ne!(judge_job_id, blind_job_id);

    let judged = client.call(
        11,
        "flect_submit_verdict",
        &json!({
            "job_id": judge_job_id,
            "verdict": {
                "alignment": "UNCERTAIN", "agreements": [], "missing_requirements": [],
                "unrequested_changes": [], "violated_constraints": [],
                "potential_side_effects": [], "uncertainties": ["Offline protocol fixture"],
                "evidence": [], "confidence": 0.4, "recommended_action": "REQUEST_MORE_CONTEXT"
            },
            "model": "test-judge", "model_selection": "explicit"
        }),
    );
    assert_eq!(judged["result"]["isError"], false);
    assert_eq!(
        judged["result"]["structuredContent"]["model_calls"][0]["provider"],
        "codex-native"
    );
    judged
}

struct McpClient {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl McpClient {
    fn spawn(directory: &Path) -> Self {
        Self::spawn_with_command(Command::new(env!("CARGO_BIN_EXE_flect")), directory)
    }

    fn spawn_with_repository_context(process_directory: &Path, repository: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_flect"));
        command.env("FLECT_MCP_REPOSITORY_ROOT", repository);
        Self::spawn_with_command(command, process_directory)
    }

    fn spawn_with_command(mut command: Command, directory: &Path) -> Self {
        let mut child = command
            .current_dir(directory)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input: Some(input),
            output,
        }
    }

    fn request(&mut self, request: &Value) -> Value {
        self.write(request);
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "MCP server closed stdout");
        serde_json::from_str(&line).unwrap()
    }

    fn notify(&mut self, notification: &Value) {
        self.write(notification);
    }

    fn call(&mut self, id: u64, name: &str, arguments: &Value) -> Value {
        self.request(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}}))
    }

    fn write(&mut self, message: &Value) {
        let input = self.input.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &message).unwrap();
        writeln!(input).unwrap();
        input.flush().unwrap();
    }

    fn finish(mut self) {
        drop(self.input.take());
        assert!(self.child.wait().unwrap().success());
    }
}

fn git_output<const N: usize>(directory: &Path, arguments: [&str; N]) -> std::path::PathBuf {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success());
    Path::new(std::str::from_utf8(&output.stdout).unwrap().trim())
        .canonicalize()
        .unwrap()
}

fn git<const N: usize>(directory: &Path, arguments: [&str; N]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
