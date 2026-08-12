//! Trusted application workflows shared by CLI and MCP adapters.

mod agent;
mod evidence;

pub use agent::{AgentService, AgentWorkflowError, CleanupOptions, CleanupReport};
pub use evidence::{
    EvidenceError, materialize_judge_verdict, sanitize_verdict_evidence, validate_verdict_evidence,
};
