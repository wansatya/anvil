//! MCP client (SPEC §15): stdio JSON-RPC servers, tool discovery, and
//! execution. MCP tools surface as `mcp.<server>.<tool>` in the same
//! registry as native tools (SPEC §16).

pub mod client;
pub mod tool;

pub use client::{McpClient, McpConfig, McpError, McpToolDef};
pub use tool::McpToolAdapter;
