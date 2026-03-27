# Loop V2 Design

## Architecture

Loop v2 is split into three layers:

1. `codex-loop`
   - data model
   - parser
   - trigger queue persistence
   - validation helpers
2. native TUI `Loop Manager`
   - forms
   - menus
   - queue ordering
   - user-visible loop state
3. `codex-loop-runtime`
   - execution setting normalization
   - runtime prompt/input construction
   - reusable runtime helpers shared by loop surfaces
4. loop runtime orchestration
   - timer scheduling
   - hidden-thread lifecycle
   - before-turn and after-turn hooks

## Source of Truth

The design intentionally avoids compatibility shims:

- trigger bindings are authoritative for what can fire
- trigger queue files are authoritative for cross-loop ordering
- runtime should not reconstruct triggers from older timer-only fields

## Runtime Modes

### Embed

Runs in the main-thread execution path.

Typical uses:

- before-turn prompt steering
- after-turn lightweight follow-up checks
- timer-driven main-thread automation

Risk:

- `timer + embed` can effectively automate the main thread, so the UI must surface that clearly

### Ephemeral

Runs in a hidden thread with no retained rollout.

Typical uses:

- short-lived checks
- one-off timer work
- hidden status inspection

### Persistent

Runs in a hidden thread with a retained rollout.

Typical uses:

- long-lived directors
- background managers
- loop agents that accumulate private state over time

## Response Delivery

Loop v2 keeps the response mode explicit:

- `as_assistant`
- `as_user`

Empty loop completions are still valid; they simply do not inject a main-thread message.

## Queue Synchronization Rules

When a trigger binding is added:

- append a queue entry to the corresponding phase queue

When a trigger binding is deleted:

- remove its queue entry

When a trigger binding changes phase:

- remove it from the old queue
- append it to the new queue

When a loop is deleted:

- remove all queue entries that reference that loop

## UI Design

`Loop Manager` stays the top-level menu name.

Top-level actions:

- create loop agent
- trigger queue
- loop list

Per-loop trigger editing is local:

- add trigger
- edit trigger
- enable/disable trigger
- delete trigger

Global ordering is separate:

- trigger queue
  - timer
  - before turn
  - after turn

This separation keeps loop ownership and workspace execution policy distinct.

## Future Backend Service

`claw gateway` should be a separate backend service that can reuse:

- trigger definitions
- queue ordering
- forwarding rules
- response routing

It should not be embedded into the first loop v2 runtime rollout.
