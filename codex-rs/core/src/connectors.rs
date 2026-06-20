use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use codex_api::SharedAuthProvider;
pub use codex_app_server_protocol::AppBranding;
pub use codex_app_server_protocol::AppInfo;
pub use codex_app_server_protocol::AppMetadata;
use codex_connectors::AllConnectorsCacheKey;
use codex_connectors::DirectoryListResponse;
use codex_exec_server::EnvironmentManager;
use codex_tools::DiscoverableTool;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::plugins::list_tool_suggest_discoverable_plugins;
use codex_config::AppsRequirementsToml;
use codex_config::types::AppToolApproval;
use codex_config::types::AppsConfigToml;
use codex_config::types::ToolSuggestDiscoverableType;
use codex_core_plugins::PluginsManager;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::default_client::create_client;
use codex_login::default_client::originator;

const DIRECTORY_CONNECTORS_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppToolPolicy {
    pub enabled: bool,
    pub approval: AppToolApproval,
}

impl Default for AppToolPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            approval: AppToolApproval::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AccessibleConnectorsStatus {
    pub connectors: Vec<AppInfo>,
    pub codex_apps_ready: bool,
}

pub async fn list_accessible_connectors_from_mcp_tools(
    config: &Config,
) -> anyhow::Result<Vec<AppInfo>> {
    let _ = config;
    Ok(Vec::new())
}

pub(crate) async fn list_tool_suggest_discoverable_tools_with_auth(
    config: &Config,
    auth: Option<&CodexAuth>,
    accessible_connectors: &[AppInfo],
) -> anyhow::Result<Vec<DiscoverableTool>> {
    let directory_connectors =
        list_directory_connectors_for_tool_suggest_with_auth(config, auth).await?;
    let connector_ids = tool_suggest_connector_ids(config).await;
    let discoverable_connectors =
        codex_connectors::filter::filter_tool_suggest_discoverable_connectors(
            directory_connectors,
            accessible_connectors,
            &connector_ids,
            originator().value.as_str(),
        )
        .into_iter()
        .map(DiscoverableTool::from);
    let discoverable_plugins = list_tool_suggest_discoverable_plugins(config)
        .await?
        .into_iter()
        .map(DiscoverableTool::from);
    Ok(discoverable_connectors
        .chain(discoverable_plugins)
        .collect())
}

pub async fn list_cached_accessible_connectors_from_mcp_tools(
    config: &Config,
) -> Option<Vec<AppInfo>> {
    let _ = config;
    Some(Vec::new())
}

pub async fn list_accessible_connectors_from_mcp_tools_with_options(
    config: &Config,
    force_refetch: bool,
) -> anyhow::Result<Vec<AppInfo>> {
    let _ = force_refetch;
    list_accessible_connectors_from_mcp_tools(config).await
}

pub async fn list_accessible_connectors_from_mcp_tools_with_options_and_status(
    config: &Config,
    force_refetch: bool,
) -> anyhow::Result<AccessibleConnectorsStatus> {
    let _ = force_refetch;
    Ok(AccessibleConnectorsStatus {
        connectors: list_accessible_connectors_from_mcp_tools(config).await?,
        codex_apps_ready: false,
    })
}

pub async fn list_accessible_connectors_from_mcp_tools_with_environment_manager(
    config: &Config,
    force_refetch: bool,
    environment_manager: &EnvironmentManager,
) -> anyhow::Result<AccessibleConnectorsStatus> {
    let _ = (force_refetch, environment_manager);
    Ok(AccessibleConnectorsStatus {
        connectors: list_accessible_connectors_from_mcp_tools(config).await?,
        codex_apps_ready: false,
    })
}

async fn tool_suggest_connector_ids(config: &Config) -> HashSet<String> {
    let plugins_input = config.plugins_config_input();
    let mut connector_ids = PluginsManager::new(config.codex_home.to_path_buf())
        .plugins_for_config(&plugins_input)
        .await
        .capability_summaries()
        .iter()
        .flat_map(|plugin| plugin.app_connector_ids.iter())
        .map(|connector_id| connector_id.0.clone())
        .collect::<HashSet<_>>();
    connector_ids.extend(
        config
            .tool_suggest
            .discoverables
            .iter()
            .filter(|discoverable| discoverable.kind == ToolSuggestDiscoverableType::Connector)
            .map(|discoverable| discoverable.id.clone()),
    );
    let disabled_connector_ids = config
        .tool_suggest
        .disabled_tools
        .iter()
        .filter(|disabled_tool| disabled_tool.kind == ToolSuggestDiscoverableType::Connector)
        .map(|disabled_tool| disabled_tool.id.as_str())
        .collect::<HashSet<_>>();
    connector_ids.retain(|connector_id| !disabled_connector_ids.contains(connector_id.as_str()));
    connector_ids
}

async fn list_directory_connectors_for_tool_suggest_with_auth(
    config: &Config,
    auth: Option<&CodexAuth>,
) -> anyhow::Result<Vec<AppInfo>> {
    if !config.features.enabled(Feature::Apps) {
        return Ok(Vec::new());
    }

    let loaded_auth;
    let auth = if let Some(auth) = auth {
        Some(auth)
    } else {
        let auth_manager =
            AuthManager::shared_from_config(config, /*enable_codex_api_key_env*/ false).await;
        loaded_auth = auth_manager.auth().await;
        loaded_auth.as_ref()
    };
    let Some(auth) = auth.filter(|auth| auth.uses_codex_backend()) else {
        return Ok(Vec::new());
    };

    let account_id = match auth.get_account_id() {
        Some(account_id) if !account_id.is_empty() => account_id,
        _ => return Ok(Vec::new()),
    };
    let auth_provider = codex_model_provider::auth_provider_from_auth(auth);
    let is_workspace_account = auth.is_workspace_account();
    let cache_key = AllConnectorsCacheKey::new(
        config.chatgpt_base_url.clone(),
        Some(account_id.clone()),
        auth.get_chatgpt_user_id(),
        is_workspace_account,
    );

    codex_connectors::list_all_connectors_with_options(
        cache_key,
        is_workspace_account,
        /*force_refetch*/ false,
        |path| {
            let auth_provider = auth_provider.clone();
            async move {
                chatgpt_get_request_with_auth_provider::<DirectoryListResponse>(
                    config,
                    path,
                    auth_provider,
                )
                .await
            }
        },
    )
    .await
}

async fn chatgpt_get_request_with_auth_provider<T: DeserializeOwned>(
    config: &Config,
    path: String,
    auth_provider: SharedAuthProvider,
) -> anyhow::Result<T> {
    let client = create_client();
    let url = format!("{}{}", config.chatgpt_base_url, path);
    let response = client
        .get(&url)
        .headers(auth_provider.to_auth_headers())
        .header("Content-Type", "application/json")
        .timeout(DIRECTORY_CONNECTORS_TIMEOUT)
        .send()
        .await
        .context("failed to send request")?;

    if response.status().is_success() {
        response
            .json()
            .await
            .context("failed to parse JSON response")
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("request failed with status {status}: {body}");
    }
}

pub fn with_app_enabled_state(mut connectors: Vec<AppInfo>, config: &Config) -> Vec<AppInfo> {
    let user_apps_config = read_user_apps_config(config);
    let requirements_apps_config = config.config_layer_stack.requirements_toml().apps.as_ref();
    if user_apps_config.is_none() && requirements_apps_config.is_none() {
        return connectors;
    }

    for connector in &mut connectors {
        if let Some(apps_config) = user_apps_config.as_ref()
            && (apps_config.default.is_some()
                || apps_config.apps.contains_key(connector.id.as_str()))
        {
            connector.is_enabled = app_is_enabled(apps_config, Some(connector.id.as_str()));
        }

        if requirements_apps_config
            .and_then(|apps| apps.apps.get(connector.id.as_str()))
            .is_some_and(|app| app.enabled == Some(false))
        {
            connector.is_enabled = false;
        }
    }

    connectors
}

fn read_apps_config(config: &Config) -> Option<AppsConfigToml> {
    let apps_config = read_user_apps_config(config);
    let had_apps_config = apps_config.is_some();
    let mut apps_config = apps_config.unwrap_or_default();
    apply_requirements_apps_constraints(
        &mut apps_config,
        config.config_layer_stack.requirements_toml().apps.as_ref(),
    );
    if had_apps_config || apps_config.default.is_some() || !apps_config.apps.is_empty() {
        Some(apps_config)
    } else {
        None
    }
}

fn read_user_apps_config(config: &Config) -> Option<AppsConfigToml> {
    config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .and_then(|value| AppsConfigToml::deserialize(value).ok())
}

fn apply_requirements_apps_constraints(
    apps_config: &mut AppsConfigToml,
    requirements_apps_config: Option<&AppsRequirementsToml>,
) {
    let Some(requirements_apps_config) = requirements_apps_config else {
        return;
    };

    for (app_id, requirement) in &requirements_apps_config.apps {
        if requirement.enabled != Some(false) {
            continue;
        }
        let app = apps_config.apps.entry(app_id.clone()).or_default();
        app.enabled = false;
    }
}

fn app_is_enabled(apps_config: &AppsConfigToml, connector_id: Option<&str>) -> bool {
    let default_enabled = apps_config
        .default
        .as_ref()
        .map(|defaults| defaults.enabled)
        .unwrap_or(true);

    connector_id
        .and_then(|connector_id| apps_config.apps.get(connector_id))
        .map(|app| app.enabled)
        .unwrap_or(default_enabled)
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
