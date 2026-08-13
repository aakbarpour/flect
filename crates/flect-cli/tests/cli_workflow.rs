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
            "kind = \"mock\"\nprotocol = \"responses\"\nbase_url = \"https://api.openai.com/v1\"\napi_key_env = \"OPENAI_API_KEY\"\nmodel = \"gpt-5.6-luna\"",
            "kind = \"api\"\nprotocol = \"responses\"\nbase_url = \"https://api.openai.com/v1\"\napi_key_env = \"OPENAI_API_KEY\"\nmodel = \"custom-model\"",
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

#[test]
#[allow(clippy::too_many_lines)]
fn direct_judge_submission_is_strict_and_does_not_use_chat_text() {
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), ["init", "-b", "main"]);
    git(
        repository.path(),
        ["config", "user.email", "tests@flect.local"],
    );
    git(repository.path(), ["config", "user.name", "Flect Tests"]);
    fs::write(repository.path().join("app.txt"), "old\n").unwrap();
    git(repository.path(), ["add", "app.txt"]);
    git(repository.path(), ["commit", "-m", "base"]);
    assert_success(flect(repository.path(), ["init"]));
    git(repository.path(), ["add", ".gitignore", "flect.toml"]);
    git(repository.path(), ["commit", "-m", "configure flect"]);
    assert_success(flect(
        repository.path(),
        ["start", "--task", "Change app behavior"],
    ));
    fs::write(repository.path().join("app.txt"), "new\n").unwrap();

    let blind = flect(repository.path(), ["--json", "agent", "prepare-blind"]);
    assert_success(&blind);
    let blind: serde_json::Value = serde_json::from_slice(&blind.stdout).unwrap();
    let blind_job_id = blind["job_id"].as_str().unwrap();
    let text = tempfile::tempdir().unwrap();
    let objective = text.path().join("objective.txt");
    let after = text.path().join("after.txt");
    let side_effect = text.path().join("side-effect.txt");
    let side_effect_reason = text.path().join("side-effect-reason.txt");
    fs::write(&objective, "Change app behavior").unwrap();
    fs::write(&after, "New behavior").unwrap();
    fs::write(&side_effect, "Callers observe new behavior").unwrap();
    fs::write(&side_effect_reason, "This restates the changed behavior.").unwrap();
    assert_success(flect(
        repository.path(),
        [
            "--json",
            "agent",
            "verifier-begin",
            "--job",
            blind_job_id,
            "--model",
            "gpt-5.6-terra",
            "--model-selection",
            "explicit",
        ],
    ));
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .args([
                "--json",
                "agent",
                "verifier-add-side-effect",
                "--job",
                blind_job_id,
                "--text-file",
            ])
            .arg(&side_effect)
            .output()
            .unwrap(),
    );
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .args([
                "--json",
                "agent",
                "verifier-set-objective",
                "--job",
                blind_job_id,
                "--text-file",
            ])
            .arg(&objective)
            .output()
            .unwrap(),
    );
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .args([
                "--json",
                "agent",
                "verifier-add-after",
                "--job",
                blind_job_id,
                "--text-file",
            ])
            .arg(&after)
            .output()
            .unwrap(),
    );
    assert_success(flect(
        repository.path(),
        [
            "--json",
            "agent",
            "verifier-add-scope",
            "--job",
            blind_job_id,
            "--file",
            "app.txt",
        ],
    ));
    assert_success(flect(
        repository.path(),
        [
            "--json",
            "agent",
            "verifier-set-confidence",
            "--job",
            blind_job_id,
            "0.9",
        ],
    ));
    assert_success(flect(
        repository.path(),
        ["--json", "agent", "verifier-submit", "--job", blind_job_id],
    ));
    assert_success(flect(
        repository.path(),
        ["--json", "agent", "verifier-commit", "--job", blind_job_id],
    ));
    let judge = Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(repository.path())
        .args([
            "--json",
            "agent",
            "prepare-reconciliation",
            "--blind-job",
            blind_job_id,
        ])
        .output()
        .unwrap();
    assert_success(&judge);
    let judge: serde_json::Value = serde_json::from_slice(&judge.stdout).unwrap();
    let judge_job_id = judge["job_id"].as_str().unwrap();
    assert!(judge.get("submission_file").is_none());
    assert_success(flect(
        repository.path(),
        [
            "--json",
            "agent",
            "judge-begin",
            "--job",
            judge_job_id,
            "--model",
            "gpt-5.6-terra",
            "--model-selection",
            "explicit",
        ],
    ));
    let invalid_kind = flect(
        repository.path(),
        [
            "--json",
            "agent",
            "judge-add-finding",
            "--job",
            judge_job_id,
            "--kind",
            "structural",
            "--text-file",
            "missing.txt",
        ],
    );
    assert!(!invalid_kind.status.success());
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .args([
                "--json",
                "agent",
                "judge-mark-side-effect-not-distinct",
                "--job",
                judge_job_id,
                "--candidate",
                "side_effect/0",
                "--reason-file",
            ])
            .arg(&side_effect_reason)
            .output()
            .unwrap(),
    );
    assert_success(flect(
        repository.path(),
        [
            "--json",
            "agent",
            "judge-set-alignment",
            "--job",
            judge_job_id,
            "same",
        ],
    ));
    assert_success(flect(
        repository.path(),
        [
            "--json",
            "agent",
            "judge-set-confidence",
            "--job",
            judge_job_id,
            "0.9",
        ],
    ));
    assert_success(flect(
        repository.path(),
        ["--json", "agent", "judge-submit", "--job", judge_job_id],
    ));
    let reused = flect(
        repository.path(),
        ["--json", "agent", "judge-submit", "--job", judge_job_id],
    );
    assert!(!reused.status.success());
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
