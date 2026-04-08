<div align="center">

# Codex Enhanced

> The Codex distribution built for real 24/7 use.

[中文版本](./README.zh-CN.md) · [Website](https://codex-enhanced.com) · [Workflows](./docs/workflows.md) · [Structured Input UI](./docs/tui-request-user-input.md)

</div>

`codex-enhanced` is a Codex distribution built on top of the OpenAI Codex CLI Rust stack. It is focused less on prompt theater and more on turning Codex into an operator surface that can stay online across accounts, sessions, workflows, and external message channels.

If you need a terminal chatbot, the base Codex experience is already strong. This distribution is for the next step: keeping Codex running as a controllable workspace system instead of a single ephemeral chat loop.

## Why codex-enhanced

Most AI CLI projects compete on model access and UI polish.

`codex-enhanced` moves the investment somewhere else:

- multi-subscription account management instead of manual env switching
- multi-profile routing and fallback instead of a single fragile default
- resumable sessions instead of repeated context rebuilds
- workflow triggers and background jobs instead of one-turn-at-a-time operation
- Feishu bridge entrypoints instead of terminal-only interaction
- lower-noise operator UX instead of more surface-level prompt ceremony

That is the core idea: make Codex feel less like a terminal chatbot and more like a persistent control surface.

## What You Can Do

| Capability | Entry point | What it enables |
| --- | --- | --- |
| Multi-profile routing | `/profile` | Switch named profiles at runtime, manage fallback routes, and recover from rate limits or auth failures without rewriting local environment state. |
| Workflow orchestration | `/workflow` | Manage `.codex/workflows/*.yaml`, run jobs manually, and attach triggers such as `before_turn`, `after_turn`, `interval`, `cron`, and `file_watch`. |
| Session continuity | `/resume` | Reconnect to saved work instead of reconstructing long-running context from scratch. |
| External message bridge | `/clawbot` | Bind workspace-local Feishu sessions to Codex threads, capture unread messages, and forward final replies back out. |
| UI and alignment control | `/settings`, `question`, keyboard chords | Reduce noise, collect structured answers in the TUI, and keep operator interactions explicit. |

## Quickstart

Install from PyPI:

```bash
pip3 install -U codex-enhanced
codex-enhanced
```

Run from source in this repository:

```bash
just codex
```

`just codex` runs `cargo run --bin codex -- ...` from the `codex-rs` workspace, which is the fastest way to inspect or develop the Rust TUI locally.

## Typical Operator Flows

### 1. Route traffic across multiple accounts and profiles

`/profile` opens a dedicated management panel for:

- named profiles
- runtime switching
- fallback routes
- switching policies for rate limits, auth failures, and service overload

This is the difference between "change a key and restart the tool" and "keep the operator surface online."

### 2. Turn conversations into jobs

`/workflow` manages workflow definitions directly from `.codex/workflows/*.yaml`.

Supported trigger families include:

- `before_turn`
- `after_turn`
- `manual`
- `file_watch`
- `idle`
- `interval`
- `cron`

That lets Codex participate in repeatable automation loops instead of only answering the current prompt.

Minimal example:

```yaml
name: director

triggers:
  - id: pulse
    type: interval
    every: 30m
    enabled: true
    jobs: [notify]

jobs:
  notify:
    enabled: true
    context: ephemeral
    response: assistant
    steps:
      - prompt: |
          Send a concise update on the current workspace state.
```

Documentation:

- [`docs/workflows.md`](./docs/workflows.md)

### 3. Resume work instead of restarting it

`/resume` and the underlying thread/session plumbing let you reconnect to saved work instead of paying the cost of rebuilding context every time a session is interrupted.

This matters once Codex is doing real work over hours or days rather than a single short exchange.

### 4. Bring Feishu into the same loop

`/clawbot` connects workspace-local Feishu sessions, thread binding, unread message queues, and reply forwarding into the same runtime.

In practice, that means:

- a Feishu chat can be bound to the current Codex thread
- inbound messages can enter the active workspace loop
- final replies can be sent back to Feishu
- session and binding state stays local to the workspace runtime

## What Ships Today

These capabilities are already implemented in the repository:

- multi-subscription account management and runtime account display
- multi-profile API management and `/profile` route switching
- `/workflow` task orchestration
- `/resume` for saved sessions
- `/settings` for UI information control
- `/clawbot` for Feishu send and receive flows
- stronger `question`-based alignment interactions
- keyboard chord support
- PyPI packaging and release flow for `codex-enhanced`

## Inspectable by Design

This project is intentionally built so the important runtime state stays visible:

- profile routing state lives in `accounts/profile-router.json`
- workflows live in `.codex/workflows/*.yaml`
- clawbot state is stored under `.codex/clawbot/`
- structured operator answers are collected through the TUI `question` flow instead of being guessed from ambiguous free text

This distribution is opinionated, but it is not trying to hide the system from the operator.

## Repository Map

If you want to inspect or extend the project, start here:

- [`codex-rs/`](./codex-rs) contains the Rust workspace, including the CLI, TUI, workflow support, app-server pieces, and clawbot integration
- [`sdk/python-runtime-enhanced/`](./sdk/python-runtime-enhanced) contains the Python wheel packaging for `codex-enhanced`
- [`docs/workflows.md`](./docs/workflows.md) explains workflow files, triggers, and job management
- [`docs/tui-request-user-input.md`](./docs/tui-request-user-input.md) explains the structured input overlay used for `question`

## Capability Boundaries

### What it is built to solve

This distribution is strong at connecting the agent to real operating workflows.

Current strengths:

- multi-account and multi-profile routing
- long-session recovery and continuity
- workspace-local workflow orchestration
- Feishu clawbot integration
- local TUI information shaping and visibility control
- stronger alignment flows for human-in-the-loop operation

### What it is not trying to be

- a replacement for every official hosted or distributed Codex surface
- a general-purpose IM automation hub beyond the current Feishu focus
- a zero-configuration black box for business workflow automation

## Project Attribution

This project builds on the OpenAI Codex CLI Rust, TUI, and app-server foundation, then pushes harder on the parts that matter in sustained use: account operations, session continuity, workflows, Feishu entrypoints, lower-noise UI, and operator ergonomics.

If you only need a Codex that chats in a terminal, the official distribution is already enough.

If you need a Codex that can stay online across accounts, inputs, tasks, and long-running sessions, that is the point of this distribution.

## License

Apache-2.0
