---
name: codex-loop-debug
description: Use when debugging Codex loop behavior such as before-turn or after-turn ordering, queue semantics, until_no_followup behavior, /stop handling, or stale running background loop UI in TUI.
---

Treat loop issues as scheduler problems first, UI problems second.

Debug in this order:

1. Confirm the active workspace and inspect:
   - `.codex/loop/loop_timers.json`
   - `.codex/loop/loop_trigger_queues.json`
2. Separate these failure classes:
   - trigger configuration is empty or wrong
   - scheduler queue / round semantics are wrong
   - follow-up submission keeps the chain alive
   - `/stop` is not interrupting the active loop task
   - TUI background loop indicator is stale
   - timer scheduling is armed incorrectly after startup / resume
   - timer due calculation is skipping the current round and jumping to the next one
3. For `after-turn`, reason in rounds:
   - all handlers in trigger order
   - each follow-up drained serially
   - next round only after the current follow-up queue is done
4. For “it keeps running forever”, check whether the loop keeps generating follow-up user turns. If yes, `until_no_followup` will never stop.
5. For UI banner bugs, verify whether scheduler state is really empty before blaming `chatwidget`.
6. Treat loop modes as core scheduling semantics, not cosmetic options:
   - `embed`: submit the loop prompt into the main thread as a user turn
   - `ephemeral`: fork compacted main-thread context into a hidden thread for one run, then discard it
   - `persistent`: fork compacted main-thread context once, then keep its own retained context on later runs
   - if `persistent` also consumes recent main-thread messages on later runs, verify that logic separately from the retained thread history

Useful principles:

- Loop is a scheduling core, not just a UI feature. Debug the scheduler state machine first.
- One thread should have one serial after-turn runner.
- Queue state is a better source of truth than a single boolean gate.
- On startup / respawn issues, verify that loop timers were actually loaded and armed, not just read from disk.
- If the user asks for fail-fast behavior, do not preserve legacy loop semantics just to smooth migration.
