use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::path::PathBuf;

pub const PROFILE_ROUTER_STATE_RELATIVE_PATH: &str = "accounts/profile-router.json";
const PROFILE_ROUTER_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRouteEntry {
    pub profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRouterState {
    pub version: u32,
    pub active_profile_id: Option<String>,
    pub routes: Vec<ProfileRouteEntry>,
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
    pub fn contains_profile(&self, profile_id: &str) -> bool {
        self.routes
            .iter()
            .any(|route| route.profile_id == profile_id)
    }

    pub fn add_profile(&mut self, profile_id: &str) -> bool {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() || self.contains_profile(profile_id) {
            return false;
        }

        self.routes.push(ProfileRouteEntry {
            profile_id: profile_id.to_string(),
        });
        if self.active_profile_id.is_none() {
            self.active_profile_id = Some(profile_id.to_string());
        }
        true
    }

    pub fn remove_profile(&mut self, profile_id: &str) -> bool {
        let original_len = self.routes.len();
        self.routes.retain(|route| route.profile_id != profile_id);
        if self.routes.len() == original_len {
            return false;
        }

        if self.active_profile_id.as_deref() == Some(profile_id) {
            self.active_profile_id = self.routes.first().map(|route| route.profile_id.clone());
        }
        true
    }

    pub fn set_active_profile(&mut self, profile_id: &str) -> bool {
        if !self.contains_profile(profile_id) {
            return false;
        }
        let next = Some(profile_id.to_string());
        if self.active_profile_id == next {
            false
        } else {
            self.active_profile_id = next;
            true
        }
    }

    pub fn move_profile(&mut self, profile_id: &str, move_up: bool) -> bool {
        let Some(index) = self
            .routes
            .iter()
            .position(|route| route.profile_id == profile_id)
        else {
            return false;
        };

        let swap_with = if move_up {
            index.checked_sub(1)
        } else if index + 1 < self.routes.len() {
            Some(index + 1)
        } else {
            None
        };
        let Some(swap_with) = swap_with else {
            return false;
        };

        self.routes.swap(index, swap_with);
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileRoutingTrigger {
    NormalTurn,
    RetryAfterFallbackEligibleError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileRouterDecisionReason {
    KeepActiveProfile,
    SelectedPersistedActiveProfile,
    SelectedFirstRoute,
    RetryWithFallbackProfile,
    NoRouteConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteProfileRequest {
    pub trigger: ProfileRoutingTrigger,
    pub active_profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRouterDecision {
    pub profile_id: Option<String>,
    pub reason: ProfileRouterDecisionReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultProfileRouter;

impl DefaultProfileRouter {
    pub fn select_profile(
        &self,
        state: &ProfileRouterState,
        request: &RouteProfileRequest,
    ) -> ProfileRouterDecision {
        if state.routes.is_empty() {
            return ProfileRouterDecision {
                profile_id: None,
                reason: ProfileRouterDecisionReason::NoRouteConfigured,
            };
        }

        match request.trigger {
            ProfileRoutingTrigger::NormalTurn => {
                if let Some(active_profile_id) = request
                    .active_profile_id
                    .as_deref()
                    .or(state.active_profile_id.as_deref())
                    .filter(|profile_id| state.contains_profile(profile_id))
                {
                    let reason = if request.active_profile_id.as_deref() == Some(active_profile_id)
                    {
                        ProfileRouterDecisionReason::KeepActiveProfile
                    } else {
                        ProfileRouterDecisionReason::SelectedPersistedActiveProfile
                    };
                    return ProfileRouterDecision {
                        profile_id: Some(active_profile_id.to_string()),
                        reason,
                    };
                }

                ProfileRouterDecision {
                    profile_id: state.routes.first().map(|route| route.profile_id.clone()),
                    reason: ProfileRouterDecisionReason::SelectedFirstRoute,
                }
            }
            ProfileRoutingTrigger::RetryAfterFallbackEligibleError => {
                let active_profile_id = request
                    .active_profile_id
                    .as_deref()
                    .or(state.active_profile_id.as_deref());
                let fallback = state
                    .routes
                    .iter()
                    .find(|route| Some(route.profile_id.as_str()) != active_profile_id)
                    .map(|route| route.profile_id.clone());
                let has_fallback = fallback.is_some();
                ProfileRouterDecision {
                    profile_id: fallback,
                    reason: if has_fallback {
                        ProfileRouterDecisionReason::RetryWithFallbackProfile
                    } else {
                        ProfileRouterDecisionReason::NoRouteConfigured
                    },
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRouterStore {
    codex_home: PathBuf,
}

impl ProfileRouterStore {
    pub fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    pub fn path(&self) -> PathBuf {
        self.codex_home.join(PROFILE_ROUTER_STATE_RELATIVE_PATH)
    }

    pub fn load(&self) -> io::Result<ProfileRouterState> {
        let path = self.path();
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(io::Error::other),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(ProfileRouterState::default()),
            Err(err) => Err(err),
        }
    }

    pub fn save(&self, state: &ProfileRouterState) -> io::Result<()> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
        fs::write(path, contents)
    }

    pub fn update<F>(&self, updater: F) -> io::Result<ProfileRouterState>
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
mod tests {
    use pretty_assertions::assert_eq;

    use super::DefaultProfileRouter;
    use super::ProfileRouteEntry;
    use super::ProfileRouterDecision;
    use super::ProfileRouterDecisionReason;
    use super::ProfileRouterState;
    use super::ProfileRoutingTrigger;
    use super::RouteProfileRequest;

    fn state() -> ProfileRouterState {
        ProfileRouterState {
            version: 1,
            active_profile_id: Some("pig".to_string()),
            routes: vec![
                ProfileRouteEntry {
                    profile_id: "pig".to_string(),
                },
                ProfileRouteEntry {
                    profile_id: "r2".to_string(),
                },
            ],
        }
    }

    #[test]
    fn normal_turn_keeps_requested_active_profile() {
        let router = DefaultProfileRouter;
        let request = RouteProfileRequest {
            trigger: ProfileRoutingTrigger::NormalTurn,
            active_profile_id: Some("pig".to_string()),
        };

        assert_eq!(
            router.select_profile(&state(), &request),
            ProfileRouterDecision {
                profile_id: Some("pig".to_string()),
                reason: ProfileRouterDecisionReason::KeepActiveProfile,
            }
        );
    }

    #[test]
    fn retry_selects_next_profile() {
        let router = DefaultProfileRouter;
        let request = RouteProfileRequest {
            trigger: ProfileRoutingTrigger::RetryAfterFallbackEligibleError,
            active_profile_id: Some("pig".to_string()),
        };

        assert_eq!(
            router.select_profile(&state(), &request),
            ProfileRouterDecision {
                profile_id: Some("r2".to_string()),
                reason: ProfileRouterDecisionReason::RetryWithFallbackProfile,
            }
        );
    }

    #[test]
    fn remove_active_profile_promotes_first_remaining_route() {
        let mut state = state();
        assert!(state.remove_profile("pig"));
        assert_eq!(state.active_profile_id.as_deref(), Some("r2"));
    }
}
