# Slash commands

For an overview of Codex CLI slash commands, see [this documentation](https://developers.openai.com/codex/cli/slash-commands).

For TUI workflow management with `/workflow`, see [Workflows](workflows.md).

## `/insight`

`/insight` scans local Codex session rollouts and writes an offline HTML analysis report to `~/.codex/reports/`.

What it includes:

- active + archived local sessions
- main threads + sub-agent threads
- roll-up from child threads into the parent/root session, with per-thread drill-down kept in the report
- token usage, wall-clock span, exec/tool/patch counts, and failure counts
- exact metrics where the rollout history persists them directly
- estimated timing breakdowns where historical rollout data only supports approximation

Report behavior:

- output is a single self-contained HTML file
- report is readable offline with no external assets
- dashboard sections come first, followed by deeper per-session drill-down
- common patterns and efficiency suggestions fall back to local heuristics when an AI summary layer is unavailable

Notes on timing precision:

- exec command, MCP tool, and dynamic tool durations are exact when present in the rollout
- wall-clock session spans are derived from persisted rollout timestamps
- model time and user-wait time are reported as estimates for historical sessions, because older rollout data does not persist a complete end-to-end timing decomposition for every turn
