# Cockburn Core

This reference distills the parts of Alistair Cockburn's method that matter most in day-to-day product and engineering work.

## Core Idea

A use case is a behavioral contract for the system under discussion. The contract is organized around a primary actor's goal and must protect the interests of all relevant stakeholders.

The three concepts that most often decide quality are:

- scope: what is the system under discussion
- primary actor: who has the goal
- level: how high or low the goal is

If any of these are wrong, the use case will usually drift into feature soup.

## Goal Levels

Use these as a practical filter:

- cloud: very high summary; strategy or lifecycle context
- kite: summary; spans multiple user-goal sessions
- sea-level: user goal; the default level for feature design
- fish or underwater: subfunction; below user goal, sometimes useful
- clam: too low-level; usually should not be a standalone use case

Practical rule:

- RPD and implementation planning should usually center on sea-level use cases.
- Cloud and kite are useful to explain why a feature exists.
- Fish or underwater is useful only when a risky or reusable subflow needs its own treatment.
- Clam usually indicates accidental decomposition into UI or code detail.

## Formats

Use the lightest format that preserves clarity.

### Brief

Use when scoping or prioritizing.

- 2 to 6 sentences
- mentions the most important activity and failures

### Casual

Use when exploring or collaborating in a low-ceremony setting.

- title
- primary actor
- scope
- level
- one or two paragraphs of prose

### Fully Dressed

Use when requirements must survive handoff, review, estimation, or implementation.

- use case name
- context of use or goal in context
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
- technology and data variations list
- related information if needed

## What Goes Where

### Preconditions

What must already be true before the use case starts.

### Minimal Guarantees

What remains protected on every exit, especially when the main goal fails.

Typical examples:

- audit log exists
- request is traceable
- no double charge
- draft is preserved
- user sees a recoverable failure state

### Success Guarantees

What is true if the goal succeeds.

Typical examples:

- order is placed
- payment is authorized
- ticket is created
- state transition is committed

### Main Success Scenario

The straight path from trigger to success guarantee when nothing significant goes wrong.

Guidance:

- write 3 to 9 meaningful steps when possible
- each step should move the story forward
- write intent and responsibility, not GUI choreography

### Extensions

Alternative or exception paths attached to a specific step.

Use extensions for:

- invalid input
- policy rejection
- integration failure
- timeout
- cancellation
- partial completion
- fallback behavior

### Technology and Data Variations

Use this when the same behavior stays the same but the technology or data shape varies.

Do not hide true behavioral branches here.

## Breadth-First Progression

Cockburn's method works best in passes:

1. create an actor-goal list
2. identify scope and level
3. write brief or casual use cases
4. fully dress only the priority or risky ones
5. derive tests, estimates, and implementation slices

This is better than trying to fully specify every use case on day one.

## Quality Checks

Ask these before accepting a use case:

- Is scope explicit and stable?
- Is the primary actor the one with the goal?
- Is the level right, and preferably sea-level for implementation-driving work?
- Does the main success scenario run from trigger to success guarantee?
- Are stakeholder interests named?
- Are minimal guarantees protecting those interests on failure exits?
- Are important extensions attached to concrete steps?
- Are technology/data variations separated from behavioral branches?
- Is the text free of premature UI or code detail?

## RPD Mapping

Translate a good use case into product artifacts like this:

- scope -> product boundary and non-goals
- primary actor -> target user or initiating system
- stakeholders and interests -> business, ops, legal, finance, support constraints
- preconditions -> feature flags, account states, permissions, setup assumptions
- minimal guarantees -> resilience and compliance requirements
- success guarantees -> acceptance criteria
- main success scenario -> happy path narrative
- extensions -> error cases, edge cases, exception rules
- technology/data variations -> platform or channel differences

## Engineering Mapping

Translate a good use case into implementation artifacts like this:

- trigger -> entrypoint, API, event, UI action, job, or schedule
- main scenario steps -> commands, services, handlers, and state transitions
- supporting actors -> dependencies and interfaces
- minimal guarantees -> logging, idempotency, retries, compensations
- success guarantees -> assertions and integration tests
- extensions -> negative tests and recovery paths

## Source Pointers

Primary references used for this distilled guide:

- Alistair Cockburn, "Structuring Use Cases with Goals": https://www.cs.otago.ac.nz/coursework/cosc461/usecases.htm
- Alistair Cockburn, "Use Case Template": https://www.cs.otago.ac.nz/coursework/cosc461/uctempla.htm
- Book extract for "Writing Effective Use Cases": https://www.cs.otago.ac.nz/coursework/cosc461/weucx.pdf
