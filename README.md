# Codex Enhanced

Codex Enhanced is a standalone public distribution of Codex focused on
multi-account ChatGPT operations, fullscreen TUI workflow improvements, and a
smaller long-term fork maintenance surface.

This repository is maintained as its own GitHub project instead of a GitHub
fork. It tracks upstream Codex where practical, while keeping product-specific
behavior behind a dedicated extension layer so future changes can converge on
plugins instead of repeated invasive rebases.

## Why This Version Exists

Upstream Codex already provides a strong local coding agent. The main problem
this project solves is operational:

- switching between multiple ChatGPT accounts should not require manual file
  juggling
- rate-limit and usage-limit handling should be able to fail over to another
  account automatically
- fullscreen TUI workflows should expose session and account operations in one
  operator-facing control surface
- fork-specific behavior should move toward a stable extension boundary instead
  of expanding the patch set in core runtime code

## What Is Included

- Managed account storage under `~/.codex/accounts`
- `codex login --auth <alias>` for capturing multiple ChatGPT logins into named
  account slots
- Account pool metadata with stable IDs, aliases, cooldown state, and inferred
  usage windows
- Threshold-based account routing and one-shot retry on explicit
  limit/rejection failures for normal user turns in the fullscreen TUI path
- A `Ctrl-P` control panel with:
  - global session picker
  - account selection
  - alias rename submenu
  - current-session fork entry point
- A dedicated `codex-rs/ext` crate for fork-owned extension state and host
  compatibility groundwork

## Current Scope

The current milestone is a practical MVP for daily use, not the final extension
architecture.

Implemented now:

- managed ChatGPT account registry and auth snapshot layout
- account activation and alias management in the TUI
- control-panel-driven session and account operations
- inferred cooldown recording from explicit limit errors
- login-time account registration

Planned next:

- broader automatic account routing coverage beyond the current fullscreen TUI
  path
- observable switch reasons and richer operator status views
- hook/interceptor expansion
- capability-negotiated WASM plugins built on top of `codex-ext`

## Repository Layout

- [codex-rs/ext](./codex-rs/ext)  
  Fork-owned extension crate for account pool state, auth snapshots, and future
  plugin host compatibility.
- [codex-rs/tui](./codex-rs/tui)  
  Fullscreen local TUI implementation.
- [codex-rs/tui_app_server](./codex-rs/tui_app_server)  
  App-server-backed TUI implementation that mirrors relevant UX changes.
- [docs/fork-extension-mvp.md](./docs/fork-extension-mvp.md)  
  Fork proposal, MVP design, and phased roadmap.

## Build And Run

Build the Rust CLI locally:

```bash
cd codex-rs
cargo build -p codex-cli
./target/debug/codex
```

Install it into your shell path:

```bash
cd codex-rs
cargo build --release -p codex-cli
sudo ln -sf "$(pwd)/target/release/codex" /usr/local/bin/codex
codex --help
```

## Managed Account Quick Start

Register multiple ChatGPT logins into the managed account pool:

```bash
codex login --auth primary
codex login --auth backup
codex login status
```

Start Codex, then use:

- `Ctrl-P -> Sessions` to open the global session picker
- `Ctrl-P -> Accounts` to switch the active managed account
- `Ctrl-P -> Accounts -> Rename` to rename account aliases

Managed account state is stored under:

```text
~/.codex/accounts/
├── account-pool.json
└── <account-id>/
    └── auth.json
```

## Upstream Relationship

This project is based on OpenAI Codex and keeps upstream history so changes can
be rebased and audited cleanly. The maintenance goal is to keep the fork-owned
delta small, explicit, and increasingly isolated behind `codex-ext`.

## License

This repository remains licensed under the [Apache-2.0 License](LICENSE).
