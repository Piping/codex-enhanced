//! CLI login commands and their direct-user observability surfaces.
//!
//! The TUI path already installs a broader tracing stack with feedback, OpenTelemetry, and other
//! interactive-session layers. Direct `codex login` intentionally does less: it preserves the
//! existing stderr/browser UX and adds only a small file-backed tracing layer for login-specific
//! targets. Keeping that setup local avoids pulling the TUI's session-oriented logging machinery
//! into a one-shot CLI command while still producing a durable `codex-login.log` artifact that
//! support can request from users.

use codex_accounts::AccountPoolStore;
use codex_accounts::ManagedAccountSnapshot;
use codex_accounts::persist_current_managed_account_snapshot;
use codex_accounts::persist_managed_account_auth_snapshot;
use codex_core::CodexAuth;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::AuthMode;
use codex_core::auth::CLIENT_ID;
use codex_core::auth::load_auth_dot_json;
use codex_core::auth::login_with_api_key;
use codex_core::auth::logout;
use codex_core::config::Config;
use codex_login::ServerOptions;
use codex_login::run_device_code_login;
use codex_login::run_login_server;
use codex_protocol::config_types::ForcedLoginMethod;
use codex_utils_cli::CliConfigOverrides;
use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tracing_appender::non_blocking;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const CHATGPT_LOGIN_DISABLED_MESSAGE: &str =
    "ChatGPT login is disabled. Use API key login instead.";
const API_KEY_LOGIN_DISABLED_MESSAGE: &str =
    "API key login is disabled. Use ChatGPT login instead.";
const LOGIN_SUCCESS_MESSAGE: &str = "Successfully logged in";

/// Installs a small file-backed tracing layer for direct `codex login` flows.
///
/// This deliberately duplicates a narrow slice of the TUI logging setup instead of reusing it
/// wholesale. The TUI stack includes session-oriented layers that are valuable for interactive
/// runs but unnecessary for a one-shot login command. Keeping the direct CLI path local lets this
/// command produce a durable `codex-login.log` artifact without coupling it to the TUI's broader
/// telemetry and feedback initialization.
fn init_login_file_logging(config: &Config) -> Option<WorkerGuard> {
    let log_dir = match codex_core::config::log_dir(config) {
        Ok(log_dir) => log_dir,
        Err(err) => {
            eprintln!("Warning: failed to resolve login log directory: {err}");
            return None;
        }
    };

    if let Err(err) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "Warning: failed to create login log directory {}: {err}",
            log_dir.display()
        );
        return None;
    }

    let mut log_file_opts = OpenOptions::new();
    log_file_opts.create(true).append(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        log_file_opts.mode(0o600);
    }

    let log_path = log_dir.join("codex-login.log");
    let log_file = match log_file_opts.open(&log_path) {
        Ok(log_file) => log_file,
        Err(err) => {
            eprintln!(
                "Warning: failed to open login log file {}: {err}",
                log_path.display()
            );
            return None;
        }
    };

    let (non_blocking, guard) = non_blocking(log_file);
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("codex_cli=info,codex_core=info,codex_login=info"));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_target(true)
        .with_ansi(false)
        .with_filter(env_filter);

    // Direct `codex login` otherwise relies on ephemeral stderr and browser output.
    // Persist the same login targets to a file so support can inspect auth failures
    // without reproducing them through TUI or app-server.
    if let Err(err) = tracing_subscriber::registry().with(file_layer).try_init() {
        eprintln!(
            "Warning: failed to initialize login log file {}: {err}",
            log_path.display()
        );
        return None;
    }

    Some(guard)
}

fn print_login_server_start(actual_port: u16, auth_url: &str) {
    eprintln!(
        "Starting local login server on http://localhost:{actual_port}.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{auth_url}\n\nOn a remote or headless machine? Use `codex login --device-auth` instead."
    );
}

fn now_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(i64::MAX)
}

fn upsert_managed_account_snapshot(
    codex_home: &Path,
    snapshot: &ManagedAccountSnapshot,
    set_active: bool,
) -> std::io::Result<()> {
    let account_id = snapshot.profile.id.clone();
    AccountPoolStore::new(codex_home.to_path_buf()).update(|state| {
        state.upsert_account(snapshot.profile.clone());
        if set_active {
            state.set_active_account(&account_id, now_timestamp());
        }
    })?;
    Ok(())
}

fn snapshot_existing_managed_account_before_login(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    let Some(snapshot) =
        persist_current_managed_account_snapshot(codex_home, auth_credentials_store_mode)?
    else {
        return Ok(());
    };
    upsert_managed_account_snapshot(codex_home, &snapshot, /*set_active*/ false)
}

fn register_current_managed_account_after_login(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    managed_account_alias: Option<&str>,
) -> std::io::Result<()> {
    let Some(auth) = load_auth_dot_json(codex_home, auth_credentials_store_mode)? else {
        return Ok(());
    };
    let Some(snapshot) =
        persist_managed_account_auth_snapshot(codex_home, &auth, managed_account_alias)?
    else {
        return Ok(());
    };
    upsert_managed_account_snapshot(codex_home, &snapshot, /*set_active*/ true)
}

pub async fn login_with_chatgpt(
    codex_home: PathBuf,
    forced_chatgpt_workspace_id: Option<String>,
    cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
    managed_account_alias: Option<String>,
) -> std::io::Result<()> {
    snapshot_existing_managed_account_before_login(&codex_home, cli_auth_credentials_store_mode)?;

    let opts = ServerOptions::new(
        codex_home.clone(),
        CLIENT_ID.to_string(),
        forced_chatgpt_workspace_id,
        cli_auth_credentials_store_mode,
    );
    let server = run_login_server(opts)?;

    print_login_server_start(server.actual_port, &server.auth_url);

    server.block_until_done().await?;
    register_current_managed_account_after_login(
        &codex_home,
        cli_auth_credentials_store_mode,
        managed_account_alias.as_deref(),
    )
}

pub async fn run_login_with_chatgpt(
    cli_config_overrides: CliConfigOverrides,
    managed_account_alias: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting browser login flow");

    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();

    match login_with_chatgpt(
        config.codex_home,
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
        managed_account_alias,
    )
    .await
    {
        Ok(_) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_with_api_key(
    cli_config_overrides: CliConfigOverrides,
    api_key: String,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting api key login flow");

    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Chatgpt)) {
        eprintln!("{API_KEY_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    match login_with_api_key(
        &config.codex_home,
        &api_key,
        config.cli_auth_credentials_store_mode,
    ) {
        Ok(_) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub fn read_api_key_from_stdin() -> String {
    let mut stdin = std::io::stdin();

    if stdin.is_terminal() {
        eprintln!(
            "--with-api-key expects the API key on stdin. Try piping it, e.g. `printenv OPENAI_API_KEY | codex login --with-api-key`."
        );
        std::process::exit(1);
    }

    eprintln!("Reading API key from stdin...");

    let mut buffer = String::new();
    if let Err(err) = stdin.read_to_string(&mut buffer) {
        eprintln!("Failed to read API key from stdin: {err}");
        std::process::exit(1);
    }

    let api_key = buffer.trim().to_string();
    if api_key.is_empty() {
        eprintln!("No API key provided via stdin.");
        std::process::exit(1);
    }

    api_key
}

/// Login using the OAuth device code flow.
pub async fn run_login_with_device_code(
    cli_config_overrides: CliConfigOverrides,
    issuer_base_url: Option<String>,
    client_id: Option<String>,
    managed_account_alias: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting device code login flow");
    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }
    if let Err(err) = snapshot_existing_managed_account_before_login(
        &config.codex_home,
        config.cli_auth_credentials_store_mode,
    ) {
        eprintln!("Error preparing managed account snapshots: {err}");
        std::process::exit(1);
    }

    let codex_home = config.codex_home.clone();
    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
    let mut opts = ServerOptions::new(
        config.codex_home,
        client_id.unwrap_or(CLIENT_ID.to_string()),
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    );
    if let Some(iss) = issuer_base_url {
        opts.issuer = iss;
    }
    match run_device_code_login(opts).await {
        Ok(()) => {
            if let Err(err) = register_current_managed_account_after_login(
                &codex_home,
                config.cli_auth_credentials_store_mode,
                managed_account_alias.as_deref(),
            ) {
                eprintln!("Error finalizing managed account login: {err}");
                std::process::exit(1);
            }
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in with device code: {e}");
            std::process::exit(1);
        }
    }
}

/// Prefers device-code login (with `open_browser = false`) when headless environment is detected, but keeps
/// `codex login` working in environments where device-code may be disabled/feature-gated.
/// If `run_device_code_login` returns `ErrorKind::NotFound` ("device-code unsupported"), this
/// falls back to starting the local browser login server.
pub async fn run_login_with_device_code_fallback_to_browser(
    cli_config_overrides: CliConfigOverrides,
    issuer_base_url: Option<String>,
    client_id: Option<String>,
    managed_account_alias: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting login flow with device code fallback");
    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }
    if let Err(err) = snapshot_existing_managed_account_before_login(
        &config.codex_home,
        config.cli_auth_credentials_store_mode,
    ) {
        eprintln!("Error preparing managed account snapshots: {err}");
        std::process::exit(1);
    }

    let codex_home = config.codex_home.clone();
    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
    let mut opts = ServerOptions::new(
        config.codex_home,
        client_id.unwrap_or(CLIENT_ID.to_string()),
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    );
    if let Some(iss) = issuer_base_url {
        opts.issuer = iss;
    }
    opts.open_browser = false;

    match run_device_code_login(opts.clone()).await {
        Ok(()) => {
            if let Err(err) = register_current_managed_account_after_login(
                &codex_home,
                config.cli_auth_credentials_store_mode,
                managed_account_alias.as_deref(),
            ) {
                eprintln!("Error finalizing managed account login: {err}");
                std::process::exit(1);
            }
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                eprintln!("Device code login is not enabled; falling back to browser login.");
                match run_login_server(opts) {
                    Ok(server) => {
                        print_login_server_start(server.actual_port, &server.auth_url);
                        match server.block_until_done().await {
                            Ok(()) => {
                                if let Err(err) = register_current_managed_account_after_login(
                                    &codex_home,
                                    config.cli_auth_credentials_store_mode,
                                    managed_account_alias.as_deref(),
                                ) {
                                    eprintln!("Error finalizing managed account login: {err}");
                                    std::process::exit(1);
                                }
                                eprintln!("{LOGIN_SUCCESS_MESSAGE}");
                                std::process::exit(0);
                            }
                            Err(e) => {
                                eprintln!("Error logging in: {e}");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error logging in: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("Error logging in with device code: {e}");
                std::process::exit(1);
            }
        }
    }
}

pub async fn run_login_status(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    match CodexAuth::from_auth_storage(&config.codex_home, config.cli_auth_credentials_store_mode) {
        Ok(Some(auth)) => match auth.auth_mode() {
            AuthMode::ApiKey => match auth.get_token() {
                Ok(api_key) => {
                    eprintln!("Logged in using an API key - {}", safe_format_key(&api_key));
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Unexpected error retrieving API key: {e}");
                    std::process::exit(1);
                }
            },
            AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens => {
                eprintln!("Logged in using ChatGPT");
                std::process::exit(0);
            }
        },
        Ok(None) => {
            eprintln!("Not logged in");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_logout(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    match logout(&config.codex_home, config.cli_auth_credentials_store_mode) {
        Ok(true) => {
            eprintln!("Successfully logged out");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("Not logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging out: {e}");
            std::process::exit(1);
        }
    }
}

async fn load_config_or_exit(cli_config_overrides: CliConfigOverrides) -> Config {
    let cli_overrides = match cli_config_overrides.parse_overrides() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing -c overrides: {e}");
            std::process::exit(1);
        }
    };

    match Config::load_with_cli_overrides(cli_overrides).await {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading configuration: {e}");
            std::process::exit(1);
        }
    }
}

fn safe_format_key(key: &str) -> String {
    if key.len() <= 13 {
        return "***".to_string();
    }
    let prefix = &key[..8];
    let suffix = &key[key.len() - 5..];
    format!("{prefix}***{suffix}")
}

#[cfg(test)]
mod tests {
    use codex_app_server_protocol::AuthMode as ApiAuthMode;
    use codex_core::auth::AuthCredentialsStoreMode;
    use codex_core::auth::AuthDotJson;
    use codex_core::auth::save_auth;
    use codex_core::token_data::TokenData;
    use codex_ext::AccountPoolStore;
    use codex_ext::ManagedAccountAuthStore;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::login::register_current_managed_account_after_login;
    use crate::login::snapshot_existing_managed_account_before_login;

    use super::safe_format_key;

    #[test]
    fn formats_long_key() {
        let key = "sk-proj-1234567890ABCDE";
        assert_eq!(safe_format_key(key), "sk-proj-***ABCDE");
    }

    #[test]
    fn short_key_returns_stars() {
        let key = "sk-proj-12345";
        assert_eq!(safe_format_key(key), "***");
    }

    fn fake_jwt(email: &str, account_id: &str, plan_type: &str) -> String {
        let header = json!({"alg":"none","typ":"JWT"});
        let payload = json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_plan_type": plan_type,
            },
        });
        let encode = |value: serde_json::Value| -> String {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&value).expect("serialize"))
        };
        format!("{}.{}.sig", encode(header), encode(payload))
    }

    fn chatgpt_auth(account_id: &str, email: &str) -> AuthDotJson {
        AuthDotJson {
            auth_mode: Some(ApiAuthMode::Chatgpt),
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: codex_core::token_data::parse_chatgpt_jwt_claims(&fake_jwt(
                    email, account_id, "pro",
                ))
                .expect("id token"),
                access_token: fake_jwt(email, account_id, "pro"),
                refresh_token: "refresh-token".to_string(),
                account_id: Some(account_id.to_string()),
            }),
            last_refresh: None,
        }
    }

    #[test]
    fn snapshot_existing_managed_account_before_login_copies_root_auth() {
        let tempdir = tempdir().expect("tempdir");
        let auth = chatgpt_auth("acct-existing", "existing@example.com");
        save_auth(tempdir.path(), &auth, AuthCredentialsStoreMode::File).expect("save root auth");

        snapshot_existing_managed_account_before_login(
            tempdir.path(),
            AuthCredentialsStoreMode::File,
        )
        .expect("snapshot existing auth");

        let managed_auth = ManagedAccountAuthStore::new(tempdir.path().to_path_buf())
            .load_account_auth("acct-existing")
            .expect("managed auth");
        let pool = AccountPoolStore::new(tempdir.path().to_path_buf())
            .load()
            .expect("load account pool");

        assert_eq!(managed_auth, auth);
        assert_eq!(pool.accounts[0].alias, "acct-existing");
    }

    #[test]
    fn register_current_managed_account_after_login_saves_alias_and_marks_active() {
        let tempdir = tempdir().expect("tempdir");
        let auth = chatgpt_auth("acct-primary", "primary@example.com");
        save_auth(tempdir.path(), &auth, AuthCredentialsStoreMode::File).expect("save root auth");

        register_current_managed_account_after_login(
            tempdir.path(),
            AuthCredentialsStoreMode::File,
            Some("Primary"),
        )
        .expect("register managed account");

        let managed_auth = ManagedAccountAuthStore::new(tempdir.path().to_path_buf())
            .load_account_auth("acct-primary")
            .expect("managed auth");
        let pool = AccountPoolStore::new(tempdir.path().to_path_buf())
            .load()
            .expect("load account pool");

        assert_eq!(managed_auth, auth);
        assert_eq!(pool.active_account_id.as_deref(), Some("acct-primary"));
        assert_eq!(pool.accounts[0].alias, "Primary");
    }
}
