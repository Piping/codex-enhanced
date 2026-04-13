# /dream Design

## Goal

`/dream` should stop meaning "manually rerun the startup memories pipeline".

It should mean:

1. read the current thread
2. run one explicit retrospective with the LLM
3. update repo-local memory artifacts
4. update repo-local `AGENTS.md`
5. update relevant repo-local `SKILL.md` files and create new repo-local skills when the retrospective identifies a reusable workflow
6. rebuild a local offline memory index
7. start a fresh thread so the next session picks up the new repo guidance

Only the LLM call should require network access. Everything else must work from local files and local state.

## Non-goals

- Do not reuse the startup memories phase-1 job claim logic.
- Do not require embeddings, Postgres, or an external memory service.
- Do not delegate file updates to a writing sub-agent. The model should return structured JSON and Rust should own the writes.

## High-level flow

1. TUI issues `thread/dream/start`.
2. App-server loads the active thread and calls a dedicated core dream pipeline.
3. Core:
   - loads the current thread rollout
   - extracts model-visible rollout items
   - collects repo root, existing repo memory, repo-root `AGENTS.md`, visible thread fragments, and repo-local skill candidates discovered under `.codex/skills/`
   - sends a structured retrospective prompt to the model
   - validates and redacts the JSON response
   - writes managed sections into repo artifacts
   - rebuilds a local BM25-oriented index file
4. App-server returns artifact paths and a next-session hint.
5. TUI starts a fresh session and shows the generated hint and updated artifact paths.

## Storage layout

All repo-local dream artifacts live under:

`<repo_root>/.codex/memory/`

Planned files:

- `MEMORY.md`
- `next_session.md`
- `index.json`
- `threads/<thread_id>/retrospective.md`

Managed updates outside the memory directory:

- `<repo_root>/AGENTS.md`
- repo-local `SKILL.md` files selected for updates
- new repo-local `SKILL.md` files created under `<repo_root>/.codex/skills/`

## File update policy

`/dream` owns managed sections instead of rewriting whole files.

Markers:

- `<!-- codex:dream:start -->`
- `<!-- codex:dream:end -->`

Behavior:

- if markers exist, replace the block
- if markers do not exist, append one managed block
- if the target file does not exist, create it with a small header and one managed block

This keeps the rest of the file user-owned and makes the update deterministic.

## LLM output contract

The model returns structured JSON with:

- `threadTitle`
- `threadSummaryMd`
- `memoryBlockMd`
- `nextSessionHintMd`
- `agentsBlockMd`
- `skills[]`
  - `path`
  - `blockMd`
- `newSkills[]`
  - `name`
  - `description`
  - `contentsMd`

Validation rules:

- existing skill paths must be repo-local and must match discovered candidates
- new skills are created only under `<repo_root>/.codex/skills/<slug>/SKILL.md`
- all stored strings are secret-redacted before writing

## Retrieval / index

`/dream` should rebuild a local index file after every run.

V1:

- store normalized documents in `index.json`
- include generated memory artifacts plus the managed dream blocks for `AGENTS.md` and updated `SKILL.md`
- keep the index local and file-based
- provide a small BM25 search helper in core for future retrieval wiring

V1 deliberately avoids embeddings. A later version can add optional local embeddings.

## API shape

New v2 app-server RPC:

- `thread/dream/start`

Request:

- `threadId`

Response:

- `memoryRoot`
- `retrospectivePath`
- `updatedAgentsPath`
- `updatedSkillPaths`
- `nextSessionHint`

## Why not reuse `thread/memories/update`

`thread/memories/update` currently maps to the startup memories pipeline, which scans eligible idle rollouts. `/dream` must target the current thread directly, so it needs a dedicated entrypoint and dedicated semantics.

## TODO

- [x] Write a dedicated `/dream` design
- [x] Add a core `dream` module with prompt building, context extraction, storage, and index generation
- [x] Add a dedicated `CodexThread::run_dream_pipeline_now()`
- [x] Add app-server RPC `thread/dream/start`
- [x] Switch TUI `/dream` to the new RPC
- [x] Write repo-local artifacts under `.codex/memory/`
- [x] Update repo-root `AGENTS.md` through a managed block
- [x] Update repo-local used `SKILL.md` files through managed blocks
- [x] Rebuild a local index file and add BM25 helper coverage
- [x] Add core tests for storage, indexing, and dream end-to-end behavior
- [x] Add protocol and app-server tests
- [x] Update app-server API docs

## Follow-up work

- retrieval wiring so fresh sessions can actively use `.codex/memory/index.json`
- optional local embedding index inspired by `cocoindex-code`
- nested `AGENTS.md` targeting instead of repo-root-only updates
- richer retrieval over auto-discovered repo-local skills and generated retrospectives
