# Codex Tool Design Principles [proposal]

Status: proposal

This document proposes a default design stance for Codex tools, based on direct user feedback gathered through structured prompts. It is intended to guide future tool design, tool API evolution, and approval UX decisions across Codex surfaces.

## Goals

Codex tools should optimize for the following, in order:

1. Reliable task completion.
2. Strong traceability and execution evidence.
3. Low-friction operation with bounded safety controls.
4. Extensibility without excessive schema or UX complexity.

The proposal explicitly does not optimize for maximum autonomy at any cost, nor for fully manual operation. The preferred stance is a balanced default with clear risk boundaries.

## Core principles

### 1. Prioritize task completion over tool purity

Tools exist to help Codex complete user work, not to preserve a minimal or elegant abstraction at the expense of success rate. A tool system that is slightly redundant but materially more reliable is preferable to a perfectly uniform system that fails in real workflows.

### 2. Use a mixed abstraction stack

Codex should expose both:

- low-level primitives such as command execution, patch application, browsing, and structured questioning
- a limited set of higher-level capabilities where the workflow is stable and repeatedly valuable

This avoids forcing every task through brittle high-level tools while still leaving room for opinionated workflow helpers.

### 3. Prefer medium-granularity tool boundaries

Tool sets should avoid both extremes:

- too few tools, where each tool becomes overloaded and hard to steer
- too many tools, where discovery, selection, and maintenance become noisy

A medium-granularity tool catalog is preferred, with clear responsibilities and predictable calling conventions.

### 4. Keep schemas compact, but enforce strong conventions

Tool APIs should generally prefer semi-structured inputs and outputs:

- keep core fields explicit and typed
- allow flexible text where rigid structure adds little value
- avoid repeating long schema descriptions that inflate token usage

Short descriptions plus strong naming conventions are preferred over verbose per-call schema payloads.

### 5. Default to balanced autonomy

The preferred default is:

- routine, low-risk work proceeds automatically
- higher-risk work requires approval
- obviously dangerous actions are hard-blocked rather than delegated to model judgment

This keeps the system usable without normalizing destructive or unverifiable behavior.

## Approval and safety model

### 6. Favor preauthorization over repetitive prompts

When approval is required, the preferred model is scoped preauthorization rather than repeated per-command interruptions. Command-prefix-based authorization is a strong default because it maps well to practical workflows such as tests, builds, and diagnostics.

Approval UX should be short and purpose-driven. The system should explain what it wants to do and why, without burying the user in unnecessary detail.

### 7. Hard-block a small set of dangerous actions

Some actions should not be left to ordinary approval flow. A narrow class of obviously dangerous operations should be blocked by policy, regardless of model confidence.

### 8. Safety should be risk-layered, not uniformly restrictive

The preferred tradeoff is not "safety first" in every case. It is capability-first within a risk-layered system:

- low-risk operations should have low friction
- high-risk operations should have stronger controls
- unverifiable claims of execution should never be acceptable

## Observability and trust

### 9. Every tool call should be traceable

Traceability is not optional. At minimum, the system should retain a per-call record of:

- tool name
- high-level purpose
- key inputs
- timestamp
- result status
- failure reason when applicable

### 10. Execution should carry lightweight proof

When a tool performs an action, the default evidence bar should be lightweight but real. For command execution, the preferred baseline is:

- exit code
- key output

Where possible, tools should also point to external verification artifacts such as file changes, test results, or request logs.

### 11. Progress updates should stay concise

The preferred progress model is "key steps visible." Users generally want:

- a short command or action summary
- a short result summary

Progress should not devolve into log spam, but it also should not disappear until the end.

## Tool behavior

### 12. Failures should trigger limited self-healing

When a tool fails, the preferred behavior is:

1. Attempt a small, safe amount of self-recovery.
2. If recovery fails, report the failure with context.

The system should avoid both extremes of infinite silent retries and immediate escalation for every routine hiccup.

### 13. Retry semantics should be tool-declared

Retryability should not be guessed blindly. Tools should explicitly declare whether they are safe to retry and under what conditions. The platform may still make the final retry decision, but the tool must provide the signal.

### 14. Cancellation should support compensation where possible

For long-running or side-effecting tools, cancellation should aim for more than best-effort termination. The preferred model is cancellation plus compensation or cleanup when feasible.

### 15. Partial success should be a first-class result

Tools should be able to return an explicit partial-success state rather than collapsing mixed outcomes into either "success" or "failure." This is especially important for multi-step operations and tool batches.

## State, recovery, and compatibility

### 16. Prefer lightweight short-term state

Tools should primarily rely on current task context and lightweight short-term state. Long-term memory can exist, but it should be explicit and bounded rather than silently shaping every interaction.

### 17. Resume should combine checkpoints and logs

Long-running work should be recoverable through a hybrid of:

- explicit checkpoints where useful
- event or call logs for replay and auditing

### 18. Preserve strong API compatibility

Tool APIs should evolve carefully. Backward compatibility is a strong preference, with migration paths when changes are unavoidable.

## Tool-specific guidance

### 19. `web` should make search a first-class capability

The most important gap in web tooling is native search. Browsing alone is not enough. The preferred default search result shape is:

- title
- short summary
- source

Browser-style interaction can expand later, with session and cookie handling as an especially valuable follow-on capability.

### 20. `question` should be used for constraints and decisions, not as a crutch

`question` is best suited for:

- collecting user constraints and preferences up front
- handling meaningful branch decisions mid-task

It should not be used as a substitute for basic local context gathering. Repetitive questioning before inspecting the workspace erodes trust quickly.

The most valuable near-term improvements to `question` are:

- multi-select answers
- ranked multi-select
- default values
- basic validation for required fields and simple formats

Conditional branching may be useful later, but is not required for an initial improved version.

### 21. Subagents should stay explicitly user-authorized

Delegation is valuable, but it should remain visible and intentional. The preferred model is a main agent orchestrator that uses subagents only after explicit user authorization.

## Tool API guidance

The following fields are strong candidates for a common tool contract:

- `name`
- `description`
- `trigger_conditions`
- `dependencies`
- `timeout`
- `retry`
- `idempotent`
- `cancelable`
- `requires_approval`

Tool cards or UI surfaces should, at minimum, expose:

- name
- description
- trigger conditions
- dependencies

Implementation details such as caching can remain mostly hidden unless they materially affect behavior.

## Non-goals

This proposal does not attempt to:

- define a single universal schema for every tool
- require all tools to expose the same UI surface
- replace product-specific policy or sandbox constraints
- prescribe a complete browser automation model

## Evaluation criteria

A future tool or API change is aligned with this proposal if it tends to improve one or more of the following without materially regressing the others:

- completion reliability
- user trust
- evidence and traceability
- approval ergonomics
- token efficiency
- extensibility

## Next steps

Potential follow-up work:

1. Define a minimal shared tool metadata contract.
2. Redesign `web` around first-class search plus clearer result objects.
3. Expand `question` to support ranked multi-select and defaults.
4. Standardize per-call execution evidence and partial-success reporting.
5. Review approval flows for repeated same-class command prompts.
