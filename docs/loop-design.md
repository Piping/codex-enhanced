# `/loop` design

This document describes the current Rust TUI implementation of `/loop`.

## Design summary

`/loop` is implemented as a workspace-local scheduler owned by `App`. Each timer
creates hidden, ephemeral Codex threads on demand. The implementation reuses the
existing thread runtime instead of inventing a second model execution path.

The high-level split is:

- parsing and slash dispatch in the TUI input layer
- timer state, persistence, and scheduling in `tui/src/app/loop_timers.rs`
- runtime policy and thread creation through existing core thread APIs
- main-thread mirroring as TUI-only history cells

## Primary modules

- `tui/src/slash_command.rs`
  - declares `/loop`
- `tui/src/chatwidget.rs`
  - dispatches `/loop ...` into an app event
- `tui/src/app_event.rs`
  - defines loop management events
- `tui/src/app/loop_timers.rs`
  - owns parsing, persistence, scheduling, hidden execution, and mirroring
- `core/src/config/types.rs`
  - defines `tui.loop.completion_mirror_mode`
- `core/src/config/mod.rs`
  - loads the loop config into runtime `Config`

## Persistence model

Timers are persisted per workspace in:

```text
.codex/loop_timers.json
```

`LoopTimersState` keeps the in-memory view:

- `timers`
- scheduler tasks
- active hidden runs
- mirrored history cells keyed by thread id

Persistence is intentionally simple JSON because timers are small, local, and
workspace-scoped.

## Scheduling model

Two schedule forms are supported:

- interval
- cron

Intervals are parsed from compact tokens like `5m` or `1h30m`.

Cron expressions are normalized to the format expected by the `cron` crate:

- 5-field input becomes `0 <expr> *`
- 6-field input becomes `<expr> *`
- 7-field input is used as-is

Each timer has one scheduler task that sleeps until the next due time and then
sends `AppEvent::TriggerLoopTimer`.

## Hidden execution threads

Each run creates a new hidden thread with:

- `ephemeral = true`
- `approval_policy = never`
- read-only sandbox
- network disabled
- `include_apply_patch_tool = false`
- extra developer instructions that forbid side effects

The thread source is tagged as:

```text
SessionSource::SubAgent(SubAgentSource::Other("loop"))
```

That keeps `/loop` executions separate from the visible main-thread workflow
while still using the normal Codex thread runtime.

## Context model

`/loop` does not start from an empty world. It derives a compact initial
history from the current rollout:

- keep `SessionMeta` when available
- keep the latest `TurnContext` when available
- take a tail of recent rollout items up to a fixed token budget

The current budget is approximately 2000 tokens, estimated from serialized item
size.

This is a deliberate compromise:

- enough recent context for scheduled prompts to stay relevant
- small enough to avoid replaying the full conversation every time

## Result handling

The hidden thread listener watches for:

- completed agent messages
- turn completion
- error events

When the run finishes:

1. the timer completion timestamp is persisted
2. the hidden run is shut down and removed
3. a compact info cell is created for the main thread
4. the configured mirrored payload is appended as local TUI history

Only the latest final answer is mirrored. Intermediate hidden-thread transcript
items stay private.

## Why mirroring happens in the TUI layer

The mirror cells are presentation decisions, not core rollout facts.

That is why `/loop` records them as TUI-managed history cells instead of trying
to inject the full hidden execution transcript into the rollout. This preserves
the invariant that scheduled background work stays isolated unless the UI chooses
to expose a compact result.

## Current tradeoffs

- TUI-only: no parallel app-server implementation yet
- hidden runs are one-shot threads, not reusable workers
- config is intentionally small
- result mirroring is transcript-oriented, not a structured data channel

## Future extensions

Possible follow-up work:

- add app-server parity
- add “run now” from `Loop Manager`
- add per-timer labels instead of showing only prompt prefixes
- add structured delivery targets beyond the main transcript
