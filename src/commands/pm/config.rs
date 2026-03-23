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

/// Per-host minimum skip depths.  The effective skip for a path is:
///   `max(trie_derived_skip, host_skip.get(host).copied().unwrap_or(0))`
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Config {
    #[serde(default)]
    pub host_skip: HashMap<String, usize>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
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
    if cfg.host_skip.is_empty() {
        println!("  (no host_skip entries — using trie-derived depths only)");
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
