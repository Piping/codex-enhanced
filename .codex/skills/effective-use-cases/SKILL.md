---
name: effective-use-cases
description: Use when writing, reviewing, or refining product RPD/PRD, feature specs, acceptance criteria, or implementation plans that should be grounded in Alistair Cockburn-style effective use cases. Apply goal-level thinking, explicit scope, primary actor, stakeholders and interests, preconditions, minimal guarantees, success guarantees, main success scenario, extensions, and technology/data variations. Use for Chinese or English requests involving use cases, actor-goal analysis, edge cases, feature slicing, or turning product requirements into implementation-ready behavior.
---
# Effective Use Cases

Use this skill to turn a feature request into a behavioral contract, then carry that contract forward into RPD and implementation.

The method is not "fill a template blindly." The method is:

1. Find the right goal.
2. Set the right system boundary.
3. Name the actor who has the goal.
4. Protect stakeholder interests on both success and failure paths.
5. Write the main success scenario and extensions at the right level.
6. Convert the result into acceptance and implementation slices.

Read [references/cockburn-core.md](references/cockburn-core.md) when you need the concise theory, templates, or quality checks.

## Default Stance

- Default to black-box, system-scope use cases for product requirements unless the user explicitly wants business-process modeling or white-box internals.
- Default to user-goal, sea-level use cases for features that should drive design and implementation.
- Treat summary use cases as context and roadmap, not as implementation-ready requirements.
- Treat subfunction use cases as supporting detail only when a lower-level interaction is reused or risky.
- Keep UI mechanics, screen choreography, API field trivia, and code structure out of the use case unless the user explicitly asks for them.

## Workflow

1. Identify the system under discussion.
   Name the black box explicitly. If the scope is unstable, stop and fix it before writing more.

2. Build an actor-goal list.
   List primary actor, supporting actors, off-stage stakeholders, and each actor's goal.
   If several goals appear, split them into separate use cases instead of forcing them into one flow.

3. Pick the goal level.
   Prefer user-goal level first.
   Ask "Can the primary actor go away happy after this?" If yes, it is likely sea-level.
   Ask "Why is the actor doing this?" to move upward.
   Ask "How is this done?" to move downward.

4. Stage the precision.
   Start with brief or casual use cases when exploring.
   Escalate to fully dressed use cases for risky, ambiguous, cross-team, regulated, or implementation-driving requirements.

5. Write the use case body.
   Include:
   - name as a short active verb phrase
   - scope
   - level
   - primary actor
   - stakeholders and interests
   - preconditions
   - minimal guarantees
   - success guarantees
   - trigger
   - main success scenario
   - extensions
   - technology and data variations when relevant

6. Check completeness through stakeholder protection.
   For every stakeholder interest, ask:
   - where is it satisfied on success?
   - where is it protected on failure?
   Missing answers usually mean missing requirements.

7. Derive product outputs.
   Convert the use case set into:
   - in-scope and out-of-scope behavior
   - acceptance criteria
   - edge cases and exception handling
   - operational guarantees such as logging, audit, rollback, notification, or compliance rules

8. Derive implementation outputs.
   Map each use case into:
   - domain commands or user intents
   - validation rules
   - external integrations
   - state transitions
   - failure handling and retries
   - test cases
   - delivery slices

## Output Shape

When the user asks for an RPD, PRD, feature spec, or implementation plan, prefer this structure:

1. Scope and goal level
2. Actor and stakeholder map
3. Use case inventory
4. One or more fully dressed priority use cases
5. Acceptance criteria
6. Implementation slices
7. Open questions

## RPD Rules

When writing or reviewing an RPD:

- Lead with user-goal use cases, not screens or endpoints.
- Put summary use cases before detailed ones if the feature spans multiple sessions or phases.
- Mark non-goals explicitly when nearby flows are easy to confuse.
- Separate true behavior changes from technology/data variations.
- Force failure paths into `Extensions`, not into vague prose like "handle errors gracefully."
- Write minimal guarantees even when the main goal fails. This often surfaces logging, idempotency, audit, draft-saving, and user-notification requirements.

## Implementation Rules

When using the skill during engineering:

- Keep one implementation slice tied to one user-goal or one risky extension.
- Do not implement directly from a summary use case without first breaking it into sea-level cases.
- Convert each extension into at least one concrete test.
- Convert minimal guarantees into resilience requirements.
- Convert success guarantees into done criteria.
- If the code design introduces behavior not present in the use case, either update the use case or justify the implementation-only concern separately.

## Review Heuristics

Flag these problems early:

- no explicit scope
- primary actor is actually a department, team, or vague persona with multiple goals
- one use case contains several user goals
- the main success scenario does not run to a clear success guarantee
- extensions are missing, especially for validation, dependency failure, timeout, cancellation, or partial completion
- stakeholder interests are unnamed, so audit, compliance, finance, support, or operations needs vanish
- the text describes UI clicks or internal components instead of intent and responsibility
- the team is trying to estimate or implement from cloud or kite level only

## Good Default Prompt

If the user is vague, treat the task as:
"Use effective-use-cases to turn this request into a goal-driven RPD: identify scope, actors, stakeholders, user-goal use cases, guarantees, main success scenarios, extensions, acceptance criteria, and implementation slices."
