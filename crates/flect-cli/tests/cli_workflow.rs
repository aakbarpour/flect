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
    let directory = tempfile::tempdir().unwrap();
    let echo = directory.path().join("echo.json");
    fs::write(
        &echo,
        format!(
            r#"{{"job_id":"{blind_job_id}","echoed_spec":{{"apparent_objective":"Change app behavior","behavior_before":[],"behavior_after":["New behavior"],"affected_scope":[{{"file":"app.txt","symbol":null}}],"side_effects":[],"assumptions":[],"uncertainties":[],"confidence":0.9}},"model":"gpt-5.6-terra","model_selection":"explicit"}}"#
        ),
    )
    .unwrap();
    assert_success(
        Command::new(env!("CARGO_BIN_EXE_flect"))
            .current_dir(repository.path())
            .args(["--json", "agent", "submit-echo", "--submission"])
            .arg(&echo)
            .output()
            .unwrap(),
    );
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
    let submission = Path::new(judge["submission_file"].as_str().unwrap());

    let verdict = format!(
        r#"{{"job_id":"{judge_job_id}","verdict":{{"alignment":"SAME","findings":[],"confidence":0.9}},"model":"gpt-5.6-terra","model_selection":"explicit"}}"#
    );
    fs::write(submission, format!("```json\n{verdict}\n```\n")).unwrap();
    let fenced = submit_verdict(repository.path(), submission);
    assert!(!fenced.status.success());
    assert_eq!(
        fs::read_to_string(submission).unwrap(),
        format!("```json\n{verdict}\n```\n")
    );

    let extra = format!(
        r#"{{"job_id":"{judge_job_id}","verdict":{{"alignment":"SAME","findings":[],"confidence":0.9}},"model":"gpt-5.6-terra","model_selection":"explicit","extra":true}}"#
    );
    fs::write(submission, extra).unwrap();
    let extra_key = submit_verdict(repository.path(), submission);
    assert!(!extra_key.status.success());

    let mismatched = r#"{"job_id":"judge_not_the_prepared_job","verdict":{"alignment":"SAME","findings":[],"confidence":0.9},"model":"gpt-5.6-terra","model_selection":"explicit"}"#;
    fs::write(submission, mismatched).unwrap();
    let mismatched = submit_verdict(repository.path(), submission);
    assert!(!mismatched.status.success());

    let fabricated = format!(
        r#"{{"job_id":"{judge_job_id}","verdict":{{"alignment":"PARTIAL","findings":[{{"kind":"missing_requirement","text":"Missing","evidence_ref":"hunk/999"}}],"confidence":0.9}},"model":"gpt-5.6-terra","model_selection":"explicit"}}"#
    );
    fs::write(submission, fabricated).unwrap();
    let fabricated = submit_verdict(repository.path(), submission);
    assert!(!fabricated.status.success());

    fs::write(submission, &verdict).unwrap();
    let substituted = directory.path().join("substituted").join(
        submission
            .file_name()
            .expect("Flect-generated submission file has a name"),
    );
    fs::create_dir_all(substituted.parent().unwrap()).unwrap();
    fs::write(&substituted, &verdict).unwrap();
    let substituted = submit_verdict(repository.path(), &substituted);
    assert!(!substituted.status.success());
    let accepted = submit_verdict(repository.path(), submission);
    assert_success(&accepted);
    let reused = submit_verdict(repository.path(), submission);
    assert!(!reused.status.success());
}

fn flect<const N: usize>(directory: &Path, arguments: [&str; N]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

fn submit_verdict(directory: &Path, submission: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(directory)
        .args(["--json", "agent", "submit-verdict", "--submission-file"])
        .arg(submission)
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
