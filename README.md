# Codex Enhanced

Codex Enhanced is an operator-first Codex distribution for people who want a
local coding agent with better session operations, loop-driven automation, and
workspace-backed Feishu clawbot workflows.

It stays close to upstream Codex where practical, but focuses fork effort on
the places that matter in daily operations: account switching, visible control
surfaces, repeatable release milestones, and automation that can be managed
from inside the TUI instead of bolted on from the outside.

Website: `https://codex-enhanced.com`

![Codex Enhanced hero](./docs/assets/readme-hero.svg)

![Codex Enhanced splash](./.github/codex-cli-splash.png)

## Why This Fork Exists

Upstream Codex already provides a strong local coding agent. This fork is about
operability:

- switching between multiple ChatGPT accounts should not require manual auth
  file juggling
- loop automation should be visible, interruptible, and predictable from the
  TUI
- external chat entry points should bind to real Codex threads instead of
  living in a separate bot stack
- fork-owned behavior should move toward explicit extension boundaries instead
  of spreading through core runtime code

## What You Get

| Area | What is implemented now |
| --- | --- |
| Managed accounts | Named account slots under `~/.codex/accounts`, login-time registration, stable aliases, and operator-facing switching |
| TUI control surface | `Ctrl-P` control panel for sessions, accounts, clawbot management, and current-session workflows |
| Loop runtime | Before-turn and after-turn loop runners with queued scheduling, per-loop progress, `/stop` cancellation, and info cells in the chat stream |
| Clawbot | Workspace-backed `codex-clawbot` runtime, Feishu session discovery, manual bind, cached unread flush, and final answer forwarding |
| Fork boundary | Dedicated fork-owned crates and a release flow that keeps the fork delta reviewable |

## Operator Surface

| Overview | Release train |
| --- | --- |
| ![Operator surface](./docs/assets/operator-surface.svg) | ![Recent release train](./docs/assets/release-train.svg) |

## Recent Releases

### `v0.1.11`

- fixed the empty `after-turn` path so the TUI does not leave a stale
  `Running background loop` banner behind
- hardened background loop state cleanup for the latest scheduler flow

### `v0.1.10`

- added the `codex-clawbot` crate for workspace-backed Feishu session bridging
- added clawbot control-panel flows for session list, manual bind, retry, scan,
  clear, flush, and configuration
- made after-turn loop rounds responsive, surfaced per-loop queue progress, and
  restored `/stop` cancellation for hidden loop runs

### `v0.1.9`

- shipped loop v2 runtime updates and aligned release artifacts around the new
  scheduler flow
- refreshed TUI and `tui_app_server` snapshots for the newer account and status
  surfaces
- aligned stop-cleanup and app-server widget behavior with the current TUI

## Clawbot And Loop Workflow

Recent releases moved this fork beyond simple account management:

- Feishu sessions can be discovered, scanned, or manually bound to the current
  Codex thread from `Ctrl-P -> Clawbot -> Sessions`
- unread Feishu messages can be cached before binding, flushed into the bound
  thread in order, and tagged in the TUI as `Feishu message`
- loop-generated activity now shows explicit progress and emits `Loop agent
  reply` info cells so operators can see where automation output came from
- bound threads can forward their final assistant answer back into the linked
  Feishu session

## Repository Layout

- [codex-rs/ext](./codex-rs/ext)  
  Fork-owned extension crate for account pool state, auth snapshots, and future
  plugin host compatibility.
- [codex-rs/clawbot](./codex-rs/clawbot)  
  Workspace-backed clawbot runtime for Feishu session persistence, binding, and
  provider integration.
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
- `Ctrl-P -> Clawbot -> Sessions` to manage Feishu sessions and bind one to the
  current thread

Managed account state is stored under:

```text
~/.codex/accounts/
├── account-pool.json
└── <account-id>/
    └── auth.json
```

Workspace-backed clawbot state is stored under:

```text
<workspace>/.codex/clawbot/
├── bindings.json
├── config.toml
├── inbound_receipts.json
├── runtime.json
├── sessions.json
└── unread_messages.jsonl
```

## Upstream Relationship

This project is based on OpenAI Codex and keeps upstream history so changes can
be rebased and audited cleanly. The maintenance goal is to keep the fork-owned
delta small, explicit, and increasingly isolated behind `codex-ext`,
`codex-clawbot`, and other dedicated extension layers instead of broad runtime
patches.

## License

This repository remains licensed under the [Apache-2.0 License](LICENSE).
