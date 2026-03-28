# Clawbot Message Actions Proposal

## Situation

`codex-clawbot` currently treats outbound IM delivery as a text-only path:

- inbound Feishu messages are normalized into `session_id + message_id + text`
- the text is submitted into a bound Codex thread
- the final assistant text is forwarded back to the bound session

That model is sufficient for plain text bridging, but it still needs one small
operator-facing acknowledgement:

1. inbound messages should receive an immediate automatic reaction to confirm
   receipt

There is a second operator requirement:

- when a turn originated from clawbot, the operator must be able to force the
  turn into a non-interactive mode where question / permission / approval
  prompts do not block the remote session

## Task

Keep clawbot simple while making bound external turns safer and more legible:

- react once to the exact inbound provider message when it is accepted
- forward only plain text replies to the correct session
- auto-dismiss confirmation-driven interactions for remote-safe turns

## First Principles

The core abstraction is still "external text bridge with a lightweight ack".

That yields two design rules:

1. only the initial auto-ack should use a provider-specific message action
2. non-interactive clawbot turns must be enforced both at submission time and
   at interactive-event time

## Proposal

For clawbot-originated turns, the TUI should:

- submit a normal text turn without a clawbot-specific output schema
- keep the pending turn tracking needed for non-interactive handling
- add an immediate automatic `😛` reaction when the inbound message is drained
- forward only the final plain text response back to the bound session

For non-interactive clawbot turns, the TUI should:

- submit the turn with a granular approval policy that rejects permission and
  approval prompts
- auto-answer `request_user_input` with an empty response

## Result

This phase keeps the fork on the KISS path:

- one provider-specific message action for the initial Feishu auto-ack
- plain text forwarding for all later assistant replies
- a future phase can still promote richer Feishu actions into dedicated tools
  without reintroducing reply-envelope parsing into the hot path
