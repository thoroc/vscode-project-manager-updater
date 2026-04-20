use anyhow::{Context, Result};
use dialoguer::{Input, Select};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::cache;
use super::config::Config;
use super::log::log_info;

pub fn projects_json_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join("Library/Application Support/Code/User/globalStorage/alefragnani.project-manager/projects.json"))
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

    // leaf_count: group prefix → number of projects that are DIRECT children
    // of that group (i.e. the project name is the very next segment).
    // Used to distinguish singleton leaf-groups (skip → host fallback) from
    // multi-project leaf-groups (keep group name as a meaningful tag).
    let mut leaf_count: HashMap<Vec<String>, usize> = HashMap::new();

    let segs_list: Vec<(String, Vec<String>)> = paths
        .iter()
        .filter_map(|p| {
            Path::new(p).strip_prefix(watch_root).ok().map(|rel| {
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
        // The parent of the project name is segs[..segs.len()-1].
        if segs.len() >= 2 {
            *leaf_count
                .entry(segs[..segs.len() - 1].to_vec())
                .or_insert(0) += 1;
        }
    }

    segs_list
        .into_iter()
        .map(|(path, segs)| (path, trie_skip_depth(&segs, &trie, &leaf_count)))
        .collect()
}

/// Walks `segs` through `trie`, following the path's own branch.
///
/// A segment is skipped when:
/// - Its subtree has exactly one distinct child (structural pass-through), OR
/// - It is a leaf-group (no sub-levels in the trie) containing only one
///   direct project — a singleton group adds no filtering value.
///
/// The host (index 0) is always skipped.
fn trie_skip_depth(
    segs: &[String],
    trie: &std::collections::HashMap<Vec<String>, std::collections::HashSet<String>>,
    leaf_count: &std::collections::HashMap<Vec<String>, usize>,
) -> usize {
    if segs.len() < 2 {
        return 0; // direct child of watch_root — no host to skip
    }
    let mut skip = 1; // always skip the VCS host
    for i in 1..segs.len().saturating_sub(1) {
        // segs[..=i] is the prefix ending at segs[i] (inclusive).
        // trie[segs[..=i]] gives the sub-level children of segs[i].
        match trie.get(&segs[..=i]) {
            Some(children) if children.len() == 1 => {
                // One sub-level forward — but only a pure pass-through when
                // there are *no* direct leaf projects at this level.
                // If leaf_count > 0 the node is a mixed group (some repos
                // live here directly, others nest deeper) and is meaningful.
                let direct = leaf_count.get(&segs[..=i]).copied().unwrap_or(0);
                if direct == 0 {
                    skip = i + 1;
                } else {
                    break;
                }
            }
            None => {
                // segs[i] is a leaf-group: projects are its direct children.
                // Only worth tagging if it contains more than one project.
                let n = leaf_count.get(&segs[..=i]).copied().unwrap_or(0);
                if n <= 1 {
                    skip = i + 1; // singleton → fall back to host
                }
                break;
            }
            _ => break, // multiple sub-groups → meaningful level, stop
        }
    }
    skip
}

pub fn read_projects(path: &Path) -> Result<Vec<Project>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("invalid JSON in {}", path.display()))
}

pub fn write_projects(path: &Path, projects: &[Project]) -> Result<()> {
    // Backup first
    if path.exists() {
        let bak = path.with_extension("json.bak");
        fs::copy(path, &bak).with_context(|| format!("cannot back up {}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(projects).context("JSON serialisation failed")?;
    fs::write(path, json).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

pub(crate) fn add_to(path: &Path, root: &Path, projects_path: &Path, cfg: &Config) -> Result<()> {
    let resolved = std::fs::canonicalize(path)
        .with_context(|| format!("cannot resolve {}", path.display()))?;
    if !resolved.join(".git").exists() {
        anyhow::bail!("{} does not contain a .git directory", resolved.display());
    }

    let mut projects = read_projects(projects_path)?;

    let path_str = resolved.to_string_lossy().to_string();
    if projects.iter().any(|p| p.root_path == path_str) {
        log_info!("Project already present: {}", path_str);
        return Ok(());
    }

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
    let skip = trie_skip.max(cfg.floor_for(&host));
    let tags = cfg.rename_tags(compute_tags(&resolved, root, skip));
    let project = Project::new(resolved.clone(), tags);
    log_info!("Adding project: {}", path_str);
    projects.push(project);
    write_projects(projects_path, &projects)?;
    cache::update_paths(root, &projects)?;
    Ok(())
}

pub(crate) fn remove_from(name: &str, root: &Path, projects_path: &Path) -> Result<bool> {
    let mut projects = read_projects(projects_path)?;
    let before = projects.len();
    projects.retain(|p| !(Path::new(&p.root_path).starts_with(root) && p.name == name));
    if projects.len() == before {
        return Ok(false);
    }
    write_projects(projects_path, &projects)?;
    cache::update_paths(root, &projects)?;
    Ok(true)
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
    let projects_path = projects_json_path()?;
    let cfg = Config::load()?;
    add_to(&target, root, &projects_path, &cfg)
}

pub fn remove(name: Option<&str>, root: &Path) -> Result<()> {
    let projects_path = projects_json_path()?;
    let projects = read_projects(&projects_path)?;

    let managed: Vec<String> = projects
        .iter()
        .filter(|p| Path::new(&p.root_path).starts_with(root))
        .map(|p| p.name.clone())
        .collect();

    if managed.is_empty() {
        log_info!("No managed projects found under {}.", root.display());
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

    if remove_from(&target, root, &projects_path)? {
        log_info!("Removed project: {}", target);
    } else {
        log_info!("Project not found: {}", target);
    }
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
        let project = std::path::Path::new("/home/user/Projects/gitlab/org/ns/group/my-repo");
        // skip=3 means gitlab/org/ns are noise → tag is group
        assert_eq!(compute_tags(project, root, 3), vec!["group"]);
    }

    #[test]
    fn tags_multiple_segments_when_deep_enough() {
        let root = std::path::Path::new("/home/user/Projects");
        let project = std::path::Path::new("/home/user/Projects/gitlab/org/ns/infra/images/my-app");
        assert_eq!(compute_tags(project, root, 3), vec!["infra", "images"]);
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
    fn skip_depth_2_for_singleton_leaf_groups() {
        // Each group has only one project → singleton, not useful for filtering.
        // Both get skip=2 so compute_tags falls back to the host name.
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/github/group-a/repo-a".to_string(),
            "/Projects/github/group-b/repo-b".to_string(),
        ];
        let map = path_skip_map(&paths, root);
        assert_eq!(map["/Projects/github/group-a/repo-a"], 2);
        assert_eq!(map["/Projects/github/group-b/repo-b"], 2);
    }

    #[test]
    fn skip_depth_1_when_group_has_direct_and_nested_projects() {
        // group has direct repos AND one nested sub-group (mixed).
        // Must NOT be treated as a pass-through even though the trie has one child.
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/github/group/repo-a".to_string(), // direct leaf
            "/Projects/github/group/repo-b".to_string(), // direct leaf
            "/Projects/github/group/sub-group/repo-c".to_string(), // nested
        ];
        let map = path_skip_map(&paths, root);
        assert_eq!(map["/Projects/github/group/repo-a"], 1);
        assert_eq!(map["/Projects/github/group/repo-b"], 1);
        assert_eq!(map["/Projects/github/group/sub-group/repo-c"], 1);
    }

    #[test]
    fn skip_depth_1_when_group_has_multiple_projects() {
        // group-a has two projects → meaningful for filtering → skip=1.
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/github/group-a/repo-a".to_string(),
            "/Projects/github/group-a/repo-b".to_string(),
        ];
        let map = path_skip_map(&paths, root);
        assert_eq!(map["/Projects/github/group-a/repo-a"], 1);
        assert_eq!(map["/Projects/github/group-a/repo-b"], 1);
    }

    #[test]
    fn skip_depth_per_branch_not_global() {
        // acme has only one child (platform) → acme is a pass-through, skip=2.
        // platform has two children (team-a, team-b) → platform is meaningful, stop at skip=2.
        // oss has two children (group-x, group-y) → diverges immediately, skip=1.
        let root = std::path::Path::new("/Projects");
        let paths = vec![
            "/Projects/gitlab/acme/platform/team-a/repo-a".to_string(),
            "/Projects/gitlab/acme/platform/team-b/repo-b".to_string(),
            "/Projects/gitlab/oss/group-x/repo-c".to_string(),
            "/Projects/gitlab/oss/group-y/repo-d".to_string(),
        ];
        let map = path_skip_map(&paths, root);
        // acme skipped (pass-through) → tags start at platform: [platform, team-a/b]
        assert_eq!(map["/Projects/gitlab/acme/platform/team-a/repo-a"], 2);
        assert_eq!(map["/Projects/gitlab/acme/platform/team-b/repo-b"], 2);
        // oss diverges immediately → tags start at oss: [oss, group-x/y]
        assert_eq!(map["/Projects/gitlab/oss/group-x/repo-c"], 1);
        assert_eq!(map["/Projects/gitlab/oss/group-y/repo-d"], 1);
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

    // ── add_to ────────────────────────────────────────────────────────────────

    fn make_git_repo(base: &std::path::Path, rel: &str) -> PathBuf {
        let repo = base.join(rel);
        fs::create_dir_all(repo.join(".git")).unwrap();
        repo
    }

    #[test]
    fn add_to_adds_new_project() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let repo = make_git_repo(&root, "github/org/my-repo");
        let projects_path = tmp.path().join("projects.json");
        write_projects(&projects_path, &[]).unwrap();
        let cfg = super::super::config::Config::default();
        add_to(&repo, &root, &projects_path, &cfg).unwrap();
        let loaded = read_projects(&projects_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "my-repo");
    }

    #[test]
    fn add_to_is_no_op_when_already_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let repo = make_git_repo(&root, "github/org/my-repo");
        let projects_path = tmp.path().join("projects.json");
        write_projects(&projects_path, &[]).unwrap();
        let cfg = super::super::config::Config::default();
        add_to(&repo, &root, &projects_path, &cfg).unwrap();
        add_to(&repo, &root, &projects_path, &cfg).unwrap();
        let loaded = read_projects(&projects_path).unwrap();
        assert_eq!(loaded.len(), 1, "duplicate add must be a no-op");
    }

    #[test]
    fn add_to_errors_when_no_git_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let not_a_repo = root.join("not-a-repo");
        fs::create_dir_all(&not_a_repo).unwrap();
        let projects_path = tmp.path().join("projects.json");
        write_projects(&projects_path, &[]).unwrap();
        let cfg = super::super::config::Config::default();
        let result = add_to(&not_a_repo, &root, &projects_path, &cfg);
        assert!(result.is_err(), "should error when .git is absent");
    }

    // ── remove_from ───────────────────────────────────────────────────────────

    #[test]
    fn remove_from_removes_existing_project() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let projects_path = tmp.path().join("projects.json");
        let project = Project::new(root.join("my-repo"), vec![]);
        write_projects(&projects_path, &[project]).unwrap();
        let removed = remove_from("my-repo", &root, &projects_path).unwrap();
        assert!(removed);
        let loaded = read_projects(&projects_path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn remove_from_returns_false_when_project_absent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let projects_path = tmp.path().join("projects.json");
        write_projects(&projects_path, &[]).unwrap();
        let removed = remove_from("nonexistent", &root, &projects_path).unwrap();
        assert!(!removed);
    }

    #[test]
    fn remove_from_preserves_projects_outside_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let projects_path = tmp.path().join("projects.json");
        let inside = Project::new(root.join("my-repo"), vec![]);
        let outside = Project {
            name: "external".to_string(),
            root_path: "/some/other/path/external".to_string(),
            paths: vec![],
            tags: vec![],
            enabled: true,
            profile: String::new(),
        };
        write_projects(&projects_path, &[inside, outside]).unwrap();
        remove_from("my-repo", &root, &projects_path).unwrap();
        let loaded = read_projects(&projects_path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "external");
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
