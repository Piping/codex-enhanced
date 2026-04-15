use std::env;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::FeishuProviderRuntime;
use codex_clawbot::ProviderEvent;
use tokio::runtime::Builder;
use tokio::sync::mpsc;

fn main() -> Result<()> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let (workspace_root, timeout_secs) = parse_args()?;
    let runtime = ClawbotRuntime::load(workspace_root.clone())
        .with_context(|| format!("failed to load workspace {}", workspace_root.display()))?;
    let provider = runtime
        .feishu_provider()
        .context("missing [feishu] config in workspace clawbot config")?;

    eprintln!(
        "probe starting: workspace={} timeout_secs={timeout_secs}",
        workspace_root.display()
    );

    let (provider_event_tx, mut provider_event_rx) = mpsc::unbounded_channel();
    let provider_task = tokio::spawn(run_provider(provider, provider_event_tx));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                eprintln!("probe timeout reached");
                break;
            }
            maybe_event = provider_event_rx.recv() => {
                let Some(event) = maybe_event else {
                    eprintln!("provider event channel closed");
                    break;
                };
                print_event(event);
            }
        }
    }

    provider_task.abort();
    let _ = provider_task.await;
    Ok(())
}

async fn run_provider(
    provider: FeishuProviderRuntime,
    provider_event_tx: mpsc::UnboundedSender<ProviderEvent>,
) {
    if let Err(error) = provider.run(provider_event_tx).await {
        eprintln!("provider exited with error: {error}");
    }
}

fn print_event(event: ProviderEvent) {
    match event {
        ProviderEvent::RuntimeStateUpdated(state) => {
            println!(
                "{{\"type\":\"runtime_state\",\"connection\":\"{:?}\",\"last_error\":{},\"updated_at\":{}}}",
                state.connection,
                serde_json::to_string(&state.last_error).unwrap_or_else(|_| "null".to_string()),
                state
                    .updated_at
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "null".to_string()),
            );
        }
        ProviderEvent::SessionUpserted(session) => {
            println!(
                "{{\"type\":\"session_upserted\",\"session_id\":{},\"status\":\"{:?}\"}}",
                serde_json::to_string(&session.session_id)
                    .unwrap_or_else(|_| "\"<encode_failed>\"".to_string()),
                session.status,
            );
        }
        ProviderEvent::SessionRemoved(session) => {
            println!(
                "{{\"type\":\"session_removed\",\"session_id\":{}}}",
                serde_json::to_string(&session.session_id)
                    .unwrap_or_else(|_| "\"<encode_failed>\"".to_string()),
            );
        }
        ProviderEvent::InboundMessage(message) => {
            println!(
                "{{\"type\":\"inbound_message\",\"session_id\":{},\"message_id\":{}}}",
                serde_json::to_string(&message.session.session_id)
                    .unwrap_or_else(|_| "\"<encode_failed>\"".to_string()),
                serde_json::to_string(&message.message_id)
                    .unwrap_or_else(|_| "\"<encode_failed>\"".to_string()),
            );
        }
    }
}

fn parse_args() -> Result<(PathBuf, u64)> {
    let mut args = env::args().skip(1);
    let workspace_root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(env::current_dir().context("failed to resolve current directory")?);
    let timeout_secs = args
        .next()
        .as_deref()
        .map(str::parse)
        .transpose()
        .context("failed to parse timeout seconds")?
        .unwrap_or(60);
    Ok((workspace_root, timeout_secs))
}
