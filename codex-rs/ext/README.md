# codex-ext

`codex-ext` is the fork-owned extension layer for long-lived customization.

Current MVP scope:

- stable capability-negotiation shapes for future plugin runtimes
- jump-target and workspace-spawn models for fork-owned host extensions
- persisted multi-account pool state
- default account router model for threshold-based fallback
- data structures intentionally decoupled from current TUI/core internals

This crate does not yet load WASM modules. The immediate goal is to keep the
fork-specific policy surface isolated so future upgrades only touch a narrow set
of host integration points.
