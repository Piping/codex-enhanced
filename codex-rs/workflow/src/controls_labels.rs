use crate::definition::WorkflowContextMode;
use crate::definition::WorkflowResponseMode;
use crate::definition::WorkflowTriggerKind;
use crate::editor::WorkflowJobEditableField;
use crate::editor::WorkflowTriggerEditableField;
use crate::editor::WorkflowTriggerType;

pub fn workflow_context_label(context: WorkflowContextMode) -> &'static str {
    match context {
        WorkflowContextMode::Embed => "Embed",
        WorkflowContextMode::Ephemeral => "Ephemeral",
    }
}

pub fn workflow_trigger_type_label(trigger_type: WorkflowTriggerType) -> &'static str {
    match trigger_type {
        WorkflowTriggerType::Manual => "Manual",
        WorkflowTriggerType::BeforeTurn => "Before Turn",
        WorkflowTriggerType::AfterTurn => "After Turn",
        WorkflowTriggerType::FileWatch => "File Watch",
        WorkflowTriggerType::Idle => "Idle",
        WorkflowTriggerType::Interval => "Interval",
        WorkflowTriggerType::Cron => "Cron",
    }
}

pub fn workflow_response_label(response: WorkflowResponseMode) -> &'static str {
    match response {
        WorkflowResponseMode::Assistant => "Assistant",
        WorkflowResponseMode::User => "User",
    }
}

pub fn workflow_job_field_label(field: WorkflowJobEditableField) -> &'static str {
    match field {
        WorkflowJobEditableField::Needs => "needs",
        WorkflowJobEditableField::Steps => "steps",
    }
}

pub fn workflow_trigger_field_label(field: WorkflowTriggerEditableField) -> &'static str {
    match field {
        WorkflowTriggerEditableField::Id => "id",
        WorkflowTriggerEditableField::Jobs => "jobs",
        WorkflowTriggerEditableField::Parameter => "parameter",
        WorkflowTriggerEditableField::BindThread => "bind_thread",
    }
}

pub(crate) fn workflow_trigger_kind_display(kind: &WorkflowTriggerKind) -> String {
    match kind {
        WorkflowTriggerKind::Manual => "Manual".to_string(),
        WorkflowTriggerKind::BeforeTurn => "Before Turn".to_string(),
        WorkflowTriggerKind::AfterTurn => "After Turn".to_string(),
        WorkflowTriggerKind::FileWatch => "File Watch".to_string(),
        WorkflowTriggerKind::Idle { after } => format!("Idle ({after})"),
        WorkflowTriggerKind::Interval { every } => format!("Interval ({every})"),
        WorkflowTriggerKind::Cron { cron } => format!("Cron ({cron})"),
    }
}

pub(crate) fn workflow_trigger_type_description(trigger_type: WorkflowTriggerType) -> &'static str {
    match trigger_type {
        WorkflowTriggerType::Manual => "Run only when triggered from the workflow menu.",
        WorkflowTriggerType::BeforeTurn => "Run automatically before the next user turn.",
        WorkflowTriggerType::AfterTurn => "Run automatically after the current turn finishes.",
        WorkflowTriggerType::FileWatch => {
            "Run automatically when workspace files change. Overlapping runs are skipped."
        }
        WorkflowTriggerType::Idle => "Run after the workspace has been idle for a duration.",
        WorkflowTriggerType::Interval => "Run on a fixed repeating interval.",
        WorkflowTriggerType::Cron => "Run on a cron schedule.",
    }
}

pub(crate) fn workflow_trigger_matches_type(
    kind: &WorkflowTriggerKind,
    trigger_type: WorkflowTriggerType,
) -> bool {
    matches!(
        (kind, trigger_type),
        (&WorkflowTriggerKind::Manual, WorkflowTriggerType::Manual)
            | (
                &WorkflowTriggerKind::BeforeTurn,
                WorkflowTriggerType::BeforeTurn
            )
            | (
                &WorkflowTriggerKind::AfterTurn,
                WorkflowTriggerType::AfterTurn
            )
            | (
                &WorkflowTriggerKind::FileWatch,
                WorkflowTriggerType::FileWatch
            )
            | (&WorkflowTriggerKind::Idle { .. }, WorkflowTriggerType::Idle)
            | (
                &WorkflowTriggerKind::Interval { .. },
                WorkflowTriggerType::Interval
            )
            | (&WorkflowTriggerKind::Cron { .. }, WorkflowTriggerType::Cron)
    )
}

pub(crate) fn workflow_trigger_parameter_metadata(
    kind: &WorkflowTriggerKind,
) -> Option<(&'static str, String)> {
    match kind {
        WorkflowTriggerKind::Idle { after } => {
            Some(("Idle Delay", format!("Current `after`: `{after}`.")))
        }
        WorkflowTriggerKind::Interval { every } => {
            Some(("Interval", format!("Current `every`: `{every}`.")))
        }
        WorkflowTriggerKind::Cron { cron } => {
            Some(("Cron Schedule", format!("Current `cron`: `{cron}`.")))
        }
        WorkflowTriggerKind::Manual
        | WorkflowTriggerKind::BeforeTurn
        | WorkflowTriggerKind::AfterTurn
        | WorkflowTriggerKind::FileWatch => None,
    }
}

pub(crate) fn workflow_trigger_types() -> [WorkflowTriggerType; 7] {
    [
        WorkflowTriggerType::Manual,
        WorkflowTriggerType::BeforeTurn,
        WorkflowTriggerType::AfterTurn,
        WorkflowTriggerType::FileWatch,
        WorkflowTriggerType::Idle,
        WorkflowTriggerType::Interval,
        WorkflowTriggerType::Cron,
    ]
}
