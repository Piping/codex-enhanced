# Loop V2 Proposal

## Goal

Promote `/loop` from a timer-only helper into a general automation runtime with:

- multiple trigger kinds
- explicit context modes
- explicit response modes
- per-loop execution/security settings
- workspace-level trigger ordering

`Loop Manager` remains the single TUI surface for loop creation, editing, and execution order.

## Scope

Loop v2 covers:

- triggers:
  - `manual`
  - `timer`
  - `before_turn`
  - `after_turn`
- context modes:
  - `embed`
  - `ephemeral`
  - `persistent`
- response modes:
  - `as_assistant`
  - `as_user`
- security modes:
  - `inherited`
  - `specified_directory`

Out of scope for phase 1:

- channel transport backends such as `claw gateway`
- cross-device or remote loop management APIs

## Key Decisions

### Loop-local triggers, workspace-global ordering

A loop owns its trigger bindings, but it does not own global trigger order.

- loop config can add, edit, enable, disable, and delete its own trigger bindings
- workspace `Trigger Queue` controls cross-loop ordering for each trigger phase

### Fail-fast model changes

Loop v2 does not try to preserve silent compatibility with older timer-only loop files.

- trigger bindings are the source of truth
- queue files are the source of truth for cross-loop ordering
- old timer-only shape is not auto-upgraded via hidden fallback logic

### Context modes

- `embed`: execute directly in the main-thread context
- `ephemeral`: execute in a hidden short-lived thread
- `persistent`: execute in a hidden long-lived thread with its own rollout

### Response modes

- empty results are valid and simply do not inject a main-thread message
- `as_assistant` mirrors the loop result as an assistant message
- `as_user` submits the loop result as a user message

Loop-generated user submissions must not recursively trigger loops again.

### Security modes

- `inherited`: follow the parent thread execution policy
- `specified_directory`: inherit the parent policy, but constrain file writes to configured roots and allow a per-loop cwd override

## TUI Surfaces

`Loop Manager` contains:

- `Create Loop Agent`
- `Trigger Queue`
- loop list

Per-loop actions contain:

- prompt
- action
- context mode
- response mode
- security mode
- execution settings
- triggers
- run now
- enable/disable
- delete

`Trigger Queue` contains:

- timer queue
- before turn queue
- after turn queue

Each queue supports reordering across loops.
