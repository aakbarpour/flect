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
            "affected_scope": ["app.txt"],
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
