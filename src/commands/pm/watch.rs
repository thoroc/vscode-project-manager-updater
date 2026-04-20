use anyhow::{Context, Result};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult};
use std::sync::mpsc;
use std::time::Duration;

use super::log::log_info;
use super::scan::run_scan;

/// Creates and arms a debounced watcher on `root`. Extracted for testability.
pub(crate) fn setup_watcher(
    root: &std::path::Path,
) -> Result<(
    notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
    mpsc::Receiver<DebounceEventResult>,
)> {
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer =
        new_debouncer(Duration::from_secs(2), tx).context("failed to create file watcher")?;
    debouncer
        .watcher()
        .watch(root, RecursiveMode::Recursive)
        .with_context(|| format!("cannot watch {}", root.display()))?;
    Ok((debouncer, rx))
}

pub fn run_watch(root: &std::path::Path) -> Result<()> {
    log_info!("Running initial scan before watching…");
    run_scan(root)?;

    log_info!("Watching {} for changes (Ctrl-C to stop)…", root.display());

    let (_debouncer, rx) = setup_watcher(root)?;

    for result in rx {
        match result {
            Ok(events) if !events.is_empty() => {
                log_info!("Change detected ({} events), running scan…", events.len());
                if let Err(e) = run_scan(root) {
                    log_info!("Scan error: {e:#}");
                }
            }
            Ok(_) => {}
            Err(e) => {
                log_info!("Watcher error: {e}");
            }
        }
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_errors_on_nonexistent_path() {
        let result = setup_watcher(std::path::Path::new("/nonexistent/path/xyz"));
        assert!(result.is_err(), "watching a nonexistent path must fail");
    }
}
