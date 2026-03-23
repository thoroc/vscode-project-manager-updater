# vscode-project-manager-updater

A compiled Rust CLI that keeps the [VSCode Project Manager](https://marketplace.visualstudio.com/items?itemName=alefragnani.project-manager) `projects.json` in sync with git repositories under a configurable root directory (default: `~/Projects`).

## How it works

`vscode-pmu` scans a root directory (default: `~/Projects`) for git repositories and reconciles them into VSCode's `projects.json`. Two modes:

- **On-demand** — run `vscode-pmu scan` or `vscode-pmu refresh` manually
- **Daemon** — run `vscode-pmu install` to register a launchd agent that starts at login and watches for filesystem changes in real time

### Directory scanning

The scanner walks `<root>` up to **6 levels deep**. The following directories are never descended into:

| Category | Excluded |
|---|---|
| Hidden directories | anything starting with `.` (except `.git` itself) |
| Package managers | `node_modules`, `vendor` |
| Build artifacts | `build`, `dist`, `out`, `debug`, `release`, `target`, `coverage` |

### Tag derivation

The first path segment after `<root>` becomes the project tag in VSCode Project Manager:

| Path | Tag |
|---|---|
| `<root>/github/org/repo` | `github` |
| `<root>/gitlab/group/sub/repo` | `gitlab` |
| `<root>/my-repo` (direct child) | _(none)_ |

### Cache

After each filesystem walk the discovered paths are written to `~/.cache/vscode-pmu/<sanitized-root>.cache`. Subsequent `scan` calls reuse this cache for 24 hours, making them near-instant. Use `refresh` to force a new walk. Different `--root` values maintain independent caches.

## Prerequisites

- macOS (launchd integration is macOS-only)
- [Rust toolchain](https://rustup.rs) (to build from source)
- VSCode with the [Project Manager extension](https://marketplace.visualstudio.com/items?itemName=alefragnani.project-manager) installed

## Installation

### 1. Build

```sh
cd ~/.config/scripts/vscode-project-manager-updater
cargo build --release
```

### 2. Install the binary system-wide

**Option A — via `cargo install` (recommended, installs to `~/.cargo/bin`)**

```sh
cargo install --path .
```

`~/.cargo/bin` is on `$PATH` by default after a standard Rust installation.

**Option B — system-wide to `/usr/local/bin`**

```sh
sudo install -m 755 target/release/vscode-pmu /usr/local/bin/vscode-pmu
```

Verify it is accessible:

```sh
which vscode-pmu
vscode-pmu --version
```

### 3. Register the launchd daemon

Once the binary is on `$PATH`, register it as a launchd user agent so it starts automatically at login:

```sh
vscode-pmu daemon install
```

This embeds the binary path into a plist at `~/Library/LaunchAgents/com.thomasroche.vscode-project-manager-updater.plist` and loads the agent.

> **Note:** run `vscode-pmu daemon install` again any time you update the binary to refresh the embedded path in the plist.

### 4. Set up the git hook (optional)

To auto-register repos on `git clone`:

```sh
vscode-pmu hooks install
```

### Uninstall

```sh
vscode-pmu hooks remove                # remove git hook
vscode-pmu daemon remove               # remove the launchd agent
sudo rm /usr/local/bin/vscode-pmu      # if installed via option B
```

## Commands

```sh
vscode-pmu [--root <DIR>] <COMMAND>

Global options:
  --root <DIR>   Root directory to scan for git repositories.
                 Defaults to ~/Projects.

Commands:
  scan              Scan <root> and update projects.json (uses cache if fresh)
  add [path]        Add a git repository; prompts for path if not provided
  remove [name]     Remove a project; shows pick-list if name not provided
  refresh           Invalidate cache and force a full re-scan
  daemon <ACTION>   Manage the launchd background daemon
  hooks <ACTION>    Manage the git post-checkout hook
```

The `--root` flag is global and can be placed before or after the subcommand:

```sh
vscode-pmu --root ~/Work scan
vscode-pmu --root /srv/repos refresh
```

> **Note:** the cache is keyed to the root directory, so different roots maintain independent caches.

### scan

Reconciles `projects.json` with `<root>` (default `~/Projects`). Uses the 24-hour path cache when available; falls back to a full walk otherwise.

```sh
vscode-pmu scan
vscode-pmu --root ~/Work scan
```

### add

Adds a git repository to `projects.json`. Pass a path directly, or omit it to be prompted.

```sh
vscode-pmu add ~/Projects/github/my-org/new-repo   # direct
vscode-pmu add                                       # interactive prompt
```

The path must contain a `.git` directory.

### remove

Removes a project from `projects.json`. Pass a name directly, or omit it to select from a list of managed projects.

```sh
vscode-pmu remove new-repo   # by name
vscode-pmu remove            # interactive pick-list
```

### refresh

Deletes the cache and forces a full walk regardless of cache age.

```sh
vscode-pmu refresh
```

### daemon install / daemon remove

Manages a launchd user agent that runs `vscode-pmu watch` automatically at login.

| Path | Purpose |
|---|---|
| `~/Library/LaunchAgents/com.thomasroche.vscode-project-manager-updater.plist` | launchd plist |
| `~/.cache/vscode-pmu/watch.log` | daemon log |

```sh
vscode-pmu daemon install   # write plist and load agent
vscode-pmu daemon remove    # unload agent and remove plist
```

### hooks install / hooks remove

Manages the git template hook so that every `git clone` automatically registers the new repo with VSCode Project Manager.

`hooks install` writes the `post-checkout` hook and sets `git config --global init.templateDir`. `hooks remove` reverses both steps.

| Path | Purpose |
|---|---|
| `~/.config/git/templates/hooks/post-checkout` | hook invoked by git after every clone |

```sh
vscode-pmu hooks install   # write hook and set init.templateDir
vscode-pmu hooks remove    # remove hook and unset init.templateDir
```

> **Note:** `daemon` and `hooks` are independent — you can use either or both.

## projects.json

Located at:

```sh
~/Library/Application Support/Code/User/globalStorage/alefragnani.project-manager/projects.json
```

A backup is written as `projects.json.bak` before every write. Each managed entry follows the Project Manager schema:

```json
{
  "name": "repo",
  "rootPath": "/Users/you/Projects/github/org/repo",
  "paths": [],
  "tags": ["github"],
  "enabled": true,
  "profile": ""
}
```

Entries whose `rootPath` is outside `<root>` are preserved untouched during reconciliation.

## Project structure

```sh
vscode-project-manager-updater/
├── Cargo.toml
├── Cargo.lock
├── .gitignore            # excludes /target
├── README.md
└── src/
    ├── main.rs
    └── commands/
        └── pm/
            ├── mod.rs        # CLI definition and dispatch
            ├── scan.rs       # walkdir scan + filter logic
            ├── cache.rs      # 24h path cache (~/.cache/vscode-pmu/)
            ├── projects.rs   # projects.json read/write + add/remove
            ├── watch.rs      # notify-based file watcher
            ├── launchd.rs    # launchd plist install/uninstall
            └── hook.rs       # git post-checkout hook init/eject
```

## Development

```sh
# Run all unit tests
cargo test

# Debug build
cargo build
```

Tests use `tempfile` for isolated file I/O and never touch the real `projects.json` or path cache.
