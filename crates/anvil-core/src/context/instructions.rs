//! Hierarchical project instructions (SPEC §13): `ANVIL.md` plus
//! `.anvil/AGENTS.md`, with deeper files overriding general ones.

/// Load the instruction chain for `target` (a file or dir): root first,
/// deeper files last. Returns `(path, content)` pairs.
pub fn load_chain(
    workspace_root: &std::path::Path,
    target: &std::path::Path,
) -> Vec<(std::path::PathBuf, String)> {
    let mut chain = Vec::new();
    // Root-level files apply everywhere.
    for name in ["ANVIL.md", ".anvil/AGENTS.md"] {
        let p = workspace_root.join(name);
        if let Ok(content) = std::fs::read_to_string(&p) {
            if !content.trim().is_empty() {
                chain.push((p, content));
            }
        }
    }
    // Directory chain from root toward the target.
    let target_dir = if target.is_file() {
        target.parent().map(|p| p.to_path_buf())
    } else {
        Some(target.to_path_buf())
    };
    let mut dirs = Vec::new();
    let mut cur = target_dir;
    while let Some(d) = cur {
        if d == workspace_root {
            break;
        }
        // Only descend within the workspace.
        if !d.starts_with(workspace_root) {
            break;
        }
        dirs.push(d.clone());
        cur = d.parent().map(|p| p.to_path_buf());
    }
    dirs.reverse();
    for d in dirs {
        let p = d.join("ANVIL.md");
        if let Ok(content) = std::fs::read_to_string(&p) {
            if !content.trim().is_empty() {
                chain.push((p, content));
            }
        }
    }
    chain
}

/// Render the chain as one instructions block (later = higher precedence).
pub fn render(chain: &[(std::path::PathBuf, String)]) -> String {
    if chain.is_empty() {
        return String::new();
    }
    let mut out = String::from("Project instructions (more specific sections override general ones):\n");
    for (path, content) in chain {
        out.push_str(&format!("\n--- {} ---\n{}\n", path.display(), content.trim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_orders_root_before_specific() {
        let base = std::env::temp_dir().join(format!("anvil-instr-{}", std::process::id()));
        let sub = base.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("ANVIL.md"), "root rules").unwrap();
        std::fs::write(sub.join("ANVIL.md"), "src rules").unwrap();
        let chain = load_chain(&base, &sub.join("foo.rs"));
        assert_eq!(chain.len(), 2);
        assert!(chain[0].1.contains("root"));
        assert!(chain[1].1.contains("src"));
        assert!(render(&chain).contains("root rules"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
