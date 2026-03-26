use codex_backend_client::Client as BackendClient;
use codex_core::CodexAuth;
use codex_protocol::protocol::RateLimitSnapshot;
use tracing::debug;

pub(crate) async fn fetch_rate_limits(base_url: String, auth: CodexAuth) -> Vec<RateLimitSnapshot> {
    match BackendClient::from_auth(base_url, &auth) {
        Ok(client) => match client.get_rate_limits_many().await {
            Ok(snapshots) => snapshots,
            Err(err) => {
                debug!(error = ?err, "failed to fetch rate limits from /usage");
                Vec::new()
            }
        },
        Err(err) => {
            debug!(error = ?err, "failed to construct backend client for rate limits");
            Vec::new()
        }
    }
}
