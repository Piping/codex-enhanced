use std::path::PathBuf;
use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OperationStats {
    pub(crate) count: u32,
    pub(crate) failures: u32,
    pub(crate) duration: Duration,
}

impl OperationStats {
    pub(crate) fn add_sample(&mut self, failed: bool, duration: Duration) {
        self.count = self.count.saturating_add(1);
        if failed {
            self.failures = self.failures.saturating_add(1);
        }
        self.duration = self.duration.saturating_add(duration);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PatchStats {
    pub(crate) count: u32,
    pub(crate) failures: u32,
    pub(crate) changed_files: u32,
}

impl PatchStats {
    pub(crate) fn add_sample(&mut self, failed: bool, changed_files: usize) {
        self.count = self.count.saturating_add(1);
        if failed {
            self.failures = self.failures.saturating_add(1);
        }
        self.changed_files = self
            .changed_files
            .saturating_add(u32::try_from(changed_files).unwrap_or(u32::MAX));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AggregateMetrics {
    pub(crate) total_tokens: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) reasoning_output_tokens: i64,
    pub(crate) exec_commands: OperationStats,
    pub(crate) mcp_tool_calls: OperationStats,
    pub(crate) dynamic_tool_calls: OperationStats,
    pub(crate) web_search_count: u32,
    pub(crate) image_generation_count: u32,
    pub(crate) view_image_count: u32,
    pub(crate) patches: PatchStats,
    pub(crate) api_error_count: u32,
    pub(crate) parse_errors: usize,
    pub(crate) user_message_count: u32,
    pub(crate) completed_turn_count: u32,
    pub(crate) estimated_user_wait: Duration,
    pub(crate) cumulative_thread_span: Duration,
    pub(crate) residual_runtime_estimate: Duration,
}

impl AggregateMetrics {
    pub(crate) fn add_assign(&mut self, other: &Self) {
        self.total_tokens += other.total_tokens;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.exec_commands.count = self
            .exec_commands
            .count
            .saturating_add(other.exec_commands.count);
        self.exec_commands.failures = self
            .exec_commands
            .failures
            .saturating_add(other.exec_commands.failures);
        self.exec_commands.duration = self
            .exec_commands
            .duration
            .saturating_add(other.exec_commands.duration);
        self.mcp_tool_calls.count = self
            .mcp_tool_calls
            .count
            .saturating_add(other.mcp_tool_calls.count);
        self.mcp_tool_calls.failures = self
            .mcp_tool_calls
            .failures
            .saturating_add(other.mcp_tool_calls.failures);
        self.mcp_tool_calls.duration = self
            .mcp_tool_calls
            .duration
            .saturating_add(other.mcp_tool_calls.duration);
        self.dynamic_tool_calls.count = self
            .dynamic_tool_calls
            .count
            .saturating_add(other.dynamic_tool_calls.count);
        self.dynamic_tool_calls.failures = self
            .dynamic_tool_calls
            .failures
            .saturating_add(other.dynamic_tool_calls.failures);
        self.dynamic_tool_calls.duration = self
            .dynamic_tool_calls
            .duration
            .saturating_add(other.dynamic_tool_calls.duration);
        self.web_search_count = self.web_search_count.saturating_add(other.web_search_count);
        self.image_generation_count = self
            .image_generation_count
            .saturating_add(other.image_generation_count);
        self.view_image_count = self.view_image_count.saturating_add(other.view_image_count);
        self.patches.count = self.patches.count.saturating_add(other.patches.count);
        self.patches.failures = self.patches.failures.saturating_add(other.patches.failures);
        self.patches.changed_files = self
            .patches
            .changed_files
            .saturating_add(other.patches.changed_files);
        self.api_error_count = self.api_error_count.saturating_add(other.api_error_count);
        self.parse_errors = self.parse_errors.saturating_add(other.parse_errors);
        self.user_message_count = self
            .user_message_count
            .saturating_add(other.user_message_count);
        self.completed_turn_count = self
            .completed_turn_count
            .saturating_add(other.completed_turn_count);
        self.estimated_user_wait = self
            .estimated_user_wait
            .saturating_add(other.estimated_user_wait);
        self.cumulative_thread_span = self
            .cumulative_thread_span
            .saturating_add(other.cumulative_thread_span);
        self.residual_runtime_estimate = self
            .residual_runtime_estimate
            .saturating_add(other.residual_runtime_estimate);
    }

    pub(crate) fn tool_call_count(&self) -> u32 {
        self.mcp_tool_calls
            .count
            .saturating_add(self.dynamic_tool_calls.count)
            .saturating_add(self.web_search_count)
            .saturating_add(self.image_generation_count)
            .saturating_add(self.view_image_count)
    }

    pub(crate) fn exact_tool_runtime(&self) -> Duration {
        self.exec_commands
            .duration
            .saturating_add(self.mcp_tool_calls.duration)
            .saturating_add(self.dynamic_tool_calls.duration)
    }

    pub(crate) fn total_failures(&self) -> u32 {
        self.exec_commands
            .failures
            .saturating_add(self.mcp_tool_calls.failures)
            .saturating_add(self.dynamic_tool_calls.failures)
            .saturating_add(self.patches.failures)
            .saturating_add(self.api_error_count)
    }

    pub(crate) fn counted_operations(&self) -> u32 {
        self.exec_commands
            .count
            .saturating_add(self.mcp_tool_calls.count)
            .saturating_add(self.dynamic_tool_calls.count)
            .saturating_add(self.patches.count)
            .saturating_add(self.api_error_count)
    }

    pub(crate) fn failure_rate(&self) -> f64 {
        let total = self.counted_operations();
        if total == 0 {
            0.0
        } else {
            self.total_failures() as f64 / total as f64
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollectedThread {
    pub(crate) thread_id: ThreadId,
    pub(crate) parent_thread_id: Option<ThreadId>,
    pub(crate) depth: Option<i32>,
    pub(crate) title: String,
    pub(crate) cwd: PathBuf,
    pub(crate) rollout_path: PathBuf,
    pub(crate) archived: bool,
    pub(crate) source_label: String,
    pub(crate) agent_nickname: Option<String>,
    pub(crate) agent_role: Option<String>,
    pub(crate) agent_path: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) first_event_at: Option<DateTime<Utc>>,
    pub(crate) last_event_at: Option<DateTime<Utc>>,
    pub(crate) metrics: AggregateMetrics,
}

impl CollectedThread {
    pub(crate) fn wall_clock_span(&self) -> Duration {
        self.metrics.cumulative_thread_span
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CollectionResult {
    pub(crate) threads: Vec<CollectedThread>,
    pub(crate) scanned_files: usize,
    pub(crate) skipped_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootSessionSummary {
    pub(crate) root_thread_id: ThreadId,
    pub(crate) title: String,
    pub(crate) cwd: PathBuf,
    pub(crate) rollout_path: PathBuf,
    pub(crate) archived: bool,
    pub(crate) earliest_event_at: Option<DateTime<Utc>>,
    pub(crate) latest_event_at: Option<DateTime<Utc>>,
    pub(crate) wall_clock_span: Duration,
    pub(crate) metrics: AggregateMetrics,
    pub(crate) threads: Vec<CollectedThread>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InsightOverview {
    pub(crate) total_root_sessions: usize,
    pub(crate) total_threads: usize,
    pub(crate) archived_threads: usize,
    pub(crate) scanned_files: usize,
    pub(crate) skipped_files: usize,
    pub(crate) metrics: AggregateMetrics,
    pub(crate) earliest_event_at: Option<DateTime<Utc>>,
    pub(crate) latest_event_at: Option<DateTime<Utc>>,
    pub(crate) history_span: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NarrativeMode {
    LocalHeuristics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InsightReportData {
    pub(crate) generated_at: DateTime<Utc>,
    pub(crate) codex_home: PathBuf,
    pub(crate) report_path: PathBuf,
    pub(crate) overview: InsightOverview,
    pub(crate) roots: Vec<RootSessionSummary>,
    pub(crate) common_patterns: Vec<String>,
    pub(crate) efficiency_suggestions: Vec<String>,
    pub(crate) narrative_mode: NarrativeMode,
}
