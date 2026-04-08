# Codex Enhanced

[中文版本](./README.zh-CN.md)

> The Codex distribution built for real 24/7 use.

`codex-enhanced` is a Codex distribution built on top of the OpenAI Codex CLI Rust stack. The goal is not to wrap the agent in more prompt theater. The goal is to turn Codex into an operator surface that can keep running across accounts, sessions, workflows, and external message channels.

Website: `https://codex-enhanced.com`

## Why codex-enhanced

Most AI CLIs compete on three things:

- models
- UI polish

This distribution moves in the opposite direction:

- assume the base agent is already strong enough
- invest in multi-account routing, profile switching, long-session recovery, task orchestration, Feishu entrypoints, and cleaner operator UX
- make Codex feel less like a terminal chatbot and more like a control surface that can receive messages, pick up tasks, and stay aligned over time

If you need a Codex you can actually keep online and keep using, that is the point of this distribution.

## Examples

### 1. Multi-subscription and multi-profile operation, not manual env switching

`/profile` opens a dedicated management panel with support for:

- named profiles
- runtime switching
- fallback routes
- profile switching policies for rate limits, auth failures, and service overload

### 2. Conversations can be orchestrated as jobs

`/workflow` manages `.codex/workflows/*.yaml` directly, with support for:

- `before_turn`
- `after_turn`
- `manual`
- `file_watch`
- `idle`
- `interval`
- `cron`

That makes Codex more than a prompt-response loop. It becomes a node inside a repeatable workflow.

Docs:

- [`docs/workflows.md`](./docs/workflows.md)

### 3. Any session can be resumed instead of restarted

`/resume` and the thread/session plumbing let you reconnect to saved work instead of rebuilding context from scratch every time.

### 4. The terminal is not the only entrypoint

`/clawbot` connects workspace-local Feishu sessions, thread binding, unread message queues, and reply forwarding into the same loop. A Feishu chat can be bound to the current thread, external messages can enter Codex, and final replies can be sent back out.

## Install

The primary install path for this distribution is PyPI:

```bash
pip3 install -U codex-enhanced
codex-enhanced
```

## Capability Boundaries and Layering

### What it is built to solve

This distribution is strong at connecting the agent to real operating workflows, not at reinventing the underlying model platform.

Current strengths:

- multi-subscription account management
- multi-profile API routing and fallback
- long-session recovery and continuity
- workspace-local workflow orchestration
- Feishu clawbot integration
- local TUI information shaping and visibility control
- stronger alignment flows via `question` and keyboard chord support

### What it is not trying to be

- a replacement for every official hosted or distributed Codex surface
- a general-purpose IM automation hub beyond the current Feishu focus
- a zero-configuration black box for business workflow automation

## Shipping Today

These are already implemented in the repository:

- multi-subscription account management and runtime account display
- multi-profile API management and `/profile` route switching
- `/workflow` task orchestration
- `/resume` for arbitrary saved sessions
- `/settings` for UI information control
- `/clawbot` for Feishu send and receive flows
- `pypi-release` distribution pipeline
- stronger `question`-based alignment interactions
- chord shortcut support

## How It Works

The system is designed to stay inspectable:

- profile routing state is persisted in `accounts/profile-router.json`
- workflows live directly in `.codex/workflows/*.yaml`
- Feishu clawbot sessions, bindings, and unread queues are managed by a workspace-local runtime
- the `question` tool collects structured answers in the TUI instead of falling back to guesswork in free text
- UI and interaction changes are heavily covered by snapshot tests for direct review

Useful docs and examples:

- [`docs/workflows.md`](./docs/workflows.md)
- [`docs/tui-request-user-input.md`](./docs/tui-request-user-input.md)

## Project Attribution

This project does not start from zero. It builds on the OpenAI Codex CLI Rust, TUI, and app-server foundation, then pushes harder on the parts that matter in sustained use: account operations, session continuity, workflows, Feishu entrypoints, lower-noise UI, and operator ergonomics.

## Closing

If you only need a Codex that chats in a terminal, the official distribution is already enough.

If you need a Codex that can stay online across accounts, inputs, tasks, and long-running sessions, the opening line still applies:

**The Codex distribution built for real 24/7 use.**

## License

Apache-2.0
