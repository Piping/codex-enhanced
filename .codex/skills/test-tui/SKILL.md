---
name: test-tui
description: Guide for testing Codex TUI interactively
---

You can start and use Codex TUI to verify changes. 

Important notes:

Start interactively.
Always set RUST_LOG="trace" when starting the process.
Pass `-c log_dir=<some_temp_dir>` argument to have logs written to a specific directory to help with debugging.
When sending a test message programmatically, send text first, then send Enter in a separate write (do not send text + Enter in one burst).
Use `just codex` target to run - `just codex -c ...`

<!-- codex:dream:start -->
## Dream Notes

## Interactive TUI verification notes

- For temp repos or fresh `CODEX_HOME`, preseed project trust in `CODEX_HOME/config.toml` before PTY verification, or the trust/onboarding flow can dominate the terminal output even with `--no-alt-screen`.
- On macOS temp dirs, add trust entries for both `/tmp/...` and `/private/tmp/...` forms of the repo path.
- When submitting a prompt programmatically, send the text first and then send Enter separately; sending them together is less reliable.
- For `/dream` PTY checks, expect the command to run in the background and then report updated memory/AGENTS/skill paths in the active thread.
<!-- codex:dream:end -->
