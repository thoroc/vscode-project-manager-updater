use anyhow::{Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const HOOK_CONTENT: &str = r#"#!/usr/bin/env bash
# post-checkout hook — auto-register new git clones with VSCode Project Manager
# Managed by vscode-pmu. Run `vscode-pmu hooks remove` to remove.
#
# Git passes:
#   $1 = previous HEAD (all-zeros SHA on a fresh clone)
#   $2 = new HEAD
#   $3 = 1 (branch checkout) | 0 (file checkout)

PREV_HEAD="$1"
IS_BRANCH_CHECKOUT="$3"
NULL_SHA="0000000000000000000000000000000000000000"

# Only act on branch checkouts
[[ "${IS_BRANCH_CHECKOUT}" != "1" ]] && exit 0

# Only act on fresh clones (no previous HEAD)
[[ "${PREV_HEAD}" != "${NULL_SHA}" ]] && exit 0

if command -v vscode-pmu &>/dev/null; then
  vscode-pmu add "$(pwd)"
fi
"#;

fn template_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".config/git/templates"))
}

fn hook_path() -> Result<PathBuf> {
    Ok(template_dir()?.join("hooks/post-checkout"))
}

// ── Internal path-parameterised helpers (also used by tests) ─────────────────

pub(crate) fn write_hook_at(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(path, HOOK_CONTENT)
        .with_context(|| format!("cannot write hook {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("cannot chmod hook {}", path.display()))?;
    Ok(())
}

pub(crate) fn remove_hook_at(path: &Path) -> Result<bool> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("cannot remove hook {}", path.display()))?;
        return Ok(true);
    }
    Ok(false)
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn install() -> Result<()> {
    let hook = hook_path()?;
    let template_dir = template_dir()?;

    write_hook_at(&hook)?;

    let output = Command::new("git")
        .args([
            "config",
            "--global",
            "init.templateDir",
            &template_dir.to_string_lossy(),
        ])
        .output()
        .context("failed to run git config")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git config failed: {stderr}");
    }

    eprintln!(
        "[{}] Hook written: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        hook.display()
    );
    eprintln!(
        "[{}] git init.templateDir set to {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        template_dir.display()
    );
    eprintln!(
        "[{}] New `git clone` operations will auto-register with VSCode Project Manager.",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    Ok(())
}

pub fn remove() -> Result<()> {
    let hook = hook_path()?;

    // Unset init.templateDir (ignore error — may not be set)
    let _ = Command::new("git")
        .args(["config", "--global", "--unset", "init.templateDir"])
        .output();

    if remove_hook_at(&hook)? {
        eprintln!(
            "[{}] Hook removed: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            hook.display()
        );
    } else {
        eprintln!(
            "[{}] Hook not found — nothing to remove.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }

    eprintln!(
        "[{}] git init.templateDir unset.",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    // ── HOOK_CONTENT ──────────────────────────────────────────────────────────

    #[test]
    fn hook_content_has_shebang() {
        assert!(HOOK_CONTENT.starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn hook_content_calls_vscode_pmu_add() {
        assert!(HOOK_CONTENT.contains("vscode-pmu add"));
    }

    #[test]
    fn hook_content_guards_on_null_sha() {
        assert!(HOOK_CONTENT.contains("0000000000000000000000000000000000000000"));
    }

    #[test]
    fn hook_content_guards_on_branch_checkout() {
        assert!(HOOK_CONTENT.contains("IS_BRANCH_CHECKOUT"));
    }

    // ── write_hook_at ─────────────────────────────────────────────────────────

    #[test]
    fn write_hook_creates_file_with_correct_content() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("post-checkout");
        write_hook_at(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, HOOK_CONTENT);
    }

    #[test]
    fn write_hook_sets_executable_permission() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("post-checkout");
        write_hook_at(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o755, 0o755, "hook must be executable");
    }

    #[test]
    fn write_hook_creates_parent_directories() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("hooks/post-checkout");
        write_hook_at(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_hook_overwrites_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("post-checkout");
        fs::write(&path, "old content").unwrap();
        write_hook_at(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, HOOK_CONTENT);
    }

    // ── remove_hook_at ────────────────────────────────────────────────────────

    #[test]
    fn remove_hook_deletes_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("post-checkout");
        write_hook_at(&path).unwrap();
        assert!(path.exists());
        let removed = remove_hook_at(&path).unwrap();
        assert!(removed);
        assert!(!path.exists());
    }

    #[test]
    fn remove_hook_returns_false_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent");
        let removed = remove_hook_at(&path).unwrap();
        assert!(!removed);
    }
}
