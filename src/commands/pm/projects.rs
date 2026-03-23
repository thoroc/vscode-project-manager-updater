use anyhow::{Context, Result};
use dialoguer::{Input, Select};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::cache;

pub fn projects_json_path() -> PathBuf {
    dirs::home_dir()
        .expect("no home dir")
        .join("Library/Application Support/Code/User/globalStorage/alefragnani.project-manager/projects.json")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub name: String,
    #[serde(rename = "rootPath")]
    pub root_path: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub profile: String,
}

fn default_true() -> bool {
    true
}

impl Project {
    pub fn new(root_path: PathBuf, watch_root: &Path) -> Self {
        let name = root_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let tag = compute_tag(&root_path, watch_root);
        Project {
            name,
            root_path: root_path.to_string_lossy().to_string(),
            paths: vec![],
            tags: if tag.is_empty() { vec![] } else { vec![tag] },
            enabled: true,
            profile: String::new(),
        }
    }
}

pub fn compute_tag(root_path: &Path, watch_root: &Path) -> String {
    if let Ok(rel) = root_path.strip_prefix(watch_root) {
        let mut comps = rel.components();
        let first = comps.next();
        let second = comps.next();
        if second.is_some() {
            // project is nested — first segment is the tag
            if let Some(c) = first {
                return c.as_os_str().to_string_lossy().to_string();
            }
        }
        // direct child of watch_root
        return String::new();
    }
    String::new()
}

pub fn read_projects(path: &Path) -> Result<Vec<Project>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("invalid JSON in {}", path.display()))
}

pub fn write_projects(path: &Path, projects: &[Project]) -> Result<()> {
    // Backup first
    if path.exists() {
        let bak = path.with_extension("json.bak");
        fs::copy(path, &bak)
            .with_context(|| format!("cannot back up {}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(projects).context("JSON serialisation failed")?;
    fs::write(path, json)
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

pub fn add(path: Option<&Path>, root: &Path) -> Result<()> {
    let target: PathBuf = match path {
        Some(p) => p.to_path_buf(),
        None => {
            let input: String = Input::new()
                .with_prompt("Path to git repository")
                .interact_text()?;
            PathBuf::from(input.trim())
        }
    };

    let resolved = fs::canonicalize(&target)
        .with_context(|| format!("cannot resolve {}", target.display()))?;
    if !resolved.join(".git").exists() {
        anyhow::bail!("{} does not contain a .git directory", resolved.display());
    }

    let projects_path = projects_json_path();
    let mut projects = read_projects(&projects_path)?;

    let path_str = resolved.to_string_lossy().to_string();
    if projects.iter().any(|p| p.root_path == path_str) {
        eprintln!(
            "[{}] Project already present: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            path_str
        );
        return Ok(());
    }

    let project = Project::new(resolved.clone(), root);
    eprintln!(
        "[{}] Adding project: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        path_str
    );
    projects.push(project);
    write_projects(&projects_path, &projects)?;
    cache::update_paths(root, &projects)?;
    Ok(())
}

pub fn remove(name: Option<&str>, root: &Path) -> Result<()> {
    let projects_path = projects_json_path();
    let mut projects = read_projects(&projects_path)?;

    let managed: Vec<String> = projects
        .iter()
        .filter(|p| Path::new(&p.root_path).starts_with(root))
        .map(|p| p.name.clone())
        .collect();

    if managed.is_empty() {
        eprintln!(
            "[{}] No managed projects found under {}.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            root.display()
        );
        return Ok(());
    }

    let target = match name {
        Some(n) => n.to_string(),
        None => {
            let idx = Select::new()
                .with_prompt("Select a project to remove")
                .items(&managed)
                .interact()?;
            managed[idx].clone()
        }
    };

    let before = projects.len();
    projects.retain(|p| {
        !(Path::new(&p.root_path).starts_with(root) && p.name == target)
    });

    if projects.len() == before {
        eprintln!(
            "[{}] Project not found: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            target
        );
        return Ok(());
    }

    eprintln!(
        "[{}] Removed project: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        target
    );
    write_projects(&projects_path, &projects)?;
    cache::update_paths(root, &projects)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── compute_tag ───────────────────────────────────────────────────────────

    #[test]
    fn tag_is_empty_for_direct_child() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/Projects/my-repo");
        assert_eq!(compute_tag(project, root), "");
    }

    #[test]
    fn tag_is_first_segment_for_nested_child() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/Projects/github/my-repo");
        assert_eq!(compute_tag(project, root), "github");
    }

    #[test]
    fn tag_is_first_segment_for_deeply_nested() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/Projects/gitlab/org/group/my-repo");
        assert_eq!(compute_tag(project, root), "gitlab");
    }

    #[test]
    fn tag_is_empty_for_path_outside_watch_root() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/other/my-repo");
        assert_eq!(compute_tag(project, root), "");
    }

    // ── Project::new ──────────────────────────────────────────────────────────

    #[test]
    fn project_new_sets_name_from_last_segment() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("my-project");
        fs::create_dir_all(&repo).unwrap();
        let p = Project::new(repo, tmp.path());
        assert_eq!(p.name, "my-project");
    }

    #[test]
    fn project_new_enabled_by_default() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let p = Project::new(repo, tmp.path());
        assert!(p.enabled);
    }

    #[test]
    fn project_new_empty_paths_and_profile() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let p = Project::new(repo, tmp.path());
        assert!(p.paths.is_empty());
        assert_eq!(p.profile, "");
    }

    // ── read_projects ─────────────────────────────────────────────────────────

    #[test]
    fn read_projects_returns_empty_vec_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        let projects = read_projects(&path).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn read_projects_parses_valid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        let json = r#"[{"name":"foo","rootPath":"/projects/foo","paths":[],"tags":["github"],"enabled":true,"profile":""}]"#;
        fs::write(&path, json).unwrap();
        let projects = read_projects(&path).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "foo");
        assert_eq!(projects[0].root_path, "/projects/foo");
        assert_eq!(projects[0].tags, vec!["github"]);
    }

    #[test]
    fn read_projects_parses_empty_array() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        fs::write(&path, "[]").unwrap();
        let projects = read_projects(&path).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn read_projects_errors_on_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        fs::write(&path, "not json").unwrap();
        assert!(read_projects(&path).is_err());
    }

    // ── write_projects ────────────────────────────────────────────────────────

    #[test]
    fn write_projects_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        let projects = vec![Project {
            name: "bar".to_string(),
            root_path: "/projects/bar".to_string(),
            paths: vec![],
            tags: vec!["gitlab".to_string()],
            enabled: true,
            profile: String::new(),
        }];
        write_projects(&path, &projects).unwrap();
        let loaded = read_projects(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "bar");
        assert_eq!(loaded[0].tags, vec!["gitlab"]);
    }

    #[test]
    fn write_projects_creates_backup_when_file_exists() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        fs::write(&path, "[]").unwrap();
        write_projects(&path, &[]).unwrap();
        let bak = tmp.path().join("projects.json.bak");
        assert!(bak.exists(), ".bak file should be created");
    }

    #[test]
    fn write_projects_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("deep/nested/projects.json");
        write_projects(&path, &[]).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_projects_produces_valid_json_array() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("projects.json");
        write_projects(&path, &[]).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_array());
    }

    // ── serde defaults ────────────────────────────────────────────────────────

    #[test]
    fn deserialise_project_with_minimal_fields() {
        let json = r#"{"name":"x","rootPath":"/x"}"#;
        let p: Project = serde_json::from_str(json).unwrap();
        assert_eq!(p.name, "x");
        assert!(p.enabled);
        assert!(p.paths.is_empty());
        assert!(p.tags.is_empty());
        assert_eq!(p.profile, "");
    }
}
