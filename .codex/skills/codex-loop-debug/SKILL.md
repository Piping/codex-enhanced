---
name: codex-loop-debug
description: Use when debugging Codex loop behavior such as before-turn or after-turn ordering, queue semantics, until_no_followup behavior, /stop handling, or stale running background loop UI in TUI.
---

Treat loop issues as scheduler problems first, UI problems second.

Debug in this order:

1. Confirm the active workspace and inspect:
   - `.codex/loop_timers.json`
   - `.codex/loop_trigger_queues.json`
2. Separate these failure classes:
   - trigger configuration is empty or wrong
   - scheduler queue / round semantics are wrong
   - follow-up submission keeps the chain alive
   - `/stop` is not interrupting the active loop task
   - TUI background loop indicator is stale
3. For `after-turn`, reason in rounds:
   - all handlers in trigger order
   - each follow-up drained serially
   - next round only after the current follow-up queue is done
4. For “it keeps running forever”, check whether the loop keeps generating follow-up user turns. If yes, `until_no_followup` will never stop.
5. For UI banner bugs, verify whether scheduler state is really empty before blaming `chatwidget`.

Useful principles:

- One thread should have one serial after-turn runner.
- Queue state is a better source of truth than a single boolean gate.
- If the user asks for fail-fast behavior, do not preserve legacy loop semantics just to smooth migration.

