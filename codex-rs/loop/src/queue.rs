use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::model::PersistedLoopTimer;
use crate::trigger::LoopTriggerPhase;
use crate::trigger_bindings;

const LOOP_TRIGGER_QUEUE_FILE_NAME: &str = "loop_trigger_queues.json";
const LOOP_METADATA_DIR_NAME: &str = "loop";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedLoopTriggerQueuesFile {
    #[serde(default)]
    pub queues: Vec<LoopTriggerQueue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopTriggerQueue {
    pub phase: LoopTriggerPhase,
    #[serde(default)]
    pub entries: Vec<LoopTriggerQueueEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct LoopTriggerQueueEntry {
    pub loop_id: String,
    pub binding_id: String,
}

pub fn load_loop_trigger_queues(cwd: &Path) -> std::io::Result<PersistedLoopTriggerQueuesFile> {
    let path = loop_trigger_queues_path(cwd);
    if !path.exists() {
        return Ok(PersistedLoopTriggerQueuesFile::default());
    }
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(std::io::Error::other)
}

pub fn loop_trigger_queues_path(cwd: &Path) -> PathBuf {
    cwd.join(".codex")
        .join(LOOP_METADATA_DIR_NAME)
        .join(LOOP_TRIGGER_QUEUE_FILE_NAME)
}

pub fn sync_trigger_queues_with_timers(
    queues: &mut PersistedLoopTriggerQueuesFile,
    timers: &BTreeMap<String, PersistedLoopTimer>,
) {
    let valid_entries = timers
        .iter()
        .flat_map(|(loop_id, timer)| {
            trigger_bindings(timer).into_iter().map(move |binding| {
                (
                    binding.kind.phase(),
                    LoopTriggerQueueEntry {
                        loop_id: loop_id.clone(),
                        binding_id: binding.id,
                    },
                )
            })
        })
        .fold(
            BTreeMap::<LoopTriggerPhase, Vec<LoopTriggerQueueEntry>>::new(),
            |mut map, (phase, entry)| {
                map.entry(phase).or_default().push(entry);
                map
            },
        );

    for phase in LoopTriggerPhase::USER_SELECTABLE {
        let queue = ensure_queue(queues, phase);
        let valid_for_phase = valid_entries
            .get(&phase)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>();

        queue
            .entries
            .retain(|entry| valid_for_phase.contains(entry));

        for entry in valid_entries.get(&phase).into_iter().flatten() {
            if !queue.entries.contains(entry) {
                queue.entries.push(entry.clone());
            }
        }
    }
}

pub fn move_trigger_queue_entry(
    queues: &mut PersistedLoopTriggerQueuesFile,
    phase: LoopTriggerPhase,
    loop_id: &str,
    binding_id: &str,
    direction: QueueMoveDirection,
) -> bool {
    let queue = ensure_queue(queues, phase);
    let Some(index) = queue
        .entries
        .iter()
        .position(|entry| entry.loop_id == loop_id && entry.binding_id == binding_id)
    else {
        return false;
    };

    let target = match direction {
        QueueMoveDirection::Up if index > 0 => index - 1,
        QueueMoveDirection::Down if index + 1 < queue.entries.len() => index + 1,
        _ => return false,
    };
    queue.entries.swap(index, target);
    true
}

pub fn queue_entries_for_phase(
    queues: &PersistedLoopTriggerQueuesFile,
    phase: LoopTriggerPhase,
) -> &[LoopTriggerQueueEntry] {
    queues
        .queues
        .iter()
        .find(|queue| queue.phase == phase)
        .map_or(&[], |queue| queue.entries.as_slice())
}

fn ensure_queue(
    queues: &mut PersistedLoopTriggerQueuesFile,
    phase: LoopTriggerPhase,
) -> &mut LoopTriggerQueue {
    if let Some(index) = queues.queues.iter().position(|queue| queue.phase == phase) {
        &mut queues.queues[index]
    } else {
        queues.queues.push(LoopTriggerQueue {
            phase,
            entries: Vec::new(),
        });
        let index = queues.queues.len().saturating_sub(1);
        &mut queues.queues[index]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMoveDirection {
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    use super::LoopTriggerQueueEntry;
    use super::PersistedLoopTriggerQueuesFile;
    use super::QueueMoveDirection;
    use super::move_trigger_queue_entry;
    use super::queue_entries_for_phase;
    use super::sync_trigger_queues_with_timers;
    use crate::LoopMode;
    use crate::LoopSchedule;
    use crate::LoopTriggerBinding;
    use crate::LoopTriggerKind;
    use crate::LoopTriggerPhase;
    use crate::PersistedLoopExecutionSettings;
    use crate::PersistedLoopTimer;

    fn sample_timer() -> PersistedLoopTimer {
        PersistedLoopTimer {
            id: "director".to_string(),
            mode: LoopMode::Persistent,
            prompt: "review progress".to_string(),
            action: None,
            execution: PersistedLoopExecutionSettings::default(),
            schedule: LoopSchedule::Interval {
                display: "5m".to_string(),
                seconds: 300,
            },
            enabled: true,
            rollout_path: None,
            created_at_unix_seconds: 1,
            last_scheduled_at_unix_seconds: None,
            last_completed_at_unix_seconds: None,
            trigger_bindings: vec![
                LoopTriggerBinding {
                    id: "trigger-1".to_string(),
                    enabled: true,
                    kind: LoopTriggerKind::Timer {
                        schedule: LoopSchedule::Interval {
                            display: "5m".to_string(),
                            seconds: 300,
                        },
                    },
                },
                LoopTriggerBinding {
                    id: "trigger-2".to_string(),
                    enabled: true,
                    kind: LoopTriggerKind::Idle {
                        after: LoopSchedule::Interval {
                            display: "30m".to_string(),
                            seconds: 1_800,
                        },
                    },
                },
                LoopTriggerBinding {
                    id: "trigger-3".to_string(),
                    enabled: true,
                    kind: LoopTriggerKind::AfterTurn,
                },
            ],
            context_mode: crate::LoopContextMode::Persistent,
            response_mode: crate::LoopResponseMode::Assistant,
            security_mode: crate::LoopSecurityMode::Inherited,
        }
    }

    #[test]
    fn sync_trigger_queues_adds_missing_entries() {
        let mut timers = BTreeMap::new();
        timers.insert("director".to_string(), sample_timer());
        let mut queues = PersistedLoopTriggerQueuesFile::default();

        sync_trigger_queues_with_timers(&mut queues, &timers);

        assert_eq!(
            vec![LoopTriggerQueueEntry {
                loop_id: "director".to_string(),
                binding_id: "trigger-1".to_string(),
            }],
            queue_entries_for_phase(&queues, LoopTriggerPhase::Timer)
        );
        assert_eq!(
            vec![LoopTriggerQueueEntry {
                loop_id: "director".to_string(),
                binding_id: "trigger-2".to_string(),
            }],
            queue_entries_for_phase(&queues, LoopTriggerPhase::Idle)
        );
        assert_eq!(
            vec![LoopTriggerQueueEntry {
                loop_id: "director".to_string(),
                binding_id: "trigger-3".to_string(),
            }],
            queue_entries_for_phase(&queues, LoopTriggerPhase::AfterTurn)
        );
    }

    #[test]
    fn sync_trigger_queues_creates_idle_queue_when_empty() {
        let mut timers = BTreeMap::new();
        timers.insert("director".to_string(), sample_timer());
        let mut queues = PersistedLoopTriggerQueuesFile::default();

        sync_trigger_queues_with_timers(&mut queues, &timers);

        assert_eq!(LoopTriggerPhase::USER_SELECTABLE.len(), queues.queues.len());
        assert_eq!(
            vec![LoopTriggerQueueEntry {
                loop_id: "director".to_string(),
                binding_id: "trigger-2".to_string(),
            }],
            queue_entries_for_phase(&queues, LoopTriggerPhase::Idle)
        );
    }

    #[test]
    fn move_trigger_queue_entry_swaps_neighbors() {
        let mut queues = PersistedLoopTriggerQueuesFile {
            queues: vec![super::LoopTriggerQueue {
                phase: LoopTriggerPhase::BeforeTurn,
                entries: vec![
                    LoopTriggerQueueEntry {
                        loop_id: "a".to_string(),
                        binding_id: "trigger-1".to_string(),
                    },
                    LoopTriggerQueueEntry {
                        loop_id: "b".to_string(),
                        binding_id: "trigger-1".to_string(),
                    },
                ],
            }],
        };

        assert!(move_trigger_queue_entry(
            &mut queues,
            LoopTriggerPhase::BeforeTurn,
            "b",
            "trigger-1",
            QueueMoveDirection::Up,
        ));

        assert_eq!(
            vec![
                LoopTriggerQueueEntry {
                    loop_id: "b".to_string(),
                    binding_id: "trigger-1".to_string(),
                },
                LoopTriggerQueueEntry {
                    loop_id: "a".to_string(),
                    binding_id: "trigger-1".to_string(),
                },
            ],
            queue_entries_for_phase(&queues, LoopTriggerPhase::BeforeTurn)
        );
    }
}
