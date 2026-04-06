use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use codex_clawbot::CLAWBOT_RELATIVE_DIR;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::FeishuConfig;
use open_lark::openlark_client;
use open_lark::openlark_client::ws_client::EventDispatcherHandler;
use open_lark::openlark_client::ws_client::LarkWsClient;
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::sync::mpsc;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(2);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const DEFAULT_DUMP_FILE_NAME: &str = "feishu_payload_dump.jsonl";

fn main() -> Result<()> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let workspace_root = workspace_root_from_args()?;
    let runtime = ClawbotRuntime::load(workspace_root.clone()).with_context(|| {
        format!(
            "failed to load clawbot config from {}",
            workspace_root.display()
        )
    })?;
    let feishu = runtime
        .snapshot()
        .config
        .feishu
        .clone()
        .context("missing [feishu] config in .codex/clawbot/config.toml")?;
    if !feishu.has_api_credentials() {
        return Err(anyhow!(
            "missing app_id/app_secret in clawbot feishu config"
        ));
    }

    let dump_path = dump_path_from_args(&workspace_root);
    ensure_parent_dir(&dump_path)?;
    eprintln!("workspace: {}", workspace_root.display());
    eprintln!("dump file: {}", dump_path.display());

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    loop {
        match run_once(&feishu, &dump_path).await {
            Ok(()) => {
                eprintln!(
                    "feishu websocket exited; reconnecting in {}s",
                    reconnect_delay.as_secs()
                );
            }
            Err(err) => {
                eprintln!(
                    "feishu websocket failed: {err}; reconnecting in {}s",
                    reconnect_delay.as_secs()
                );
            }
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

async fn run_once(feishu: &FeishuConfig, dump_path: &Path) -> Result<()> {
    let ws_config = Arc::new(build_websocket_config(feishu)?);
    let (payload_tx, mut payload_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let dump_path = dump_path.to_path_buf();
    let payload_task = tokio::spawn(async move {
        while let Some(payload) = payload_rx.recv().await {
            if let Err(err) = append_payload_record(&dump_path, &payload) {
                eprintln!("failed to write payload record: {err}");
            }
        }
    });

    let event_handler = EventDispatcherHandler::builder()
        .payload_sender(payload_tx)
        .build();
    eprintln!("feishu websocket connected; waiting for events...");
    let result = LarkWsClient::open(ws_config, event_handler)
        .await
        .map_err(|err| anyhow!("websocket runtime failed: {err}"));
    payload_task.abort();
    let _ = payload_task.await;
    result
}

fn workspace_root_from_args() -> Result<PathBuf> {
    Ok(match env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => env::current_dir().context("failed to resolve current directory")?,
    })
}

fn dump_path_from_args(workspace_root: &Path) -> PathBuf {
    env::args().nth(2).map_or_else(
        || {
            workspace_root
                .join(CLAWBOT_RELATIVE_DIR)
                .join(DEFAULT_DUMP_FILE_NAME)
        },
        PathBuf::from,
    )
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))
}

fn build_websocket_config(feishu: &FeishuConfig) -> Result<openlark_client::Config> {
    openlark_client::Config::builder()
        .app_id(feishu.app_id.clone())
        .app_secret(feishu.app_secret.clone())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| anyhow!("failed to build websocket config: {err}"))
}

fn append_payload_record(dump_path: &Path, payload: &[u8]) -> Result<()> {
    let raw_text = String::from_utf8_lossy(payload).into_owned();
    let parsed_payload = serde_json::from_slice::<Value>(payload).ok();
    let event_type = parsed_payload
        .as_ref()
        .and_then(|value| value.pointer("/header/event_type"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let recorded_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs() as i64;
    let record = match parsed_payload {
        Some(payload) => serde_json::json!({
            "recorded_at": recorded_at,
            "event_type": event_type,
            "payload": payload,
        }),
        None => serde_json::json!({
            "recorded_at": recorded_at,
            "event_type": event_type,
            "payload_text": raw_text,
        }),
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dump_path)
        .with_context(|| format!("failed to open {}", dump_path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&record)?)
        .with_context(|| format!("failed to append {}", dump_path.display()))
}
