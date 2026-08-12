use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn offline_workflow_persists_a_blind_verdict() {
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

    assert_success(flect(repository.path(), ["init"]));
    git(repository.path(), ["add", ".gitignore", "flect.toml"]);
    git(repository.path(), ["commit", "-m", "configure flect"]);
    assert_success(flect(
        repository.path(),
        ["start", "--task", "Add new behavior"],
    ));
    fs::write(repository.path().join("app.txt"), "new behavior\n").unwrap();

    let dry_run = flect(repository.path(), ["--json", "verify", "--dry-run"]);
    assert_success(&dry_run);
    let dry_run: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_run["request_sent"], false);
    assert_eq!(dry_run["runner"]["provider"], "mock");
    assert_eq!(dry_run["context_policy"], "focused");
    assert_eq!(dry_run["included"]["patch_files"][0], "app.txt");

    let inspection = flect(repository.path(), ["--json", "inspect"]);
    assert_success(&inspection);
    let inspection: serde_json::Value = serde_json::from_slice(&inspection.stdout).unwrap();
    assert_eq!(inspection["manifest"]["context_policy"], "focused");
    assert_eq!(inspection["patch"]["files"][0]["path"], "app.txt");

    let echoed_directory = tempfile::tempdir().unwrap();
    let echoed_path = echoed_directory.path().join("echoed.json");
    fs::write(
        &echoed_path,
        r#"{
            "apparent_objective": "Add new behavior",
            "behavior_before": ["Old behavior was present"],
            "behavior_after": ["New behavior is present"],
            "affected_scope": [{"file": "app.txt", "symbol": null}],
            "side_effects": [],
            "assumptions": [],
            "uncertainties": [],
            "confidence": 0.9
        }"#,
    )
    .unwrap();
    let verification = Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(repository.path())
        .args(["--json", "verify", "--echoed-spec"])
        .arg(&echoed_path)
        .output()
        .unwrap();
    assert_success(&verification);
    let verification: serde_json::Value = serde_json::from_slice(&verification.stdout).unwrap();
    assert_eq!(verification["verdict"]["alignment"], "SAME");
    let bundle = serde_json::to_string(&verification["bundle"]).unwrap();
    assert!(!bundle.contains("Add new behavior"));
}

#[test]
fn doctor_reports_api_credential_readiness_without_exposing_values() {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "-b", "main"]);
    assert_success(flect(repository.path(), ["init"]));

    let config_path = repository.path().join("flect.toml");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace(
            "kind = \"mock\"",
            "kind = \"api\"\nmodel = \"custom-model\"",
        )
        .replace("OPENAI_API_KEY", "FLECT_DOCTOR_TEST_KEY");
    fs::write(config_path, config).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(repository.path())
        .env_remove("FLECT_DOCTOR_TEST_KEY")
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["runner"]["kind"], "api");
    assert_eq!(report["runner"]["model"], "custom-model");
    assert_eq!(
        report["runner"]["credential"]["environment"],
        "FLECT_DOCTOR_TEST_KEY"
    );
    assert_eq!(report["runner"]["credential"]["available"], false);
    assert_eq!(report["ready"], false);
    assert_eq!(report["mcp"]["available"], true);
    assert_eq!(report["mcp"]["protocol"], "2025-11-25");
    assert_eq!(report["verification_modes"]["api"]["configured"], true);
    assert_eq!(report["verification_modes"]["api"]["ready"], false);
    assert_eq!(
        report["verification_modes"]["codex_agent"]["readiness"],
        "unknown"
    );
    assert_eq!(
        report["verification_modes"]["codex_agent"]["workspace_isolation"],
        "structural"
    );
    assert!(report["codex"]["available"].is_boolean());
}

#[test]
fn config_commands_set_and_show_validated_runner_values() {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "-b", "main"]);
    assert_success(flect(repository.path(), ["init"]));

    assert_success(flect(
        repository.path(),
        ["config", "set", "runner.model", "custom-model"],
    ));
    assert_success(flect(
        repository.path(),
        ["config", "set", "runner.kind", "api"],
    ));
    let output = flect(repository.path(), ["--json", "config", "show"]);
    assert_success(&output);
    let config: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(config["runner"]["kind"], "api");
    assert_eq!(config["runner"]["model"], "custom-model");
    assert!(
        !fs::read_to_string(repository.path().join("flect.toml"))
            .unwrap()
            .contains("API key")
    );

    let invalid = flect(
        repository.path(),
        ["config", "set", "runner.confidence_threshold", "2"],
    );
    assert!(!invalid.status.success());
}

fn flect<const N: usize>(directory: &Path, arguments: [&str; N]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

fn git<const N: usize>(directory: &Path, arguments: [&str; N]) {
    assert_success(
        Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .unwrap(),
    );
}

fn assert_success(output: impl std::borrow::Borrow<Output>) {
    let output = output.borrow();
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
