# `/loop` proposal

This document explains why Codex TUI has a workspace-local `/loop` command and
what problem it is intended to solve.

## Problem

Some prompts are useful on a schedule instead of as one-off turns:

- periodic status checks
- recurring summaries
- repeated comparison or monitoring prompts tied to one workspace

Before `/loop`, the user had to either:

- remember to rerun the same prompt manually
- keep a long-lived side thread around and poll it manually
- use an external scheduler that could not see the current Codex workspace

That created friction and also made it hard to keep scheduled work separated
from the main conversation.

## Goals

- Let the user schedule a prompt directly from the TUI with `/loop`.
- Keep timers local to the current workspace instead of global.
- Run each scheduled execution in a hidden thread so the main thread stays
  clean.
- Mirror only a small, intentional result back into the main thread.
- Give the user a simple manager in the control panel to inspect, enable,
  disable, or delete timers.

## Non-goals

- `/loop` is not a general background job runner.
- `/loop` is not a multi-agent orchestration feature.
- `/loop` should not mutate files, spawn agents, or perform side-effectful
  actions.
- `/loop` is not currently an app-server API feature; the first implementation
  is TUI-only.

## User experience

The desired experience is:

1. Create a timer with `/loop 5m <prompt>` or `/loop <cron> <prompt>`.
2. Let Codex execute the prompt in the background on schedule.
3. See a compact note in the main thread telling you which loop ran.
4. See only the configured mirrored output, not the hidden execution history.
5. Manage existing timers from `Ctrl-P -> Loop Manager`.

## Why the hidden-thread model

Running `/loop` in the main thread would pollute the user-visible transcript
and future model context with repeated scheduler chatter. A hidden thread keeps
the scheduled execution isolated while still reusing the existing thread and
streaming infrastructure.
