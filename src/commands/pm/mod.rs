pub mod cache;
pub mod config;
pub mod hook;
pub mod launchd;
pub(crate) mod log;
pub mod projects;
pub mod scan;
pub mod watch;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// The subcommand name used internally by the launchd daemon.
pub(crate) const WATCH_SUBCOMMAND: &str = "watch";

fn default_root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join("Projects"))
}

#[derive(Parser)]
#[command(
    name = "vscode-pmu",
    about = "Manage VSCode Project Manager projects.json",
    arg_required_else_help = true,
    version
)]
pub struct Cli {
    /// Root directory to scan for git repositories.
    /// Defaults to ~/Projects.
    #[arg(long, value_name = "DIR", global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub subcommand: Subcommands,
}

#[derive(Subcommand)]
pub enum Subcommands {
    /// Scan <root> and update projects.json (uses cache if fresh)
    Scan,
    /// Add a git repository; prompts for path if not provided
    Add {
        /// Path to the git repository
        path: Option<PathBuf>,
    },
    /// Remove a project; shows a pick-list if name not provided
    Remove {
        /// Name of the project to remove
        name: Option<String>,
    },
    /// Invalidate cache and force a full re-scan
    Refresh,
    /// Manage the launchd background daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Manage the git post-checkout hook
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },
    /// Manage per-host tag skip depth configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Watch <root> for filesystem changes (used internally by the daemon)
    #[command(hide = true)]
    Watch,
}

#[derive(Subcommand)]
pub enum DaemonAction {
    /// Install and start the launchd agent
    Install,
    /// Stop and remove the launchd agent
    Remove,
}

#[derive(Subcommand)]
pub enum HooksAction {
    /// Write the post-checkout hook and set git init.templateDir
    Install,
    /// Remove the post-checkout hook and unset git init.templateDir
    Remove,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set the minimum tag-skip depth for a VCS host
    SetSkip {
        /// VCS host directory name (e.g. gitlab, github)
        host: String,
        /// Number of path segments to skip when deriving tags
        depth: usize,
    },
    /// Print the current configuration
    Show,
}

pub fn run(cli: Cli) -> Result<()> {
    let root = cli.root.map(Ok).unwrap_or_else(default_root)?;
    match cli.subcommand {
        Subcommands::Scan => scan::run_scan(&root),
        Subcommands::Add { path } => projects::add(path.as_deref(), &root),
        Subcommands::Remove { name } => projects::remove(name.as_deref(), &root),
        Subcommands::Refresh => {
            cache::delete(&root)?;
            scan::run_scan(&root)
        }
        Subcommands::Daemon { action } => match action {
            DaemonAction::Install => launchd::install(),
            DaemonAction::Remove => launchd::remove(),
        },
        Subcommands::Hooks { action } => match action {
            HooksAction::Install => hook::install(),
            HooksAction::Remove => hook::remove(),
        },
        Subcommands::Config { action } => match action {
            ConfigAction::SetSkip { host, depth } => config::run_set_skip(&host, depth),
            ConfigAction::Show => config::run_show(),
        },
        Subcommands::Watch => watch::run_watch(&root),
    }
}
