//! Trusted application workflows shared by CLI and MCP adapters.

mod agent;
mod evidence;

pub use agent::{AgentService, AgentWorkflowError};
pub use evidence::{EvidenceError, sanitize_verdict_evidence, validate_verdict_evidence};
