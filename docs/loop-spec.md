# `/loop` spec

This document describes the user-visible behavior of `/loop` in the Rust TUI.

## Scope

- Applies to `codex-rs/tui`
- Timers are workspace-local
- Scheduled executions run in hidden threads
- Main-thread mirroring is configurable

## Command syntax

`/loop` accepts either:

- an interval followed by a prompt
- a cron expression followed by a prompt

Examples:

```text
/loop 5m summarize what changed in this repo
/loop 1h30m check for flaky test patterns
/loop */15 * * * * summarize recent failures
```

Accepted interval units:

- `s`
- `m`
- `h`
- `d`

Intervals may be combined in one token, for example `1h30m`.

Accepted cron formats:

- 5-field cron
- 6-field cron
- 7-field cron

## Timer lifecycle

When the user creates a timer:

1. Codex parses the schedule and prompt.
2. The timer is persisted to the workspace-local timer file.
3. The timer is immediately scheduled.
4. The first execution is triggered right away.

Existing timers are reloaded when the TUI opens in the same workspace.

## Control panel

`Ctrl-P` includes `Loop Manager`.

`Loop Manager` shows:

- the timer prompt
- the schedule
- whether the timer is disabled
- the last completion time when available

Per-timer actions:

- `Enable`
- `Disable`
- `Delete`

## Execution policy

Each scheduled run uses a hidden execution thread with these constraints:

- approval policy is forced to `never`
- sandbox is forced to read-only
- network access is disabled
- `apply_patch` is disabled
- the prompt is treated as read-only scheduled work

The execution thread is not intended to perform side effects.

## Main-thread mirroring

After a successful run, the main thread always receives a compact info message:

- the loop id prefix
- the schedule when available
- a prompt prefix

Example shape:

```text
Loop 1234abcd (5m) ran: summarize what changed...
```

Then Codex mirrors one of two payloads, controlled by config:

- `prompt-and-response`
  - info message
  - `/loop <prompt>` user cell
  - latest assistant final message
- `response-only`
  - info message
  - latest assistant final message

Hidden execution history is never mirrored back into the main thread.

## Persistence

Timers are stored in the workspace-local file:

```text
.codex/loop_timers.json
```

Each timer stores:

- id
- prompt
- schedule
- enabled state
- creation time
- last scheduled time
- last completed time

## Configuration

`[tui.loop]` currently supports:

```toml
[tui.loop]
completion_mirror_mode = "prompt-and-response"
```

Supported values:

- `prompt-and-response`
- `response-only`

## Error behavior

- Invalid syntax shows a TUI error message.
- Failed scheduled runs show a TUI error message.
- Missing or invalid timer records are ignored or rejected without crashing the
  TUI.

## Current exclusions

- No app-server API for `/loop`
- No dedicated retry policy
- No per-timer custom sandbox/tool policy
- No manual “run now” action
