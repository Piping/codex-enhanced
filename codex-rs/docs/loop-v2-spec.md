# Loop V2 Spec

## Persistent Model

Each workspace stores:

- `.codex/loop/loop_timers.json`
  - loop definitions
- `.codex/loop/loop_trigger_queues.json`
  - workspace ordering for trigger phases

## Loop Agent

```rust
PersistedLoopTimer {
    id,
    prompt,
    action,
    enabled,
    mode,
    context_mode,
    response_mode,
    security_mode,
    execution,
    trigger_bindings,
    rollout_path,
    created_at_unix_seconds,
    last_scheduled_at_unix_seconds,
    last_completed_at_unix_seconds,
}
```

Notes:

- `mode` is retained for current runtime ownership of hidden state
- `context_mode` is the v2 semantic mode
- `trigger_bindings` are authoritative

## Trigger Bindings

```rust
LoopTriggerBinding {
    id,
    enabled,
    kind,
}
```

```rust
LoopTriggerKind =
    Timer { schedule }
  | BeforeTurn
  | AfterTurn
```

`manual` is always available as `Run Now` and is not persisted as a binding.

## Trigger Queue

```rust
PersistedLoopTriggerQueuesFile {
    queues: Vec<LoopTriggerQueue>,
}
```

```rust
LoopTriggerQueue {
    phase,
    entries,
}
```

```rust
LoopTriggerQueueEntry {
    loop_id,
    binding_id,
}
```

Queue phases:

- `timer`
- `before_turn`
- `after_turn`

## Ordering

When a phase fires:

1. look up the queue for that phase
2. walk queue entries from top to bottom
3. resolve `(loop_id, binding_id)`
4. skip missing, disabled, or invalid entries
5. execute matched loop triggers in order

## Response Semantics

- `as_assistant`: append assistant message
- `as_user`: queue a user message into the main thread
- empty loop completions are allowed implicitly; they simply produce no main-thread message

Loop-generated user messages do not re-trigger loop hooks.

## Agent Tooling

Loop agents are managed through the shared harness/service layer.
TUI Codex sessions also expose a model-visible `loop` function tool that forwards
create, list, info, update, and delete operations into that shared service.
The service writes workspace-local loop metadata into `.codex/loop/loop_timers.json`
and `.codex/loop/loop_trigger_queues.json`.

## Security Semantics

### Inherited

- inherit parent thread approvals
- inherit parent thread tool access
- inherit parent thread cwd unless overridden elsewhere

### Specified Directory

- inherit parent approvals and tool access
- constrain writable roots to configured directories
- optionally override cwd

## Hook Semantics

### Before Turn

- phase fires before a main-thread user turn is submitted
- loop may contribute additional user-context text
- runtime decides how to merge that contribution into the outgoing turn

### After Turn

- phase fires after the assistant final response completes

### Timer

- phase fires when a timer trigger becomes due
- if multiple timer triggers become due around the same time, queue ordering decides execution order
