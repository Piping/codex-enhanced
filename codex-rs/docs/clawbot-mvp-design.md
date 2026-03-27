# Clawbot MVP Design

## Proposal

Add a new fork-owned crate, `codex-clawbot`, to host the runtime, provider
adapters, workspace state, and thread-bridge logic for IM-driven Codex
conversations.

The first supported provider is Feishu private chat. The first supported host
surface is `codex-rs/tui` only.

This crate should not be folded into `codex-ext`.

Reason:

- `codex-ext` is currently the fork-owned extension data/model layer.
- Clawbot is a live runtime with network connections, reconnection, queueing,
  workspace-local persistence, and TUI integration.
- Mixing those concerns would make both crates harder to evolve and test.

`codex-clawbot` should own:

- provider-agnostic session and binding models
- Feishu gateway runtime
- workspace-local state under `.codex/clawbot/`
- inbound/outbound queue orchestration
- thread binding and message routing decisions

`codex-ext` can stay focused on long-lived extension-facing models.

## Situation

Current repository capabilities relevant to this MVP:

- `codex-rs/tui` already has a fork-owned `Ctrl-P` control panel.
- The fork already manages workspace-level operational state and fork-owned UI
  affordances.
- Thread operations already exist in TUI, but there is no IM/provider bridge.
- The repository includes a standalone Python Feishu bot reference under
  `feishu_bot/`, with a gateway/worker split and a Feishu WS adapter.

User-confirmed product constraints:

- MVP host surface: local `tui` only
- MVP provider: Feishu only
- IM mode: private chat only
- one IM session binds to one Codex thread
- binding target for MVP: current thread only
- unbound sessions cache unread messages until manually connected
- after binding, cached unread messages are flushed into the thread in order
- only final assistant output is forwarded to Feishu
- failures should return an error message to Feishu
- no approval flow; assume full permission
- local and remote inputs can both target the same thread, but must queue
- local interrupt actions may cancel the running turn
- persistence should live under `workspace/.codex`
- restart behavior only needs to resume bindings and accept new messages; it
  does not need to backfill missed history

## Task

Build a minimally correct clawbot runtime that:

- discovers Feishu private-chat sessions
- lets the operator manually connect a session to the current thread
- flushes cached unread messages into that thread in order
- automatically forwards later final answers from that thread back to the bound
  Feishu session
- survives Feishu WS reconnects and process restarts without losing the binding
  model

## First Principles

The core system is not "a new control panel menu item".

The core system is:

- an external session source
- a persisted mapping from external session to Codex thread
- a serialized queue that turns external messages into user turns
- a reverse path that turns final assistant output into provider replies

The control panel is only the operator surface for that runtime.

That leads to two architectural decisions:

1. The source of truth must live in a runtime/store layer, not in TUI popup
   state.
2. Provider-specific code must be isolated behind a small adapter boundary so
   future Slack/Weixin support does not rewrite the thread-binding semantics.

## Why `codex-clawbot`

Recommended new workspace member:

- `codex-rs/clawbot`

Recommended crate name:

- `codex-clawbot`

Why a separate crate:

- keeps Feishu SDK coupling out of `codex-tui`
- keeps runtime/network code out of `codex-ext`
- gives the fork a clear place for provider adapters and binding state
- allows unit testing the runtime and queue model without TUI harnesses
- keeps future provider growth additive

## Feishu Reference Mapping

The Python reference in `feishu_bot/` is useful mainly for transport and
runtime structure, not for direct feature parity.

Reference takeaways:

- `feishu_bot/design.md` separates transport from business logic
- `feishu_bot/src/feishu_bot/runtime/gateway.py` owns the single WS connection
  and converts raw SDK callbacks into normalized payloads
- the Python design forwards events sequentially into a worker

Rust MVP mapping:

- keep the "normalize provider events early" idea
- keep a single Feishu WS owner inside `codex-clawbot`
- replace the Python worker subprocess with an in-process queue/dispatcher
- replace command routing with thread binding + turn submission

What we should not copy:

- no gateway/worker split for MVP
- no command parser
- no card workflows
- no mention gating requirement for private chat

## High-Level Architecture

```text
codex-tui
  -> control panel / clawbot panel
  -> current thread context

codex-clawbot
  -> runtime
  -> provider::feishu
  -> store
  -> binding registry
  -> thread bridge
  -> outbound forwarder

workspace/.codex/clawbot
  -> config
  -> discovered sessions
  -> bindings
  -> unread cache
  -> runtime state
```

### Integration Boundary

`codex-tui` should depend on `codex-clawbot` for:

- session list/state snapshots
- connect/disconnect actions
- cached unread counts
- runtime status
- notifications back into TUI when session state changes

`codex-clawbot` should not depend on popup-specific TUI types.

The integration boundary should use plain Rust structs and events.

## Module Layout

Recommended initial layout:

```text
codex-rs/clawbot/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── config.rs
    ├── model.rs
    ├── store.rs
    ├── runtime.rs
    ├── events.rs
    ├── binding.rs
    ├── queue.rs
    ├── bridge.rs
    ├── provider/
    │   ├── mod.rs
    │   └── feishu.rs
    └── tests/
```

Suggested responsibilities:

- `config.rs`
  Loads workspace-local clawbot config from `.codex/clawbot/config.toml` or
  similar.
- `model.rs`
  Shared data structures: provider id, session id, binding id, unread message,
  runtime status.
- `store.rs`
  Persistence for discovered sessions, bindings, unread cache, and restart
  metadata.
- `runtime.rs`
  Top-level runtime lifecycle, startup, reconnect, and event fan-in/fan-out.
- `events.rs`
  Internal event enums flowing between provider, queue, bridge, and TUI.
- `binding.rs`
  Session-thread mapping logic and invariants.
- `queue.rs`
  Per-thread FIFO queue, drain policy, and interrupt coordination.
- `bridge.rs`
  Converts provider messages into thread submissions and final assistant output
  into provider replies.
- `provider/feishu.rs`
  Feishu-specific WS client, message normalization, and outbound message send.

## Persistence Model

All clawbot state should live under:

```text
<workspace>/.codex/clawbot/
```

Recommended files:

```text
.codex/clawbot/
├── config.toml
├── sessions.json
├── bindings.json
├── unread_messages.jsonl
└── runtime.json
```

Suggested contents:

- `config.toml`
  provider credentials and runtime flags
- `sessions.json`
  discovered provider sessions and connection metadata
- `bindings.json`
  persisted `session -> thread` mapping
- `unread_messages.jsonl`
  cached inbound text messages for unbound sessions
- `runtime.json`
  last-known runtime metadata useful for UI recovery

MVP persistence rules:

- bindings must survive restart
- discovered sessions may survive restart best-effort
- unread cache must survive restart
- reconnect only needs to accept new messages after startup
- no history backfill is required

## Core Data Model

Recommended normalized entities:

### ProviderSession

- `provider`: `"feishu"`
- `session_id`
- `display_name`
- `status`
  values: `discovered`, `connected`, `disconnected`, `error`
- `unread_count`
- `last_message_at`
- `last_error`

For Feishu private chat, `session_id` should be stable and provider-owned. In
practice this is likely the chat id.

### SessionBinding

- `provider`
- `session_id`
- `thread_id`
- `bound_at`
- `state`
  values: `active`, `paused`, `error`

MVP invariant:

- one session maps to at most one thread
- one binding routes automatically once active

### CachedUnreadMessage

- `provider`
- `session_id`
- `message_id`
- `text`
- `received_at`

MVP message payload is intentionally minimal:

- text only
- no attachment support
- no quoted reply reconstruction
- no sender identity injected into the user message body

## Thread Binding Semantics

MVP binding rule:

- the operator selects a discovered Feishu private-chat session
- the operator chooses `Connect To Current Thread`
- the current thread becomes the routing target for that session

After binding:

- all cached unread messages for that session are flushed into the queue in
  order
- all future inbound messages for that session become queued user turns for the
  bound thread
- final assistant output from that thread is forwarded to that same session

This is a persistent binding model, not a global "active session redirect"
toggle.

That is the simplest model that satisfies the user requirement:

- "one Feishu session corresponds to one Codex thread"
- "current model input/output is redirected"

In implementation terms, the redirect is achieved by the binding itself.

## Queue and Turn Model

We need one serialized queue per bound thread.

Sources of work:

- flushed cached unread messages
- new inbound Feishu messages
- optional local TUI submissions targeting the same thread

Queue rules:

- enqueue inbound provider messages in arrival order
- if a thread is idle, start draining immediately
- if a turn is running, keep later messages queued
- only one queued provider message may actively drive a turn at a time
- local interrupt actions may cancel the active turn
- once a turn finishes, dequeue the next message

MVP simplification:

- remote input always uses normal user turns
- no special "steer" path
- no streaming output forwarding
- only final output or failure is forwarded

## Message Transformation

Inbound Feishu message to Codex:

- extract plain text only
- create one queued user turn with that text

Outbound Codex to Feishu:

- wait for final assistant message
- send final text reply to bound session

Failure path:

- if turn submission fails, send a provider error reply
- if the model/tool run ends in failure, send a provider error reply
- if provider send fails, mark session/binding status as error in runtime state

Recommended error text shape:

- short, operator-readable, not raw stack traces
- include enough detail to distinguish user error, provider send failure, and
  Codex runtime failure

## Feishu Adapter Scope

MVP Feishu adapter responsibilities:

- open and maintain WS connection
- normalize private-chat message events into `ProviderEvent::InboundText`
- expose connection status to runtime/UI
- send plain text outbound replies

MVP Feishu adapter non-goals:

- cards
- file/image handling
- group chats
- callbacks/actions
- history sync
- mention parsing

The Python reference already shows the right normalization point: transport
callbacks should be converted into internal payloads as early as possible.

## TUI Surface

Add a new control-panel item in `codex-rs/tui`:

- `Clawbot`

Suggested initial panel content:

- runtime connection status
- list of discovered Feishu private-chat sessions
- unread count per session
- current binding status
- last error if present

Suggested MVP actions:

- `Connect To Current Thread`
- `Disconnect`
- `Flush Cached Messages`
- `Retry Connection`

Suggested selection subtitle examples:

- unbound session:
  `Feishu private chat with 3 unread messages waiting for connection.`
- bound session:
  `Bound to the current thread and routing final answers automatically.`
- error session:
  `Last send or runtime error needs operator attention.`

## Why Not Route Through app-server

For this MVP, do not build the first version on top of an internal
app-server-websocket client.

Reasons:

- local `tui` is the only confirmed surface
- app-server websocket transport is documented as experimental / unsupported
- adding an extra RPC hop increases complexity before we have a stable binding
  model
- the real problem here is queueing and thread binding, not remote transport

If we later need a remote clawbot service, we can move the bridge boundary to
app-server after the local runtime semantics are proven.

## Integration Points in `codex-tui`

Expected host integration areas:

- control panel item registration
- session list popup construction
- app event additions for clawbot actions
- notification hooks when a thread finishes and final assistant output is ready
- lifecycle startup/shutdown hooks for the workspace runtime

Keep TUI integration thin:

- TUI dispatches operator intents
- runtime owns session state and routing

## Testing Plan

Unit tests in `codex-clawbot`:

- binding invariants
- unread cache flush order
- queue drain order
- reconnect status transitions
- restart recovery from persisted bindings
- provider error to runtime state mapping

TUI tests in `codex-tui`:

- control panel includes clawbot entry
- clawbot session list snapshots
- session status rendering snapshots
- connect/disconnect action wiring

Mock/provider tests:

- fake Feishu inbound text event normalization
- outbound final reply formatting
- duplicate message id handling if we later choose to dedupe

MVP note:

- the current user requirement does not need strong dedupe semantics across
  restart/backfill, so duplicate-prevention can stay local to a running process
  in phase 1

## Implementation Phases

### Phase 1: Crate and Models

- add `codex-rs/clawbot`
- register `codex-clawbot` in workspace
- define config/model/store/runtime skeleton
- add provider abstraction with Feishu placeholder

### Phase 2: Workspace State and Runtime

- implement workspace-local persistence under `.codex/clawbot`
- implement runtime startup/shutdown and state snapshots
- surface runtime status for TUI

### Phase 3: Feishu Private Chat Adapter

- implement Feishu WS connection
- normalize inbound private-chat text messages
- implement plain-text outbound replies
- implement reconnect status transitions

### Phase 4: Thread Binding and Queues

- implement `Connect To Current Thread`
- persist session-thread bindings
- cache unread messages for unbound sessions
- flush unread messages after binding
- implement per-thread serialized drain

### Phase 5: TUI Control Panel

- add `Clawbot` entry in `Ctrl-P`
- add session list panel
- add connect/disconnect/retry actions
- add session status snapshots

### Phase 6: Final Answer Routing

- hook final assistant output for bound threads
- forward final text to Feishu
- return runtime/model failures as Feishu error text

## Deferred

- `tui_app_server` parity
- non-Feishu providers
- group chat
- cards
- attachments/images/files
- sender identity injection into the thread
- approval flows in IM
- app-server-hosted clawbot runtime
- historical message backfill

## Validation Commands

Once implementation starts, validate with:

```bash
cd /Users/bytedance/code/codex
git status --short
```

```bash
cd /Users/bytedance/code/codex/codex-rs
cargo test -p codex-clawbot
```

```bash
cd /Users/bytedance/code/codex/codex-rs
cargo test -p codex-tui
```

```bash
cd /Users/bytedance/code/codex
just argument-comment-lint
```
