use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use anyhow::anyhow;
use open_lark::openlark_client;
use open_lark::openlark_client::ws_client::EventDispatcherHandler;
use open_lark::openlark_client::ws_client::LarkWsClient;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio::time::MissedTickBehavior;

use super::provider_events_from_payload;
use super::runtime_state;
use crate::append_diagnostic_event;
use crate::config::FeishuConfig;
use crate::model::ConnectionStatus;
use crate::provider::ProviderEvent;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_WATCHDOG_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(super) async fn run_with_reconnect(
    workspace_root: PathBuf,
    config: FeishuConfig,
    provider_event_tx: mpsc::UnboundedSender<ProviderEvent>,
) -> Result<()> {
    if !config.has_api_credentials() {
        let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
            ConnectionStatus::Unconfigured,
            Some("missing app_id/app_secret".to_string()),
        )?));
        return Err(anyhow!("missing app_id/app_secret"));
    }

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    loop {
        match run_once(workspace_root.as_path(), &config, &provider_event_tx).await {
            Ok(()) => {
                let _ = append_diagnostic_event(
                    workspace_root.as_path(),
                    "feishu.runtime_disconnected",
                    serde_json::json!({
                        "reconnect_delay_secs": reconnect_delay.as_secs(),
                    }),
                );
                let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
                    ConnectionStatus::Disconnected,
                    Some(format!(
                        "Feishu websocket runtime exited; reconnecting in {}s",
                        reconnect_delay.as_secs()
                    )),
                )?));
            }
            Err(error) => {
                let _ = append_diagnostic_event(
                    workspace_root.as_path(),
                    "feishu.runtime_failed",
                    serde_json::json!({
                        "error": error.to_string(),
                        "reconnect_delay_secs": reconnect_delay.as_secs(),
                    }),
                );
                let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
                    ConnectionStatus::Error,
                    Some(format!(
                        "Feishu websocket runtime failed: {error}; reconnecting in {}s",
                        reconnect_delay.as_secs()
                    )),
                )?));
            }
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

async fn run_once(
    workspace_root: &Path,
    config: &FeishuConfig,
    provider_event_tx: &mpsc::UnboundedSender<ProviderEvent>,
) -> Result<()> {
    let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
        ConnectionStatus::Connecting,
        /*last_error*/ None,
    )?));

    let ws_config = Arc::new(build_websocket_config(config)?);
    let (payload_tx, mut payload_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (last_payload_at_tx, last_payload_at_rx) = watch::channel(Instant::now());
    let event_handler = EventDispatcherHandler::builder()
        .payload_sender(payload_tx)
        .build();
    let payload_provider_event_tx = provider_event_tx.clone();
    let payload_config = config.clone();
    let payload_workspace_root = workspace_root.to_path_buf();
    let payload_task = tokio::spawn(async move {
        while let Some(payload) = payload_rx.recv().await {
            let _ = last_payload_at_tx.send(Instant::now());
            let _ = append_diagnostic_event(
                payload_workspace_root.as_path(),
                "feishu.raw_payload",
                payload_debug_value(&payload),
            );
            for event in provider_events_from_payload(
                &payload,
                &payload_config,
                payload_workspace_root.as_path(),
            ) {
                let _ = payload_provider_event_tx.send(event);
            }
        }
    });
    let last_payload_at_rx = last_payload_at_rx;

    let _ = append_diagnostic_event(
        workspace_root,
        "feishu.runtime_connected",
        serde_json::json!({}),
    );
    let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
        ConnectionStatus::Connected,
        /*last_error*/ None,
    )?));

    let mut websocket_task =
        tokio::spawn(async move { LarkWsClient::open(ws_config, event_handler).await });
    let mut idle_watchdog = tokio::time::interval(IDLE_WATCHDOG_POLL_INTERVAL);
    idle_watchdog.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            websocket_result = &mut websocket_task => {
                payload_task.abort();
                return websocket_result
                    .map_err(|error| anyhow!("Feishu websocket runtime task failed: {error}"))?
                    .map_err(|error| anyhow!("Feishu websocket runtime failed: {error}"));
            }
            _ = idle_watchdog.tick() => {
                let last_payload_at = *last_payload_at_rx.borrow();
                if idle_timeout_exceeded(last_payload_at, Instant::now()) {
                    let _ = append_diagnostic_event(
                        workspace_root,
                        "feishu.runtime_idle_timeout",
                        serde_json::json!({
                            "idle_timeout_secs": IDLE_TIMEOUT.as_secs(),
                        }),
                    );
                    websocket_task.abort();
                    let _ = websocket_task.await;
                    payload_task.abort();
                    return Err(anyhow!(
                        "Feishu websocket idle timeout after {}s without payloads",
                        IDLE_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }
}

pub(super) fn build_websocket_config(config: &FeishuConfig) -> Result<openlark_client::Config> {
    openlark_client::Config::builder()
        .app_id(config.app_id.clone())
        .app_secret(config.app_secret.clone())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| anyhow!("failed to build Feishu websocket config: {error}"))
}

fn payload_debug_value(payload: &[u8]) -> serde_json::Value {
    serde_json::from_slice(payload).unwrap_or_else(|_| {
        serde_json::json!({
            "raw": String::from_utf8_lossy(payload),
        })
    })
}

fn idle_timeout_exceeded(last_payload_at: Instant, now: Instant) -> bool {
    now.duration_since(last_payload_at) >= IDLE_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::IDLE_TIMEOUT;
    use super::idle_timeout_exceeded;
    use std::time::Duration;
    use tokio::time::Instant;

    #[test]
    fn idle_timeout_triggers_at_threshold() {
        let last_payload_at = Instant::now();

        assert!(!idle_timeout_exceeded(
            last_payload_at,
            last_payload_at + IDLE_TIMEOUT - Duration::from_millis(1),
        ));
        assert!(idle_timeout_exceeded(
            last_payload_at,
            last_payload_at + IDLE_TIMEOUT,
        ));
    }
}
