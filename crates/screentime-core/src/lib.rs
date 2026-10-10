//! Shared data model and categorisation rules for screentime.

use std::collections::BTreeMap;

use serde::Deserialize;

/// SQLite schema for the usage store.
pub const SCHEMA: &str = include_str!("schema.sql");

/// Fallback category when no rule matches.
pub const OTHER: &str = "Other";

const BUILTIN_RULES: &str = include_str!("../../../data/rules.toml");

/// App and domain to category mapping, loaded from `rules.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
    #[serde(default)]
    pub domains: BTreeMap<String, String>,
    #[serde(default)]
    pub globs: BTreeMap<String, String>,
}

impl Rules {
    pub fn parse(src: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(src)
    }

    /// The default rules shipped in `data/rules.toml`.
    pub fn builtin() -> Self {
        Self::parse(BUILTIN_RULES).expect("built-in rules.toml is valid")
    }

    /// Category for a desktop app id: exact match first, then globs.
    pub fn categorize_app(&self, app_id: &str) -> &str {
        if let Some(c) = self.apps.get(app_id) {
            return c;
        }
        let bare = app_id.strip_suffix(".desktop").unwrap_or(app_id);
        self.globs
            .iter()
            .find(|(pat, _)| glob_match(pat, bare) || glob_match(pat, app_id))
            .map_or(OTHER, |(_, c)| c.as_str())
    }

    /// Category for a domain, matching the rule itself or any parent domain
    /// (`www.youtube.com` and `m.youtube.com` both match `youtube.com`).
    pub fn categorize_domain(&self, domain: &str) -> &str {
        let mut d = domain.trim_end_matches('.').to_ascii_lowercase();
        loop {
            if let Some(c) = self.domains.get(&d) {
                return c;
            }
            match d.split_once('.') {
                Some((_, rest)) if rest.contains('.') => d = rest.to_string(),
                _ => return OTHER,
            }
        }
    }
}

/// Minimal case-insensitive glob supporting `*` only.
fn glob_match(pat: &str, s: &str) -> bool {
    let (pat, s) = (pat.to_ascii_lowercase(), s.to_ascii_lowercase());
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !s.starts_with(first) || !s[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &s[first.len()..s.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_parse() {
        let r = Rules::builtin();
        assert!(!r.apps.is_empty() && !r.domains.is_empty());
    }

    #[test]
    fn apps() {
        let r = Rules::builtin();
        assert_eq!(r.categorize_app("code.desktop"), "Developer Tools");
        assert_eq!(r.categorize_app("jetbrains-idea.desktop"), "Developer Tools");
        assert_eq!(r.categorize_app("unknown.desktop"), OTHER);
    }

    #[test]
    fn domains_match_parents() {
        let r = Rules::builtin();
        assert_eq!(r.categorize_domain("www.youtube.com"), "Entertainment");
        assert_eq!(r.categorize_domain("en.wikipedia.org"), "Information & Reading");
        assert_eq!(r.categorize_domain("GitHub.com"), "Developer Tools");
        assert_eq!(r.categorize_domain("example.com"), OTHER);
    }

    #[test]
    fn globs() {
        assert!(glob_match("jetbrains-*", "jetbrains-idea"));
        assert!(glob_match("*terminal*", "org.gnome.Terminal"));
        assert!(!glob_match("a*b*c", "acb"));
    }
}
