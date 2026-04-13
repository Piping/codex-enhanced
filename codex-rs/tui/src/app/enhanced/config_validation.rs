use std::collections::HashSet;
use std::io;

use codex_core::config::Config;
use toml::Value as TomlValue;

use super::App;
use crate::profile_router::PROFILE_ROUTER_STATE_RELATIVE_PATH;
use crate::profile_router::ProfileRouteEntry;
use crate::profile_router::ProfileRouterState;
use crate::profile_router::ProfileRouterStore;

const CONFIG_PROFILE_NOT_FOUND_PREFIX: &str = "config profile `";
const CONFIG_PROFILE_NOT_FOUND_SUFFIX: &str = "` not found";

pub(super) fn missing_config_profile_name(error: &io::Error) -> Option<String> {
    if error.kind() != io::ErrorKind::NotFound {
        return None;
    }

    let message = error.to_string();
    message
        .strip_prefix(CONFIG_PROFILE_NOT_FOUND_PREFIX)?
        .strip_suffix(CONFIG_PROFILE_NOT_FOUND_SUFFIX)
        .map(ToOwned::to_owned)
}

fn current_profile_ids(config: &Config) -> HashSet<String> {
    config
        .config_layer_stack
        .effective_config()
        .get("profiles")
        .and_then(TomlValue::as_table)
        .map(|profiles| profiles.keys().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn sanitized_profile_router_state(
    router_state: &ProfileRouterState,
    config: &Config,
) -> ProfileRouterState {
    let valid_profile_ids = current_profile_ids(config);
    let mut seen_profile_ids = HashSet::with_capacity(router_state.routes.len());
    let routes = router_state
        .routes
        .iter()
        .filter_map(|route| {
            let profile_id = route.profile_id.clone();
            (valid_profile_ids.contains(&profile_id) && seen_profile_ids.insert(profile_id.clone()))
                .then_some(ProfileRouteEntry { profile_id })
        })
        .collect::<Vec<_>>();
    let active_profile_id = router_state
        .active_profile_id
        .as_ref()
        .filter(|profile_id| routes.iter().any(|route| route.profile_id == **profile_id))
        .cloned();

    ProfileRouterState {
        active_profile_id,
        routes,
        ..ProfileRouterState::default()
    }
}

fn load_profile_router_state(store: &ProfileRouterStore) -> ProfileRouterState {
    match store.load() {
        Ok(state) => state,
        Err(err) => {
            tracing::warn!(
                error = %err,
                path = PROFILE_ROUTER_STATE_RELATIVE_PATH,
                "failed to load profile router state; using defaults"
            );
            ProfileRouterState::default()
        }
    }
}

impl App {
    pub(super) fn load_profile_router_state(&self) -> ProfileRouterState {
        sanitized_profile_router_state(
            &load_profile_router_state(&self.profile_router_store()),
            &self.config,
        )
    }

    pub(super) fn validate_config_dependent_state(&self, config: &Config) {
        let store = ProfileRouterStore::new(config.codex_home.clone());
        let current_state = load_profile_router_state(&store);
        let sanitized_state = sanitized_profile_router_state(&current_state, config);
        if sanitized_state == current_state {
            return;
        }

        if let Err(err) = store.replace(&sanitized_state) {
            tracing::warn!(
                error = %err,
                path = PROFILE_ROUTER_STATE_RELATIVE_PATH,
                "failed to persist sanitized profile router state after config refresh"
            );
            return;
        }

        tracing::info!(
            path = PROFILE_ROUTER_STATE_RELATIVE_PATH,
            "removed stale config-dependent profile router entries after config refresh"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::missing_config_profile_name;
    use super::sanitized_profile_router_state;
    use crate::profile_router::ProfileRouteEntry;
    use crate::profile_router::ProfileRouterState;
    use codex_core::config::ConfigBuilder;
    use pretty_assertions::assert_eq;
    use std::io;
    use tempfile::tempdir;

    #[test]
    fn missing_config_profile_name_extracts_profile_id() {
        let error = io::Error::new(io::ErrorKind::NotFound, "config profile `pig` not found");

        assert_eq!(missing_config_profile_name(&error), Some("pig".to_string()));
    }

    #[tokio::test]
    async fn sanitized_profile_router_state_drops_orphans_and_duplicates() {
        let codex_home = tempdir().expect("tempdir");
        std::fs::write(
            codex_home.path().join("config.toml"),
            r#"
[profiles.primary]
model = "gpt-5"

[profiles.secondary]
model = "gpt-5"
"#,
        )
        .expect("write config");
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("load config");
        let router_state = ProfileRouterState {
            version: 1,
            active_profile_id: Some("orphan".to_string()),
            routes: vec![
                ProfileRouteEntry {
                    profile_id: "secondary".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "secondary".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "orphan".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "primary".to_string(),
                },
            ],
        };

        assert_eq!(
            sanitized_profile_router_state(&router_state, &config),
            ProfileRouterState {
                version: 1,
                active_profile_id: None,
                routes: vec![
                    ProfileRouteEntry {
                        profile_id: "secondary".to_string(),
                    },
                    ProfileRouteEntry {
                        profile_id: "primary".to_string(),
                    },
                ],
            }
        );
    }
}
