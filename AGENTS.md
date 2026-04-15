# Repository Instructions

- Keep this file limited to repo-wide guidance.
- Rust workflow, validation, and architecture rules for `codex-rs/` live in `codex-rs/AGENTS.md`.
- More specific `codex-rs/` guidance lives in nested files, including:
  - `codex-rs/tui/AGENTS.md`
  - `codex-rs/core/AGENTS.md`
  - `codex-rs/app-server-protocol/AGENTS.md`
  - deeper directory-specific `AGENTS.md` files such as `codex-rs/tui/src/bottom_pane/AGENTS.md`
- When multiple `AGENTS.md` files apply, use the closest one for the directory you are editing.
- For requirement changes and bug fixes, always run a PTY-based validation that exercises the actual behavior before considering the work done.

<!-- codex:dream:start -->
## Dream Guidance

## Dream Guidance

### Paper artifact location
- When a session in this repo downloads or organizes papers, store them under `~/Documents/logseq/source/papers/` by default unless the user asks for a different destination.
- Treat this as the default landing path for local paper PDFs and related notes, rather than ad-hoc temp locations.
<!-- codex:dream:end -->
