//! Restrained human-readable terminal output.

use flect_core::{BlindBundle, EchoedSpec, RunRecord, VerificationRecord};
use serde_json::Value;

pub fn run_created(run: &RunRecord) {
    println!("Flect run created\n");
    println!("Run      {}", run.id);
    println!("Base     {}", short_revision(&run.base_revision));
    println!("Task     captured");
    println!(
        "Spec     captured {}\n",
        if run.model_calls.is_empty() {
            "deterministically"
        } else {
            "semantically"
        }
    );
    model_calls(&run.model_calls);
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
    model_calls(&record.model_calls);
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

pub fn dry_run(value: &Value) {
    println!("Flect verification dry run\n");
    println!("Request sent    no");
    println!(
        "Runner          {}",
        value["runner"]["provider"].as_str().unwrap_or("unknown")
    );
    println!(
        "Model           {}",
        value["runner"]["model"]
            .as_str()
            .unwrap_or("not configured")
    );
    println!(
        "Context         {}",
        value["context_policy"].as_str().unwrap_or("unknown")
    );
    println!("\nIncluded patch files");
    for file in value["included"]["patch_files"]
        .as_array()
        .into_iter()
        .flatten()
    {
        println!("  {}", file.as_str().unwrap_or("unknown"));
    }
    println!("\nIncluded context files");
    for file in value["included"]["context_files"]
        .as_array()
        .into_iter()
        .flatten()
    {
        println!("  {}", file.as_str().unwrap_or("unknown"));
    }
    println!("\nExcluded files");
    for excluded in value["excluded"].as_array().into_iter().flatten() {
        println!(
            "  {} — {}",
            excluded["path"].as_str().unwrap_or("unknown"),
            excluded["reason"].as_str().unwrap_or("unspecified")
        );
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
        value["runner"]["kind"].as_str().unwrap_or("unknown")
    );
    if let Some(model) = value["runner"]["model"].as_str() {
        println!("Model          {model}");
    }
    if let Some(fallback) = value["runner"]["fallback_model"].as_str() {
        println!("Fallback       {fallback}");
    }
    if let Some(credential) = value["runner"]["credential"].as_object() {
        let name = credential["environment"].as_str().unwrap_or("unknown");
        let status = if credential["available"].as_bool().unwrap_or(false) {
            "available"
        } else {
            "missing"
        };
        println!("API key        {status} ({name})");
    }
    println!(
        "Codex CLI      {}",
        if value["codex"]["available"].as_bool().unwrap_or(false) {
            "available"
        } else {
            "unavailable"
        }
    );
    println!(
        "Codex Skill    {}",
        value["codex"]["skill"].as_str().unwrap_or("not checked")
    );
    println!("MCP server     available (`flect mcp`)");
    println!(
        "API mode       {} ({})",
        if value["verification_modes"]["api"]["ready"]
            .as_bool()
            .unwrap_or(false)
        {
            "ready"
        } else {
            "not ready"
        },
        value["verification_modes"]["api"]["isolation"]
            .as_str()
            .unwrap_or("unknown")
    );
    println!(
        "Agent mode     runtime capability unknown (workspace isolation: {})",
        value["verification_modes"]["codex_agent"]["workspace_isolation"]
            .as_str()
            .unwrap_or("unknown")
    );
    println!(
        "\n{}",
        if value["ready"].as_bool().unwrap_or(false) {
            "Ready."
        } else {
            "Not ready. Resolve the checks above and run `flect doctor` again."
        }
    );
}

fn short_revision(revision: &str) -> &str {
    revision.get(..8).unwrap_or(revision)
}

fn model_calls(calls: &[flect_core::ModelCallRecord]) {
    if calls.is_empty() {
        return;
    }
    println!("\nModel routing");
    for call in calls {
        println!(
            "  {} attempt {}  {}  {}{}",
            call.stage,
            call.attempt,
            call.model,
            if call.accepted {
                "accepted"
            } else {
                "escalated"
            },
            call.estimated_cost_usd
                .map_or_else(String::new, |cost| { format!("  estimated ${cost:.6}") })
        );
        if let Some(reason) = &call.escalation_reason {
            println!("    {reason}");
        }
    }
}
