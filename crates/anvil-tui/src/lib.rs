//! Anvil TUI (SPEC §5, §26).
//! Ratatui + Crossterm. Async: never blocks on model/tool work,
//! all runtime progress arrives as [`AgentEvent`]s over a channel.

pub mod app;
pub mod history;
pub mod ui;

pub use app::{App, SlashAction};
