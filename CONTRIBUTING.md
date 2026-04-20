# Contributing

## Prerequisites

Install [lefthook](https://github.com/evilmartians/lefthook) to run pre-commit and pre-push checks locally.

**Via mise (recommended):**
```sh
mise install
lefthook install
```

**Directly:**
```sh
# macOS
brew install lefthook

# or via cargo
cargo install lefthook

lefthook install
```

Lefthook runs `cargo fmt --check` and `cargo clippy` on commit, and `cargo test` on push.

## Commands

```sh
# Run all unit tests
cargo test

# Debug build
cargo build
```

Tests use `tempfile` for isolated file I/O and never touch the real `projects.json` or path cache.
