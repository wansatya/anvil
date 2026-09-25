//! Skills (SPEC §14): reusable domain capabilities in `SKILL.md` files.
//! Discovery loads only name + description; the full body is loaded lazily
//! when the agent selects a skill — never bulk-loaded into context.

use std::path::PathBuf;

pub mod builtin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    Builtin,
    Project,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
    pub source: SkillSource,
    /// Embedded body for built-ins (no filesystem needed).
    pub embedded: Option<&'static str>,
}

/// Where skills live.
pub fn project_dir(workspace_root: &std::path::Path) -> PathBuf {
    workspace_root.join(".anvil/skills")
}

pub fn global_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .map(|h| PathBuf::from(h).join(".config/anvil/skills"))
}

/// Discover skills (project first, then global). Only frontmatter is parsed.
pub fn discover(workspace_root: &std::path::Path) -> Vec<Skill> {
    let mut out = Vec::new();
    let roots = [
        (project_dir(workspace_root), SkillSource::Project),
        (global_dir().unwrap_or_default(), SkillSource::Global),
    ];
    for (root, source) in roots {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        let mut entries: Vec<_> = rd.flatten().filter(|e| e.path().is_dir()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if let Some((name, description)) = parse_frontmatter(&entry.path()) {
                out.push(Skill { name, description, dir: entry.path(), source, embedded: None });
            }
        }
    }
    out
}

/// Everything Anvil knows: built-ins first, then project, then global.
/// Project/global entries shadow a built-in of the same name (override).
pub fn all(workspace_root: &std::path::Path) -> Vec<Skill> {
    let mut out = builtin::builtin_skills();
    for s in discover(workspace_root) {
        if let Some(i) = out.iter().position(|b| b.name == s.name) {
            out[i] = s;
        } else {
            out.push(s);
        }
    }
    out
}

/// Parse `SKILL.md` frontmatter (`name:`/`description:` between `---`
/// fences). Falls back to the directory name; missing file → `None`.
pub fn parse_frontmatter(dir: &std::path::Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let fallback = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "(unnamed)".to_string());
    Some(parse_frontmatter_from(&content).unwrap_or((fallback, String::new())))
}

/// Frontmatter from raw text; `None` when no usable fence/content.
pub fn parse_frontmatter_from(content: &str) -> Option<(String, String)> {
    let mut name = String::new();
    let mut description = String::new();
    let mut lines = content.lines();
    if lines.next().map(|l| l.trim()) != Some("---") {
        return None;
    }
    let mut closed = false;
    for line in lines {
        let t = line.trim();
        if t == "---" {
            closed = true;
            break;
        }
        if let Some(v) = t.strip_prefix("name:") {
            let v = clean_scalar(v);
            if !v.is_empty() {
                name = v;
            }
        } else if let Some(v) = t.strip_prefix("description:") {
            let v = clean_scalar(v);
            if !v.is_empty() {
                description = v;
            }
        }
    }
    if !closed || name.is_empty() {
        return None;
    }
    Some((name, description))
}

fn clean_scalar(v: &str) -> String {
    v.trim().trim_matches('"').trim_matches('\'').trim().to_string()
}

/// Load the full skill body (minus frontmatter) for injection into context.
/// Built-ins come from the embedded text; others read `SKILL.md` from disk.
pub fn load_body(skill: &Skill) -> anyhow::Result<String> {
    if let Some(text) = skill.embedded {
        return Ok(strip_frontmatter(text).trim().to_string());
    }
    let content = std::fs::read_to_string(skill.dir.join("SKILL.md"))?;
    Ok(strip_frontmatter(&content).trim().to_string())
}

fn strip_frontmatter(content: &str) -> &str {
    let mut lines = content.lines();
    if lines.next().map(|l| l.trim()) != Some("---") {
        return content;
    }
    let mut depth = 0;
    for line in lines {
        depth += line.len() + 1;
        if line.trim() == "---" {
            return &content[depth.min(content.len())..];
        }
    }
    content
}

/// Compact catalog for the system prompt: name + description only.
pub fn render_catalog(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from("Available skills (ask to use one by name; full instructions load on demand):\n");
    for s in skills {
        let scope = match s.source {
            SkillSource::Builtin => "builtin",
            SkillSource::Project => "project",
            SkillSource::Global => "global",
        };
        let desc = if s.description.is_empty() { "(no description)" } else { &s.description };
        out.push_str(&format!("- {} [{scope}]: {desc}\n", s.name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> PathBuf {
        let base = std::env::temp_dir().join(format!("anvil-skill-{}", std::process::id()));
        let dir = base.join(".anvil/skills/cpp-review");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: cpp-review\ndescription: Review C++ code.\n---\n\n# C++ Review\n\nCheck ownership.\n",
        )
        .unwrap();
        base
    }

    #[test]
    fn discover_and_lazy_load() {
        let base = setup();
        let skills = discover(&base);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "cpp-review");
        assert_eq!(skills[0].source, SkillSource::Project);
        let body = load_body(&skills[0]).unwrap();
        assert!(body.contains("Check ownership"));
        assert!(!body.contains("description:"));
        assert!(render_catalog(&skills).contains("cpp-review"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_skill_file_is_skipped() {
        let base = std::env::temp_dir().join(format!("anvil-skill-miss-{}", std::process::id()));
        std::fs::create_dir_all(base.join(".anvil/skills/empty")).unwrap();
        assert!(discover(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
