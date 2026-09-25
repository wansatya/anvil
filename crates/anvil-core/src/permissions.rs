//! Permission evaluation (SPEC §12, Phase 9).
//! Every tool maps to a category; the policy decides allow/ask/deny.
//! `ask` pauses the loop for an approval decision; `Always` grants stick
//! for the session.

use std::collections::HashSet;

use anvil_tools::ToolCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Allow,
    Ask,
    Deny,
}

impl Policy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "allow" => Some(Policy::Allow),
            "ask" => Some(Policy::Ask),
            "deny" => Some(Policy::Deny),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PermissionSet {
    pub read: Policy,
    pub search: Policy,
    pub git_read: Policy,
    pub write: Policy,
    pub shell: Policy,
    pub destructive: Policy,
    pub network: Policy,
    /// Session-scoped `Always` grants by tool name.
    pub always_allow: HashSet<String>,
}

impl PermissionSet {
    /// SPEC §12 defaults.
    pub fn defaults() -> Self {
        Self {
            read: Policy::Allow,
            search: Policy::Allow,
            git_read: Policy::Allow,
            write: Policy::Ask,
            shell: Policy::Ask,
            destructive: Policy::Ask,
            network: Policy::Ask,
            always_allow: HashSet::new(),
        }
    }

    /// Policy for a concrete tool invocation. Finer-grained tools
    /// (search, git_*) map to their own knobs, not blanket `read`.
    pub fn for_tool(&self, tool_name: &str, category: ToolCategory) -> Policy {
        let short = tool_name.rsplit('.').next().unwrap_or(tool_name);
        let base = match category {
            ToolCategory::ReadOnly => {
                if short == "search" {
                    self.search
                } else if short.starts_with("git_") {
                    self.git_read
                } else {
                    self.read
                }
            }
            ToolCategory::SafeWrite => self.write,
            ToolCategory::Shell => self.shell,
            ToolCategory::Destructive => self.destructive,
            ToolCategory::Network => self.network,
        };
        if base == Policy::Ask
            && (self.always_allow.contains(tool_name) || self.always_allow.contains(short))
        {
            Policy::Allow
        } else {
            base
        }
    }

    /// Record a session-scoped `Always` grant.
    pub fn grant_always(&mut self, tool_name: &str) {
        self.always_allow.insert(tool_name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_mapping_and_always_grant() {
        let mut p = PermissionSet::defaults();
        assert_eq!(p.for_tool("anvil.read_file", ToolCategory::ReadOnly), Policy::Allow);
        assert_eq!(p.for_tool("anvil.shell", ToolCategory::Shell), Policy::Ask);
        assert_eq!(p.for_tool("anvil.git_status", ToolCategory::ReadOnly), Policy::Allow);
        p.grant_always("anvil.shell");
        assert_eq!(p.for_tool("anvil.shell", ToolCategory::Shell), Policy::Allow);
        // grant does not override Deny
        p.shell = Policy::Deny;
        assert_eq!(p.for_tool("anvil.shell", ToolCategory::Shell), Policy::Deny);
    }

    #[test]
    fn policy_parses() {
        assert_eq!(Policy::from_str("Allow"), Some(Policy::Allow));
        assert_eq!(Policy::from_str("DENY"), Some(Policy::Deny));
        assert_eq!(Policy::from_str("x"), None);
    }
}
