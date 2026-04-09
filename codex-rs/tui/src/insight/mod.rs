mod aggregator;
mod collector;
mod report;
mod types;

use std::path::Path;

use chrono::Utc;
use codex_core::config::Config;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::history_cell;

use self::aggregator::aggregate;
use self::collector::collect_sessions;
use self::report::write_report_html;
use self::types::InsightReportData;

pub(crate) fn spawn_report_generation(config: Config, app_event_tx: AppEventSender) {
    tokio::spawn(async move {
        match generate_report(config).await {
            Ok(output) => {
                let message = format!("Insight report generated: {}", output.report_path.display());
                let hint = Some(
                    "Patterns and suggestions used local heuristics fallback in this build."
                        .to_string(),
                );
                app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                    history_cell::new_info_event(message, hint),
                )));
            }
            Err(err) => {
                app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                    history_cell::new_error_event(format!(
                        "Failed to generate /insight report: {err}"
                    )),
                )));
            }
        }
    });
}

pub(crate) fn start_message() -> String {
    "Generating /insight report from local sessions...".to_string()
}

async fn generate_report(config: Config) -> anyhow::Result<InsightReportData> {
    let collection = collect_sessions(&config).await?;
    let aggregated = aggregate(collection);
    let generated_at = Utc::now();
    let report_path = report_path(config.codex_home.as_path(), generated_at);
    let data = InsightReportData {
        generated_at,
        codex_home: config.codex_home.clone(),
        report_path,
        overview: aggregated.overview,
        roots: aggregated.roots,
        common_patterns: aggregated.common_patterns,
        efficiency_suggestions: aggregated.efficiency_suggestions,
        narrative_mode: aggregated.narrative_mode,
    };
    write_report_html(&data).await?;
    Ok(data)
}

fn report_path(codex_home: &Path, generated_at: chrono::DateTime<Utc>) -> std::path::PathBuf {
    codex_home.join("reports").join(format!(
        "insight-{}.html",
        generated_at.format("%Y%m%d-%H%M%S")
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use chrono::Utc;
    use codex_core::config::ConfigBuilder;
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

    use super::collector::collect_sessions;
    use super::generate_report;

    fn write_rollout(codex_home: &PathBuf, relative_dir: &str, thread_id: ThreadId) {
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
        write_rollout(&codex_home, "sessions", ThreadId::new());
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.clone())
            .build()
            .await
            .expect("config");
        config.sqlite_home = codex_home.clone();

        let collection = collect_sessions(&config).await.expect("collect sessions");
        assert_eq!(collection.scanned_files, 1);
        assert_eq!(collection.threads.len(), 1);

        let report = generate_report(config).await.expect("generate report");
        assert_eq!(report.overview.total_threads, 1);
        assert!(report.report_path.exists());
        assert!(report.report_path.starts_with(codex_home.join("reports")));
        assert!(report.generated_at <= Utc::now());
    }
}
