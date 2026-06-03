# Codex Enhanced

`codex-enhanced` packages the Codex CLI as a platform-specific Python wheel.
It is intended for users who want to install the enhanced CLI with `pip` and
run it directly without managing a separate native binary release.

Website: `https://codex-enhanced.com`

## Highlights

- Loop automation with clear context modes:
  - `embed`: submit the loop prompt into the main thread as a normal user turn
  - `ephemeral`: fork compacted context for one run, then discard it
  - `persistent`: keep a private retained context and refresh it with recent
    main-thread messages
- Fast `respawn` support so the CLI can restart and resume the current session

## Install

```bash
pip install codex-enhanced
```

## Run

```bash
codex-enhanced
```

Each wheel includes the native `codex` binary for its target platform. This
package is wheel-only and is not intended to publish a source distribution.
