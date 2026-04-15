# Use Cases Derived From Commits Since `4fd5c35`

This document summarizes the product and user use cases that can be inferred from the commits in the range:

- base: `4fd5c35c4f6f51048f47c8680ed0f6a26c608f68`
- head: `dae2b6356` (`v0.1.28`)

The analysis uses the `effective-use-cases` method.
It treats the repository as a black-box system first and derives user-goal behavior from the observed commit themes, docs, and release flow.

## Scope And Goal Level

- Scope: `codex-enhanced` as a persistent user surface for long-running Codex work across sessions, profiles, workflows, and external message channels.
- Goal level: mostly sea-level user-goal use cases, plus one maintainer-facing operational use case for packaging and release.
- Source basis: commit messages, changed docs, release workflow changes, and feature surfaces exposed in TUI slash commands and runtime docs.

### Non-goals For This Analysis

- It does not try to describe every refactor, replay commit, or CI repair as a separate use case.
- It does not treat internal module splits as user-visible goals unless they change externally meaningful behavior.
- It treats fork-CI stabilization as supporting engineering infrastructure, not as a primary end-user use case.

## Actor And Stakeholder Map

### Primary actors

- Workspace user: uses Codex day-to-day in TUI/CLI to run, resume, steer, and monitor work.
- External collaborator: sends messages from Feishu into a bound Codex workflow.
- Release maintainer: publishes `codex-enhanced` as a multi-platform Python wheel release.

### Supporting actors

- Model provider / profile endpoint: serves inference and may rate-limit, overload, or fail auth.
- Workflow scheduler and app-server runtime: executes triggers, jobs, and follow-up turns.
- Feishu platform: supplies inbound messages and accepts outbound replies/reaction updates.
- Local filesystem and repo state: stores workflow YAML, profile routing, memory artifacts, and reports.

### Off-stage stakeholders and interests

- Team members relying on continuity: want saved sessions, jump navigation, and recoverable state instead of context rebuild.
- Repository maintainers: want local memory, updated `AGENTS.md`, and reusable skills from prior work.
- PyPI consumers: want an installable wheel with the correct embedded native runtime per platform.
- Users under interruption: want retries, fallback, visible failures, and non-silent degradation.

## Commit Theme Clusters

These are the main behavioral clusters visible in the commit range.

| Cluster | Representative commits | Inferred behavior direction |
| --- | --- | --- |
| Profile routing and session continuity | `c7d306a2a`, `d1461727f`, `ac08abeae`, `fed91c14a`, `ff71e811a`, `d6da73601` | Keep Codex online across profile failure, respawn, thread routing, and resumed work. |
| Workflow orchestration and follow-up automation | `a59a3a6a6`, `6e759655a`, `29a75c238`, `ea74b8961`, `ee7ff5e54`, `71ba01403`, `58f90f9ae`, `8049f9d1a` | Turn prompts into repeatable jobs with triggers, timeouts, retries, bound-thread routing, and non-blocking follow-up turns. |
| Feishu clawbot bridge | `55ec5cdb5`, `063dfd100`, `2ee91453f`, `ff8875c17`, `9951bec11`, `acd2b529d`, `27b0b33e9` | Bind external chats to Codex threads and keep inbound/outbound message delivery stable. |
| Human-in-the-loop control and low-noise TUI | `cb099a038`, `329b4e1ae`, `406389863`, `0f571b53c`, `83ad3dfe2`, `5b9d0af78`, `415cad316` | Make the user surface navigable, structured, and less noisy during long sessions. |
| Insight and retrospective memory | `55d4e11aa`, `88e75bc1d`, `ce9700ab5`, `5b71b8ecf`, `10ab81459` | Convert session history into reports, repo memory, updated instructions, and reusable skills. |
| Packaging and release hardening | `cbd9e0d7d`, `b30d46c3b`, `4194a80f1`, `d359e3ffa`, `dae2b6356` | Ship reliable tagged releases and multi-platform runtime wheels without mismatched artifacts. |

## Use Case Inventory

| ID | Use case | Primary actor | Goal |
| --- | --- | --- | --- |
| UC-1 | Keep Codex running across profiles and failures | Workspace user | Continue work despite rate limits, auth failures, or provider-specific outages. |
| UC-2 | Resume and navigate long-running workspace sessions | Workspace user | Re-enter a saved thread and recover operational context quickly. |
| UC-3 | Define and manage workspace-local workflows | Workspace user | Turn recurring work into runnable jobs and triggers stored in repo-local YAML. |
| UC-4 | Run follow-up automation without blocking the main thread | Workspace user | Let turn completion or file events trigger more work while keeping the main conversation responsive. |
| UC-5 | Bridge Feishu conversations into Codex threads | External collaborator and workspace user | Accept inbound Feishu messages, bind them to the right thread, and return final replies outward. |
| UC-6 | Keep the user in the loop with structured control | Workspace user | Give explicit answers, inspect hidden state, jump through history, and reduce UI noise during extended operation. |
| UC-7 | Inspect local session behavior offline | Workspace user | Generate an inspectable report from rollout history without relying on hosted analytics. |
| UC-8 | Convert a completed thread into reusable repo memory | Workspace user | Run `/dream` to update memory, `AGENTS.md`, and repo-local skills for future sessions. |
| UC-9 | Publish a correct multi-platform `codex-enhanced` release | Release maintainer | Build, tag, validate, and publish wheels whose embedded runtime matches the release version. |

## Fully Dressed Priority Use Cases

## UC-1 Keep Codex Running Across Profiles And Failures

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User: ongoing work should not stop because one profile fails.
  - Team relying on the user surface: runtime should remain routable without manual config surgery.
  - Model provider account owner: failures should be handled explicitly, not hidden.
- Preconditions:
  - The user has configured one or more profiles.
  - Runtime is active in a repo or workspace session.
- Minimal guarantees:
  - Failure state is surfaced.
  - Existing routing/config state is preserved.
  - Session continuity is not silently broken by profile switch behavior.
- Success guarantees:
  - Work continues on a usable profile.
  - The user can see or manage the fallback route explicitly.
- Trigger:
  - A provider errors due to rate limit, auth failure, or overload, or the user wants to switch runtime profile.
- Main success scenario:
  1. User opens or uses profile routing controls.
  2. Codex detects the current profile is unsuitable or the user selects another route.
  3. Codex keeps session/runtime continuity while switching the active route.
  4. The current thread remains usable without manual environment rewriting.
- Extensions:
  - 2a. No fallback route is configured:
    Codex reports the issue and leaves the user in control instead of inventing a route.
  - 3a. Thread unsubscribe or session handoff would break continuity:
    Codex preserves the bound thread/session behavior and avoids losing the current work surface.
  - 3b. The CLI respawns:
    Session arguments and routing context are preserved across respawn.

## UC-3 Define And Manage Workspace-Local Workflows

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User: recurring work should become explicit YAML-backed automation, not ad hoc repeated prompts.
  - Repo collaborators: workflow definitions should stay local, inspectable, and editable.
  - Runtime: invalid or unrunnable workflow conditions should fail visibly.
- Preconditions:
  - The workspace exists and can store `.codex/workflows/*.yaml`.
  - The user can open `/workflow`.
- Minimal guarantees:
  - Workflow definitions remain local files.
  - Invalid YAML or disabled/unrunnable job states are visible.
  - Manual execution does not silently no-op.
- Success guarantees:
  - User can create, edit, enable, disable, and run jobs/triggers.
  - Trigger parameters such as `interval`, `cron`, `idle`, `file_watch`, and `bind_thread` are retained in workflow state.
- Trigger:
  - The user wants repeatable background automation tied to the workspace.
- Main success scenario:
  1. User opens `/workflow`.
  2. Codex lists workflow files, jobs, triggers, and background task state.
  3. User edits YAML directly or uses structured menu actions.
  4. Codex persists the workflow definition locally.
  5. User runs a job or enables a trigger.
  6. Runtime executes the selected workflow with the configured context and response mode.
- Extensions:
  - 3a. No workflow file exists:
    Codex offers to create a starter file and opens it in the editor.
  - 5a. Trigger resolves only to disabled or unrunnable jobs:
    Codex fails visibly instead of silently discarding the run.
  - 6a. A step times out:
    Codex applies configured timeout/retry behavior and surfaces failure if recovery is exhausted.
  - 6b. A `file_watch` trigger is already queued or running:
    Duplicate events are skipped according to overlap policy instead of creating a storm.

## UC-5 Bridge Feishu Conversations Into Codex Threads

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: external collaborator, supported by workspace user
- Stakeholders and interests:
  - External collaborator: message should reach the right Codex thread and receive a reply.
  - User: bindings should remain local, inspectable, and recoverable.
  - Runtime: stale bindings and delivery failures should be reconciled, not compounded.
- Preconditions:
  - Feishu integration is configured for the workspace.
  - A Codex thread can be bound to a Feishu session/channel.
- Minimal guarantees:
  - Incoming messages are not silently dropped when binding state is stale.
  - Failure to send reactions or replies is surfaced and can be retried or reconciled.
  - Bindings remain stored in workspace-local state.
- Success guarantees:
  - Inbound message is routed into the correct Codex operational loop.
  - Final reply is delivered back to Feishu.
  - Session jump and bound-thread continuity remain usable even after reload or respawn.
- Trigger:
  - A new Feishu message arrives for a bound conversation, or the user manages bindings from `/clawbot`.
- Main success scenario:
  1. User binds a Feishu session to a Codex thread.
  2. Collaborator sends a message in Feishu.
  3. Codex runtime receives the event and maps it to the bound workspace thread.
  4. Codex processes the inbound message in the correct execution mode.
  5. Codex sends the final reply back to Feishu and updates local state.
- Extensions:
  - 3a. Binding is stale or references unloaded thread state:
    Codex reconciles the binding or restores jump continuity instead of leaving the session orphaned.
  - 4a. Runtime has respawned:
    Clawbot runtime restarts and reconnects to the existing operational state.
  - 5a. Reaction cancellation or delivery update fails:
    Codex reports the failure and avoids pretending the external state was updated.

## UC-8 Convert A Completed Thread Into Reusable Repo Memory

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - Future user sessions: should inherit useful repo-local guidance from prior work.
  - Repo maintainers: want updates to remain deterministic and confined to managed sections.
  - Security-sensitive users: secrets must not be copied into generated artifacts.
- Preconditions:
  - A thread with useful historical content exists.
  - Repo-local storage is available.
- Minimal guarantees:
  - Existing user-owned file content outside managed dream blocks is preserved.
  - Writes stay repo-local.
  - Output is validated and redacted before persistence.
- Success guarantees:
  - Repo-local memory artifacts are written.
  - Managed blocks in `AGENTS.md` and selected `SKILL.md` files are updated.
  - A fresh session can start with an explicit next-session hint.
- Trigger:
  - User invokes `/dream` for the current thread.
- Main success scenario:
  1. User runs `/dream`.
  2. Codex loads the current thread and relevant local context.
  3. Codex runs a dedicated retrospective prompt.
  4. Codex validates and redacts the structured result.
  5. Codex writes memory files, managed instruction blocks, and skill updates.
  6. Codex rebuilds the local memory index and starts a fresh thread with the next-session hint.
- Extensions:
  - 4a. The model returns paths outside the repo:
    Codex rejects them and does not write unsafe output.
  - 5a. A target file does not exist:
    Codex creates a small file with a managed block rather than failing outright.
  - 5b. Managed markers already exist:
    Codex replaces only the managed block, not the entire file.

## UC-9 Publish A Correct Multi-Platform `codex-enhanced` Release

- Scope: release workflow for `codex-enhanced`
- Level: sea-level operational goal
- Primary actor: release maintainer
- Stakeholders and interests:
  - End users: published wheel should install and include the correct native binary.
  - Maintainer: tag, runtime version, and embedded artifacts must match.
  - Release automation: concurrency and artifact reuse should not deadlock or publish the wrong bits.
- Preconditions:
  - Release version is chosen.
  - Tag and source state are ready.
- Minimal guarantees:
  - Version mismatch between tag and runtime package fails before publish.
  - Artifact reuse and publish flow are explicit.
  - Windows/macOS/Linux wheel packaging is separated and visible.
- Success guarantees:
  - Tagged release produces matching wheel artifacts.
  - Wheels are published to PyPI.
  - GitHub release assets are attached for the tagged version.
- Trigger:
  - Maintainer pushes a `v*.*.*` tag or dispatches the release workflow manually.
- Main success scenario:
  1. Maintainer creates a release tag.
  2. Workflow validates that the tag matches `sdk/python-runtime-enhanced/pyproject.toml`.
  3. Release assets and platform-specific runtime binaries are prepared.
  4. Wheels are built for supported targets.
  5. Publish step uploads wheels to PyPI.
  6. GitHub release step attaches release artifacts to the same tag.
- Extensions:
  - 2a. Tag version and runtime version differ:
    Workflow stops before publish.
  - 3a. Earlier artifact run should be reused:
    Workflow downloads the selected artifacts and validates they contain wheel files.
  - 4a. Windows runtime packaging fails:
    Build is isolated per target and does not obscure which platform failed.
  - 5a. Concurrency would deadlock or cancel the wrong release:
    Workflow uses per-tag concurrency scoping to keep release runs isolated.

## Acceptance Criteria

- Users can keep work running across profile errors without manual environment rewriting.
- Saved sessions can be resumed with enough navigation and visibility control to recover context quickly.
- Workflows are repo-local, editable, runnable, and visibly fail when misconfigured or unrunnable.
- After-turn and background workflow activity does not freeze the main interactive thread.
- Feishu session bindings survive routine runtime disruptions and support visible inbound/outbound handling.
- `/dream` produces repo-local memory updates through managed sections instead of ad hoc file rewrites.
- `/insight` generates an offline report from local rollout history.
- Tagged `codex-enhanced` releases validate version alignment before publishing wheels.

## Implementation Slices Suggested By The Use Cases

- Slice 1: profile-router runtime continuity, fallback policy, respawn/session-arg preservation.
- Slice 2: session continuity UX including resume picker, jump-to-message, timestamps, and visibility preferences.
- Slice 3: workflow scheduler/runtime, trigger semantics, timeout/retry handling, and TUI workflow controls.
- Slice 4: Feishu clawbot runtime bridge, binding store, message delivery, and admin/session recovery flows.
- Slice 5: structured human-in-the-loop controls including `question`, richer file reading/search tools, and lower-noise TUI affordances.
- Slice 6: retrospective stack including `/dream`, repo-local memory storage, managed `AGENTS.md` updates, and generated skill updates.
- Slice 7: offline observability via `/insight`.
- Slice 8: release automation for Python runtime packaging, artifact reuse, and multi-platform validation.

## Open Questions

- Should `/dream` stay repo-root-focused, or should future iterations target nested `AGENTS.md` scopes automatically?
- Should profile fallback become policy-driven enough to express different failure classes separately, or is the current route model sufficient?
- Should Feishu remain the only external bridge, or is the intended use case actually "generic external user inbox" with Feishu as the first adapter?
- Should `/insight` remain a local offline artifact only, or should it eventually feed runtime guidance and profile/workflow tuning loops?
- Which current TUI continuity improvements are true user-goal behavior, and which should remain implementation detail rather than product surface?
