use anyhow::Result;
use walkdir::WalkDir;

use super::cache;
use super::config::Config;
use super::projects::{
    compute_tags, path_skip_map, read_projects, write_projects, Project, projects_json_path,
};

const MAX_DEPTH: usize = 6;

/// Names of directories to prune (never descend into).
const PRUNE_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "build",
    "dist",
    "out",
    "debug",
    "release",
    "target",
    "coverage",
];

/// Returns false for directories that should be pruned during scanning.
/// depth == 0 is the root itself — always visit.
pub(crate) fn should_visit_dir(name: &str, depth: usize) -> bool {
    if depth == 0 {
        return true;
    }
    // Skip all hidden dirs except .git
    if name.starts_with('.') && name != ".git" {
        return false;
    }
    // Skip known heavy artifact dirs
    if PRUNE_DIRS.contains(&name) {
        return false;
    }
    true
}

/// Walk `root` up to `max_depth + 1` levels deep and return paths of
/// directories that directly contain a `.git` subdirectory.
pub(crate) fn discover_git_repos_in(root: &std::path::Path, max_depth: usize) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();

    for entry in WalkDir::new(root)
        .max_depth(max_depth + 1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            should_visit_dir(&name, e.depth())
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_name() == ".git" && entry.file_type().is_dir() {
            if let Some(parent) = entry.path().parent() {
                found.push(parent.to_string_lossy().to_string());
            }
        }
    }
    found
}

/// Returns the effective skip depth for `abs_path`, taking the maximum of the
/// trie-derived value and the per-host floor from `cfg`.
fn effective_skip(
    abs_path: &str,
    root: &std::path::Path,
    trie_skip: usize,
    cfg: &Config,
) -> usize {
    let host = std::path::Path::new(abs_path)
        .strip_prefix(root)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default();
    trie_skip.max(cfg.floor_for(&host))
}

pub fn run_scan(root: &std::path::Path) -> Result<()> {
    let projects_path = projects_json_path();
    let cfg = Config::load()?;

    eprintln!(
        "[{}] Starting scan of {}…",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        root.display()
    );

    let discovered: Vec<String> = if cache::is_fresh(root) {
        eprintln!(
            "[{}] Using fresh cache.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        cache::read_paths(root)?
    } else {
        eprintln!(
            "[{}] Cache stale or absent — walking filesystem…",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        let paths = discover_git_repos_in(root, MAX_DEPTH);
        cache::write_paths(root, &paths)?;
        eprintln!(
            "[{}] Found {} git repos.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            paths.len()
        );
        paths
    };

    let existing = read_projects(&projects_path)?;

    let (mut inside, outside): (Vec<Project>, Vec<Project>) = existing
        .into_iter()
        .partition(|p| {
            std::path::Path::new(&p.root_path)
                .starts_with(root)
        });

    inside.retain(|p| std::path::Path::new(&p.root_path).exists());

    // Pool existing + discovered paths to compute per-host skip depths.
    let all_paths: Vec<String> = {
        let mut v: Vec<String> = inside.iter().map(|p| p.root_path.clone()).collect();
        for p in &discovered {
            if !v.contains(p) {
                v.push(p.clone());
            }
        }
        v
    };
    let skip_map = path_skip_map(&all_paths, root);

    // Retag existing inside projects with the refined algorithm.
    for project in &mut inside {
        let trie_skip = skip_map.get(&project.root_path).copied().unwrap_or(1);
        let skip = effective_skip(&project.root_path, root, trie_skip, &cfg);
        project.retag(root, skip);
    }

    let existing_paths: std::collections::HashSet<String> =
        inside.iter().map(|p| p.root_path.clone()).collect();

    for path_str in &discovered {
        if !existing_paths.contains(path_str) {
            let pb = std::path::PathBuf::from(path_str);
            if pb.exists() {
                let trie_skip = skip_map.get(path_str).copied().unwrap_or(1);
                let skip = effective_skip(path_str, root, trie_skip, &cfg);
                let tags = compute_tags(&pb, root, skip);
                inside.push(Project::new(pb, tags));
            }
        }
    }

    let mut merged = outside;
    merged.extend(inside);

    write_projects(&projects_path, &merged)?;
    cache::update_paths(root, &merged)?;

    eprintln!(
        "[{}] projects.json updated ({} total entries).",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        merged.len()
    );
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── should_visit_dir ──────────────────────────────────────────────────────

    #[test]
    fn depth_zero_always_visited() {
        assert!(should_visit_dir("anything", 0));
        assert!(should_visit_dir(".hidden", 0));
        assert!(should_visit_dir("node_modules", 0));
    }

    #[test]
    fn hidden_dirs_are_pruned() {
        assert!(!should_visit_dir(".cache", 1));
        assert!(!should_visit_dir(".tmp", 1));
        assert!(!should_visit_dir(".next", 1));
        assert!(!should_visit_dir(".gitlab-ci-local", 2));
    }

    #[test]
    fn dot_git_is_not_pruned() {
        assert!(should_visit_dir(".git", 1));
        assert!(should_visit_dir(".git", 3));
    }

    #[test]
    fn prune_dirs_are_excluded() {
        for name in &[
            "node_modules", "vendor", "build", "dist", "out",
            "debug", "release", "target", "coverage",
        ] {
            assert!(!should_visit_dir(name, 1), "{name} should be pruned");
        }
    }

    #[test]
    fn normal_dirs_are_visited() {
        assert!(should_visit_dir("src", 1));
        assert!(should_visit_dir("github", 2));
        assert!(should_visit_dir("my-project", 3));
    }

    // ── discover_git_repos_in ─────────────────────────────────────────────────

    fn make_git_repo(base: &std::path::Path, rel: &str) {
        let repo = base.join(rel);
        fs::create_dir_all(repo.join(".git")).unwrap();
    }

    #[test]
    fn finds_direct_child_repo() {
        let tmp = TempDir::new().unwrap();
        make_git_repo(tmp.path(), "my-repo");
        let found = discover_git_repos_in(tmp.path(), 6);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("my-repo"));
    }

    #[test]
    fn finds_nested_repos() {
        let tmp = TempDir::new().unwrap();
        make_git_repo(tmp.path(), "github/org/repo-a");
        make_git_repo(tmp.path(), "gitlab/group/sub/repo-b");
        let mut found = discover_git_repos_in(tmp.path(), 6);
        found.sort();
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|p| p.ends_with("repo-a")));
        assert!(found.iter().any(|p| p.ends_with("repo-b")));
    }

    #[test]
    fn skips_repos_inside_hidden_dirs() {
        let tmp = TempDir::new().unwrap();
        make_git_repo(tmp.path(), ".tmp/hidden-repo");
        let found = discover_git_repos_in(tmp.path(), 6);
        assert!(found.is_empty(), "repos inside hidden dirs must be skipped");
    }

    #[test]
    fn skips_repos_inside_node_modules() {
        let tmp = TempDir::new().unwrap();
        make_git_repo(tmp.path(), "node_modules/some-dep");
        let found = discover_git_repos_in(tmp.path(), 6);
        assert!(found.is_empty(), "repos inside node_modules must be skipped");
    }

    #[test]
    fn skips_repos_inside_build_artifacts() {
        let tmp = TempDir::new().unwrap();
        for dir in &["build", "dist", "target", "coverage"] {
            make_git_repo(tmp.path(), &format!("{dir}/nested"));
        }
        let found = discover_git_repos_in(tmp.path(), 6);
        assert!(found.is_empty(), "repos inside artifact dirs must be skipped");
    }

    #[test]
    fn depth_limit_excludes_too_deep_repos() {
        let tmp = TempDir::new().unwrap();
        // 4 levels deep — max_depth=2 means find looks up to depth 3
        make_git_repo(tmp.path(), "a/b/c/deep-repo");
        let found = discover_git_repos_in(tmp.path(), 2);
        assert!(found.is_empty(), "repos beyond max_depth must not appear");
    }

    #[test]
    fn depth_limit_includes_repos_within_range() {
        let tmp = TempDir::new().unwrap();
        make_git_repo(tmp.path(), "a/b/repo");    // depth 3 from root → .git at depth 4
        let found = discover_git_repos_in(tmp.path(), 3); // max_depth + 1 = 4, reaches .git
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn empty_root_returns_empty_list() {
        let tmp = TempDir::new().unwrap();
        let found = discover_git_repos_in(tmp.path(), 6);
        assert!(found.is_empty());
    }
}
