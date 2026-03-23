use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const PLIST_LABEL: &str = "com.thomasroche.vscode-project-manager-updater";

fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(format!("Library/LaunchAgents/{PLIST_LABEL}.plist")))
}

fn log_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".cache/vscode-pmu"))
}

fn build_plist(binary_path: &str, log_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{PLIST_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{binary_path}</string>
    <string>watch</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log_path}/watch.log</string>
  <key>StandardErrorPath</key>
  <string>{log_path}/watch.log</string>
</dict>
</plist>
"#
    )
}

pub fn install() -> Result<()> {
    let binary_path = std::env::current_exe().context("cannot determine current executable")?;
    let binary_path_str = binary_path.to_string_lossy().to_string();

    let log_dir = log_dir()?;
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("cannot create log dir {}", log_dir.display()))?;
    let log_path_str = log_dir.to_string_lossy().to_string();

    let plist = build_plist(&binary_path_str, &log_path_str);
    let plist_path = plist_path()?;

    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    // Unload if already loaded (ignore errors — may not be loaded yet)
    let _ = Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .output();

    fs::write(&plist_path, &plist)
        .with_context(|| format!("cannot write plist {}", plist_path.display()))?;

    let load_output = Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output()
        .context("failed to run launchctl load")?;

    if !load_output.status.success() {
        let stderr = String::from_utf8_lossy(&load_output.stderr);
        anyhow::bail!("launchctl load failed: {stderr}");
    }

    eprintln!(
        "[{}] Installed and loaded {PLIST_LABEL}.",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    eprintln!(
        "[{}] Plist: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        plist_path.display()
    );
    eprintln!(
        "[{}] Logs: {}/watch.log",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        log_path_str
    );
    Ok(())
}

pub fn remove() -> Result<()> {
    let plist_path = plist_path()?;

    if plist_path.exists() {
        let unload_output = Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .output()
            .context("failed to run launchctl unload")?;

        if !unload_output.status.success() {
            let stderr = String::from_utf8_lossy(&unload_output.stderr);
            eprintln!(
                "[{}] Warning: launchctl unload: {stderr}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }

        fs::remove_file(&plist_path)
            .with_context(|| format!("cannot remove plist {}", plist_path.display()))?;

        eprintln!(
            "[{}] Uninstalled {PLIST_LABEL}.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    } else {
        eprintln!(
            "[{}] Plist not found — nothing to uninstall.",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }
    Ok(())
}
