//! Effective configuration (SPEC §23-24). File parsing lands fully in
//! Phase 1-hardening; MVP resolves defaults + env overrides.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveConfig {
    pub model: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub workspace: String,
    pub max_iterations: u32,
}

/// Re-exported so CLI code keeps a single import site.
pub use anvil_core::{DEFAULT_BASE_URL, DEFAULT_MODEL};

impl Default for EffectiveConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            has_api_key: false,
            workspace: ".".to_string(),
            max_iterations: 50,
        }
    }
}

fn env_first(keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Ok(v) = std::env::var(k) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// The actual API key value (never logged or printed — SPEC §28).
/// Precedence: environment first, then the local config file, so
/// `/connect` persists across restarts without re-entry.
pub fn api_key() -> String {
    if let Some(k) = env_first(&["ANVIL_API_KEY", "OPENCODE_ZEN_API_KEY", "OPENCODE_API_KEY"]) {
        return k;
    }
    file_values().2.unwrap_or_default()
}

pub fn load() -> EffectiveConfig {
    let mut cfg = EffectiveConfig::default();
    // File first, env wins (SPEC §23-24).
    let (file_model, file_url, file_key) = file_values();
    if let Some(m) = file_model {
        cfg.model = m;
    }
    if let Some(u) = file_url {
        cfg.base_url = u;
    }
    if let Some(m) = env_first(&["ANVIL_MODEL"]) {
        cfg.model = m;
    }
    if let Some(u) = env_first(&["ANVIL_BASE_URL", "OPENCODE_BASE_URL"]) {
        cfg.base_url = u;
    }
    if env_first(&["ANVIL_API_KEY", "OPENCODE_ZEN_API_KEY", "OPENCODE_API_KEY"]).is_some()
        || file_key.is_some()
    {
        cfg.has_api_key = true;
    }
    if let Ok(cwd) = std::env::current_dir() {
        cfg.workspace = cwd.display().to_string();
    }
    cfg
}

/// Config file path. `ANVIL_CONFIG` overrides (SPEC §24),
/// otherwise `~/.config/anvil/config.yaml` (SPEC §23).
pub fn config_file_path() -> PathBuf {
    if let Some(p) = env_first(&["ANVIL_CONFIG"]) {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/anvil/config.yaml")
}

/// Minimal top-level `key: value` reader (blank lines, comments,
/// single/double quotes tolerated). Only matches non-indented lines so
/// nested provider blocks are left alone.
fn read_simple_key(content: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in content.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some(rest) = t.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let v = unquote(rest.trim());
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

fn file_values() -> (Option<String>, Option<String>, Option<String>) {
    let content = match std::fs::read_to_string(config_file_path()) {
        Ok(c) => c,
        Err(_) => return (None, None, None),
    };
    (
        read_simple_key(&content, "model"),
        read_simple_key(&content, "base_url"),
        read_simple_key(&content, "api_key"),
    )
}

/// One MCP server declaration from the config file (SPEC §15):
/// ```yaml
/// mcp:
///   servers:
///     docs:
///       command: npx
///       args: ["-y", "@modelcontextprotocol/server-docs"]
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerDecl {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Parsed `mcp.servers` map; unknown file content is tolerated.
pub fn mcp_servers() -> Vec<(String, McpServerDecl)> {
    #[derive(Debug, Deserialize, Default)]
    struct Root {
        #[serde(default)]
        mcp: McpSection,
    }
    #[derive(Debug, Deserialize, Default)]
    struct McpSection {
        #[serde(default)]
        servers: std::collections::HashMap<String, McpServerDecl>,
    }
    let Ok(content) = std::fs::read_to_string(config_file_path()) else {
        return Vec::new();
    };
    let root: Root = match serde_yaml::from_str(&content) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<_> = root.mcp.servers.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.retain(|(_, s)| !s.command.trim().is_empty());
    out
}

/// Strip one layer of YAML quoting (inverse of `yaml_escape`).
fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        unescape_double(&v[1..v.len() - 1])
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

/// Left-to-right unescape (chained `replace` calls corrupt `\\\"`).
fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(c2) => out.push(c2),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Quote a scalar for YAML output.
fn yaml_escape(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Merge top-level `key: value` pairs into the config file, preserving
/// comments and unknown keys. `None` removes the key. The file (and its
/// parent dir) is locked down to the owner, since it may hold an API key.
fn write_merged(updates: &[(&str, Option<String>)]) -> anyhow::Result<PathBuf> {
    let path = config_file_path();
    if let Some(parent) = path.parent() {
        // Lock down only directories we create ourselves — never touch
        // pre-existing ones (e.g. /tmp or $HOME) or their permissions.
        let mut missing = Vec::new();
        let mut cur = Some(parent);
        while let Some(p) = cur {
            if p.exists() || p.as_os_str().is_empty() {
                break;
            }
            missing.push(p.to_path_buf());
            cur = p.parent();
        }
        std::fs::create_dir_all(parent)?;
        for p in missing {
            lock_down(&p)?;
        }
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut done = vec![false; updates.len()];
    let mut out = String::new();
    for line in existing.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let t = line.trim();
        let mut replaced = false;
        if !indented && !t.starts_with('#') {
            for (i, (key, value)) in updates.iter().enumerate() {
                if !done[i] && t.starts_with(&format!("{key}:")) {
                    if let Some(v) = value {
                        out.push_str(&format!("{key}: {}\n", yaml_escape(v)));
                    }
                    // None drops the line (key removal).
                    done[i] = true;
                    replaced = true;
                    break;
                }
            }
        }
        if !replaced {
            out.push_str(line);
            out.push('\n');
        }
    }
    for (i, (key, value)) in updates.iter().enumerate() {
        if !done[i] {
            if let Some(v) = value {
                out.push_str(&format!("{key}: {}\n", yaml_escape(v)));
            }
        }
    }
    std::fs::write(&path, out)?;
    lock_down(&path)?;
    Ok(path)
}

/// Owner-only permissions for config paths (config may hold an API key).
/// Best effort on Windows, enforced on Unix.
fn lock_down(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 0o700 for dirs, 0o600 for files.
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

/// Persist provider URL + model to the config file, merging with whatever
/// is already there (comments and unknown keys are preserved).
pub fn save_provider(model: &str, base_url: &str) -> anyhow::Result<PathBuf> {
    write_merged(&[
        ("model", Some(model.to_string())),
        ("base_url", Some(base_url.to_string())),
    ])
}

/// Persist the API key to the local config file (owner-only permissions),
/// so `/connect` survives restarts. An empty key removes the stored one.
/// Environment variables still take precedence when set.
pub fn save_api_key(key: &str) -> anyhow::Result<PathBuf> {
    let value = if key.trim().is_empty() { None } else { Some(key.trim().to_string()) };
    write_merged(&[("api_key", value)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests share one process env; serialize `ANVIL_CONFIG` access.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_config(body: &str, f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "anvil-cfg-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.yaml");
        std::fs::write(&path, body).unwrap();
        std::env::set_var("ANVIL_CONFIG", &path);
        f();
        std::env::remove_var("ANVIL_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_values_are_read_and_nested_keys_ignored() {
        with_temp_config(
            "# comment\nmodel: \"file-model\"\nproviders:\n  x:\n    model: nested\n",
            || {
                let (m, u, k) = file_values();
                assert_eq!(m.as_deref(), Some("file-model"));
                assert_eq!(u, None);
                assert_eq!(k, None);
            },
        );
    }

    #[test]
    fn api_key_roundtrip_env_wins_and_removal() {
        with_temp_config("model: m\n", || {
            for v in ["ANVIL_API_KEY", "OPENCODE_ZEN_API_KEY", "OPENCODE_API_KEY"] {
                std::env::remove_var(v);
            }
            assert_eq!(api_key(), "");
            // tricky value with quotes and backslashes survives the roundtrip
            save_api_key("sk-test\\\"quo'te\\\\end").unwrap();
            assert_eq!(api_key(), "sk-test\\\"quo'te\\\\end");
            assert!(load().has_api_key);
            // env beats file
            std::env::set_var("ANVIL_API_KEY", "env-key");
            assert_eq!(api_key(), "env-key");
            std::env::remove_var("ANVIL_API_KEY");
            // empty input clears the stored key
            save_api_key("   ").unwrap();
            assert_eq!(api_key(), "");
            assert!(!load().has_api_key);
        });
    }

    #[test]
    #[cfg(unix)]
    fn config_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        // Point at a not-yet-existing subdir so save creates + locks it.
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "anvil-cfg-perm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let target = dir.join("sub").join("config.yaml");
        std::env::set_var("ANVIL_CONFIG", &target);
        let path = save_api_key("secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config file must be owner-only");
        let leaf = path.parent().unwrap();
        let dmode = std::fs::metadata(leaf).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700, "created config dir must be owner-only");
        std::env::remove_var("ANVIL_CONFIG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_servers_parsed_and_sorted() {
        with_temp_config(
            "model: m\nmcp:\n  servers:\n    b:\n      command: npx\n      args: [\"-y\", \"x\"]\n    a:\n      command: \"\"\n",
            || {
                let servers = mcp_servers();
                assert_eq!(servers.len(), 1);
                assert_eq!(servers[0].0, "b");
                assert_eq!(servers[0].1.args, vec!["-y", "x"]);
            },
        );
        with_temp_config("not: [valid", || {
            assert!(mcp_servers().is_empty());
        });
    }

    #[test]
    fn save_provider_merges_and_preserves_comments() {
        with_temp_config("# keep me\nmodel: \"old\"\n", || {
            let path = save_provider("new-model", "https://example.com/v1").unwrap();
            let content = std::fs::read_to_string(path).unwrap();
            assert!(content.contains("# keep me"));
            assert!(content.contains("model: \"new-model\""));
            assert!(content.contains("base_url: \"https://example.com/v1\""));
            assert!(!content.contains("\"old\""));
        });
    }
}
