//! Workspace discovery (SPEC §13): root from `.git`, else current dir.

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: std::path::PathBuf,
}

impl Workspace {
    /// Walk up from `start` to the nearest `.git` dir (or an `ANVIL.md`
    /// marker); fall back to `start` itself.
    pub fn discover(start: &std::path::Path) -> Self {
        let mut cur = if start.is_file() {
            start.parent().map(|p| p.to_path_buf())
        } else {
            Some(start.to_path_buf())
        };
        while let Some(dir) = cur {
            if dir.join(".git").exists() || dir.join("ANVIL.md").exists() {
                return Self { root: dir };
            }
            cur = dir.parent().map(|p| p.to_path_buf());
        }
        Self { root: start.to_path_buf() }
    }

    /// Workspace-relative display path (falls back to absolute).
    pub fn rel(&self, path: &std::path::Path) -> String {
        path.strip_prefix(&self.root)
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_falls_back_to_start() {
        let dir = std::env::temp_dir().join(format!("anvil-ws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ws = Workspace::discover(&dir);
        assert_eq!(ws.root, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_finds_git_root() {
        let base = std::env::temp_dir().join(format!("anvil-ws2-{}", std::process::id()));
        let sub = base.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        let ws = Workspace::discover(&sub);
        assert_eq!(ws.root, base);
        let _ = std::fs::remove_dir_all(&base);
    }
}
