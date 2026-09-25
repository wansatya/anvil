//! Shell execution (SPEC §11). Potentially destructive: default policy is
//! `ask` — the agent must obtain approval before calling.

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolOutput};

pub struct Shell;

#[async_trait::async_trait]
impl Tool for Shell {
    fn name(&self) -> &'static str {
        "anvil.shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in the workspace (sh -c). Returns combined output and exit status."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout_secs": {"type": "integer", "description": "Override (max 600)"}
            },
            "required": ["command"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Shell
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("command").and_then(|c| c.as_str()).unwrap_or("?").to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let command = crate::arg_str(&args, "command")?;
        if command.trim().is_empty() || command.len() > 8000 {
            return Err(ToolError::InvalidArgs("command must be 1..8000 chars".into()));
        }
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(ctx.shell_timeout_secs)
            .clamp(1, 600);
        // Minimal secret hygiene for the invoked environment (SPEC §28):
        // API keys are never *added* here; the child inherits the user's env.
        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .current_dir(&ctx.workspace)
            .env("ANVIL_TOOL", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ToolError::Failed(format!("spawn failed: {e}")))?;
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| ToolError::Failed(format!("timed out after {timeout_secs}s")))?
        .map_err(|e| ToolError::Failed(format!("wait failed: {e}")))?;
        let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.trim().is_empty() {
            combined.push_str("\n[stderr]\n");
            combined.push_str(&stderr);
        }
        let code = out.status.code().unwrap_or(-1);
        let summary = format!("exit {code}");
        let output = ctx.truncate(combined.trim());
        if out.status.success() {
            Ok(ToolOutput::ok(&summary, &output))
        } else {
            Ok(ToolOutput::fail(&summary, &output))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_echo_and_failure() {
        let dir = std::env::temp_dir().join(format!("anvil-sh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = ToolContext::new(&dir);
        let s = Shell;
        let ok = s.execute(serde_json::json!({"command": "echo hi"}), ctx.clone()).await.unwrap();
        assert!(ok.success && ok.output.contains("hi"));
        let bad = s.execute(serde_json::json!({"command": "exit 3"}), ctx).await.unwrap();
        assert!(!bad.success && bad.summary == "exit 3");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
