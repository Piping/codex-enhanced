# `/loop` design

This document describes the current Rust TUI implementation of `/loop`.

## Design summary

`/loop` is implemented as a workspace-local scheduler owned by `App`. It supports:

- one-shot loops that keep firing on schedule, but use a fresh hidden ephemeral thread every run
- persistent loops that keep a stable id and resume their own hidden rollout

The implementation reuses the existing Codex thread runtime instead of inventing a second model
execution path.

The high-level split is:

- parsing and slash dispatch in the TUI input layer
- loop state, persistence, and scheduling in `tui/src/app/loop_timers.rs`
- loop execution permission policy in `tui/src/app/loop_execution.rs`
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
- `tui/src/app/loop_execution.rs`
  - owns per-loop execution overrides
  - resolves per-loop cwd and writable directories into runtime sandbox overrides
- `tui/src/app/loop_timers.rs`
  - owns persistence, scheduling, hidden execution, and mirroring

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
- per-loop `execution`
  - optional cwd override
  - writable-directory overrides
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

One-shot loops keep rescheduling future due times too. The difference is not the scheduler; it is
the execution context. Every one-shot trigger runs in a fresh hidden ephemeral thread and does not
reuse private rollout history from earlier runs.

Persistent loops keep rescheduling future runs unless disabled or deleted.

## Hidden execution threads

Every loop run uses the same hidden-session source tag:

```text
SessionSource::SubAgent(SubAgentSource::Other("loop"))
```

That keeps `/loop` executions separate from the visible main-thread workflow while still using the
normal Codex thread runtime.

No strict `/loop`-specific clamp is applied by default anymore. A loop starts from the main
thread's runtime config and then applies only its own explicit execution overrides.

If a loop has no execution overrides:

- it inherits the parent cwd
- it inherits the parent sandbox / approval policy
- it inherits the parent tool availability

If a loop has a cwd override:

- that override becomes the execution cwd for that loop only

If a loop has writable directories configured:

- the runtime switches that loop to `WorkspaceWrite`
- those directories become the only writable roots
- read-only access stays broad enough to inspect the rest of the workspace
- other loops still keep their own inherited or overridden policy

This keeps the execution model local to each loop instead of introducing one shared workspace-level
loop policy.

### One-shot runs

One-shot loops start a fresh hidden thread on every trigger with:

- `ephemeral = true`
- compact initial history forked from the current main-thread rollout

The hidden thread is destroyed after completion, but the timer record stays persisted so the next
scheduled trigger can start another fresh run.

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

- create loops from a guided submenu
- inspect configured loops
- open per-loop actions
- run a loop immediately
- edit prompt
- edit schedule
- edit action
- edit delivery mode
- edit execution settings
- enable / disable
- delete

`Create Loop Agent` opens a submenu with:

- `One-Shot Loop`
- `Persistent Loop`

Each loop action menu includes `Execution Settings`, which owns:

- a per-loop cwd override
- a per-loop writable-directory list
- reset actions back to the main-thread defaults

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
2. the hidden run is shut down and removed
3. `Loop Manager` is refreshed so the running marker disappears
4. a compact completion info cell is created for the main thread
5. the configured mirrored payload is appended as local TUI history
6. the effective per-loop delivery mode may enqueue a follow-up user turn

Only the latest final answer is mirrored. Intermediate hidden-thread transcript items stay private.

## Delivery modes

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

## Future extensions

Possible follow-up work:

- app-server parity
- retry policy and backoff
- structured delivery into the main execution layer
- multiple context-injection strategies beyond `latest 3 + original prompt`
