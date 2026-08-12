//! Restrained human-readable terminal output.

use flect_core::{BlindBundle, EchoedSpec, RunRecord, VerificationRecord};
use serde_json::Value;

pub fn run_created(run: &RunRecord) {
    println!("Flect run created\n");
    println!("Run      {}", run.id);
    println!("Base     {}", short_revision(&run.base_revision));
    println!("Task     captured");
    println!("Spec     captured deterministically\n");
    println!("Ready for implementation.");
}

pub fn verification(record: &VerificationRecord) {
    println!("Flect\n");
    println!("Patch");
    println!(
        "  {} files\n  +{} / -{}\n",
        record.bundle.patch.files.len(),
        record.bundle.patch.insertions,
        record.bundle.patch.deletions
    );
    println!("Blind verification");
    for entry in &record.bundle.blindness_report.entries {
        println!(
            "  {:20} {}",
            entry.source.to_ascii_lowercase(),
            entry.status
        );
    }
    println!(
        "  {:20} {}\n",
        "context", record.bundle.manifest.context_policy
    );
    println!(
        "Echoed intent\n\n  {}\n",
        record.echoed_spec.apparent_objective
    );
    println!("Alignment\n\n  {}\n", record.verdict.alignment);
    for evidence in &record.verdict.evidence {
        if let Some(file) = &evidence.file {
            println!("{file}");
        }
        println!("  {}\n", evidence.description);
    }
    println!(
        "Recommended action\n\n  {}",
        record.verdict.recommended_action
    );
}

pub fn echo(echoed: &EchoedSpec) {
    println!("Apparent changes\n");
    if echoed.behavior_after.is_empty() {
        println!("  {}", echoed.apparent_objective);
    } else {
        for (index, behavior) in echoed.behavior_after.iter().enumerate() {
            println!("{}. {behavior}", index + 1);
        }
    }
    if !echoed.affected_scope.is_empty() {
        println!("\nAffected areas\n");
        for scope in &echoed.affected_scope {
            println!("  {scope}");
        }
    }
    if !echoed.uncertainties.is_empty() {
        println!("\nUncertainties\n");
        for uncertainty in &echoed.uncertainties {
            println!("  {uncertainty}");
        }
    }
}

pub fn inspection(bundle: &BlindBundle) {
    println!("Verifier bundle\n");
    println!("Context      {}", bundle.manifest.context_policy);
    println!("Payload      {} bytes", bundle.manifest.total_bytes);
    println!("Patch files  {}", bundle.manifest.patch_files.len());
    for path in &bundle.manifest.patch_files {
        println!("  {path}");
    }
    println!("Context files  {}", bundle.manifest.context_files.len());
    for path in &bundle.manifest.context_files {
        println!("  {path}");
    }
    if !bundle.manifest.excluded_paths.is_empty() {
        println!("\nExcluded");
        for excluded in &bundle.manifest.excluded_paths {
            println!("  {} — {}", excluded.path, excluded.reason);
        }
    }
    println!("\nBlindGuard");
    for entry in &bundle.blindness_report.entries {
        println!(
            "  {:20} {} ({:?})",
            entry.source, entry.status, entry.assurance
        );
    }
    for limitation in &bundle.blindness_report.limitations {
        println!("\nLimitation: {limitation}");
    }
}

pub fn doctor(value: &Value) {
    println!("Flect doctor\n");
    println!(
        "Git            {}",
        value["git"].as_str().unwrap_or("unknown")
    );
    println!(
        "Repository     {}",
        value["repository"].as_str().unwrap_or("unknown")
    );
    println!(
        "Configuration  {}",
        value["configuration"].as_str().unwrap_or("unknown")
    );
    println!(
        "Runner         {}",
        value["runner_provider"].as_str().unwrap_or("unknown")
    );
    println!(
        "\n{}",
        if value["ready"].as_bool().unwrap_or(false) {
            "Ready for deterministic verification."
        } else {
            "Not ready. Resolve the checks above and run `flect doctor` again."
        }
    );
}

fn short_revision(revision: &str) -> &str {
    revision.get(..8).unwrap_or(revision)
}
