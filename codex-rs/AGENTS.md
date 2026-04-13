# Rust/codex-rs

Use this file for general Rust guidance in `codex-rs/`. More specific guidance lives in nested
`AGENTS.md` files.

- Crate names are prefixed with `codex-`. For example, the `core` folder's crate is named
  `codex-core`.
- When using `format!` and you can inline variables into `{}`, always do that.
- Install any commands the repo relies on, such as `just`, `rg`, or `cargo-insta`, before using
  them.
- Never add or modify any code related to `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` or
  `CODEX_SANDBOX_ENV_VAR`.
  - `CODEX_SANDBOX_NETWORK_DISABLED=1` is set whenever shell commands run in the sandbox. Existing
    checks against `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` are often intentional early exits for
    tests that cannot run there.
  - `CODEX_SANDBOX=seatbelt` is set for child processes spawned with Seatbelt. Tests that need to
    run Seatbelt themselves may intentionally early exit when they detect that state.
- Always collapse `if` statements when Clippy's `collapsible_if` lint applies.
- Always inline `format!` args when Clippy's `uninlined_format_args` lint applies.
- Use method references over closures when Clippy's
  `redundant_closure_for_method_calls` lint applies.
- Avoid bool or ambiguous `Option` parameters that force callers to write hard-to-read code such as
  `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other idiomatic Rust API
  shapes when they keep the callsite self-documenting.
- When you cannot make that API change and still need a small positional-literal callsite in Rust,
  follow the `argument_comment_lint` convention:
  - Use an exact `/*param_name*/` comment before opaque literal arguments such as `None`, booleans,
    and numeric literals when passing them by position.
  - Do not add these comments for string or char literals unless the comment adds real clarity.
  - The parameter name in the comment must exactly match the callee signature.
- Do not run Bazel-related commands unless you are in CI.
- Do not run `just argument-comment-lint` unless you are in CI.
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- Newly added traits should include doc comments that explain their role and how implementations are
  expected to use them.
- When writing tests, prefer comparing the equality of entire objects over fields one by one.
- When making a change that adds or changes an API, ensure that the documentation in `docs/` is up
  to date when applicable.
- If you change `ConfigToml` or nested config types, run `just write-config-schema` to update
  `core/config.schema.json`.
- Do not create small helper methods that are referenced only once.
- Avoid large modules:
  - Prefer adding new modules instead of growing existing ones.
  - Target Rust modules under 500 LoC, excluding tests.
  - If a file exceeds roughly 800 LoC, add new functionality in a new module instead of extending
    the existing file unless there is a strong documented reason not to.
  - This applies especially to high-touch files that already attract unrelated changes, such as
    `tui/src/app.rs`, `tui/src/bottom_pane/chat_composer.rs`, `tui/src/bottom_pane/footer.rs`,
    `tui/src/chatwidget.rs`, and `tui/src/bottom_pane/mod.rs`.
  - When extracting code from a large module, move related tests and module or type docs toward the
    new implementation so the invariants stay close to the code that owns them.
- Resist adding code to `codex-core`. See `core/AGENTS.md` when working in or around that crate.

## Validation Workflow

- Optimize for a short local edit loop. Do not automatically run `just fmt`, `cargo check`,
  `cargo test`, `just test`, `just fix`, or other broad validation commands during routine local
  iteration.
- When you need to validate `codex-rs` changes locally, run this sequence from `codex-rs/`:
  1. `cargo build -p codex-cli`
  2. `bash install_local.sh`
  3. a PTY test that exercises the changed behavior
- Keep other checks and tests in the release or tag flow unless the user explicitly asks to run
  them earlier.
