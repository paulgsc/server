#![cfg(feature = "stream-orch")]

pub mod config;
pub mod error;
pub mod orchestrator;
pub mod schedule;
pub mod state;
pub mod tick;
pub mod types;

pub use config::OrchestratorConfig;
pub use error::{OrchestratorError, Result};
pub use orchestrator::StreamOrchestrator;
pub use schedule::{SceneSchedule, ScheduledElement};
pub use state::{OrchestratorState, StreamStatus};
pub use tick::TickEngine;
pub use types::{SceneConfig, SceneId, TimeMs};
