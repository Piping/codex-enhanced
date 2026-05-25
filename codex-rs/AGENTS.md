# codex-rs Agent Notes

This file keeps durable, repo-specific guidance for future Codex work in `codex-rs`.

Detailed task history and validation logs live in [progress.md](codex-rs/progress.md).

## Working Agreement

- Keep `AGENTS.md` focused on stable rules, recurring traps, and reusable checklists.
- Record task-by-task execution details, temporary findings, and validation transcripts in `progress.md`.
- When a progress entry reveals a recurring pattern, promote the durable part back into this file.
- For newly added code, do not introduce ad hoc top-level files into an existing module tree.
- New functionality should either:
  - live in a dedicated crate, or
  - live under the owning module's `enhanced/` subtree, for example `codex-rs/tui/src/enhanced/...`.

## Release And CI Notes

- Before creating or pushing a `codex-enhanced` release tag, use `just release-codex-enhanced <version>` from the repo root. Do not edit `sdk/python-runtime-enhanced/pyproject.toml` by hand; the recipe updates the version, commits it, tags it, and pushes both refs together.
- For future `codex-enhanced` version bumps, increment only the patch component; keep major and minor unchanged.
- When cutting a release tag, also update the workspace package `version` in `codex-rs/Cargo.toml` so `codex --version` matches the release tag.
- The `pypi-release` workflow uses concurrency on `github.ref_name || inputs.release_tag` with `cancel-in-progress: true`.
- A manual `workflow_dispatch` for the same release tag cancels an in-progress tag-triggered run; treat that as expected behavior, not a separate failure.
- If a release rerun only needs publish or GitHub Release recovery, check whether `artifact_run_id` can reuse a prior successful artifact build instead of rebuilding every platform.

## Validation Reminders

- After Rust changes, keep the existing local validation chain: `just fmt`, `cargo build -p codex-cli`, the relevant crate tests, and scoped `just fix -p <crate>` when the change is large enough to justify it.
- Before PTY verification with the local binary, run `install target/debug/codex ~/.local/bin/` from `codex-rs/` so the debug binary is installed
- On the current local `nightly + cranelift` setup for macOS arm64, full `cargo test -p codex-tui` can still abort in CRC32-heavy paths with `llvm.aarch64.crc32b is not yet supported`; treat that as a backend limitation unless the failing stack clearly points at the change under review.
- Treat long-running GitHub Actions release matrix jobs as normal unless logs or failed steps show otherwise; `gh run watch` alone is not enough to distinguish slow compile time from a real hang.
