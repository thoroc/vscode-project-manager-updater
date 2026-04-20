use anyhow::{Context, Result};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult};
use std::sync::mpsc;
use std::time::Duration;

use super::scan::run_scan;

pub fn run_watch(root: &std::path::Path) -> Result<()> {
    eprintln!(
        "[{}] Running initial scan before watching…",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    run_scan(root)?;

    eprintln!(
        "[{}] Watching {} for changes (Ctrl-C to stop)…",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        root.display()
    );

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();

    let mut debouncer =
        new_debouncer(Duration::from_secs(2), tx).context("failed to create file watcher")?;

    debouncer
        .watcher()
        .watch(root, RecursiveMode::Recursive)
        .with_context(|| format!("cannot watch {}", root.display()))?;

    for result in rx {
        match result {
            Ok(events) if !events.is_empty() => {
                eprintln!(
                    "[{}] Change detected ({} events), running scan…",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                    events.len()
                );
                if let Err(e) = run_scan(root) {
                    eprintln!(
                        "[{}] Scan error: {e:#}",
                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                    );
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "[{}] Watcher error: {e}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                );
            }
        }
    }

    Ok(())
}
