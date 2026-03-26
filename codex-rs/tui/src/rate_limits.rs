use codex_backend_client::Client as BackendClient;
use codex_backend_client::RequestError;
use codex_core::CodexAuth;
use codex_core::auth::AuthDotJson;
use codex_protocol::protocol::RateLimitSnapshot;
use tracing::debug;

#[derive(Debug)]
pub(crate) enum ManagedAccountQuotaOutcome {
    Refreshed(Vec<RateLimitSnapshot>),
    Invalid(String),
    Error(String),
}

#[derive(Debug)]
pub(crate) struct ManagedAccountQuotaUpdate {
    pub(crate) account_id: String,
    pub(crate) display_name: String,
    pub(crate) outcome: ManagedAccountQuotaOutcome,
}

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

pub(crate) async fn fetch_managed_account_quota(
    base_url: String,
    auth: CodexAuth,
) -> ManagedAccountQuotaOutcome {
    match BackendClient::from_auth(base_url, &auth) {
        Ok(client) => fetch_managed_account_quota_from_client(client).await,
        Err(err) => ManagedAccountQuotaOutcome::Error(format!(
            "failed to construct backend client for rate limits: {err}"
        )),
    }
}

pub(crate) async fn fetch_managed_account_quota_from_auth_dot_json(
    base_url: String,
    auth: &AuthDotJson,
) -> ManagedAccountQuotaOutcome {
    match BackendClient::from_auth_dot_json(base_url, auth) {
        Ok(client) => fetch_managed_account_quota_from_client(client).await,
        Err(err) => ManagedAccountQuotaOutcome::Error(format!(
            "failed to construct backend client for rate limits: {err}"
        )),
    }
}

async fn fetch_managed_account_quota_from_client(
    client: BackendClient,
) -> ManagedAccountQuotaOutcome {
    match client.get_rate_limits_many_detailed().await {
        Ok(snapshots) => ManagedAccountQuotaOutcome::Refreshed(snapshots),
        Err(err) => classify_managed_account_quota_error(err),
    }
}

fn classify_managed_account_quota_error(err: RequestError) -> ManagedAccountQuotaOutcome {
    match err {
        RequestError::UnexpectedStatus { body, .. } => {
            let normalized_body = body.to_ascii_lowercase();
            if normalized_body.contains("deactivated workspace") {
                ManagedAccountQuotaOutcome::Invalid("deactivated workspace".to_string())
            } else {
                ManagedAccountQuotaOutcome::Error(format!(
                    "failed to fetch rate limits from /usage: {normalized_body}"
                ))
            }
        }
        RequestError::Other(err) => ManagedAccountQuotaOutcome::Error(format!(
            "failed to fetch rate limits from /usage: {err}"
        )),
    }
}
