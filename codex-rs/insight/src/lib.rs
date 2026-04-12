mod aggregator;
mod collector;
mod report;
mod types;

use std::path::Path;
use std::path::PathBuf;

use chrono::Utc;

use self::aggregator::aggregate;
use self::collector::collect_sessions;
use self::report::write_report_html;
use self::types::InsightReportData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightReportOptions {
    codex_home: PathBuf,
}

impl InsightReportOptions {
    pub fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    pub fn codex_home(&self) -> &Path {
        self.codex_home.as_path()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightReportOutput {
    pub report_path: PathBuf,
}

pub fn report_started_message() -> String {
    "Generating /insight report from local sessions...".to_string()
}

pub async fn generate_report(
    options: &InsightReportOptions,
) -> anyhow::Result<InsightReportOutput> {
    let collection = collect_sessions(options.codex_home()).await?;
    let aggregated = aggregate(collection);
    let generated_at = Utc::now();
    let report_path = report_path(options.codex_home(), generated_at);
    let data = InsightReportData {
        generated_at,
        codex_home: options.codex_home().to_path_buf(),
        report_path: report_path.clone(),
        overview: aggregated.overview,
        roots: aggregated.roots,
        common_patterns: aggregated.common_patterns,
        efficiency_suggestions: aggregated.efficiency_suggestions,
        narrative_mode: aggregated.narrative_mode,
    };
    write_report_html(&data).await?;
    Ok(InsightReportOutput { report_path })
}

fn report_path(codex_home: &Path, generated_at: chrono::DateTime<Utc>) -> PathBuf {
    codex_home.join("reports").join(format!(
        "insight-{}.html",
        generated_at.format("%Y%m%d-%H%M%S")
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use codex_protocol::ThreadId;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::ExecCommandEndEvent;
    use codex_protocol::protocol::ExecCommandSource;
    use codex_protocol::protocol::ExecCommandStatus;
    use codex_protocol::protocol::RolloutItem;
    use codex_protocol::protocol::RolloutLine;
    use codex_protocol::protocol::SessionMeta;
    use codex_protocol::protocol::SessionMetaLine;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::protocol::TokenCountEvent;
    use codex_protocol::protocol::TokenUsage;
    use codex_protocol::protocol::TokenUsageInfo;
    use codex_protocol::protocol::TurnCompleteEvent;
    use codex_protocol::protocol::UserMessageEvent;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn write_rollout(codex_home: &Path, relative_dir: &str, thread_id: ThreadId) {
        let dir = codex_home.join(relative_dir);
        fs::create_dir_all(&dir).expect("create rollout dir");
        let path =
            dir.join("rollout-2026-04-08T12-00-00-00000000-0000-0000-0000-000000000001.jsonl");
        let session_meta = RolloutLine {
            timestamp: "2026-04-08T12:00:00Z".to_string(),
            item: RolloutItem::SessionMeta(SessionMetaLine {
                meta: SessionMeta {
                    id: thread_id,
                    forked_from_id: None,
                    timestamp: "2026-04-08T12:00:00Z".to_string(),
                    cwd: PathBuf::from("/repo"),
                    originator: "codex".to_string(),
                    cli_version: "0.0.0".to_string(),
                    source: SessionSource::Cli,
                    agent_nickname: None,
                    agent_role: None,
                    agent_path: None,
                    model_provider: Some("openai".to_string()),
                    base_instructions: None,
                    dynamic_tools: None,
                    memory_mode: None,
                },
                git: None,
            }),
        };
        let user = RolloutLine {
            timestamp: "2026-04-08T12:00:05Z".to_string(),
            item: RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "## My request for Codex: inspect session data".to_string(),
                images: None,
                local_images: Vec::new(),
                text_elements: Vec::new(),
            })),
        };
        let token = RolloutLine {
            timestamp: "2026-04-08T12:00:15Z".to_string(),
            item: RolloutItem::EventMsg(EventMsg::TokenCount(TokenCountEvent {
                info: Some(TokenUsageInfo {
                    total_token_usage: TokenUsage {
                        input_tokens: 100,
                        cached_input_tokens: 0,
                        output_tokens: 20,
                        reasoning_output_tokens: 10,
                        total_tokens: 120,
                    },
                    last_token_usage: TokenUsage::default(),
                    model_context_window: Some(128000),
                }),
                rate_limits: None,
            })),
        };
        let exec = RolloutLine {
            timestamp: "2026-04-08T12:00:18Z".to_string(),
            item: RolloutItem::EventMsg(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "call-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["rg".to_string(), "todo".to_string()],
                cwd: PathBuf::from("/repo"),
                parsed_cmd: Vec::new(),
                source: ExecCommandSource::Agent,
                interaction_input: None,
                stdout: String::new(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: Duration::from_secs(2),
                formatted_output: String::new(),
                status: ExecCommandStatus::Completed,
            })),
        };
        let turn_complete = RolloutLine {
            timestamp: "2026-04-08T12:00:20Z".to_string(),
            item: RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-1".to_string(),
                last_agent_message: Some("done".to_string()),
            })),
        };

        let body = [session_meta, user, token, exec, turn_complete]
            .into_iter()
            .map(|line| serde_json::to_string(&line).expect("serialize"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, body).expect("write rollout");
    }

    #[tokio::test]
    async fn collect_and_generate_report_for_sample_rollout() {
        let temp_home = tempdir().expect("temp home");
        let codex_home = temp_home.path().to_path_buf();
        write_rollout(codex_home.as_path(), "sessions", ThreadId::new());

        let collection = collect_sessions(codex_home.as_path())
            .await
            .expect("collect sessions");
        assert_eq!(collection.scanned_files, 1);
        assert_eq!(collection.threads.len(), 1);

        let report = generate_report(&InsightReportOptions::new(codex_home.clone()))
            .await
            .expect("generate report");
        assert!(report.report_path.exists());
        assert!(report.report_path.starts_with(codex_home.join("reports")));
        assert!(report_started_message().contains("/insight"));
    }
}
