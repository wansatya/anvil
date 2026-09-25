//! MVP relevance (SPEC §18): explicit `@file` references from the user
//! message, plus recently touched files. No vector DB.

/// Extract `@path` references from text.
pub fn explicit_refs(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '@' {
            continue;
        }
        // Skip `@@` and email-like `user@host`.
        let prev = text[..i].chars().last();
        if prev.is_some_and(|p| !p.is_whitespace() && p != '(' && p != '"' && p != '\'') {
            continue;
        }
        let mut end = i + 1;
        for (j, ch) in text[i + 1..].char_indices() {
            if ch.is_whitespace() || matches!(ch, '"' | '\'' | '(' | ')' | ',' | ';') {
                break;
            }
            end = i + 1 + j + ch.len_utf8();
        }
        if end > i + 1 {
            refs.push(text[i + 1..end].trim_end_matches(&['.', ':', '!', '?']).to_string());
        }
    }
    refs.retain(|r| !r.is_empty());
    refs
}

/// Recently modified files under the workspace (mtime order, capped).
/// Respects `.gitignore` via the `ignore` walker.
pub fn recent_files(workspace_root: &std::path::Path, cap: usize) -> Vec<std::path::PathBuf> {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    let walker = ignore::WalkBuilder::new(workspace_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();
    for entry in walker.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                files.push((mtime, p.to_path_buf()));
            }
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().take(cap).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_at_refs() {
        assert_eq!(explicit_refs("fix @src/main.rs and @ Cargo.toml"), vec!["src/main.rs"]);
        assert_eq!(explicit_refs("mail me@host.com"), Vec::<String>::new());
        assert!(explicit_refs("nothing here").is_empty());
    }
}
