//! Context compilation (SPEC §17-18, §5 context, §2.2).
//! Builds the model-bound context package from instructions, files,
//! git state, conversation, skills, and tool results — within a token budget.

pub mod budget;
pub mod builder;
pub mod git;
pub mod instructions;
pub mod project;
pub mod relevance;

pub use builder::{CompiledContext, ContextBuilder, ContextInput};
pub use project::Workspace;
