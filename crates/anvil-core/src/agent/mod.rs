//! Agent execution loop (SPEC §6, Phase 3).
//! Iterative: build context → stream model → execute tools (with approval)
//! → feed results back. Terminates on final response, cancel, iteration
//! cap, or unrecoverable error.

pub mod approval;
pub mod runner;

pub use approval::{ApprovalGate, ApprovalJob};
pub use runner::{Agent, AgentConfig, AgentError, AgentOutcome};
