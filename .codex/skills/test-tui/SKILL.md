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
For this repo, run `bash codex-rs/install_local.sh` before PTY verification so `/usr/local/bin/codex` is signed and overwritten with the latest debug build.
Prefer validating the installed `codex` path, not `target/debug/codex`, when the scenario involves `/respawn` or any self-reexec flow.
Use `just codex` target to run - `just codex -c ...`
