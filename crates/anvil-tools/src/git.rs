//! Git inspection tools (SPEC §11): status + diff. Read-only.

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolOutput};

async fn git(ctx: &ToolContext, args: &[&str]) -> Result<String, ToolError> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&ctx.workspace)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| ToolError::Failed(format!("git not available: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ToolError::Failed(if err.is_empty() {
            "git command failed".to_string()
        } else {
            err
        }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub struct GitStatus;

#[async_trait::async_trait]
impl Tool for GitStatus {
    fn name(&self) -> &'static str {
        "anvil.git_status"
    }
    fn description(&self) -> &'static str {
        "Show branch, staged/modified/untracked files (git status --short + branch)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    fn preview(&self, _args: &serde_json::Value) -> String {
        "git status".to_string()
    }
    async fn execute(&self, _args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let branch = git(&ctx, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        let status = git(&ctx, &["status", "--short"]).await?;
        let body = if status.trim().is_empty() { "(clean)".to_string() } else { status.trim().to_string() };
        Ok(ToolOutput::ok(
            &format!("branch {}", branch.trim()),
            &ctx.truncate(&format!("branch {}\n{body}", branch.trim())),
        ))
    }
}

pub struct GitDiff;

#[async_trait::async_trait]
impl Tool for GitDiff {
    fn name(&self) -> &'static str {
        "anvil.git_diff"
    }
    fn description(&self) -> &'static str {
        "Show the current unstaged diff (truncated). Optional `staged: true`."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"staged": {"type": "boolean", "default": false}}
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    fn preview(&self, _args: &serde_json::Value) -> String {
        "git diff".to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let staged = args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut argv = vec!["diff", "--no-color"];
        if staged {
            argv.push("--staged");
        }
        let diff = git(&ctx, &argv).await?;
        if diff.trim().is_empty() {
            return Ok(ToolOutput::ok("no changes", "Working tree clean — no diff."));
        }
        Ok(ToolOutput::ok(
            &format!("{} lines", diff.lines().count()),
            &ctx.truncate(diff.trim()),
        ))
    }
}
