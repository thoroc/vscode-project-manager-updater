use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::projects::Project;

pub const CACHE_TTL: u64 = 86400;

/// Derive a unique cache file path for the given root directory so that
/// different roots never share the same cache.
pub fn cache_path_for(root: &Path) -> PathBuf {
    let sanitized = root
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect::<String>();
    let cache_dir = dirs::home_dir()
        .expect("no home dir")
        .join(".cache/vscode-pmu");
    cache_dir.join(format!("{sanitized}.cache"))
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
        fs::remove_file(path)
            .with_context(|| format!("cannot delete cache {}", path.display()))?;
        eprintln!(
            "[{}] Cache deleted.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }
    Ok(())
}

// ── Public API (keyed to root) ────────────────────────────────────────────────

pub fn is_fresh(root: &Path) -> bool {
    is_fresh_at(&cache_path_for(root))
}

pub fn read_paths(root: &Path) -> Result<Vec<String>> {
    read_paths_from(&cache_path_for(root))
}

pub fn write_paths(root: &Path, paths: &[String]) -> Result<()> {
    write_paths_to(&cache_path_for(root), paths)
}

pub fn update_paths(root: &Path, projects: &[Project]) -> Result<()> {
    let paths: Vec<String> = projects.iter().map(|p| p.root_path.clone()).collect();
    write_paths(root, &paths)
}

pub fn delete(root: &Path) -> Result<()> {
    delete_at(&cache_path_for(root))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn cache_path_for_is_under_cache_dir() {
        let expected_dir = dirs::home_dir().unwrap().join(".cache/vscode-pmu");
        let path = cache_path_for(Path::new("/home/user/Projects"));
        assert_eq!(path.parent().unwrap(), expected_dir);
    }

    #[test]
    fn cache_path_for_differs_per_root() {
        let a = cache_path_for(Path::new("/home/user/Projects"));
        let b = cache_path_for(Path::new("/home/user/Work"));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_path_for_is_deterministic() {
        let root = Path::new("/home/user/Projects");
        assert_eq!(cache_path_for(root), cache_path_for(root));
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
        let projects = vec![
            Project { name: "foo".to_string(), root_path: "/projects/foo".to_string(), paths: vec![], tags: vec![], enabled: true, profile: String::new() },
            Project { name: "bar".to_string(), root_path: "/projects/bar".to_string(), paths: vec![], tags: vec![], enabled: true, profile: String::new() },
        ];
        write_paths_to(f.path(), &projects.iter().map(|p| p.root_path.clone()).collect::<Vec<_>>()).unwrap();
        let got = read_paths_from(f.path()).unwrap();
        assert_eq!(got, vec!["/projects/foo", "/projects/bar"]);
    }
}
