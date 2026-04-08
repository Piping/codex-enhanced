# Workflows

Codex TUI can load workflow definitions from `.codex/workflows/*.yaml` and manage them directly from the `/workflow` menu.

## Files

- Workflow files live under `.codex/workflows/`.
- If no workflow file exists yet, `/workflow` offers `Create workflow.yaml`.
- Creating the template writes a starter file and opens it in the configured external editor.

## Root Menu

Running `/workflow` opens a flattened menu. Each workflow contributes entries in this shape:

- `<workflow name> - edit yaml`
- `<workflow name> - job - <job name>`
- `<workflow name> - trigger - <trigger id>`

The root menu also shows:

- `Background Tasks`

`Background Tasks` shows the same running and queued workflow state as `/ps`.

## Workflow File Entry

`<workflow name> - edit yaml` opens the real workflow file in the external editor.

This is the fastest path for:

- creating or removing jobs
- creating or removing triggers
- repairing invalid YAML
- editing fields that are not exposed as structured menu actions

## Job Entry

`<workflow name> - job - <job name>` opens the job management page.

The job page can:

- `Run Now`
- enable or disable the job flag in YAML
- edit common structured fields such as `context`, `response`, `needs`, and `steps`
- open the workflow YAML directly

Behavior notes:

- `Run Now` still works even if the job has `enabled: false`.
- Job `enabled: false` only affects workflow-controlled selection logic. It does not block an explicit manual `Run Now` from the menu.

## Trigger Entry

`<workflow name> - trigger - <trigger id>` opens the trigger management page.

The trigger page can:

- `Run Now`
- `Enable Trigger` or `Disable Trigger`
- change `Type`
- edit `Trigger ID`
- edit `Target Jobs`
- edit the trigger-specific parameter
- open the workflow YAML directly

Behavior notes:

- Trigger `enabled: false` disables the trigger itself.
- A disabled trigger cannot be started from `Run Now` until it is enabled again.
- `Run Now` is available for any enabled trigger type, not only `manual`.
- `After Turn` runs are dispatched as background workflow tasks after the turn finishes, so the main thread stays responsive and the transcript shows workflow start/completion cells separately.

## Trigger Types

The `Type` picker supports:

- `Manual`
- `Before Turn`
- `After Turn`
- `Idle`
- `Interval`
- `Cron`

Changing the type updates the structured trigger fields in YAML:

- `Idle` uses `after`
- `Interval` uses `every`
- `Cron` uses `cron`
- `Manual`, `Before Turn`, and `After Turn` do not require an extra schedule parameter

When the current type has a parameter, the trigger page exposes a matching action:

- `Edit Idle Delay`
- `Edit Interval`
- `Edit Cron Schedule`

## Typical Flow

1. Run `/workflow`.
2. Pick a flattened root entry such as `director - trigger - review_backlog`.
3. Use structured actions for small changes like enable/disable, `Run Now`, type changes, or parameter edits.
4. Use `edit yaml` when the change is broader than the structured menu supports.

## Example

Given this workflow:

```yaml
name: director

triggers:
  - id: pulse
    type: interval
    every: 30m
    enabled: true
    jobs: [notify]

jobs:
  notify:
    enabled: false
    context: ephemeral
    response: assistant
    steps:
      - prompt: |
          Send a concise update.
```

The root menu includes:

- `director - edit yaml`
- `director - job - notify`
- `director - trigger - pulse`

From `director - trigger - pulse`, you can:

- run it immediately
- disable the trigger
- switch `interval` to `idle`
- change the parameter from `every: 30m` to `after: 30m`
