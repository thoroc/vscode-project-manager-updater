# AGENTS.md — vscode-project-manager-updater

## Project Overview

`vscode-pmu` is a Rust CLI that keeps the VSCode Project Manager extension's `projects.json` in sync with git
repositories discovered under a configurable root directory. It supports on-demand scanning, a launchd daemon
for real-time watching, and a git hook for automatic refresh on checkout.

**Binary:** `vscode-pmu`  
**Language:** Rust (edition 2021)  
**Platform:** macOS only (launchd integration)

## Repository Structure

```text
src/
├── main.rs                  # Entry point — parses CLI and dispatches
└── commands/pm/
    ├── mod.rs               # CLI definition (clap) and top-level dispatch
    ├── scan.rs              # walkdir scan + trie-based tag derivation
    ├── cache.rs             # 24h path cache (~/.cache/vscode-pmu/)
    ├── projects.rs          # projects.json read/write + reconciliation
    ├── config.rs            # per-host skip config (~/.config/vscode-pmu/)
    ├── watch.rs             # notify-based filesystem watcher
    ├── launchd.rs           # launchd plist install/uninstall
    └── hook.rs              # git post-checkout hook init/eject
```

## CLI Commands

| Command                                      | Description                                              |
| -------------------------------------------- | -------------------------------------------------------- |
| `vscode-pmu scan`                            | Scan root dir, reconcile projects.json (uses 24h cache)  |
| `vscode-pmu refresh`                         | Force full rescan, ignoring cache                        |
| `vscode-pmu daemon install`                  | Register launchd agent for real-time watching            |
| `vscode-pmu daemon uninstall`                | Remove launchd agent                                     |
| `vscode-pmu hooks install`                   | Install git post-checkout hook                           |
| `vscode-pmu hooks remove`                    | Remove git post-checkout hook                            |
| `vscode-pmu config set-skip <host> <depth>`  | Set minimum tag skip depth for a host                    |
| `vscode-pmu config show`                     | Print current configuration                              |

## Build & Test

```sh
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Run all tests
cargo test

# Install binary globally
cargo install --path .
```

Tests use `tempfile` for isolated I/O — they never touch the real `projects.json` or path cache.

## Configuration

Config file: `~/.config/vscode-pmu/config.toml`

```toml
# Scan depth below <root> (default: 6)
max_depth = 6

# Rename derived tags before writing to projects.json
# key = display label, value = list of raw path segments
[tag_rename]
personal = ["<username>"]

# Minimum path segments to skip per VCS host
[host_skip]
github = 1
gitlab = 3
```

## Key Files (outside the repo)

| File                                                                                               | Purpose                                                                |
| -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `~/Library/Application Support/Code/User/globalStorage/alefragnani.project-manager/projects.json`  | VSCode Project Manager state (backed up as `.bak` before every write)  |
| `~/.cache/vscode-pmu/<sanitized-root>.cache`                                                       | 24h scan cache                                                         |
| `~/.config/vscode-pmu/config.toml`                                                                 | Per-host skip depths, tag renames, max_depth                           |

## Tag Derivation Algorithm

1. A prefix trie is built from all discovered paths.
2. Per-project: skip the VCS host segment, then skip any pass-through or singleton segments.
3. Remaining intermediate segments become tags; if none remain, the host is the fallback.
4. Optional `[tag_rename]` substitutions are applied after derivation.
5. Optional `[host_skip]` floors raise the effective skip: `max(trie_derived, configured_floor)`.

## Development Guidelines

- All file I/O in tests must use `tempfile` — never the real `projects.json` or cache.
- The launchd and hooks subcommands are independent; either or both can be active.
- `projects.json` entries outside `<root>` are preserved untouched during reconciliation.
- A `.bak` backup is written before every `projects.json` write.
- Config is plain TOML; no migrations needed — add new keys with sensible defaults.
