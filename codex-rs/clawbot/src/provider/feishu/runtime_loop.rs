use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use anyhow::anyhow;
use open_lark::openlark_client;
use open_lark::openlark_client::ws_client::EventDispatcherHandler;
use open_lark::openlark_client::ws_client::LarkWsClient;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::coordination::FeishuBaseCoordinator;
use super::coordination::WebsocketOwnershipState;
use super::provider_events_from_payload;
use super::runtime_state;
use crate::append_diagnostic_event;
use crate::config::FeishuConfig;
use crate::model::ConnectionStatus;
use crate::provider::ProviderEvent;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

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

    if let Some(coordination) = FeishuBaseCoordinator::new(workspace_root.as_path(), &config)? {
        return run_with_coordination(workspace_root, config, provider_event_tx, coordination)
            .await;
    }

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    loop {
        match run_once(workspace_root.as_path(), &config, &provider_event_tx).await {
            Ok(()) => {
                emit_reconnect_state(
                    workspace_root.as_path(),
                    &provider_event_tx,
                    reconnect_delay,
                    Ok(()),
                )?;
            }
            Err(error) => {
                emit_reconnect_state(
                    workspace_root.as_path(),
                    &provider_event_tx,
                    reconnect_delay,
                    Err(&error),
                )?;
            }
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

async fn run_with_coordination(
    workspace_root: PathBuf,
    config: FeishuConfig,
    provider_event_tx: mpsc::UnboundedSender<ProviderEvent>,
    coordination: FeishuBaseCoordinator,
) -> Result<()> {
    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    let mut reconnect_at = None::<Instant>;
    let mut websocket_task = None::<JoinHandle<Result<()>>>;
    let mut last_standby_message = None::<String>;

    loop {
        if let Some(handle) = websocket_task.as_ref()
            && handle.is_finished()
        {
            let result = websocket_task
                .take()
                .expect("finished websocket task should still be present")
                .await
                .map_err(|error| anyhow!("Feishu websocket runtime task failed: {error}"))?;
            emit_reconnect_state(
                workspace_root.as_path(),
                &provider_event_tx,
                reconnect_delay,
                result.as_ref().map(|_| ()).map_err(|error| error),
            )?;
            reconnect_at = Some(Instant::now() + reconnect_delay);
            reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
        }

        let websocket_state = websocket_ownership_state(&websocket_task, reconnect_at);
        let leadership = match coordination.refresh_leadership(websocket_state).await {
            Ok(leadership) => leadership,
            Err(error) => {
                if let Some(handle) = websocket_task.take() {
                    handle.abort();
                    let _ = handle.await;
                }
                let _ = append_diagnostic_event(
                    workspace_root.as_path(),
                    "feishu.coordination_failed",
                    serde_json::json!({
                        "error": error.to_string(),
                        "reconnect_delay_secs": reconnect_delay.as_secs(),
                    }),
                );
                let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
                    ConnectionStatus::Error,
                    Some(format!(
                        "Feishu Base coordination failed: {error}; retrying in {}s",
                        reconnect_delay.as_secs()
                    )),
                )?));
                last_standby_message = None;
                reconnect_at = Some(Instant::now() + reconnect_delay);
                reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                tokio::time::sleep(reconnect_delay).await;
                continue;
            }
        };

        if leadership.is_leader {
            last_standby_message = None;
            if websocket_task.is_none() && backoff_elapsed(reconnect_at) {
                websocket_task = Some(spawn_websocket_task(
                    workspace_root.clone(),
                    config.clone(),
                    provider_event_tx.clone(),
                ));
                reconnect_at = None;
                reconnect_delay = INITIAL_RECONNECT_DELAY;
            }
        } else {
            reconnect_delay = INITIAL_RECONNECT_DELAY;
            reconnect_at = None;
            if let Some(handle) = websocket_task.take() {
                handle.abort();
                let _ = handle.await;
                let _ = append_diagnostic_event(
                    workspace_root.as_path(),
                    "feishu.runtime_leadership_released",
                    serde_json::json!({
                        "leader_instance_id": leadership.leader_instance_id,
                        "leader_session_id": leadership.leader_session_id,
                    }),
                );
            }

            let standby_message = format!("Feishu standby: {}", leadership.standby_message());
            if last_standby_message.as_deref() != Some(standby_message.as_str()) {
                let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
                    ConnectionStatus::Disconnected,
                    Some(standby_message.clone()),
                )?));
                last_standby_message = Some(standby_message);
            }
        }

        tokio::time::sleep(sleep_duration(
            coordination.heartbeat_interval(),
            reconnect_at,
        ))
        .await;
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
    let event_handler = EventDispatcherHandler::builder()
        .payload_sender(payload_tx)
        .build();
    let payload_provider_event_tx = provider_event_tx.clone();
    let payload_config = config.clone();
    let payload_workspace_root = workspace_root.to_path_buf();
    let payload_task = tokio::spawn(async move {
        while let Some(payload) = payload_rx.recv().await {
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

    let _ = append_diagnostic_event(
        workspace_root,
        "feishu.runtime_connected",
        serde_json::json!({}),
    );
    let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
        ConnectionStatus::Connected,
        /*last_error*/ None,
    )?));

    let websocket_result =
        tokio::spawn(async move { LarkWsClient::open(ws_config, event_handler).await })
            .await
            .map_err(|error| anyhow!("Feishu websocket runtime task failed: {error}"))?
            .map_err(|error| anyhow!("Feishu websocket runtime failed: {error}"));
    payload_task.abort();
    let _ = payload_task.await;
    websocket_result
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

fn spawn_websocket_task(
    workspace_root: PathBuf,
    config: FeishuConfig,
    provider_event_tx: mpsc::UnboundedSender<ProviderEvent>,
) -> JoinHandle<Result<()>> {
    tokio::spawn(
        async move { run_once(workspace_root.as_path(), &config, &provider_event_tx).await },
    )
}

fn emit_reconnect_state(
    workspace_root: &Path,
    provider_event_tx: &mpsc::UnboundedSender<ProviderEvent>,
    reconnect_delay: Duration,
    result: Result<(), &anyhow::Error>,
) -> Result<()> {
    match result {
        Ok(()) => {
            let _ = append_diagnostic_event(
                workspace_root,
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
                workspace_root,
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
    Ok(())
}

fn websocket_ownership_state(
    websocket_task: &Option<JoinHandle<Result<()>>>,
    reconnect_at: Option<Instant>,
) -> WebsocketOwnershipState {
    if websocket_task.is_some() {
        return WebsocketOwnershipState::Connected;
    }
    if !backoff_elapsed(reconnect_at) {
        return WebsocketOwnershipState::BackingOff;
    }
    WebsocketOwnershipState::Idle
}

fn backoff_elapsed(reconnect_at: Option<Instant>) -> bool {
    reconnect_at.is_none_or(|instant| instant <= Instant::now())
}

fn sleep_duration(heartbeat_interval: Duration, reconnect_at: Option<Instant>) -> Duration {
    if let Some(reconnect_at) = reconnect_at {
        return reconnect_at
            .saturating_duration_since(Instant::now())
            .min(heartbeat_interval);
    }
    heartbeat_interval
}
