---
name: deep-discovery
description: "Use when the user wants a design-first workflow: read local docs like AGENTS.md and proposal/design files, ask detailed follow-up questions, use question tool where helpful, and only produce proposal/design/todos after high-confidence understanding."
---

This skill is for design and discovery work before implementation.

Default flow:

1. Read local context first:
   - `AGENTS.md`
   - `README.md`
   - `design.md`
   - `proposal.md`
   - other nearby docs the task explicitly references
2. Build understanding before proposing a solution.
3. Ask complete follow-up questions. When many answers are needed, prefer the `question` tool.
4. Keep asking until the user's real goal, constraints, and success criteria are clear enough to design against.
5. Then summarize using STAR:
   - Situation
   - Task
   - Action
   - Result
6. Use first-principles reasoning. If the requested path is not the best path, say so and explain why.

Guardrails:

- Do not rush into code when the user asked for proposal or design first.
- Do not assume the user already knows the best solution shape.
- If motivation or constraints are unclear, pause and ask instead of inventing certainty.
- Once the goal is clear, keep the proposal concrete: data model, state model, file touch points, and validation commands.
