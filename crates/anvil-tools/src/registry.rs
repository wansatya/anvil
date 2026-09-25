//! Tool execution context, output, and registry.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{Tool, ToolError};

/// What a tool run produces for transcript + model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub success: bool,
    /// One-line outcome, e.g. `312 lines`.
    pub summary: String,
    /// Full (possibly truncated) content.
    pub output: String,
}

impl ToolOutput {
    pub fn ok(summary: &str, output: &str) -> Self {
        Self { success: true, summary: summary.to_string(), output: output.to_string() }
    }
    pub fn fail(summary: &str, output: &str) -> Self {
        Self { success: false, summary: summary.to_string(), output: output.to_string() }
    }
}

/// Execution context handed to every tool.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Sandboxed workspace root.
    pub workspace: std::path::PathBuf,
    /// Max bytes kept in `ToolOutput::output`.
    pub max_output_bytes: usize,
    /// Shell timeout.
    pub shell_timeout_secs: u64,
}

impl ToolContext {
    pub fn new(workspace: &std::path::Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            max_output_bytes: 64 * 1024,
            shell_timeout_secs: 120,
        }
    }

    pub fn truncate(&self, s: &str) -> String {
        truncate_bytes(s, self.max_output_bytes)
    }
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…\n[truncated {} bytes]", &s[..end], s.len() - end)
}

/// Named registry of native tools; also builds model-facing `ToolSpec`s.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        // Accept both `anvil.read_file` and flattened `read_file`.
        if let Some(t) = self.tools.get(name) {
            return Some(t.clone());
        }
        let short = name.rsplit('.').next().unwrap_or(name);
        self.tools.values().find(|t| t.name().ends_with(short)).cloned()
    }

    pub fn specs(&self) -> Vec<anvil_model::ToolSpec> {
        let mut specs: Vec<_> = self
            .tools
            .values()
            .map(|t| {
                anvil_model::ToolSpec::new(t.name(), t.description(), t.parameters_schema())
            })
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self.get(name).ok_or_else(|| ToolError::Failed(format!("unknown tool `{name}`")))?;
        tool.execute(args, ctx).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry pre-loaded with all native tools.
pub fn native_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(crate::filesystem::ReadFile);
    r.register(crate::filesystem::WriteFile);
    r.register(crate::filesystem::EditFile);
    r.register(crate::filesystem::ListFiles);
    r.register(crate::search::Search);
    r.register(crate::shell::Shell);
    r.register(crate::git::GitStatus);
    r.register(crate::git::GitDiff);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_namespaced_and_short() {
        let r = native_registry();
        assert!(r.get("anvil.read_file").is_some());
        assert!(r.get("read_file").is_some());
        assert!(r.get("anvil.nope").is_none());
        let specs = r.specs();
        assert!(specs.iter().any(|s| s.name == "anvil.shell"));
    }

    #[test]
    fn truncate_respects_char_boundary() {
        let ctx = ToolContext::new(std::path::Path::new("/tmp"));
        let s = "héllo world, this is long";
        let t = ctx.truncate(s);
        assert!(t.len() <= 64 * 1024);
        let ctx2 = ToolContext { max_output_bytes: 5, ..ctx };
        assert_eq!(ctx2.truncate("héllo"), "héll…\n[truncated 1 bytes]");
    }
}
