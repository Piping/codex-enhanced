# `/loop` spec

This document describes the user-visible behavior of `/loop` in the Rust TUI.

## Scope

- Applies to `codex-rs/tui`
- Loops are workspace-local
- Scheduled executions run in hidden threads
- Main-thread mirroring is configurable
- `/loop` supports both one-shot and persistent loop agents

## Command syntax

`/loop` accepts either:

- a one-shot schedule followed by a prompt
- a persistent loop id, then a schedule, then a prompt
- an existing persistent loop id on its own

Examples:

```text
/loop 5m summarize what changed in this repo
/loop 1h30m check for flaky test patterns
/loop */15 * * * * summarize recent failures
/loop director 30m review overall progress and decide next priorities
/loop director
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

## Loop lifecycle

When the user creates a loop:

1. Codex parses the command as either one-shot or persistent.
2. The loop is persisted to the workspace-local timer file.
3. The next due time is computed from the schedule.
4. Execution starts only when that due time arrives, or when the user selects `Run Now`.

Persistent loops keep a stable id and their own hidden-thread rollout so later runs can resume the
same private context. One-shot loops stay scheduled too, but each trigger creates a fresh
temporary hidden thread and does not keep private rollout history between runs.

Existing loops are reloaded when the TUI opens in the same workspace.

## Control panel

`Ctrl-P` includes `Loop Manager`.

`Loop Manager` shows:

- `Create Loop Agent` as the first item
- the loop id for persistent loops
- the prompt prefix for one-shot loops
- the schedule
- whether the loop is one-shot or persistent
- whether the loop is currently running
- whether the loop is disabled
- the last completion time when available

`Create Loop Agent` opens a create submenu with:

- `One-Shot Loop`
- `Persistent Loop`

Per-loop actions:

- `Run Now`
- `Edit Prompt`
- `Edit Schedule`
- `Delivery Mode`
- `Edit Action`
- `Execution Settings`
- `Enable`
- `Disable`
- `Delete`

`Execution Settings` is scoped to the selected loop. It lets the user manage:

- a per-loop cwd override
- a per-loop writable-directory override
- reset actions that return both values to the main-thread defaults

`Run Now` still works when a loop is disabled. It triggers one immediate execution without
re-enabling future scheduled runs.

## Execution policy

Each scheduled run uses a hidden execution thread.

By default, a loop inherits the main thread runtime:

- the same cwd
- the same sandbox / approval policy
- the same tool availability

`Loop Manager -> Execution Settings` can override that per loop.

If a loop has a cwd override:

- that loop uses the configured cwd
- other loops continue using their own cwd or the session default

If a loop has writable directories configured:

- writes are allowed only inside those directories
- the loop switches to workspace-write mode for those roots
- the rest of the workspace remains read-only to that loop
- other loops are unaffected

Leaving the writable-directory list empty means the loop inherits the main-thread sandbox scope.

One-shot loops start a fresh hidden thread from compact main-thread context on every trigger.

Persistent loops resume their own hidden rollout when available. For every trigger, Codex submits a
new user turn that contains:

1. the latest 3 main-thread user/assistant messages
2. the original loop prompt

This order is intentional so the newest external context arrives first while the original loop
objective is repeated every run to reduce drift.

## Main-thread mirroring

When a scheduled run starts, the main thread receives a compact info message that
the loop is running in the background.

Example shape:

```text
Loop director (30m) is running in background: review overall progress...
```

After a successful run, the main thread always receives a compact info message:

- the loop id or id prefix
- the schedule when available
- a prompt prefix

Example shape:

```text
Loop director (30m) ran: review overall progress...
```

Then Codex mirrors main-thread transcript cells using the current code-default mirror behavior.

Hidden execution history is never mirrored back into the main thread.

## Follow-up delivery

Each loop stores its own delivery mode in `Loop Manager -> Delivery Mode`. The code-default mode is
assistant-only.

Supported values:

- `assistant-only`
  - mirror the latest loop result into the main thread as an assistant message
  - do not auto-submit a follow-up user message
- `result-as-user`
  - submit the latest loop result itself as a new user message
- `assistant-then-action-user`
  - submit the latest loop result as a new user message
  - if the loop has an `action`, append that action text at the end of the same user message

## Persistence

Loops are stored in the workspace-local file:

```text
.codex/loop_timers.json
```

Each loop stores:

- id
- mode
- prompt
- optional action
- optional delivery mode override
- per-loop execution settings
  - optional cwd override
  - writable-directory overrides
- schedule
- enabled state
- optional hidden rollout path for persistent loops
- creation time
- last scheduled time
- last completed time

## Error behavior

- Invalid syntax shows a TUI error message.
- Failed scheduled runs show a TUI error message.
- Looking up an unknown persistent loop id shows a TUI error message.
- Missing or invalid loop records are ignored or rejected without crashing the TUI.

## Current exclusions

- No app-server API for `/loop`
- No dedicated retry policy
