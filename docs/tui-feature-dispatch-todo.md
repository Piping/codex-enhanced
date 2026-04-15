# TUI Feature Dispatch TODO

This tracks the incremental refactor away from a single growing `App` event loop.

## Phase 1: Extract dispatch boundaries inside `codex-tui`

- [x] Identify the first extraction boundary with low behavior risk.
- [x] Move workflow-related `AppEvent` dispatch out of `tui/src/app.rs` into a dedicated module.
- [x] Move clawbot-related `AppEvent` dispatch out of `tui/src/app.rs` into a dedicated module.
- [x] Add a small shared helper for "replace active popup or show new popup" flows.
- [x] Add a small shared helper for external-editor launch flows.
- [x] Keep `App` as the state owner for now; do not change runtime behavior during this phase.

## Phase 2: Introduce feature-local controller surfaces

- [x] Define feature-local controller methods that group state + event handling for:
  - [x] workflow
  - [x] clawbot
  - [x] profile management
  - [x] thread actions / jump navigation
  - [x] session lifecycle / resume flows
  - [x] integration / plugin / connector flows
  - [x] settings / preferences / sandbox flows
- [ ] Reduce direct `AppEvent` matching in `App::handle_event()` so the top-level loop becomes routing-only.
- [ ] Evaluate whether `AppEvent` should gain feature wrapper variants instead of flat growth.

## Phase 3: Prepare crate boundaries

- [ ] Separate workflow core logic from TUI glue.
- [ ] Separate clawbot provider-neutral core from Feishu-specific runtime glue.
- [ ] Move profile-router persistence / fallback policy out of `codex-tui`.
- [ ] Keep rendering/UI-only code in `codex-tui`.

## Candidate crates

- `codex-workflows`
  - workflow definitions
  - YAML round-trip helpers
  - scheduler / runtime core
  - trigger overlap and execution policy
- `codex-clawbot-feishu`
  - Feishu websocket runtime
  - Feishu REST API integration
  - payload diagnostics / dump helpers
- `codex-profile-router`
  - router state store
  - fallback policy mapping

## Validation gate for each major step

- Run `just fmt` in `codex-rs`.
- Run targeted tests for touched crates.
- Run `cargo build -p codex-cli`.
- Launch the built binary in PTY mode and do a smoke test before moving to the next milestone.
