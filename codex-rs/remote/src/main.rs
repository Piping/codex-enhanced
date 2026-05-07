use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;

use anyhow::Context;
use clap::Parser;
use codex_remote::AppState;
use codex_remote::RelayConfig;
use tokio::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "codex-remote",
    about = "Minimal remote bridge for Codex sessions"
)]
struct Cli {
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    host: IpAddr,

    #[arg(long, default_value_t = 38309)]
    port: u16,

    #[arg(long)]
    relay_base_url: Option<String>,

    #[arg(long)]
    relay_workspace_id: Option<String>,

    #[arg(long)]
    relay_shared_secret: Option<String>,

    #[arg(long, default_value_t = 1500)]
    relay_sync_interval_ms: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing();

    let state = AppState::load().await?;
    if let Some(relay_config) = relay_config_from_cli(&cli)? {
        state.start_relay_sync(relay_config);
    }
    let addr = SocketAddr::new(cli.host, cli.port);
    codex_remote::serve(addr, state)
        .await
        .with_context(|| format!("failed to serve codex-remote on {addr}"))
}

fn relay_config_from_cli(cli: &Cli) -> anyhow::Result<Option<RelayConfig>> {
    let Some(base_url) = cli.relay_base_url.as_ref() else {
        return Ok(None);
    };

    let workspace_id = cli
        .relay_workspace_id
        .clone()
        .context("--relay-workspace-id is required when --relay-base-url is set")?;
    let shared_secret = cli
        .relay_shared_secret
        .clone()
        .context("--relay-shared-secret is required when --relay-base-url is set")?;

    let mut normalized = base_url.trim().to_string();
    while normalized.ends_with('/') {
        normalized.pop();
    }

    Ok(Some(RelayConfig {
        base_url: normalized,
        workspace_id,
        shared_secret,
        sync_interval: Duration::from_millis(cli.relay_sync_interval_ms.max(250)),
    }))
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("codex_remote=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
