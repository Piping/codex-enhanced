# Clawbot Message Actions Design

## Scope

This design covers:

- Feishu auto-ack reactions for inbound clawbot messages
- plain text replies for clawbot-originated turns
- a persisted clawbot turn mode that can disable interactive prompts

This design does not yet cover:

- assistant-controlled reaction replies
- persistent reaction receipts across process restarts
- transcript sanitization beyond the current clawbot info-cell annotations

## Architecture

```text
Feishu inbound message
  -> codex-clawbot provider event
  -> unread cache
  -> drain into bound thread
  -> auto-ack reaction
  -> submit normal clawbot turn
  -> record turn_id <-> turn mode
  -> TurnComplete / Error
  -> forward plain text reply
```

## Data Model

### `codex-clawbot`

- `ClawbotTurnMode`
  - `interactive`
  - `non_interactive`
- `ProviderMessageRef`
  - `provider`
  - `session_id`
  - `message_id`
- `ProviderOutboundReaction`
  - `target`
  - `emoji_type`
- `ProviderOutboundAction`
  - `Text`
  - `AddReaction`

### TUI in-memory turn tracking

Per thread, keep a FIFO of pending clawbot turns:

- `turn_id`
- `thread_id`
- `turn_mode`

The queue is in-memory because the associated turn is also in-flight process
state. If the process dies, the turn is already lost.

## Reply Handling

Clawbot-originated turns submit a normal user turn with no clawbot-specific
`final_output_json_schema`.

Reply handling stays intentionally simple:

- if the final assistant message is a non-empty string, forward it as text
- do not parse or recover legacy structured reply envelopes
- do not send assistant-controlled reactions

## Non-Interactive Turn Mode

### Submission layer

For `non_interactive` clawbot turns, use:

- `AskForApproval::Granular`
- all granular flags set to `false`

This prevents approval-driven UI from surfacing for:

- sandbox approval
- execpolicy prompt rules
- skill approval
- `request_permissions`
- MCP elicitations

### Event layer

`request_user_input` is not blocked by approval policy, so the TUI must catch it
for clawbot-originated turns and auto-answer with an empty response.

As a defensive fallback, `request_permissions` can also be auto-resolved with an
empty permission grant if it still appears.

## UI Surface

Clawbot control panel gains a persisted turn-mode option:

- `interactive`
- `non-interactive`

The transcript keeps using info cells:

- `Feishu message`
- `Feishu auto reaction`
- `Clawbot auto response`

## Feishu Mapping

Feishu implementation extends the existing text send path with:

- add reaction for the initial auto-ack

Auto-ack uses provider `emoji_type = "TONGUE"` and is rendered in the TUI as `😛`.

## Todo Sequence

1. Keep the provider message reference for inbound auto-ack
2. Keep Feishu add-reaction support for the initial auto-ack
3. Add persisted clawbot turn mode to workspace config
4. Add control-panel toggle for turn mode
5. Track clawbot-originated turn ids in the TUI
6. Auto-ack inbound Feishu messages when draining into a thread
7. Forward plain text replies on turn completion
8. Auto-answer `request_user_input` for non-interactive clawbot turns
