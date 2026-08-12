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
    assert_eq!(report["profiles"][0]["metrics"]["cases"], 40);
    assert_eq!(report["profiles"][0]["metrics"]["cases_attempted"], 40);
    assert_eq!(report["profiles"][0]["metrics"]["cases_persisted"], 40);
    assert_eq!(report["profiles"][0]["metrics"]["cases_failed"], 0);
    assert_eq!(report["profiles"][0]["metrics"]["exact_verdicts"], 40);
    assert_eq!(report["profiles"][0]["metrics"]["requests"], 120);
    assert_eq!(
        report["profiles"][0]["metrics"]["evidence_ref_validation_failures"],
        0
    );
    assert_eq!(
        report["profiles"][0]["metrics"]["estimated_cost_usd"],
        serde_json::Value::Null
    );
    assert_eq!(
        report["profiles"][0]["metrics"]["verifier_schema_compliance"]["numerator"],
        40
    );
    assert_eq!(
        report["profiles"][0]["metrics"]["judge_schema_compliance"]["numerator"],
        40
    );
    assert!(
        report["profiles"][0]["cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|case| {
                case["verifier_schema_status"] == "succeeded"
                    && case["judge_schema_status"] == "succeeded"
                    && case["evidence_validation_status"] == "succeeded"
                    && case["failure_category"].is_null()
            })
    );

    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("Reject expired tokens in the auth validator"));
}

#[test]
fn benchmark_ground_truth_and_canonical_subset_do_not_drift() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let suite: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("fixtures/evaluation/cases.json")).unwrap(),
    )
    .unwrap();
    let cases = suite["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 40);
    let canonical = cases
        .iter()
        .filter(|case| case["subset"] == "canonical-5")
        .map(|case| {
            (
                case["class"].as_str().unwrap(),
                case["expected"]["verdict"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canonical,
        vec![
            ("correct_patch", "SAME"),
            ("partial_implementation", "PARTIAL"),
            ("scope_creep", "PARTIAL"),
            ("constraint_violation", "PARTIAL"),
            ("wrong_component", "DIFFERENT"),
        ]
    );
    for case in cases {
        let findings = case["expected"]["important_findings"].as_array().unwrap();
        assert_eq!(
            findings.is_empty(),
            case["expected"]["finding_category"].is_null()
        );
        assert!(!case["expected"]["rationale"].as_str().unwrap().is_empty());
    }
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
