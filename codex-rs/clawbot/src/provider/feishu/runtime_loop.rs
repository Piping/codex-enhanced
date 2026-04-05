use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use anyhow::anyhow;
use open_lark::openlark_client;
use open_lark::openlark_client::ws_client::EventDispatcherHandler;
use open_lark::openlark_client::ws_client::LarkWsClient;
use tokio::sync::mpsc;

use super::provider_events_from_payload;
use super::runtime_state;
use crate::config::FeishuConfig;
use crate::model::ConnectionStatus;
use crate::provider::ProviderEvent;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

pub(super) async fn run_with_reconnect(
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
        match run_once(&config, &provider_event_tx).await {
            Ok(()) => {
                let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
                    ConnectionStatus::Disconnected,
                    Some(format!(
                        "Feishu websocket runtime exited; reconnecting in {}s",
                        reconnect_delay.as_secs()
                    )),
                )?));
            }
            Err(error) => {
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
    config: &FeishuConfig,
    provider_event_tx: &mpsc::UnboundedSender<ProviderEvent>,
) -> Result<()> {
    let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
        ConnectionStatus::Connecting,
        None,
    )?));

    let ws_config = Arc::new(build_websocket_config(config)?);
    let (payload_tx, mut payload_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let event_handler = EventDispatcherHandler::builder()
        .payload_sender(payload_tx)
        .build();
    let payload_provider_event_tx = provider_event_tx.clone();
    let payload_task = tokio::spawn(async move {
        while let Some(payload) = payload_rx.recv().await {
            for event in provider_events_from_payload(&payload) {
                let _ = payload_provider_event_tx.send(event);
            }
        }
    });

    let _ = provider_event_tx.send(ProviderEvent::RuntimeStateUpdated(runtime_state(
        ConnectionStatus::Connected,
        None,
    )?));

    let open_result = LarkWsClient::open(ws_config, event_handler).await;
    payload_task.abort();
    open_result.map_err(|error| anyhow!("Feishu websocket runtime failed: {error}"))
}

pub(super) fn build_websocket_config(config: &FeishuConfig) -> Result<openlark_client::Config> {
    openlark_client::Config::builder()
        .app_id(config.app_id.clone())
        .app_secret(config.app_secret.clone())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| anyhow!("failed to build Feishu websocket config: {error}"))
}
