use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;

use super::types::AggregateMetrics;
use super::types::CollectedThread;
use super::types::CollectionResult;
use super::types::InsightOverview;
use super::types::NarrativeMode;
use super::types::RootSessionSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AggregatedInsight {
    pub(crate) overview: InsightOverview,
    pub(crate) roots: Vec<RootSessionSummary>,
    pub(crate) common_patterns: Vec<String>,
    pub(crate) efficiency_suggestions: Vec<String>,
    pub(crate) narrative_mode: NarrativeMode,
}

pub(crate) fn aggregate(collection: CollectionResult) -> AggregatedInsight {
    let roots = build_root_summaries(collection.threads);
    let mut metrics = AggregateMetrics::default();
    let mut earliest_event_at = None;
    let mut latest_event_at = None;
    let archived_threads = roots
        .iter()
        .flat_map(|root| root.threads.iter())
        .filter(|thread| thread.archived)
        .count();

    for root in &roots {
        metrics.add_assign(&root.metrics);
        earliest_event_at = min_datetime(earliest_event_at, root.earliest_event_at);
        latest_event_at = max_datetime(latest_event_at, root.latest_event_at);
    }

    let history_span = match (earliest_event_at, latest_event_at) {
        (Some(first), Some(last)) if last > first => (last - first).to_std().unwrap_or_default(),
        _ => Duration::ZERO,
    };
    let overview = InsightOverview {
        total_root_sessions: roots.len(),
        total_threads: roots.iter().map(|root| root.threads.len()).sum(),
        archived_threads,
        scanned_files: collection.scanned_files,
        skipped_files: collection.skipped_files,
        metrics: metrics.clone(),
        earliest_event_at,
        latest_event_at,
        history_span,
    };

    AggregatedInsight {
        common_patterns: build_common_patterns(&overview, &roots),
        efficiency_suggestions: build_efficiency_suggestions(&overview, &roots),
        overview,
        roots,
        narrative_mode: NarrativeMode::LocalHeuristics,
    }
}

fn build_root_summaries(threads: Vec<CollectedThread>) -> Vec<RootSessionSummary> {
    let thread_by_id: HashMap<ThreadId, CollectedThread> = threads
        .into_iter()
        .map(|thread| (thread.thread_id, thread))
        .collect();

    let mut grouped = HashMap::<ThreadId, Vec<CollectedThread>>::new();
    for thread in thread_by_id.values() {
        let root_thread_id = resolve_root_thread_id(thread, &thread_by_id);
        grouped
            .entry(root_thread_id)
            .or_default()
            .push(thread.clone());
    }

    let mut roots: Vec<RootSessionSummary> = grouped
        .into_iter()
        .filter_map(|(root_thread_id, mut threads)| {
            threads.sort_by_key(|thread| (thread.depth.unwrap_or(0), thread.created_at));
            let root = threads
                .iter()
                .find(|thread| thread.thread_id == root_thread_id)
                .cloned()
                .or_else(|| threads.first().cloned())?;

            let mut metrics = AggregateMetrics::default();
            let mut earliest_event_at = None;
            let mut latest_event_at = None;
            for thread in &threads {
                metrics.add_assign(&thread.metrics);
                earliest_event_at = min_datetime(earliest_event_at, thread.first_event_at);
                latest_event_at = max_datetime(latest_event_at, thread.last_event_at);
            }
            let wall_clock_span = match (earliest_event_at, latest_event_at) {
                (Some(first), Some(last)) if last > first => {
                    (last - first).to_std().unwrap_or_default()
                }
                _ => Duration::ZERO,
            };

            Some(RootSessionSummary {
                root_thread_id,
                title: root.title,
                cwd: if root.cwd.as_os_str().is_empty() {
                    PathBuf::new()
                } else {
                    root.cwd
                },
                rollout_path: root.rollout_path,
                archived: root.archived,
                earliest_event_at,
                latest_event_at,
                wall_clock_span,
                metrics,
                threads,
            })
        })
        .collect();

    roots.sort_by(|left, right| {
        right
            .metrics
            .total_tokens
            .cmp(&left.metrics.total_tokens)
            .then_with(|| right.latest_event_at.cmp(&left.latest_event_at))
    });
    roots
}

fn resolve_root_thread_id(
    thread: &CollectedThread,
    thread_by_id: &HashMap<ThreadId, CollectedThread>,
) -> ThreadId {
    let mut current = thread;
    let mut seen = HashSet::from([current.thread_id]);
    while let Some(parent_thread_id) = current.parent_thread_id {
        let Some(parent) = thread_by_id.get(&parent_thread_id) else {
            return parent_thread_id;
        };
        if !seen.insert(parent.thread_id) {
            return current.thread_id;
        }
        current = parent;
    }
    current.thread_id
}

fn build_common_patterns(overview: &InsightOverview, roots: &[RootSessionSummary]) -> Vec<String> {
    let mut patterns = Vec::new();

    if let Some(root) = roots.first() {
        patterns.push(format!(
            "Token hotspot: `{}` is the heaviest root session with {} tokens across {} thread(s).",
            root.title,
            format_number(root.metrics.total_tokens),
            root.threads.len(),
        ));
    }

    let subagent_roots = roots.iter().filter(|root| root.threads.len() > 1).count();
    patterns.push(format!(
        "Subagent usage: {} / {} root session(s) include spawned child threads.",
        subagent_roots,
        roots.len(),
    ));

    patterns.push(format!(
        "Timing split: exact tool runtime is {}, estimated user wait is {}, and residual model/UI time is {}.",
        format_duration(overview.metrics.exact_tool_runtime()),
        format_duration(overview.metrics.estimated_user_wait),
        format_duration(overview.metrics.residual_runtime_estimate),
    ));
    patterns.push(format!(
        "Failure surface: {:.1}% failure rate across counted operations (exec + MCP + dynamic tools + patch + API errors).",
        overview.metrics.failure_rate() * 100.0,
    ));
    patterns
}

fn build_efficiency_suggestions(
    overview: &InsightOverview,
    roots: &[RootSessionSummary],
) -> Vec<String> {
    let mut suggestions = Vec::new();

    if overview.metrics.exec_commands.failures > 0
        && overview.metrics.exec_commands.failures >= overview.metrics.api_error_count
    {
        suggestions.push(
            "Exec failures dominate the observed error budget; tighten command scope and prefer read-first probes like `rg`, `sed -n`, and targeted tests before wider shell actions."
                .to_string(),
        );
    }

    if overview.metrics.total_tokens > 500_000 {
        suggestions.push(
            "Token usage is concentrated in a few long-running sessions; trigger `/compact` earlier on exploratory threads to reduce context drag."
                .to_string(),
        );
    }

    if overview.metrics.estimated_user_wait > overview.metrics.exact_tool_runtime() {
        suggestions.push(
            "Estimated user-wait time exceeds exact tool runtime; batch follow-up prompts more aggressively when a task can be specified upfront."
                .to_string(),
        );
    }

    if roots.iter().all(|root| root.threads.len() == 1) && !roots.is_empty() {
        suggestions.push(
            "No multi-thread roots were detected in this sample; for parallelizable exploration or verification work, `/agent` or subagent workflows may reduce end-to-end elapsed time."
                .to_string(),
        );
    }

    if overview.metrics.patches.changed_files > overview.metrics.patches.count.saturating_mul(3) {
        suggestions.push(
            "Patch churn is spread across many files; grouping edits by module boundary should make review and rollback cheaper."
                .to_string(),
        );
    }

    if suggestions.is_empty() {
        suggestions.push(
            "No dominant inefficiency pattern stood out from local heuristics; use the per-root drill-down to inspect the highest-token and highest-failure sessions first."
                .to_string(),
        );
    }

    suggestions
}

fn min_datetime(
    left: Option<DateTime<Utc>>,
    right: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn max_datetime(
    left: Option<DateTime<Utc>>,
    right: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(crate) fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub(crate) fn format_number(value: i64) -> String {
    let negative = value < 0;
    let digits = value.abs().to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    let formatted: String = formatted.chars().rev().collect();
    if negative {
        format!("-{formatted}")
    } else {
        formatted
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use chrono::TimeZone;
    use chrono::Utc;
    use pretty_assertions::assert_eq;

    use super::aggregate;
    use crate::types::AggregateMetrics;
    use crate::types::CollectedThread;
    use crate::types::CollectionResult;
    use codex_protocol::ThreadId;

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::from_string(value).expect("valid thread id")
    }

    fn collected_thread(
        thread_id: ThreadId,
        parent_thread_id: Option<ThreadId>,
        title: &str,
        tokens: i64,
    ) -> CollectedThread {
        let created_at = Utc.with_ymd_and_hms(2026, 4, 8, 12, 0, 0).unwrap();
        let mut metrics = AggregateMetrics {
            total_tokens: tokens,
            cumulative_thread_span: Duration::from_secs(20),
            ..AggregateMetrics::default()
        };
        metrics
            .exec_commands
            .add_sample(false, Duration::from_secs(3));
        CollectedThread {
            thread_id,
            parent_thread_id,
            depth: parent_thread_id.map(|_| 1),
            title: title.to_string(),
            cwd: PathBuf::from("/repo"),
            rollout_path: PathBuf::from(format!("/tmp/{thread_id}.jsonl")),
            archived: false,
            source_label: "cli".to_string(),
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            created_at,
            updated_at: created_at,
            first_event_at: Some(created_at),
            last_event_at: Some(created_at + chrono::Duration::seconds(20)),
            metrics,
        }
    }

    #[test]
    fn aggregate_rolls_children_into_root_session() {
        let root = thread_id("00000000-0000-0000-0000-000000000001");
        let child = thread_id("00000000-0000-0000-0000-000000000002");
        let result = aggregate(CollectionResult {
            threads: vec![
                collected_thread(root, None, "root", 120),
                collected_thread(child, Some(root), "child", 30),
            ],
            scanned_files: 2,
            skipped_files: 0,
        });

        assert_eq!(result.roots.len(), 1);
        assert_eq!(result.roots[0].root_thread_id, root);
        assert_eq!(result.roots[0].threads.len(), 2);
        assert_eq!(result.roots[0].metrics.total_tokens, 150);
        assert_eq!(result.overview.total_root_sessions, 1);
        assert_eq!(result.overview.total_threads, 2);
    }
}
