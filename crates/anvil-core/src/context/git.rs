//! Git state snapshot for context (SPEC §13/§17.7).

/// Compact branch + status summary. Never fails: returns a note instead.
pub fn summarize(workspace_root: &std::path::Path) -> String {
    let branch = run(workspace_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let status = run(workspace_root, &["status", "--short"]);
    match (branch, status) {
        (Ok(b), Ok(s)) => {
            let files: Vec<&str> = s.lines().take(30).collect();
            let extra = if s.lines().count() > 30 { "\n… (truncated)" } else { "" };
            if files.is_empty() {
                format!("git: branch {} (clean)", b.trim())
            } else {
                format!("git: branch {}\n{}{extra}", b.trim(), files.join("\n"))
            }
        }
        _ => "git: unavailable (not a repository?)".to_string(),
    }
}

fn run(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
