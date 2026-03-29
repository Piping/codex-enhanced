---
name: clawbot-feishu-debug
description: Use when debugging Codex clawbot and Feishu integration problems such as missing sessions, missing inbound messages, reaction failures, websocket health, session binding, or reply forwarding.
---

Debug clawbot in this order:

1. Check workspace-local state under `.codex/clawbot/`:
   - `config.toml`
   - `runtime.json`
   - `sessions.json`
   - `bindings.json`
   - `unread_messages.jsonl`
   - `inbound_receipts.json`
2. Distinguish these cases clearly:
   - REST send path works
   - runtime says `connected`
   - websocket inbound events are actually arriving
   These are not the same thing.
3. Verify the bot identity and app credentials match the intended Feishu app.
4. For session issues, check whether the session is auto-discovered, manually bound, reachable by the current bot, and still visible through Feishu APIs.
5. For repeated messages, check dedupe state in `inbound_receipts.json`.
6. For reaction failures, use exact official Feishu `emoji_type` names only.

Useful checks:

- `runtime.json` proving `connected` is not enough; confirm new inbound state is landing in unread / receipt files.
- If a session is bound but no inbound message lands in local state, suspect websocket delivery before suspecting thread routing.
- If send fails with “Bot/User can NOT be out of the chat”, the bound `chat_id` is invalid for the current bot.

Prefer concrete file evidence over speculation, and end with the smallest next verification command.

