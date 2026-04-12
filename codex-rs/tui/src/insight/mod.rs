use codex_core::config::Config;
use codex_insight::InsightReportOptions;
use codex_insight::generate_report;
use codex_insight::report_started_message;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::history_cell;

pub(crate) fn request_report(config: Config, app_event_tx: AppEventSender) -> String {
    spawn_report_generation(config, app_event_tx);
    report_started_message()
}

pub(crate) fn spawn_report_generation(config: Config, app_event_tx: AppEventSender) {
    tokio::spawn(async move {
        let options = InsightReportOptions::new(config.codex_home.clone());
        match generate_report(&options).await {
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
