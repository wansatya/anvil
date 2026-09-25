//! Filesystem tools: read / list / write / edit (SPEC §11).
//! All paths are sandboxed to the workspace (SPEC §28).

use crate::{arg_str, sandbox_path, Tool, ToolCategory, ToolContext, ToolError, ToolOutput};

const MAX_READ_BYTES: usize = 256 * 1024;

fn is_probably_binary(data: &[u8]) -> bool {
    data.iter().take(8000).any(|&b| b == 0)
}

pub struct ReadFile;

#[async_trait::async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "anvil.read_file"
    }
    fn description(&self) -> &'static str {
        "Read a file from the workspace. Returns numbered lines; large files are truncated."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Workspace-relative file path"},
                "offset": {"type": "integer", "description": "First line (1-based)", "default": 1},
                "limit": {"type": "integer", "description": "Max lines", "default": 200}
            },
            "required": ["path"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("path").and_then(|p| p.as_str()).unwrap_or("?").to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let path = arg_str(&args, "path")?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200).clamp(1, 2000) as usize;
        let abs = sandbox_path(&ctx.workspace, &path)?;
        let data = std::fs::read(&abs).map_err(|e| ToolError::Io(format!("{path}: {e}")))?;
        if is_probably_binary(&data) {
            return Ok(ToolOutput::ok(
                "binary file",
                &format!("{path}: binary file ({} bytes), not shown", data.len()),
            ));
        }
        let mut bytes = data;
        let truncated = bytes.len() > MAX_READ_BYTES;
        bytes.truncate(MAX_READ_BYTES);
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        let start = (offset - 1).min(total);
        let end = (start + limit).min(total);
        let numbered: Vec<String> =
            lines[start..end].iter().enumerate().map(|(i, l)| format!("{:>6}  {l}", start + i + 1)).collect();
        let summary = format!("{total} lines{}", if truncated { " (truncated)" } else { "" });
        Ok(ToolOutput::ok(&summary, &ctx.truncate(&numbered.join("\n"))))
    }
}

pub struct ListFiles;

#[async_trait::async_trait]
impl Tool for ListFiles {
    fn name(&self) -> &'static str {
        "anvil.list_files"
    }
    fn description(&self) -> &'static str {
        "List workspace files, respecting .gitignore. Optional glob filter and cap."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "glob": {"type": "string", "description": "Substring filter, e.g. \".rs\""},
                "limit": {"type": "integer", "default": 200}
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("glob").and_then(|g| g.as_str()).unwrap_or("(all files)").to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let glob = args.get("glob").and_then(|g| g.as_str()).unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200).clamp(1, 5000) as usize;
        let walker = ignore::WalkBuilder::new(&ctx.workspace)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .build();
        let mut files = Vec::new();
        for entry in walker.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let Ok(rel) = p.strip_prefix(&ctx.workspace) else { continue };
            let rel = rel.to_string_lossy().to_string();
            if !glob.is_empty() && !rel.contains(glob) {
                continue;
            }
            files.push(rel);
            if files.len() >= limit {
                break;
            }
        }
        files.sort();
        Ok(ToolOutput::ok(&format!("{} files", files.len()), &files.join("\n")))
    }
}

pub struct WriteFile;

#[async_trait::async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "anvil.write_file"
    }
    fn description(&self) -> &'static str {
        "Create or overwrite a workspace file. Requires approval by default."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::SafeWrite
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("path").and_then(|p| p.as_str()).unwrap_or("?").to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let path = arg_str(&args, "path")?;
        let content = arg_str(&args, "content")?;
        let abs = sandbox_path(&ctx.workspace, &path)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError::Io(e.to_string()))?;
        }
        std::fs::write(&abs, content.as_bytes()).map_err(|e| ToolError::Io(format!("{path}: {e}")))?;
        Ok(ToolOutput::ok("written", &format!("wrote {} ({} bytes)", path, content.len())))
    }
}

pub struct EditFile;

#[async_trait::async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "anvil.edit_file"
    }
    fn description(&self) -> &'static str {
        "Replace the exact string `old` with `new` in a file. Fails unless `old` matches exactly once."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old": {"type": "string", "description": "Exact text to replace"},
                "new": {"type": "string", "description": "Replacement text"}
            },
            "required": ["path", "old", "new"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::SafeWrite
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("path").and_then(|p| p.as_str()).unwrap_or("?").to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let path = arg_str(&args, "path")?;
        let old = arg_str(&args, "old")?;
        let new = arg_str(&args, "new")?;
        let abs = sandbox_path(&ctx.workspace, &path)?;
        let content = std::fs::read_to_string(&abs).map_err(|e| ToolError::Io(format!("{path}: {e}")))?;
        match content.match_indices(&old).count() {
            0 => return Err(ToolError::Failed("`old` text not found — no changes made".into())),
            2.. => return Err(ToolError::Failed("`old` text matches multiple regions — refusing to guess".into())),
            _ => {}
        }
        let updated = content.replacen(&old, &new, 1);
        std::fs::write(&abs, updated.as_bytes()).map_err(|e| ToolError::Io(format!("{path}: {e}")))?;
        Ok(ToolOutput::ok("edited", &format!("edited {path} (1 region replaced)")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (TempfileGuard, ToolContext) {
        let dir = std::env::temp_dir().join(format!("anvil-fs-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = ToolContext::new(&dir);
        (TempfileGuard(dir), c)
    }

    struct TempfileGuard(std::path::PathBuf);
    impl Drop for TempfileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn rand_suffix() -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::thread::current().id().hash(&mut h);
        std::time::SystemTime::now().hash(&mut h);
        h.finish()
    }

    #[tokio::test]
    async fn read_write_edit_roundtrip() {
        let (_g, c) = ctx();
        let w = WriteFile;
        w.execute(serde_json::json!({"path": "a.txt", "content": "hello\nworld\n"}), c.clone()).await.unwrap();
        let r = ReadFile;
        let out = r.execute(serde_json::json!({"path": "a.txt"}), c.clone()).await.unwrap();
        assert!(out.output.contains("1  hello") || out.output.contains("hello"));
        let e = EditFile;
        assert!(e.execute(serde_json::json!({"path": "a.txt", "old": "nope", "new": "x"}), c.clone()).await.is_err());
        assert!(e.execute(serde_json::json!({"path": "a.txt", "old": "world", "new": "WORLD"}), c.clone()).await.is_ok());
        // ambiguous old text rejected
        w.execute(serde_json::json!({"path": "b.txt", "content": "x\nx\n"}), c.clone()).await.unwrap();
        assert!(e.execute(serde_json::json!({"path": "b.txt", "old": "x", "new": "y"}), c.clone()).await.is_err());
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let (_g, c) = ctx();
        let w = WriteFile;
        for i in 0..5 {
            w.execute(serde_json::json!({"path": format!("f{i}.rs"), "content": "x"}), c.clone()).await.unwrap();
        }
        let l = ListFiles;
        let out = l.execute(serde_json::json!({"glob": ".rs"}), c.clone()).await.unwrap();
        assert_eq!(out.summary, "5 files");
    }
}
