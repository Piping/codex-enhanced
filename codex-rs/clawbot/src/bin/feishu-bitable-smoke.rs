use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_clawbot::ClawbotRuntime;
use open_lark::openlark_client;
use open_lark::openlark_core::api::ApiRequest;
use open_lark::openlark_core::http::Transport;
use serde_json::Value;

fn main() -> Result<()> {
    let workspace = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~/clawbot"));
    let workspace = expand_home(workspace);
    let runtime = ClawbotRuntime::load(workspace.clone())
        .with_context(|| format!("failed to load workspace {}", workspace.display()))?;
    let feishu = runtime
        .snapshot()
        .config
        .feishu
        .clone()
        .context("missing feishu config")?;
    let coordination = feishu
        .coordination
        .clone()
        .context("missing feishu coordination config")?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    rt.block_on(async move {
        let config = openlark_client::Config::builder()
            .app_id(feishu.app_id)
            .app_secret(feishu.app_secret)
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build websocket config")?
            .build_core_config_with_token_provider();

        let request: ApiRequest<Value> = ApiRequest::get(format!(
            "/open-apis/bitable/v1/apps/{}/tables/{}/records",
            coordination.base_token, coordination.heartbeat_table_id
        ))
        .query("page_size", "10");
        let response = Transport::<Value>::request(request, &config, Some(Default::default()))
            .await
            .context("request failed")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "success": response.is_success(),
                "msg": response.msg(),
                "data": response.data,
            }))?
        );
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

fn expand_home(path: PathBuf) -> PathBuf {
    if let Some(text) = path.to_str()
        && let Some(stripped) = text.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(stripped);
    }
    path
}
