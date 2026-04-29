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
- edit common structured fields such as `context_strategy`, `execution_strategy`, `response`, `needs`, and `steps`
- open the workflow YAML directly

Behavior notes:

- `Run Now` still works even if the job has `enabled: false`.
- Job `enabled: false` only affects workflow-controlled selection logic. It does not block an explicit manual `Run Now` from the menu.
- `context_strategy` is required on every job. The supported values are:
- `embed`: run prompt output inline against the current thread.
- `embed_compact`: compact the current main thread first, then queue the workflow follow-up inline. This is not allowed for `before_turn` triggers.
- `thread_auto`: run in a workflow child thread, forking the current primary thread when possible and otherwise starting a new one.
- `thread_new`: always start a fresh workflow child thread.
- `thread_fork`: require an existing primary thread and fork it.
- `thread_fork_compact`: fork the current primary thread, compact the child thread, then run the workflow there.
- `embed` and `embed_compact` only support prompt steps. Jobs using either strategy cannot include `run` steps.
- `execution_strategy` is also required on every job.
- `inherit_session`: inherit the current primary session's `cwd`, model, approval policy, reviewer, sandbox policy, service tier, and reasoning effort. If there is no current primary session, the workflow run fails visibly.
- `override_yolo`: inherit the current primary session's execution context, but override approvals to `Never` and sandbox to `DangerFullAccess`. If there is no current primary session, the workflow run fails visibly.

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

- Every trigger must declare `bind_thread`.
- `bind_thread: all` allows the trigger to run for any Codex primary thread using this workspace.
- `bind_thread: ["<thread-id>", ...]` restricts the trigger to the listed Codex primary thread ids.
- Omitting `bind_thread` is invalid. Codex rejects the workflow file instead of guessing a default.
- `bind_thread` only gates whether the current Codex primary thread may start the trigger. It does not replace or modify job `context_strategy`.
- Trigger `enabled: false` disables the trigger itself.
- A disabled trigger cannot be started from `Run Now` until it is enabled again.
- `Run Now` is available for any enabled trigger type, not only `manual`.
- `Run Now` still respects `bind_thread`. If the current primary thread is not allowed, the run fails visibly instead of bypassing the restriction.
- If a trigger resolves only to disabled or otherwise unrunnable jobs, the run fails visibly instead of silently doing nothing.
- `After Turn` runs are dispatched as background workflow tasks after the turn finishes, so the main thread stays responsive and the transcript shows workflow start/completion cells separately.
- `After Turn` defaults to `condition: turn_succeeded`, which means the previous turn must finish successfully before the trigger runs. Set `condition: turn_finished` when follow-up work should also run after failed turns.
- `response: user` follow-ups can recursively re-trigger `after_turn`. The chain naturally stops when the workflow returns an empty reply, because no follow-up turn is queued.
- Workflow steps default to a 30s timeout. Override this per step with `timeout`, for example `timeout: 5m`.
- Timeout failures participate in workflow step retry behavior, including one automatic timeout retry.

## Trigger Types

The `Type` picker supports:

- `Manual`
- `Before Turn`
- `After Turn`
- `File Watch`
- `Idle`
- `Interval`
- `Cron`

Changing the type updates the structured trigger fields in YAML:

- `After Turn` uses `condition` with values `turn_finished` or `turn_succeeded`
- `File Watch` watches the current workspace recursively and fires when a regular file or a directory changes
- `Idle` uses `after`
- `Interval` uses `every`
- `Cron` uses `cron`
- `Manual`, `Before Turn`, and `File Watch` do not require an extra trigger parameter

When the current type has a parameter, the trigger page exposes a matching action:

- `Edit Run Condition`
- `Edit Idle Delay`
- `Edit Interval`
- `Edit Cron Schedule`

Behavior notes:

- `File Watch` uses the same global trigger queue as the other trigger types.
- If the same `file_watch` trigger is already running or already queued, new matching file events are skipped (`overlap=skip`) instead of enqueueing duplicates.

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
    bind_thread: all
    every: 30m
    enabled: true
    jobs: [notify]

jobs:
  notify:
    enabled: false
    context_strategy: thread_auto
    execution_strategy: inherit_session
    response: assistant
    steps:
      - prompt: |
          Send a concise update.
        timeout: 2m
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
