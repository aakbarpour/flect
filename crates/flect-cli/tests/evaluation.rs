use std::path::Path;
use std::process::Command;

#[test]
fn offline_suite_is_reproducible_and_never_requires_api_access() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(&root)
        .env_remove("OPENAI_API_KEY")
        .args(["--json", "eval"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["mode"], "offline");
    assert_eq!(report["profiles"][0]["metrics"]["cases"], 10);
    assert_eq!(report["profiles"][0]["metrics"]["exact_verdicts"], 10);
    assert_eq!(report["profiles"][0]["metrics"]["requests"], 30);
    assert_eq!(
        report["profiles"][0]["metrics"]["estimated_cost_usd"],
        serde_json::Value::Null
    );

    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("Reject expired tokens in the auth validator"));
}

#[test]
fn api_profiles_require_explicit_paid_opt_in() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO_BIN_EXE_flect"))
        .current_dir(&root)
        .args([
            "eval",
            "--profiles",
            "fixtures/evaluation/profiles.example.toml",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--allow-paid-api"));
}
