use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .expect("no home dir")
        .join(".config/vscode-pmu/config.toml")
}

const DEFAULT_MAX_DEPTH: usize = 6;

const DEFAULT_TEMPLATE: &str = r#"# vscode-pmu configuration
# https://github.com/thoroc/vscode-project-manager-updater
#
# max_depth = 6
# How many directory levels below <root> to scan for git repositories.
# Increase for deeper monorepo layouts; decrease for faster scans on
# shallow trees. Default: 6.
#
# [tag_rename]
# Rename derived tags before they are written to projects.json.
# Use to replace technical path segments (e.g. a GitHub username) with
# human-friendly labels.
#
# Example: tag every project under github/<username>/… as "personal"
# instead of the raw username.
#
# [tag_rename]
# thoroc = "personal"
#
# [host_skip]
# Set a minimum number of path segments to skip per VCS host when deriving
# project tags. The first segment (the host directory itself) is always
# skipped automatically; this setting raises that floor further.
#
# The effective skip for any path is:
#   max(trie_derived_skip, host_skip value)
#
# Use this when an organisational namespace sits between the host and your
# team directories and you do not want it to appear as a tag.
#
# Example: for a layout like
#   ~/Projects/gitlab/company/platform/team/project
# set gitlab = 3 to skip company and platform, keeping only team as the tag.
#
# [host_skip]
# gitlab = 1
# github = 1
"#;

/// Per-host minimum skip depths.  The effective skip for a path is:
///   `max(trie_derived_skip, host_skip.get(host).copied().unwrap_or(0))`
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Config {
    /// How many directory levels below `<root>` to scan.  `None` → use `DEFAULT_MAX_DEPTH`.
    #[serde(default)]
    pub max_depth: Option<usize>,
    /// Renames applied to derived tags before they are written to `projects.json`.
    #[serde(default)]
    pub tag_rename: HashMap<String, String>,
    #[serde(default)]
    pub host_skip: HashMap<String, usize>,
}

impl Config {
    /// Returns the configured scan depth, falling back to [`DEFAULT_MAX_DEPTH`].
    pub fn scan_max_depth(&self) -> usize {
        self.max_depth.unwrap_or(DEFAULT_MAX_DEPTH)
    }

    /// Applies `tag_rename` substitutions to a list of derived tags.
    pub fn rename_tags(&self, tags: Vec<String>) -> Vec<String> {
        tags.into_iter()
            .map(|t| self.tag_rename.get(&t).cloned().unwrap_or(t))
            .collect()
    }

    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            // Create the file with the commented template so users can
            // discover the available options without reading the docs.
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            fs::write(&path, DEFAULT_TEMPLATE)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!(
                "[{}] Created default config at {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                path.display()
            );
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("invalid TOML in {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self).context("TOML serialisation failed")?;
        fs::write(&path, content)
            .with_context(|| format!("cannot write {}", path.display()))
    }

    /// Returns the configured minimum skip depth for `host`, or 0 if not set.
    pub fn floor_for(&self, host: &str) -> usize {
        self.host_skip.get(host).copied().unwrap_or(0)
    }
}

pub fn run_set_skip(host: &str, depth: usize) -> Result<()> {
    let mut cfg = Config::load()?;
    cfg.host_skip.insert(host.to_string(), depth);
    cfg.save()?;
    eprintln!(
        "[{}] Set skip depth for '{}' to {}.",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        host,
        depth
    );
    Ok(())
}

pub fn run_show() -> Result<()> {
    let path = config_path();
    let cfg = Config::load()?;
    println!("Config file: {}", path.display());
    println!("max_depth = {}", cfg.scan_max_depth());
    if cfg.tag_rename.is_empty() {
        println!("tag_rename: (none)");
    } else {
        println!("tag_rename:");
        let mut entries: Vec<_> = cfg.tag_rename.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());
        for (from, to) in entries {
            println!("  {} = \"{}\"", from, to);
        }
    }
    if cfg.host_skip.is_empty() {
        println!("host_skip: (none — using trie-derived depths only)");
    } else {
        println!("host_skip:");
        let mut entries: Vec<_> = cfg.host_skip.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());
        for (host, depth) in entries {
            println!("  {} = {}", host, depth);
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_max_depth_returns_default_when_unset() {
        let cfg = Config::default();
        assert_eq!(cfg.scan_max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn scan_max_depth_returns_configured_value() {
        let cfg = Config { max_depth: Some(3), ..Default::default() };
        assert_eq!(cfg.scan_max_depth(), 3);
    }

    #[test]
    fn default_template_yields_default_max_depth() {
        let cfg: Config = toml::from_str(DEFAULT_TEMPLATE).expect("template is not valid TOML");
        assert_eq!(cfg.scan_max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn max_depth_round_trips_toml() {
        let cfg = Config { max_depth: Some(4), ..Default::default() };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let loaded: Config = toml::from_str(&s).unwrap();
        assert_eq!(loaded.scan_max_depth(), 4);
    }

    #[test]
    fn rename_tags_passes_through_when_map_empty() {
        let cfg = Config::default();
        let tags = vec!["thoroc".to_string(), "github".to_string()];
        assert_eq!(cfg.rename_tags(tags.clone()), tags);
    }

    #[test]
    fn rename_tags_substitutes_matching_entry() {
        let mut cfg = Config::default();
        cfg.tag_rename.insert("thoroc".to_string(), "personal".to_string());
        let result = cfg.rename_tags(vec!["thoroc".to_string()]);
        assert_eq!(result, vec!["personal".to_string()]);
    }

    #[test]
    fn rename_tags_leaves_unmatched_tags_unchanged() {
        let mut cfg = Config::default();
        cfg.tag_rename.insert("thoroc".to_string(), "personal".to_string());
        let result = cfg.rename_tags(vec!["thoroc".to_string(), "acme".to_string()]);
        assert_eq!(result, vec!["personal".to_string(), "acme".to_string()]);
    }

    #[test]
    fn tag_rename_round_trips_toml() {
        let mut cfg = Config::default();
        cfg.tag_rename.insert("thoroc".to_string(), "personal".to_string());
        let s = toml::to_string_pretty(&cfg).unwrap();
        let loaded: Config = toml::from_str(&s).unwrap();
        assert_eq!(loaded.rename_tags(vec!["thoroc".to_string()]), vec!["personal".to_string()]);
    }

    #[test]
    fn default_template_yields_empty_tag_rename() {
        let cfg: Config = toml::from_str(DEFAULT_TEMPLATE).expect("template is not valid TOML");
        assert!(cfg.tag_rename.is_empty());
    }

    #[test]
    fn floor_for_returns_zero_when_host_absent() {
        let cfg = Config::default();
        assert_eq!(cfg.floor_for("gitlab"), 0);
    }

    #[test]
    fn floor_for_returns_configured_depth() {
        let mut cfg = Config::default();
        cfg.host_skip.insert("gitlab".to_string(), 3);
        assert_eq!(cfg.floor_for("gitlab"), 3);
    }

    #[test]
    fn default_template_is_valid_toml() {
        // The template must parse without errors (all active lines are comments).
        let cfg: Config = toml::from_str(DEFAULT_TEMPLATE).expect("template is not valid TOML");
        assert!(cfg.host_skip.is_empty(), "template should produce empty config");
    }

    #[test]
    fn round_trips_toml() {
        let mut cfg = Config::default();
        cfg.host_skip.insert("gitlab".to_string(), 3);
        cfg.host_skip.insert("github".to_string(), 1);
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let loaded: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.floor_for("gitlab"), 3);
        assert_eq!(loaded.floor_for("github"), 1);
    }
}
