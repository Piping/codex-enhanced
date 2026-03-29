---
name: enhanced-release
description: Use when the user asks to cut, push, or monitor a release tag on the enhanced Codex fork, especially when they mention enhanced-release, new tag, GitHub release workflow, or release run status.
---

For this fork, the standard release path is the `enhanced-release` GitHub workflow on `Piping/codex-enhanced`.

Default workflow:

1. Inspect `git status --short`, `git tag --sort=-version:refname`, and recent commits.
2. Determine the next patch version instead of reusing the current workspace version.
3. Update the workspace version before tagging.
4. Keep the release commit narrow. Do not pull in unrelated untracked files.
5. Create a release commit like `chore: release x.y.z`.
6. Create tag `vx.y.z`.
7. Push `main` and the tag to remote `enhanced`.
8. Query the `enhanced-release` workflow run and report the run id and status.

Guardrails:

- Do not assume `HEAD` already has the right version number.
- Do not include scratch files, local scripts, `.drawio` sources, or other unrelated untracked files unless the user explicitly asks.
- If the user asks for the workflow by name, make sure the response explicitly references `enhanced-release`.

