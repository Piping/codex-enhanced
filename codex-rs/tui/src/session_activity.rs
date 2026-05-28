use codex_app_server_protocol::ThreadItem;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionActivitySummary {
    pub interaction_rounds: u64,
    pub tool_calls: u64,
}

impl SessionActivitySummary {
    pub(crate) fn from_turn_items(items: &[ThreadItem]) -> Self {
        summarize_activity(items.iter().map(ActivityItemKind::from))
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.interaction_rounds = self
            .interaction_rounds
            .saturating_add(other.interaction_rounds);
        self.tool_calls = self.tool_calls.saturating_add(other.tool_calls);
    }

    pub fn is_empty(self) -> bool {
        self.interaction_rounds == 0 && self.tool_calls == 0
    }

    pub fn label_fragment(self) -> Option<String> {
        let mut parts = Vec::new();
        if self.interaction_rounds > 0 {
            let rounds = pluralize(
                self.interaction_rounds,
                "interaction round",
                "interaction rounds",
            );
            parts.push(format!("{} {rounds}", self.interaction_rounds));
        }
        if self.tool_calls > 0 {
            let calls = pluralize(self.tool_calls, "tool call", "tool calls");
            parts.push(format!("{} {calls}", self.tool_calls));
        }
        (!parts.is_empty()).then(|| parts.join(" • "))
    }

    pub fn summary_line(self) -> Option<String> {
        self.label_fragment()
            .map(|fragment| format!("Session activity: {fragment}"))
    }
}

#[derive(Debug, Default)]
pub(crate) struct TurnActivityTracker {
    item_kinds: Vec<ActivityItemKind>,
    seen_item_ids: HashSet<String>,
}

impl TurnActivityTracker {
    pub(crate) fn record_item(&mut self, item: &ThreadItem) {
        if self.seen_item_ids.insert(item.id().to_string()) {
            self.item_kinds.push(ActivityItemKind::from(item));
        }
    }

    pub(crate) fn finish_turn(&mut self) -> SessionActivitySummary {
        let summary = summarize_activity(self.item_kinds.iter().copied());
        self.reset();
        summary
    }

    pub(crate) fn reset(&mut self) {
        self.item_kinds.clear();
        self.seen_item_ids.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityItemKind {
    Tool,
    NonTool,
}

impl From<&ThreadItem> for ActivityItemKind {
    fn from(item: &ThreadItem) -> Self {
        if is_tool_item(item) {
            Self::Tool
        } else {
            Self::NonTool
        }
    }
}

fn summarize_activity(
    item_kinds: impl IntoIterator<Item = ActivityItemKind>,
) -> SessionActivitySummary {
    let mut saw_any_item = false;
    let mut tool_calls = 0_u64;
    let mut tool_rounds = 0_u64;
    let mut in_tool_round = false;

    for item_kind in item_kinds {
        saw_any_item = true;
        if item_kind == ActivityItemKind::Tool {
            tool_calls = tool_calls.saturating_add(1);
            if !in_tool_round {
                tool_rounds = tool_rounds.saturating_add(1);
                in_tool_round = true;
            }
        } else {
            in_tool_round = false;
        }
    }

    if !saw_any_item {
        return SessionActivitySummary::default();
    }

    SessionActivitySummary {
        interaction_rounds: tool_rounds.saturating_add(1),
        tool_calls,
    }
}

fn is_tool_item(item: &ThreadItem) -> bool {
    matches!(
        item,
        ThreadItem::CommandExecution { .. }
            | ThreadItem::FileChange { .. }
            | ThreadItem::McpToolCall { .. }
            | ThreadItem::DynamicToolCall { .. }
            | ThreadItem::CollabAgentToolCall { .. }
            | ThreadItem::WebSearch { .. }
            | ThreadItem::ImageView { .. }
            | ThreadItem::ImageGeneration { .. }
    )
}

fn pluralize(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::SessionActivitySummary;
    use super::TurnActivityTracker;
    use codex_app_server_protocol::CollabAgentState;
    use codex_app_server_protocol::CollabAgentTool;
    use codex_app_server_protocol::CollabAgentToolCallStatus;
    use codex_app_server_protocol::CommandExecutionSource;
    use codex_app_server_protocol::CommandExecutionStatus;
    use codex_app_server_protocol::DynamicToolCallStatus;
    use codex_app_server_protocol::ThreadItem;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn summary_line_omits_empty_summary() {
        assert_eq!(SessionActivitySummary::default().summary_line(), None);
    }

    #[test]
    fn summary_line_formats_rounds_and_tools() {
        let summary = SessionActivitySummary {
            interaction_rounds: 3,
            tool_calls: 7,
        };

        assert_eq!(
            summary.summary_line(),
            Some("Session activity: 3 interaction rounds • 7 tool calls".to_string())
        );
    }

    #[test]
    fn summary_line_handles_singular_labels() {
        let summary = SessionActivitySummary {
            interaction_rounds: 1,
            tool_calls: 1,
        };

        assert_eq!(
            summary.summary_line(),
            Some("Session activity: 1 interaction round • 1 tool call".to_string())
        );
    }

    #[test]
    fn from_turn_items_counts_parallel_tools_as_one_round() {
        let summary = SessionActivitySummary::from_turn_items(&[
            ThreadItem::Reasoning {
                id: "r1".to_string(),
                summary: Vec::new(),
                content: Vec::new(),
            },
            ThreadItem::CommandExecution {
                id: "exec-1".to_string(),
                command: "pwd".to_string(),
                cwd: std::env::current_dir()
                    .expect("cwd")
                    .try_into()
                    .expect("abs path"),
                process_id: None,
                source: CommandExecutionSource::UserShell,
                status: CommandExecutionStatus::Completed,
                command_actions: Vec::new(),
                aggregated_output: None,
                exit_code: Some(0),
                duration_ms: Some(1),
            },
            ThreadItem::DynamicToolCall {
                id: "tool-2".to_string(),
                namespace: None,
                tool: "search".to_string(),
                arguments: json!({}),
                status: DynamicToolCallStatus::Completed,
                content_items: None,
                success: Some(true),
                duration_ms: Some(1),
            },
            ThreadItem::AgentMessage {
                id: "m1".to_string(),
                text: "done".to_string(),
                phase: None,
                memory_citation: None,
            },
        ]);

        assert_eq!(
            summary,
            SessionActivitySummary {
                interaction_rounds: 2,
                tool_calls: 2,
            }
        );
    }

    #[test]
    fn from_turn_items_counts_multiple_tool_rounds() {
        let summary = SessionActivitySummary::from_turn_items(&[
            ThreadItem::WebSearch {
                id: "search-1".to_string(),
                query: "foo".to_string(),
                action: None,
            },
            ThreadItem::Reasoning {
                id: "r1".to_string(),
                summary: Vec::new(),
                content: Vec::new(),
            },
            ThreadItem::CollabAgentToolCall {
                id: "agent-1".to_string(),
                tool: CollabAgentTool::Wait,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "thread-a".to_string(),
                receiver_thread_ids: vec!["thread-b".to_string()],
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: HashMap::<String, CollabAgentState>::new(),
            },
            ThreadItem::Plan {
                id: "p1".to_string(),
                text: "done".to_string(),
            },
        ]);

        assert_eq!(
            summary,
            SessionActivitySummary {
                interaction_rounds: 3,
                tool_calls: 2,
            }
        );
    }

    #[test]
    fn turn_activity_tracker_counts_live_turn_without_tools() {
        let mut tracker = TurnActivityTracker::default();
        tracker.record_item(&ThreadItem::UserMessage {
            id: "user-1".to_string(),
            content: Vec::new(),
        });
        tracker.record_item(&ThreadItem::AgentMessage {
            id: "msg-1".to_string(),
            text: "OK".to_string(),
            phase: None,
            memory_citation: None,
        });

        assert_eq!(
            tracker.finish_turn(),
            SessionActivitySummary {
                interaction_rounds: 1,
                tool_calls: 0,
            }
        );
    }

    #[test]
    fn turn_activity_tracker_deduplicates_started_and_completed_tool_items() {
        let mut tracker = TurnActivityTracker::default();
        let command = ThreadItem::CommandExecution {
            id: "exec-1".to_string(),
            command: "pwd".to_string(),
            cwd: std::env::current_dir()
                .expect("cwd")
                .try_into()
                .expect("abs path"),
            process_id: None,
            source: CommandExecutionSource::UserShell,
            status: CommandExecutionStatus::InProgress,
            command_actions: Vec::new(),
            aggregated_output: None,
            exit_code: None,
            duration_ms: None,
        };
        tracker.record_item(&command);
        tracker.record_item(&ThreadItem::CommandExecution {
            id: "exec-1".to_string(),
            command: "pwd".to_string(),
            cwd: std::env::current_dir()
                .expect("cwd")
                .try_into()
                .expect("abs path"),
            process_id: None,
            source: CommandExecutionSource::UserShell,
            status: CommandExecutionStatus::Completed,
            command_actions: Vec::new(),
            aggregated_output: Some("/tmp\n".to_string()),
            exit_code: Some(0),
            duration_ms: Some(5),
        });
        tracker.record_item(&ThreadItem::AgentMessage {
            id: "msg-1".to_string(),
            text: "done".to_string(),
            phase: None,
            memory_citation: None,
        });

        assert_eq!(
            tracker.finish_turn(),
            SessionActivitySummary {
                interaction_rounds: 2,
                tool_calls: 1,
            }
        );
    }
}
