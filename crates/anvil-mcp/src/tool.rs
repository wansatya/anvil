//! MCP tools as first-class registry entries (SPEC §15-16).
//! From the agent's perspective these are indistinguishable from native tools.

use std::sync::Arc;

use crate::client::{McpClient, McpToolDef};

/// One discovered MCP tool, namespaced `mcp.<server>.<tool>`.
pub struct McpToolAdapter {
    static_name: &'static str,
    static_desc: &'static str,
    pub qualified_name: String,
    pub def: McpToolDef,
    pub preview_label: String,
    client: Arc<McpClient>,
}

impl McpToolAdapter {
    pub fn new(server: &str, def: McpToolDef, client: Arc<McpClient>) -> Self {
        let qualified_name = format!("mcp.{server}.{}", def.name);
        // Leaked once per discovered tool (bounded: one per tool per session)
        // to satisfy the registry's `&'static str` contract.
        let static_name: &'static str = Box::leak(qualified_name.clone().into_boxed_str());
        let static_desc: &'static str = if def.description.is_empty() {
            "(mcp tool)"
        } else {
            Box::leak(def.description.clone().into_boxed_str())
        };
        Self { static_name, static_desc, qualified_name, def, preview_label: server.to_string(), client }
    }
}

#[async_trait::async_trait]
impl anvil_tools::Tool for McpToolAdapter {
    fn name(&self) -> &'static str {
        self.static_name
    }
    fn description(&self) -> &'static str {
        self.static_desc
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.def.input_schema.clone()
    }
    fn category(&self) -> anvil_tools::ToolCategory {
        // MCP tools run external processes: always approval-gated.
        anvil_tools::ToolCategory::Network
    }
    fn preview(&self, _args: &serde_json::Value) -> String {
        format!("{}:{}", self.preview_label, self.def.name)
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: anvil_tools::ToolContext,
    ) -> Result<anvil_tools::ToolOutput, anvil_tools::ToolError> {
        let args = if args.is_null() { serde_json::json!({}) } else { args };
        match self.client.call_tool(&self.def.name, args).await {
            Ok(text) => Ok(anvil_tools::ToolOutput::ok("ok", &ctx.truncate(&text))),
            Err(e) => Ok(anvil_tools::ToolOutput::fail("mcp error", &e.to_string())),
        }
    }
}
