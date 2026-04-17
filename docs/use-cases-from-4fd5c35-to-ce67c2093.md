# Use Cases Derived From Commits Since `4fd5c35`

This document summarizes the product and user use cases that can be inferred from the commits in the range:

- base: `4fd5c35c4f6f51048f47c8680ed0f6a26c608f68`
- head: `ce67c2093`

The analysis uses the `effective-use-cases` method.
It treats the repository as a black-box system first and derives user-goal behavior from the observed commit themes, docs, and release flow.

## Scope And Goal Level

- Scope: `codex-enhanced` as a persistent user surface for long-running Codex work across sessions, profiles, workflows, local memory, and external message channels.
- Goal level: mostly sea-level user-goal use cases, plus a small number of maintainer-facing operational use cases where packaging or runtime coordination changes are externally meaningful.
- Source basis: commit messages, changed docs, TUI feature surfaces, release workflow changes, and the added Feishu Base coordination behavior for `clawbot`.

### Non-goals For This Analysis

- It does not try to describe every refactor, replay commit, or CI repair as a separate use case.
- It does not treat internal test harness changes or alternate build-backend support as standalone end-user goals unless they change an externally meaningful guarantee.
- It does not treat every TUI control or menu item as its own use case when those controls support a larger user goal.

## Actor And Stakeholder Map

### Primary actors

- Workspace user: uses Codex day-to-day in TUI/CLI to run, resume, steer, monitor, and automate work across long-lived sessions.
- External collaborator: sends messages from Feishu into a bound Codex workflow and expects a routed reply.
- Release maintainer: publishes `codex-enhanced` as a multi-platform Python wheel release.

### Supporting actors

- Model provider / profile endpoint: serves inference and may rate-limit, overload, or fail auth.
- Workflow scheduler and app-server runtime: executes triggers, jobs, and follow-up turns.
- Feishu platform: supplies inbound messages and accepts outbound replies, reactions, and websocket ownership.
- Feishu Base: stores shared clawbot coordination state for leadership and forced websocket preemption.
- Local filesystem and repo state: stores workflow YAML, profile routing, memory artifacts, reports, and workspace-local clawbot bindings.

### Off-stage stakeholders and interests

- Team members relying on continuity: want saved sessions, jump navigation, and recoverable state instead of context rebuild.
- Repository maintainers: want local memory, updated `AGENTS.md`, and reusable skills from prior work.
- PyPI consumers: want an installable wheel with the correct embedded native runtime per platform.
- Users running multiple Codex processes: want only one process to own the embedded Feishu websocket at a time, with explicit preemption when needed.
- Users under interruption: want retries, fallback, visible failures, and non-silent degradation.

## Commit Theme Clusters

These are the main behavioral clusters visible in the commit range.

| Cluster | Representative commits | Inferred behavior direction |
| --- | --- | --- |
| Profile routing and session continuity | `c7d306a2a`, `d1461727f`, `ac08abeae`, `fed91c14a`, `ff71e811a`, `d6da73601` | Keep Codex online across profile failure, respawn, thread routing, and resumed work. |
| Workflow orchestration and follow-up automation | `a59a3a6a6`, `6e759655a`, `29a75c238`, `ea74b8961`, `ee7ff5e54`, `71ba01403`, `58f90f9ae`, `8049f9d1a` | Turn prompts into repeatable jobs with triggers, timeouts, retries, bound-thread routing, and non-blocking follow-up turns. |
| Feishu clawbot bridge and message routing | `55ec5cdb5`, `063dfd100`, `2ee91453f`, `ff8875c17`, `9951bec11`, `acd2b529d`, `27b0b33e9` | Bind external chats to Codex threads and keep inbound and outbound message delivery stable. |
| Feishu websocket ownership coordination | `5eabe81c2`, `ce67c2093` | Let multiple Codex processes coordinate embedded Feishu websocket ownership through Feishu Base, including force-preempt and auto-provisioned coordination tables. |
| Human-in-the-loop control and low-noise TUI | `cb099a038`, `329b4e1ae`, `406389863`, `0f571b53c`, `83ad3dfe2`, `5b9d0af78`, `415cad316` | Make the user surface navigable, structured, and less noisy during long sessions. |
| Insight and retrospective memory | `55d4e11aa`, `88e75bc1d`, `ce9700ab5`, `5b71b8ecf`, `10ab81459` | Convert session history into reports, repo memory, updated instructions, and reusable skills. |
| Packaging and release hardening | `cbd9e0d7d`, `b30d46c3b`, `4194a80f1`, `d359e3ffa`, `dae2b6356` | Ship reliable tagged releases and multi-platform runtime wheels without mismatched artifacts. |
| Runtime resilience and portable validation | `b83f6f399`, `aa67cc953` | Keep streaming behavior and validation stable when retry logic or alternate codegen backends would otherwise change runtime guarantees. |

## Use Case Inventory

| ID | Use case | Primary actor | Goal |
| ----- | ----- | --- | --- |
| UC-1 | Keep Codex running across profiles and failures | Workspace user | Continue work despite rate limits, auth failures, or provider-specific outages. |
| UC-2 | Resume and navigate long-running workspace sessions | Workspace user | Re-enter a saved thread and recover operational context quickly. |
| UC-3 | Define and manage workspace-local workflows | Workspace user | Turn recurring work into runnable jobs and triggers stored in repo-local YAML. |
| UC-4 | Run follow-up automation without blocking the main thread | Workspace user | Let turn completion or file events trigger more work while keeping the main conversation responsive. |
| UC-5 | Bridge Feishu conversations into Codex threads | External collaborator and workspace user | Accept inbound Feishu messages, bind them to the right thread, and return final replies outward. |
| UC-6 | Coordinate Feishu websocket ownership across Codex processes | Workspace user | Ensure only the intended Codex process owns the embedded Feishu websocket for a given app while allowing explicit preemption. |
| UC-7 | Keep the user in the loop with structured control | Workspace user | Give explicit answers, inspect hidden state, jump through history, and reduce UI noise during extended operation. |
| UC-8 | Inspect local session behavior offline | Workspace user | Generate an inspectable report from rollout history without relying on hosted analytics. |
| UC-9 | Convert a completed thread into reusable repo memory | Workspace user | Run `/dream` to update memory, `AGENTS.md`, and repo-local skills for future sessions. |
| UC-10 | Publish a correct multi-platform `codex-enhanced` release | Release maintainer | Build, tag, validate, and publish wheels whose embedded runtime matches the release version. |

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
  - Existing routing and config state is preserved.
  - Session continuity is not silently broken by profile switch behavior.
- Success guarantees:
  - Work continues on a usable profile.
  - The user can see or manage the fallback route explicitly.
- Trigger:
  - A provider errors due to rate limit, auth failure, or overload, or the user wants to switch runtime profile.
- Main success scenario:
  1. User opens or uses profile routing controls.
  2. Codex detects the current profile is unsuitable or the user selects another route.
  3. Codex keeps session and runtime continuity while switching the active route.
  4. The current thread remains usable without manual environment rewriting.
- Extensions:
  - 2a. No fallback route is configured:
    Codex reports the issue and leaves the user in control instead of inventing a route.
  - 3a. Thread unsubscribe or session handoff would break continuity:
    Codex preserves the bound thread and session behavior and avoids losing the current work surface.
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
  - Workflow files remain local and inspectable.
  - Parse or validation failures surface without partially hidden state.
  - Failed runs do not block the rest of the interactive surface indefinitely.
- Success guarantees:
  - The user can create, edit, enable, disable, and run workflows.
  - Supported triggers and job settings execute using the intended thread and context behavior.
- Trigger:
  - The user wants to automate a recurring task or manage existing workflow jobs.
- Main success scenario:
  1. User opens `/workflow`.
  2. Codex loads workspace-local workflow definitions.
  3. User creates or edits a workflow, jobs, and triggers.
  4. Codex validates the workflow and persists the YAML locally.
  5. User runs the workflow manually or waits for a configured trigger.
  6. Codex executes the workflow without blocking unrelated user actions.
- Extensions:
  - 3a. Workflow uses timeout, retry, or background execution:
    Codex preserves those semantics across rounds and runtime boundaries.
  - 4a. YAML is invalid or no-op:
    Codex reports the failure clearly and does not pretend the workflow is active.
  - 6a. Triggered work binds to a thread:
    Codex routes the follow-up into the bound thread instead of spawning unrelated context.

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
  - A Codex thread can be bound to a Feishu session or channel.
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

## UC-6 Coordinate Feishu Websocket Ownership Across Codex Processes

- Scope: `codex-clawbot` Feishu coordination within `codex-enhanced`
- Level: sea-level operational goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User running multiple workspaces or terminals: only the intended process should hold the embedded websocket for a given Feishu app.
  - External collaborators: inbound messages should continue flowing through one live owner instead of being duplicated or lost.
  - Feishu app owner: runtime ownership state should be app-owned, inspectable, and repairable without adding a separate paid coordinator.
  - Runtime: stale leaders, split-brain, and schema drift should be surfaced instead of silently tolerated.
- Preconditions:
  - Feishu app credentials are configured.
  - `feishu.coordination.base_token` points to a Base the app can read and write.
  - One or more Codex processes may contend for the same Feishu `app_id`.
- Minimal guarantees:
  - A non-leader process does not continue pretending it owns the websocket.
  - Expired force-intent or heartbeat state stops influencing leadership.
  - Permission, schema, or table-resolution failures are surfaced with repairable guidance.
  - Coordination state remains stored in Feishu Base and owned by the app credentials, not a hidden sidecar service.
- Success guarantees:
  - Exactly one intended process acts as websocket owner for the active `app_id`.
  - A user can deliberately preempt ownership for the current session.
  - If table IDs are omitted, clawbot discovers or creates the required coordination tables and fields automatically.
- Trigger:
  - A coordinated clawbot runtime starts, refreshes leadership, or the user enables forced websocket preemption from `/clawbot`.
- Main success scenario:
  1. Codex starts clawbot runtime with Feishu coordination configured.
  2. Clawbot resolves its process identity and discovers or creates the coordination tables in Feishu Base.
  3. Clawbot writes or refreshes its heartbeat row for the current `app_id` and instance.
  4. Clawbot reads active heartbeat and force-intent rows and computes the elected owner deterministically.
  5. If elected leader, this Codex process opens or keeps the websocket and handles inbound Feishu events.
  6. If not elected, this Codex process stays in follower mode and continues publishing heartbeat only.
  7. When the user enables force connect, clawbot continuously refreshes the force-intent row for the current session until disabled.
  8. Other contenders observe the updated intent and yield websocket ownership on their next coordination refresh.
- Extensions:
  - 2a. The configured `base_token` is invalid or inaccessible:
    Clawbot surfaces the failure and does not pretend coordination is active.
  - 2b. The configured table IDs are stale or the schema drifted:
    Clawbot validates the table shape, explains what is wrong, and allows repair or recreation instead of writing into an incompatible schema.
  - 4a. The previous leader disappears without cleanup:
    Its heartbeat expires by TTL and the next eligible live contender becomes owner.
  - 4b. Contenders have equal priority:
    The deterministic tie-break falls back to `instance_id` and then `session_id`.
  - 7a. Force connect is disabled or the owning process stops refreshing:
    The force-intent expires and normal priority-based election resumes.

## UC-9 Convert A Completed Thread Into Reusable Repo Memory

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

## Acceptance Criteria

- Users can keep work running across profile errors without manual environment rewriting.
- Saved sessions can be resumed with enough navigation and visibility control to recover context quickly.
- Workflows are repo-local, editable, runnable, and visibly fail when misconfigured or unrunnable.
- After-turn and background workflow activity does not freeze the main interactive thread.
- Feishu session bindings survive routine runtime disruptions and support visible inbound and outbound handling.
- When multiple Codex processes share the same Feishu app, only the elected owner keeps the embedded websocket active.
- Users can explicitly preempt websocket ownership for the current session, and that preemption naturally expires when force refresh stops.
- Clawbot can discover or auto-provision the required Feishu Base coordination tables and fields when table IDs are not preconfigured.
- `/dream` produces repo-local memory updates through managed sections instead of ad hoc file rewrites.
- `/insight` generates an offline report from local rollout history.
- Tagged `codex-enhanced` releases validate version alignment before publishing wheels.

## Implementation Slices Suggested By The Use Cases

- Slice 1: profile-router runtime continuity, fallback policy, and respawn/session-arg preservation.
- Slice 2: session continuity UX including resume picker, jump-to-message, timestamps, and visibility preferences.
- Slice 3: workflow scheduler and runtime, trigger semantics, timeout and retry handling, and TUI workflow controls.
- Slice 4: Feishu clawbot bridge, binding store, message delivery, and session recovery flows.
- Slice 5: Feishu Base coordination backend, heartbeat and force-intent election, auto-provisioned schema management, and leader-or-follower runtime behavior.
- Slice 6: structured human-in-the-loop controls including `question`, richer file reading and search tools, and lower-noise TUI affordances.
- Slice 7: retrospective stack including `/dream`, repo-local memory storage, managed `AGENTS.md` updates, and generated skill updates.
- Slice 8: offline observability via `/insight`.
- Slice 9: release automation for Python runtime packaging, artifact reuse, and multi-platform validation.

## Open Questions

- Should Feishu Base coordination remain `app_id`-global, or should future ownership be partitioned more narrowly by channel or bound conversation?
- Should force-connect remain a persistent workspace setting, or should it become an explicit session-scoped lease with stronger expiry semantics?
- Should Feishu remain the only external bridge, or is the longer-term use case actually "generic external user inbox" with Feishu as the first adapter?
- Should `/dream` stay repo-root-focused, or should future iterations target nested `AGENTS.md` scopes automatically?
- Should `/insight` remain a local offline artifact only, or should it eventually feed runtime guidance and profile or workflow tuning loops?
