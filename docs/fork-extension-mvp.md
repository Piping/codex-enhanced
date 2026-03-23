# Fork Extension MVP

## Proposal

Maintain a forked `codex` binary while moving fork-specific behavior behind a
host-owned extension layer. The host stays small and explicit; policy and
product logic move into extension-facing data models and, later, WASM plugins.

The first fork-owned target is multi-account ChatGPT routing with a fullscreen
control panel entry point. The fork should prefer additive crates and thin
integration shims over invasive edits to `codex-core`, `codex-tui`, or
`codex-tui-app-server`.

## Design

### Principles

- Keep fork logic in a dedicated workspace crate: `codex-ext`.
- Treat host/plugin compatibility as capability negotiation, not lockstep API.
- Keep plugins behind host-controlled APIs so existing approval and sandbox
  policies remain authoritative.
- Use the TUI as a consumer of extension state, not the source of truth.
- Route account switching above the provider layer, but below UI-specific code.

### MVP Scope

1. Add `codex-ext` with:
   - host capability negotiation types
   - persisted account-pool state under `CODEX_HOME/accounts/account-pool.json`
   - persisted per-account auth snapshots under `CODEX_HOME/accounts/<account-id>/auth.json`
   - a default threshold-based account router model
   - login-time account snapshotting and alias-aware account registration
2. Add a `Ctrl-P` control panel entry point in both TUI implementations.
3. Show account-pool state in a read-only popup so the fork has a visible,
   testable operator surface before auth switching is wired into requests.

### Deferred

- WASM runtime and ABI host
- plugin-owned TUI layouts
- undo-last-user-message implementation

## Phased Todos

### Phase 1

- Land `codex-ext` data model crate.
- Expose `Ctrl-P` control panel in both TUIs.
- Render account pool status from disk.

### Phase 2

- Introduce fork-owned account registry alongside current single-auth storage.
- Add active-account selector and account alias management.
- Record inferred limit/cooldown signals from model errors.

### Phase 3

- Route normal user turns through the default account router.
- Retry one normal user turn on explicit limit/rejection errors.
- Add `codex login --auth <alias>` so browser/device-code login snapshots the
  previous root auth and stores the new account under `accounts/<account-id>/`.
- Add observable switch reasons in the control panel.

### Phase 4

- Add a WASM host with capability negotiation.
- Re-express the default account router as a built-in plugin module.
- Add hook/interceptor points for `AppStart`, `SessionStart`,
  `BeforeTurnStart`, `BeforeToolCall`, and `AfterToolCall`.
