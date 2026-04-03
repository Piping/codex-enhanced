use serde::Deserialize;
use serde::Serialize;

use crate::command::LoopSchedule;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LoopTriggerPhase {
    Timer,
    Idle,
    BeforeTurn,
    AfterTurn,
}

impl LoopTriggerPhase {
    pub const USER_SELECTABLE: [Self; 4] =
        [Self::Timer, Self::Idle, Self::BeforeTurn, Self::AfterTurn];

    pub fn title(self) -> &'static str {
        match self {
            Self::Timer => "Timer",
            Self::Idle => "Idle",
            Self::BeforeTurn => "Before Turn",
            Self::AfterTurn => "After Turn",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Timer => "Runs when timer-based loop triggers become due.",
            Self::Idle => "Runs after the main thread stays idle for the configured duration.",
            Self::BeforeTurn => "Runs before a user turn is submitted into the main thread model.",
            Self::AfterTurn => "Runs after the assistant final response completes.",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoopTriggerKind {
    Timer { schedule: LoopSchedule },
    Idle { after: LoopSchedule },
    BeforeTurn,
    AfterTurn,
}

impl LoopTriggerKind {
    pub fn phase(&self) -> LoopTriggerPhase {
        match self {
            Self::Timer { .. } => LoopTriggerPhase::Timer,
            Self::Idle { .. } => LoopTriggerPhase::Idle,
            Self::BeforeTurn => LoopTriggerPhase::BeforeTurn,
            Self::AfterTurn => LoopTriggerPhase::AfterTurn,
        }
    }

    pub fn short_label(&self) -> String {
        match self {
            Self::Timer { schedule } => format!("timer · {}", schedule.display()),
            Self::Idle { after } => format!("idle · {}", after.display()),
            Self::BeforeTurn => "before turn".to_string(),
            Self::AfterTurn => "after turn".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopTriggerBinding {
    pub id: String,
    pub enabled: bool,
    pub kind: LoopTriggerKind,
}

impl LoopTriggerBinding {
    pub fn selection_name(&self) -> String {
        self.kind.short_label()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoopContextMode {
    Embed,
    Ephemeral,
    #[default]
    Persistent,
}

impl LoopContextMode {
    pub const USER_SELECTABLE: [Self; 3] = [Self::Embed, Self::Ephemeral, Self::Persistent];

    pub fn title(self) -> &'static str {
        match self {
            Self::Embed => "Embed",
            Self::Ephemeral => "Ephemeral",
            Self::Persistent => "Persistent",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoopResponseMode {
    #[default]
    Assistant,
    User,
}

impl LoopResponseMode {
    pub const USER_SELECTABLE: [Self; 2] = [Self::Assistant, Self::User];

    pub fn title(self) -> &'static str {
        match self {
            Self::Assistant => "As Assistant Message",
            Self::User => "As User Message",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Assistant => {
                "Mirror the loop result into the main thread as an assistant message."
            }
            Self::User => "Submit the loop result back into the main thread as a user message.",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoopSecurityMode {
    #[default]
    Inherited,
    SpecifiedDirectory,
}

impl LoopSecurityMode {
    pub const USER_SELECTABLE: [Self; 2] = [Self::Inherited, Self::SpecifiedDirectory];

    pub fn title(self) -> &'static str {
        match self {
            Self::Inherited => "Inherited",
            Self::SpecifiedDirectory => "Specified Directory",
        }
    }
}

pub fn next_trigger_binding_id(bindings: &[LoopTriggerBinding]) -> String {
    let next = bindings
        .iter()
        .filter_map(|binding| binding.id.strip_prefix("trigger-"))
        .filter_map(|suffix| suffix.parse::<u32>().ok())
        .max()
        .map_or(1, |current| current.saturating_add(1));
    format!("trigger-{next}")
}

pub fn legacy_timer_binding(schedule: LoopSchedule) -> LoopTriggerBinding {
    LoopTriggerBinding {
        id: "trigger-1".to_string(),
        enabled: true,
        kind: LoopTriggerKind::Timer { schedule },
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::LoopTriggerBinding;
    use super::LoopTriggerKind;
    use super::LoopTriggerPhase;
    use super::legacy_timer_binding;
    use super::next_trigger_binding_id;
    use crate::LoopSchedule;

    #[test]
    fn next_trigger_binding_id_skips_to_next_numeric_suffix() {
        let bindings = vec![
            LoopTriggerBinding {
                id: "trigger-1".to_string(),
                enabled: true,
                kind: LoopTriggerKind::BeforeTurn,
            },
            LoopTriggerBinding {
                id: "trigger-4".to_string(),
                enabled: true,
                kind: LoopTriggerKind::AfterTurn,
            },
        ];

        assert_eq!("trigger-5", next_trigger_binding_id(&bindings));
    }

    #[test]
    fn legacy_timer_binding_uses_timer_phase() {
        let binding = legacy_timer_binding(LoopSchedule::Interval {
            display: "5m".to_string(),
            seconds: 300,
        });

        assert_eq!(LoopTriggerPhase::Timer, binding.kind.phase());
    }

    #[test]
    fn idle_binding_uses_idle_phase() {
        let binding = LoopTriggerBinding {
            id: "trigger-1".to_string(),
            enabled: true,
            kind: LoopTriggerKind::Idle {
                after: LoopSchedule::Interval {
                    display: "30m".to_string(),
                    seconds: 1_800,
                },
            },
        };

        assert_eq!(LoopTriggerPhase::Idle, binding.kind.phase());
        assert_eq!("idle · 30m", binding.selection_name());
    }
}
