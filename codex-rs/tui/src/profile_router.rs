use codex_app_server_protocol::CodexErrorInfo as AppServerCodexErrorInfo;
#[cfg(test)]
use codex_protocol::protocol::CodexErrorInfo;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::PathBuf;

pub(crate) const PROFILE_ROUTER_STATE_RELATIVE_PATH: &str = "accounts/profile-router.json";
const PROFILE_ROUTER_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileFallbackAction {
    RetrySameProfileFirst,
    SwitchProfileImmediately,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileRouteEntry {
    pub(crate) profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileRouterState {
    pub(crate) version: u32,
    pub(crate) active_profile_id: Option<String>,
    pub(crate) routes: Vec<ProfileRouteEntry>,
}

impl Default for ProfileRouterState {
    fn default() -> Self {
        Self {
            version: PROFILE_ROUTER_STATE_VERSION,
            active_profile_id: None,
            routes: Vec::new(),
        }
    }
}

impl ProfileRouterState {
    pub(crate) fn contains_profile(&self, profile_id: &str) -> bool {
        self.routes
            .iter()
            .any(|route| route.profile_id == profile_id)
    }

    pub(crate) fn set_runtime_active_profile(&mut self, profile_id: Option<&str>) -> bool {
        let next = profile_id
            .filter(|profile_id| self.contains_profile(profile_id))
            .map(ToOwned::to_owned);
        if self.active_profile_id == next {
            false
        } else {
            self.active_profile_id = next;
            true
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DefaultProfileRouter;

impl DefaultProfileRouter {
    pub(crate) fn next_profile(
        &self,
        state: &ProfileRouterState,
        active_profile_id: Option<&str>,
    ) -> Option<String> {
        let first = state.routes.first()?;
        let Some(active_profile_id) = active_profile_id else {
            return Some(first.profile_id.clone());
        };

        let active_index = state
            .routes
            .iter()
            .position(|route| route.profile_id == active_profile_id);
        match active_index {
            Some(index) => {
                let next_index = (index + 1) % state.routes.len();
                Some(state.routes[next_index].profile_id.clone())
            }
            None => Some(first.profile_id.clone()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileRouterStore {
    codex_home: PathBuf,
}

impl ProfileRouterStore {
    pub(crate) fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    fn path(&self) -> PathBuf {
        self.codex_home.join(PROFILE_ROUTER_STATE_RELATIVE_PATH)
    }

    pub(crate) fn load(&self) -> io::Result<ProfileRouterState> {
        let path = self.path();
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(io::Error::other),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(ProfileRouterState::default()),
            Err(err) => Err(err),
        }
    }

    fn save(&self, state: &ProfileRouterState) -> io::Result<()> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
        fs::write(path, contents)
    }

    pub(crate) fn update<F>(&self, updater: F) -> io::Result<ProfileRouterState>
    where
        F: FnOnce(&mut ProfileRouterState),
    {
        let mut state = self.load()?;
        updater(&mut state);
        self.save(&state)?;
        Ok(state)
    }
}

#[cfg(test)]
pub(crate) fn core_profile_fallback_action(info: &CodexErrorInfo) -> Option<ProfileFallbackAction> {
    match info {
        CodexErrorInfo::UsageLimitExceeded | CodexErrorInfo::Unauthorized => {
            Some(ProfileFallbackAction::SwitchProfileImmediately)
        }
        CodexErrorInfo::ServerOverloaded | CodexErrorInfo::InternalServerError => {
            Some(ProfileFallbackAction::RetrySameProfileFirst)
        }
        CodexErrorInfo::HttpConnectionFailed { http_status_code }
        | CodexErrorInfo::ResponseStreamConnectionFailed { http_status_code }
        | CodexErrorInfo::ResponseStreamDisconnected { http_status_code }
        | CodexErrorInfo::ResponseTooManyFailedAttempts { http_status_code } => {
            match http_status_code {
                Some(401 | 403 | 429) => Some(ProfileFallbackAction::SwitchProfileImmediately),
                Some(500..=599) | None => Some(ProfileFallbackAction::RetrySameProfileFirst),
                Some(_) => None,
            }
        }
        CodexErrorInfo::ContextWindowExceeded
        | CodexErrorInfo::BadRequest
        | CodexErrorInfo::CyberPolicy
        | CodexErrorInfo::SandboxError
        | CodexErrorInfo::ActiveTurnNotSteerable { .. }
        | CodexErrorInfo::ThreadRollbackFailed
        | CodexErrorInfo::Other => None,
    }
}

pub(crate) fn app_server_profile_fallback_action(
    info: &AppServerCodexErrorInfo,
) -> Option<ProfileFallbackAction> {
    match info {
        AppServerCodexErrorInfo::UsageLimitExceeded | AppServerCodexErrorInfo::Unauthorized => {
            Some(ProfileFallbackAction::SwitchProfileImmediately)
        }
        AppServerCodexErrorInfo::ServerOverloaded
        | AppServerCodexErrorInfo::InternalServerError => {
            Some(ProfileFallbackAction::RetrySameProfileFirst)
        }
        AppServerCodexErrorInfo::HttpConnectionFailed { http_status_code }
        | AppServerCodexErrorInfo::ResponseStreamConnectionFailed { http_status_code }
        | AppServerCodexErrorInfo::ResponseStreamDisconnected { http_status_code }
        | AppServerCodexErrorInfo::ResponseTooManyFailedAttempts { http_status_code } => {
            match http_status_code {
                Some(401 | 403 | 429) => Some(ProfileFallbackAction::SwitchProfileImmediately),
                Some(500..=599) | None => Some(ProfileFallbackAction::RetrySameProfileFirst),
                Some(_) => None,
            }
        }
        AppServerCodexErrorInfo::ContextWindowExceeded
        | AppServerCodexErrorInfo::BadRequest
        | AppServerCodexErrorInfo::ThreadRollbackFailed
        | AppServerCodexErrorInfo::SandboxError
        | AppServerCodexErrorInfo::CyberPolicy
        | AppServerCodexErrorInfo::ActiveTurnNotSteerable { .. }
        | AppServerCodexErrorInfo::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::DefaultProfileRouter;
    use super::ProfileFallbackAction;
    use super::ProfileRouteEntry;
    use super::ProfileRouterState;
    use super::ProfileRouterStore;
    use super::app_server_profile_fallback_action;
    use super::core_profile_fallback_action;
    use codex_app_server_protocol::CodexErrorInfo as AppServerCodexErrorInfo;
    use codex_protocol::protocol::CodexErrorInfo;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn next_profile_skips_current_profile() {
        let state = ProfileRouterState {
            version: 1,
            active_profile_id: Some("primary".to_string()),
            routes: vec![
                ProfileRouteEntry {
                    profile_id: "primary".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "secondary".to_string(),
                },
            ],
        };

        let fallback =
            DefaultProfileRouter.next_profile(&state, /*active_profile_id*/ Some("primary"));

        assert_eq!(fallback, Some("secondary".to_string()));
    }

    #[test]
    fn next_profile_round_robins_in_route_order() {
        let state = ProfileRouterState {
            version: 1,
            active_profile_id: Some("secondary".to_string()),
            routes: vec![
                ProfileRouteEntry {
                    profile_id: "primary".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "secondary".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "tertiary".to_string(),
                },
            ],
        };

        assert_eq!(
            DefaultProfileRouter.next_profile(&state, Some("secondary")),
            Some("tertiary".to_string())
        );
        assert_eq!(
            DefaultProfileRouter.next_profile(&state, Some("tertiary")),
            Some("primary".to_string())
        );
        assert_eq!(
            DefaultProfileRouter.next_profile(&state, None),
            Some("primary".to_string())
        );
    }

    #[test]
    fn next_profile_keeps_single_route_available() {
        let state = ProfileRouterState {
            version: 1,
            active_profile_id: Some("primary".to_string()),
            routes: vec![ProfileRouteEntry {
                profile_id: "primary".to_string(),
            }],
        };

        assert_eq!(
            DefaultProfileRouter.next_profile(&state, Some("primary")),
            Some("primary".to_string())
        );
    }

    #[test]
    fn profile_router_store_defaults_when_file_is_missing() {
        let tempdir = TempDir::new().unwrap();
        let store = ProfileRouterStore::new(tempdir.path().to_path_buf());

        let state = store.load().unwrap();

        assert_eq!(state, ProfileRouterState::default());
    }

    #[test]
    fn unexpected_status_503_is_fallback_eligible_for_core_errors() {
        let action = core_profile_fallback_action(&CodexErrorInfo::ResponseTooManyFailedAttempts {
            http_status_code: Some(503),
        });

        assert_eq!(action, Some(ProfileFallbackAction::RetrySameProfileFirst));
    }

    #[test]
    fn unexpected_status_503_is_fallback_eligible_for_app_server_errors() {
        let action = app_server_profile_fallback_action(
            &AppServerCodexErrorInfo::ResponseTooManyFailedAttempts {
                http_status_code: Some(503),
            },
        );

        assert_eq!(action, Some(ProfileFallbackAction::RetrySameProfileFirst));
    }

    #[test]
    fn runtime_active_profile_clears_when_selected_profile_is_not_routed() {
        let mut state = ProfileRouterState {
            version: 1,
            active_profile_id: Some("secondary".to_string()),
            routes: vec![ProfileRouteEntry {
                profile_id: "secondary".to_string(),
            }],
        };

        assert!(state.set_runtime_active_profile(Some("missing")));
        assert_eq!(state.active_profile_id, None);
    }
}
