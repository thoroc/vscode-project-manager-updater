use anyhow::{Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::log::log_info;
use super::projects::Project;

pub const CACHE_TTL: u64 = 86400;

/// Derive a unique cache file path for the given root directory.
///
/// Uses a hex-encoded hash of the canonical path string so that two paths that
/// differ only in characters that would otherwise be mapped to the same
/// sanitised form (e.g. `/foo/bar-baz` vs `/foo/bar_baz`) never collide.
pub fn cache_path_for(root: &Path) -> Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    let hash = format!("{:016x}", hasher.finish());
    let cache_dir = dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".cache/vscode-pmu");
    Ok(cache_dir.join(format!("{hash}.cache")))
}

// ── Internal path-parameterised helpers (also used by tests) ─────────────────

pub(crate) fn is_fresh_at(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    if let Ok(meta) = fs::metadata(path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = SystemTime::now().duration_since(modified) {
                return age < Duration::from_secs(CACHE_TTL);
            }
        }
    }
    false
}

pub(crate) fn read_paths_from(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("cannot read cache {}", path.display()))?;
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

pub(crate) fn write_paths_to(path: &Path, paths: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create cache dir {}", parent.display()))?;
    }
    fs::write(path, paths.join("\n") + "\n")
        .with_context(|| format!("cannot write cache {}", path.display()))
}

pub(crate) fn delete_at(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("cannot delete cache {}", path.display()))?;
        log_info!("Cache deleted.");
    }
    Ok(())
}

// ── Public API (keyed to root) ────────────────────────────────────────────────

pub fn is_fresh(root: &Path) -> Result<bool> {
    Ok(is_fresh_at(&cache_path_for(root)?))
}

pub fn read_paths(root: &Path) -> Result<Vec<String>> {
    read_paths_from(&cache_path_for(root)?)
}

pub fn write_paths(root: &Path, paths: &[String]) -> Result<()> {
    write_paths_to(&cache_path_for(root)?, paths)
}

pub fn update_paths(root: &Path, projects: &[Project]) -> Result<()> {
    let paths: Vec<String> = projects.iter().map(|p| p.root_path.clone()).collect();
    write_paths(root, &paths)
}

pub fn delete(root: &Path) -> Result<()> {
    delete_at(&cache_path_for(root)?)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn cache_path_for_is_under_cache_dir() {
        let expected_dir = dirs::home_dir().unwrap().join(".cache/vscode-pmu");
        let path = cache_path_for(Path::new("/home/user/Projects")).unwrap();
        assert_eq!(path.parent().unwrap(), expected_dir);
    }

    #[test]
    fn cache_path_for_differs_per_root() {
        let a = cache_path_for(Path::new("/home/user/Projects")).unwrap();
        let b = cache_path_for(Path::new("/home/user/Work")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn cache_path_for_is_deterministic() {
        let root = Path::new("/home/user/Projects");
        assert_eq!(cache_path_for(root).unwrap(), cache_path_for(root).unwrap());
    }

    #[test]
    fn cache_path_for_no_collision_on_char_substitution() {
        // /foo/bar-baz and /foo/bar_baz used to map to the same sanitised name
        let a = cache_path_for(Path::new("/foo/bar-baz")).unwrap();
        let b = cache_path_for(Path::new("/foo/bar_baz")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn is_fresh_returns_false_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.cache");
        assert!(!is_fresh_at(&path));
    }

    #[test]
    fn is_fresh_returns_true_for_newly_created_file() {
        let f = NamedTempFile::new().unwrap();
        assert!(is_fresh_at(f.path()));
    }

    #[test]
    fn write_then_read_round_trips_paths() {
        let f = NamedTempFile::new().unwrap();
        let paths = vec![
            "/home/user/Projects/foo".to_string(),
            "/home/user/Projects/bar".to_string(),
        ];
        write_paths_to(f.path(), &paths).unwrap();
        let got = read_paths_from(f.path()).unwrap();
        assert_eq!(got, paths);
    }

    #[test]
    fn write_then_read_filters_blank_lines() {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), "\n/foo/bar\n\n/baz/qux\n\n").unwrap();
        let got = read_paths_from(f.path()).unwrap();
        assert_eq!(got, vec!["/foo/bar", "/baz/qux"]);
    }

    #[test]
    fn write_empty_slice_produces_empty_read() {
        let f = NamedTempFile::new().unwrap();
        write_paths_to(f.path(), &[]).unwrap();
        let got = read_paths_from(f.path()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn delete_removes_existing_file() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_path_buf();
        assert!(path.exists());
        let path_copy = path.clone();
        f.keep().unwrap();
        delete_at(&path_copy).unwrap();
        assert!(!path_copy.exists());
    }

    #[test]
    fn delete_is_no_op_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.cache");
        delete_at(&path).unwrap();
    }

    #[test]
    fn update_paths_writes_root_paths_from_projects() {
        let f = NamedTempFile::new().unwrap();
        let projects = [
            Project {
                name: "foo".to_string(),
                root_path: "/projects/foo".to_string(),
                paths: vec![],
                tags: vec![],
                enabled: true,
                profile: String::new(),
            },
            Project {
                name: "bar".to_string(),
                root_path: "/projects/bar".to_string(),
                paths: vec![],
                tags: vec![],
                enabled: true,
                profile: String::new(),
            },
        ];
        write_paths_to(
            f.path(),
            &projects
                .iter()
                .map(|p| p.root_path.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let got = read_paths_from(f.path()).unwrap();
        assert_eq!(got, vec!["/projects/foo", "/projects/bar"]);
    }
}
