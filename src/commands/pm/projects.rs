use anyhow::{Context, Result};
use dialoguer::{Input, Select};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::cache;
use super::config::Config;

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
    pub fn new(root_path: PathBuf, tags: Vec<String>) -> Self {
        let name = root_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Project {
            name,
            root_path: root_path.to_string_lossy().to_string(),
            paths: vec![],
            tags,
            enabled: true,
            profile: String::new(),
        }
    }

    /// Recomputes tags in-place using the provided skip depth.
    pub fn retag(&mut self, watch_root: &Path, skip: usize) {
        self.tags = compute_tags(Path::new(&self.root_path), watch_root, skip);
    }
}

/// Returns meaningful tags for `root_path` under `watch_root`.
///
/// `skip` is the number of leading path components (relative to `watch_root`)
/// treated as organisational noise. The intermediate components between the
/// skipped prefix and the project directory name become the tags.
///
/// Falls back to the VCS-host name when the prefix consumes all intermediate
/// components; returns an empty vec for direct children of `watch_root`.
pub fn compute_tags(root_path: &Path, watch_root: &Path, skip: usize) -> Vec<String> {
    let Ok(rel) = root_path.strip_prefix(watch_root) else {
        return vec![];
    };
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if comps.is_empty() {
        return vec![];
    }
    let start = skip.min(comps.len().saturating_sub(1));
    let end = comps.len().saturating_sub(1); // exclude project name
    if start < end {
        comps[start..end].to_vec()
    } else if comps.len() >= 2 {
        // No intermediate segments after skip — fall back to the host name.
        vec![comps[0].clone()]
    } else {
        // Direct child of watch_root — no meaningful tag.
        vec![]
    }
}

/// For each path in `paths`, returns the skip depth — the number of leading
/// components (relative to `watch_root`) to treat as organisational noise.
///
/// The algorithm builds a prefix trie across all paths and then walks each
/// path's own branch: a segment is "noise" if its subtree fans out to exactly
/// one distinct next-level segment (i.e. it is a pass-through node).
/// The VCS host (first component after `watch_root`) is always skipped.
///
/// This allows deeply nested organisational hierarchies to be stripped
/// automatically without any hardcoded path values.
pub fn path_skip_map(
    paths: &[String],
    watch_root: &Path,
) -> std::collections::HashMap<String, usize> {
    use std::collections::{HashMap, HashSet};

    // Build trie: prefix → set of distinct values at the very next level.
    // The final segment of each path (the project directory) is never added as
    // a trie key so it cannot accidentally be treated as a pass-through node.
    let mut trie: HashMap<Vec<String>, HashSet<String>> = HashMap::new();

    let segs_list: Vec<(String, Vec<String>)> = paths
        .iter()
        .filter_map(|p| {
            Path::new(p)
                .strip_prefix(watch_root)
                .ok()
                .map(|rel| {
                    let segs: Vec<String> = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    (p.clone(), segs)
                })
        })
        .collect();

    for (_, segs) in &segs_list {
        for i in 0..segs.len().saturating_sub(1) {
            trie.entry(segs[..i].to_vec())
                .or_default()
                .insert(segs[i].clone());
        }
    }

    segs_list
        .into_iter()
        .map(|(path, segs)| (path, trie_skip_depth(&segs, &trie)))
        .collect()
}

/// Walks `segs` through `trie`, following the path's own branch.
/// Skips a segment when its subtree has exactly one distinct child
/// (pass-through node). The host (index 0) is always skipped.
fn trie_skip_depth(
    segs: &[String],
    trie: &std::collections::HashMap<
        Vec<String>,
        std::collections::HashSet<String>,
    >,
) -> usize {
    if segs.len() < 2 {
        return 0; // direct child of watch_root — no host to skip
    }
    let mut skip = 1; // always skip the VCS host
    for i in 1..segs.len().saturating_sub(1) {
        // segs[..=i] is the prefix that ends at segs[i] (inclusive).
        // trie[segs[..=i]] gives the children OF segs[i] within this subtree.
        match trie.get(&segs[..=i]) {
            Some(children) if children.len() == 1 => {
                // Only one path forward from segs[i] → it is a pass-through,
                // not a meaningful grouping label.
                skip = i + 1;
            }
            _ => break, // multiple children or leaf → meaningful level, stop
        }
    }
    skip
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

    // Derive skip depth using the trie-based algorithm over all managed paths
    // plus the new one.
    let new_path_str = resolved.to_string_lossy().into_owned();
    let mut candidate_paths: Vec<String> = projects
        .iter()
        .filter(|p| Path::new(&p.root_path).starts_with(root))
        .map(|p| p.root_path.clone())
        .collect();
    candidate_paths.push(new_path_str.clone());
    let skip_map = path_skip_map(&candidate_paths, root);
    let trie_skip = skip_map.get(&new_path_str).copied().unwrap_or(1);
    let host = resolved
        .strip_prefix(root)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default();
    let cfg = Config::load()?;
    let skip = trie_skip.max(cfg.floor_for(&host));
    let tags = compute_tags(&resolved, root, skip);
    let project = Project::new(resolved.clone(), tags);
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

    // ── compute_tags ──────────────────────────────────────────────────────────

    #[test]
    fn tags_empty_for_direct_child() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/Projects/my-repo");
        assert_eq!(compute_tags(project, root, 1), Vec::<String>::new());
    }

    #[test]
    fn tags_fallback_to_host_for_shallow_nested() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/Projects/github/my-repo");
        assert_eq!(compute_tags(project, root, 1), vec!["github"]);
    }

    #[test]
    fn tags_empty_for_path_outside_watch_root() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/other/my-repo");
        assert_eq!(compute_tags(project, root, 1), Vec::<String>::new());
    }

    #[test]
    fn tags_intermediate_segments_after_skip() {
        let root = std::path::Path::new("/home/user/Projects");
        let project =
            std::path::Path::new("/home/user/Projects/gitlab/org/ns/group/my-repo");
        // skip=3 means gitlab/org/ns are noise → tag is group
        assert_eq!(compute_tags(project, root, 3), vec!["group"]);
    }

    #[test]
    fn tags_multiple_segments_when_deep_enough() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new(
            "/home/user/Projects/gitlab/org/ns/cloudengineering/containerimages/dkr-wildfly",
        );
        assert_eq!(
            compute_tags(project, root, 3),
            vec!["cloudengineering", "containerimages"]
        );
    }

    // ── path_skip_map / trie_skip_depth ──────────────────────────────────────

    #[test]
    fn skip_depth_1_for_single_project_under_host() {
        let root = std::path::Path::new("/Projects");
        let paths = vec!["/Projects/github/my-repo".to_string()];
        let map = path_skip_map(&paths, root);
        assert_eq!(map["/Projects/github/my-repo"], 1);
    }

    #[test]
    fn skip_depth_strips_passthrough_org_segments() {
        // org leads to only one child (ns) → org is a pass-through, skip=2.
        // ns leads to two children (group-a, group-b) → ns is meaningful, stop.
        // Result: tags start at ns (skip=2), so tags include "ns" and the group.
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/gitlab/org/ns/group-a/repo-a".to_string(),
            "/Projects/gitlab/org/ns/group-b/repo-b".to_string(),
        ];
        let map = path_skip_map(&paths, root);
        assert_eq!(map["/Projects/gitlab/org/ns/group-a/repo-a"], 2);
        assert_eq!(map["/Projects/gitlab/org/ns/group-b/repo-b"], 2);
    }

    #[test]
    fn skip_depth_1_when_host_children_diverge_immediately() {
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/github/group-a/repo-a".to_string(),
            "/Projects/github/group-b/repo-b".to_string(),
        ];
        let map = path_skip_map(&paths, root);
        // github has 2 children (group-a, group-b) → diverges at depth 1 → skip=1
        assert_eq!(map["/Projects/github/group-a/repo-a"], 1);
        assert_eq!(map["/Projects/github/group-b/repo-b"], 1);
    }

    #[test]
    fn skip_depth_per_branch_not_global() {
        // plg-tech has only one child (ppl) → plg-tech is a pass-through, skip=2.
        // ppl has two children (team-a, team-b) → ppl is meaningful, stop at skip=2.
        // other-org has two children (group-x, group-y) → diverges immediately, skip=1.
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/gitlab/plg-tech/ppl/team-a/repo-a".to_string(),
            "/Projects/gitlab/plg-tech/ppl/team-b/repo-b".to_string(),
            "/Projects/gitlab/other-org/group-x/repo-c".to_string(),
            "/Projects/gitlab/other-org/group-y/repo-d".to_string(),
        ];
        let map = path_skip_map(&paths, root);
        // plg-tech skipped (pass-through) → tags start at ppl: [ppl, team-a/b]
        assert_eq!(map["/Projects/gitlab/plg-tech/ppl/team-a/repo-a"], 2);
        assert_eq!(map["/Projects/gitlab/plg-tech/ppl/team-b/repo-b"], 2);
        // other-org diverges immediately → tags start at other-org: [other-org, group-x/y]
        assert_eq!(map["/Projects/gitlab/other-org/group-x/repo-c"], 1);
        assert_eq!(map["/Projects/gitlab/other-org/group-y/repo-d"], 1);
    }

    // ── Project::new ──────────────────────────────────────────────────────────

    #[test]
    fn project_new_sets_name_from_last_segment() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("my-project");
        fs::create_dir_all(&repo).unwrap();
        let p = Project::new(repo, vec![]);
        assert_eq!(p.name, "my-project");
    }

    #[test]
    fn project_new_enabled_by_default() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let p = Project::new(repo, vec![]);
        assert!(p.enabled);
    }

    #[test]
    fn project_new_empty_paths_and_profile() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let p = Project::new(repo, vec![]);
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
