# `/loop` design

This document describes the current Rust TUI implementation of `/loop`.

## Design summary

`/loop` is implemented as a workspace-local scheduler owned by `App`. It supports:

- one-shot loops that run once in a hidden ephemeral thread
- persistent loops that keep a stable id and resume their own hidden rollout

The implementation reuses the existing Codex thread runtime instead of inventing a second model
execution path.

The high-level split is:

- parsing and slash dispatch in the TUI input layer
- loop state, persistence, and scheduling in `tui/src/app/loop_timers.rs`
- slash parsing helpers in `tui/src/app/loop_timer_command.rs`
- runtime policy and thread creation through existing core thread APIs
- main-thread mirroring as TUI-only history cells

## Primary modules

- `tui/src/slash_command.rs`
  - declares `/loop`
- `tui/src/chatwidget.rs`
  - dispatches `/loop ...` into an app event
  - opens prompt and schedule editors from `Loop Manager`
- `tui/src/app_event.rs`
  - defines loop management events
- `tui/src/app/loop_timer_command.rs`
  - parses one-shot, persistent-create, and persistent-focus forms
  - defines persisted per-loop delivery modes
- `tui/src/app/loop_timers.rs`
  - owns persistence, scheduling, hidden execution, and mirroring
- `core/src/config/types.rs`
  - defines `tui.loop.completion_mirror_mode`
- `core/src/config/mod.rs`
  - loads the loop mirror config into runtime `Config`

## Persistence model

Loops are persisted per workspace in:

```text
.codex/loop_timers.json
```

`LoopTimersState` keeps the in-memory view:

- `timers`
- scheduler tasks
- active hidden runs
- mirrored history cells keyed by thread id

Each persisted loop stores:

- `id`
- `mode`
- `prompt`
- optional `action`
- optional `delivery_mode` override
- `schedule`
- `enabled`
- `rollout_path`
- creation / last-scheduled / last-completed timestamps

Persistence stays simple JSON because loops are small, local, and workspace-scoped.

## Scheduling model

Two schedule forms are supported:

- interval
- cron

Intervals are parsed from compact tokens like `5m` or `1h30m`.

Cron expressions are normalized to the format expected by the `cron` crate:

- 5-field input becomes `0 <expr> *`
- 6-field input becomes `<expr> *`
- 7-field input is used as-is

Each loop has one scheduler task that sleeps until the next due time and then
sends `AppEvent::TriggerLoopTimer`.

One-shot loops schedule only their first due time. After that single trigger completes, the loop
record is removed.

Persistent loops keep rescheduling future runs unless disabled or deleted.

## Hidden execution threads

Every loop run uses the same hidden-session source tag:

```text
SessionSource::SubAgent(SubAgentSource::Other("loop"))
```

That keeps `/loop` executions separate from the visible main-thread workflow while still using the
normal Codex thread runtime.

All loop runs force the same policy overrides:

- `approval_policy = never`
- read-only sandbox
- `network_access = false`
- `include_apply_patch_tool = false`
- extra developer instructions that forbid side effects

### One-shot runs

One-shot loops start a fresh hidden thread with:

- `ephemeral = true`
- compact initial history forked from the current main-thread rollout

The hidden thread is destroyed after completion.

### Persistent runs

Persistent loops start with the same compact initial history on their first run, but after that they
store and reuse a hidden `rollout_path`.

Later runs:

- resume that hidden rollout through `resume_thread_from_rollout`
- submit a new user turn into the same private loop thread

This is what gives persistent loops their own long-lived context.

## Context model

There are two context sources:

1. the loop's own private hidden-thread history
2. the latest 3 main-thread user/assistant messages

The main-thread tail is loaded fresh at trigger time from the current primary rollout. It is not
written into loop persistence directly; instead it is injected into the next submitted user turn.

The submitted text is intentionally ordered as:

1. recent main-thread messages
2. the original loop prompt

The repeated prompt acts as an anchor so a persistent loop can keep its objective even after many
private turns.

## Loop Manager

`Ctrl-P -> Loop Manager` is the management surface for existing loops.

It can:

- inspect configured loops
- open per-loop actions
- run a loop immediately
- edit prompt
- edit schedule
- edit action
- edit delivery mode
- enable / disable
- delete

The slash command stays lightweight:

- `/loop <time> <prompt>` creates one-shot
- `/loop <id> <time> <prompt>` creates or updates persistent
- `/loop <id>` focuses an existing persistent loop by opening its action menu

## Result handling

The hidden thread listener watches for:

- completed agent messages
- turn completion
- error events

When the run starts:

1. the hidden thread is registered in `active_runs`
2. `Loop Manager` can render that loop as `running now`
3. a compact background-progress info cell is mirrored into the main thread

When the run finishes:

1. timestamps are updated and persisted
2. one-shot loops are removed from persistence
3. the hidden run is shut down and removed
4. `Loop Manager` is refreshed so the running marker disappears
5. a compact completion info cell is created for the main thread
6. the configured mirrored payload is appended as local TUI history
7. the effective per-loop delivery mode may enqueue a follow-up user turn

Only the latest final answer is mirrored. Intermediate hidden-thread transcript items stay private.

## Delivery modes

The loop runtime has two separate knobs:

- `completion_mirror_mode`
- per-loop `delivery_mode`

`completion_mirror_mode` controls what transcript cells appear locally in the main thread.

`delivery_mode` controls what gets submitted back into the main thread as a new user turn. It is
stored per loop, and defaults in code to `assistant-only` when no override is set:

- `assistant-only`
  - mirror the loop result back as an assistant message
  - do not auto-submit a follow-up user turn
- `result-as-user`
  - submit the loop result itself as a user message
- `assistant-then-action-user`
  - submit the loop result as a user message
  - if the loop has an action, append that action text at the end

This split keeps transcript presentation separate from follow-up execution semantics.

## Why mirroring happens in the TUI layer

The mirror cells are presentation decisions, not core rollout facts.

That is why `/loop` records them as TUI-managed history cells instead of trying to inject the full
hidden execution transcript into the rollout. This preserves the invariant that scheduled background
work stays isolated unless the UI chooses to expose a compact result.

## Current tradeoffs

- TUI-only: no parallel app-server implementation yet
- overlap policy is effectively `skip` because a running loop ignores duplicate triggers
- loop-to-main delivery is still transcript-oriented, not a structured action channel
- delivery still feeds back as plain user text, not a typed execution payload
- permissions are fixed rather than per-loop configurable

## Future extensions

Possible follow-up work:

- app-server parity
- retry policy and backoff
- structured delivery into the main execution layer
- per-loop permission presets
- multiple context-injection strategies beyond `latest 3 + original prompt`
