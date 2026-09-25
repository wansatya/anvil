//! Text search tool (SPEC §11): ripgrep when available, native fallback.

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolOutput};

pub struct Search;

#[async_trait::async_trait]
impl Tool for Search {
    fn name(&self) -> &'static str {
        "anvil.search"
    }
    fn description(&self) -> &'static str {
        "Search workspace text for a pattern. Respects .gitignore. Returns file:line matches."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "glob": {"type": "string", "description": "Optional file filter, e.g. \".rs\""},
                "limit": {"type": "integer", "default": 50}
            },
            "required": ["pattern"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("pattern").and_then(|p| p.as_str()).unwrap_or("?").to_string()
    }
    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError> {
        let pattern = crate::arg_str(&args, "pattern")?;
        let glob = args.get("glob").and_then(|g| g.as_str()).unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50).clamp(1, 500) as usize;
        if pattern.is_empty() || pattern.len() > 500 {
            return Err(ToolError::InvalidArgs("pattern must be 1..500 chars".into()));
        }
        // Prefer ripgrep (SPEC §11).
        if let Some(matches) = try_ripgrep(&ctx, &pattern, glob, limit).await {
            return Ok(matches);
        }
        Ok(native_search(&ctx, &pattern, glob, limit))
    }
}

async fn try_ripgrep(ctx: &ToolContext, pattern: &str, glob: &str, limit: usize) -> Option<ToolOutput> {
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("--no-heading")
        .arg("--line-number")
        .arg("--color=never")
        .arg(format!("--max-count={limit}"))
        .arg("--hidden")
        .arg("--glob")
        .arg("!.git/")
        .arg(pattern)
        .arg(&ctx.workspace)
        .kill_on_drop(true);
    if !glob.is_empty() {
        cmd.arg("--glob").arg(format!("*{glob}*"));
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await.ok()?.ok()?;
    if !out.status.success() && out.status.code() != Some(1) {
        return None; // rg error (e.g. bad regex) -> fall back to literal search
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Strip workspace prefix for readable paths.
    let prefix = format!("{}/", ctx.workspace.display());
    let lines: Vec<String> = text
        .lines()
        .take(limit)
        .map(|l| l.strip_prefix(&prefix).unwrap_or(l).to_string())
        .collect();
    Some(ToolOutput::ok(&format!("{} matches", lines.len()), &ctx.truncate(&lines.join("\n"))))
}

fn native_search(ctx: &ToolContext, pattern: &str, glob: &str, limit: usize) -> ToolOutput {
    let re = regex::Regex::new(pattern).ok();
    let walker = ignore::WalkBuilder::new(&ctx.workspace)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();
    let mut matches = Vec::new();
    'outer: for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Ok(rel) = p.strip_prefix(&ctx.workspace) else { continue };
        let rel = rel.to_string_lossy().to_string();
        if !glob.is_empty() && !rel.contains(glob) {
            continue;
        }
        let Ok(data) = std::fs::read(p) else { continue };
        if data.len() > 2 * 1024 * 1024 || data.iter().take(8000).any(|&b| b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&data);
        for (i, line) in text.lines().enumerate() {
            let hit = match &re {
                Some(re) => re.is_match(line),
                None => line.contains(pattern),
            };
            if hit {
                let line_short: String = line.chars().take(240).collect();
                matches.push(format!("{}:{}: {}", rel, i + 1, line_short));
                if matches.len() >= limit {
                    break 'outer;
                }
            }
        }
    }
    ToolOutput::ok(&format!("{} matches", matches.len()), &ctx.truncate(&matches.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn search_finds_text() {
        let dir = std::env::temp_dir().join(format!("anvil-se-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn main() {\n  let needle = 1;\n}\n").unwrap();
        let ctx = ToolContext::new(&dir);
        let s = Search;
        let out = s.execute(serde_json::json!({"pattern": "needle"}), ctx).await.unwrap();
        assert!(out.output.contains("a.rs:2"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
