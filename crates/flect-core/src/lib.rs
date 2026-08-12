//! Provider-neutral domain logic for Flect.

pub mod blind;
pub mod config;
pub mod context;
pub mod domain;
pub mod git;
pub mod reconcile;
pub mod state;

pub use blind::{BlindGuard, BlindGuardError, BundleContext};
pub use config::{Config, ConfigError};
pub use context::{ContextBuilder, ContextError};
pub use domain::*;
pub use git::{GitError, GitRepository};
pub use reconcile::reconcile;
pub use state::{RunStore, StateError};
