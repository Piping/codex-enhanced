use std::collections::HashSet;
use std::time::Duration;

use toml::Value as TomlValue;

use super::App;
use super::editor_helpers::ExternalEditorErrorTarget;
use crate::app_event::AppEvent;
use crate::app_event::RuntimeProfileTarget;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::legacy_core::config::Config;
use crate::profile_router::DefaultProfileRouter;
use crate::profile_router::PROFILE_ROUTER_STATE_RELATIVE_PATH;
use crate::profile_router::ProfileFallbackAction;
use crate::profile_router::ProfileRouteEntry;
use crate::profile_router::ProfileRouterState;
use crate::profile_router::ProfileRouterStore;
use crate::tui;
use codex_protocol::ThreadId;

const PROFILE_MANAGEMENT_VIEW_ID: &str = "profile-management";
const PROFILE_FALLBACK_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);

fn profile_fallback_retry_delay(attempt: u32) -> Duration {
    if attempt <= 1 {
        return Duration::ZERO;
    }

    let exponent = attempt.saturating_sub(2).min(5);
    let seconds = 1_u64 << exponent;
    Duration::from_secs(seconds).min(PROFILE_FALLBACK_RETRY_MAX_DELAY)
}

fn profile_fallback_retry_target(
    action: ProfileFallbackAction,
    router_state: &ProfileRouterState,
    active_profile_id: Option<&str>,
    same_profile_retry_consumed: bool,
) -> Option<String> {
    if matches!(action, ProfileFallbackAction::RetrySameProfileFirst)
        && !same_profile_retry_consumed
    {
        return active_profile_id.map(ToOwned::to_owned);
    }

    DefaultProfileRouter
        .next_profile(router_state, active_profile_id)
        .or_else(|| active_profile_id.map(ToOwned::to_owned))
}

fn profile_label(profile_id: Option<&str>) -> &str {
    profile_id.unwrap_or("default")
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RoutedProfileSummary {
    id: String,
    provider_label: String,
    model: Option<String>,
    base_url: Option<String>,
    route_position: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DefaultProfileSummary {
    provider_label: String,
    model: Option<String>,
    base_url: Option<String>,
}

impl App {
    pub(super) fn profile_router_store(&self) -> ProfileRouterStore {
        ProfileRouterStore::new(self.config.codex_home.to_path_buf())
    }

    pub(super) fn routed_profile_runtime_changed(
        current_config: &Config,
        next_config: &Config,
    ) -> bool {
        current_config.active_profile != next_config.active_profile
            || current_config.model_provider_id != next_config.model_provider_id
            || current_config.model_provider != next_config.model_provider
            || current_config.chatgpt_base_url != next_config.chatgpt_base_url
    }

    pub(crate) fn open_profile_management_panel(&mut self) {
        let router_state = self.profile_router_store().load().unwrap_or_default();
        let profiles = self.routed_profile_summaries(&router_state);
        self.open_selection_popup_for_view(
            PROFILE_MANAGEMENT_VIEW_ID,
            |app, active_selected_idx| {
                profile_management_root_params(
                    app.active_profile.as_deref(),
                    &app.default_profile_summary(),
                    &profiles,
                    &router_state,
                    app.chat_widget.is_task_running(),
                    active_selected_idx,
                )
            },
        );
    }

    pub(crate) async fn edit_profile_fallback_config_from_ui(&mut self, tui: &mut tui::Tui) {
        let router_state = self.profile_router_store().load().unwrap_or_default();
        let profiles = self.routed_profile_summaries(&router_state);
        if profiles.is_empty() {
            match self
                .profile_router_store()
                .update(|state| *state = ProfileRouterState::default())
            {
                Ok(_) => {
                    self.chat_widget.add_info_message(
                        "Cleared the fallback route because no named profiles are defined."
                            .to_string(),
                        /*hint*/ None,
                    );
                    self.open_profile_management_panel();
                }
                Err(err) => {
                    self.chat_widget.add_error_message(format!(
                        "Failed to update {PROFILE_ROUTER_STATE_RELATIVE_PATH}: {err}"
                    ));
                }
            }
            return;
        }

        let seed = fallback_route_editor_seed(&profiles, &router_state);
        let Ok(contents) = self
            .edit_seed_with_external_editor(tui, ExternalEditorErrorTarget::History, &seed, ".txt")
            .await
        else {
            return;
        };
        let current_profile_ids = profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>();
        match parse_fallback_route_editor_contents(&contents, &current_profile_ids) {
            Ok(ordered_profile_ids) => {
                let next_state = rewritten_router_state(&router_state, ordered_profile_ids);
                match self
                    .profile_router_store()
                    .update(|state| *state = next_state.clone())
                {
                    Ok(_) => {
                        self.chat_widget.add_info_message(
                            format!(
                                "Updated fallback route in {PROFILE_ROUTER_STATE_RELATIVE_PATH}."
                            ),
                            /*hint*/ None,
                        );
                        self.open_profile_management_panel();
                    }
                    Err(err) => {
                        self.chat_widget.add_error_message(format!(
                            "Failed to update {PROFILE_ROUTER_STATE_RELATIVE_PATH}: {err}"
                        ));
                    }
                }
            }
            Err(err) => {
                self.chat_widget.add_error_message(err);
            }
        }
    }

    fn default_profile_summary(&self) -> DefaultProfileSummary {
        let effective_config = self.config.config_layer_stack.effective_config();
        let table = effective_config.as_table();
        let root_provider_id = table
            .and_then(|table| table.get("model_provider"))
            .and_then(TomlValue::as_str)
            .unwrap_or(self.config.model_provider_id.as_str());
        let root_model = table
            .and_then(|table| table.get("model"))
            .and_then(TomlValue::as_str)
            .map(ToOwned::to_owned);
        let (provider_label, base_url) = self.provider_label_and_base_url(root_provider_id);

        DefaultProfileSummary {
            provider_label,
            model: root_model,
            base_url,
        }
    }

    fn routed_profile_summaries(
        &self,
        router_state: &ProfileRouterState,
    ) -> Vec<RoutedProfileSummary> {
        let effective_config = self.config.config_layer_stack.effective_config();
        let Some(table) = effective_config.as_table() else {
            return Vec::new();
        };
        let root_model = table
            .get("model")
            .and_then(TomlValue::as_str)
            .map(ToOwned::to_owned);
        let root_provider_id = table
            .get("model_provider")
            .and_then(TomlValue::as_str)
            .unwrap_or(self.config.model_provider_id.as_str());
        let Some(profiles) = table.get("profiles").and_then(TomlValue::as_table) else {
            return Vec::new();
        };

        let mut profile_ids = profiles.keys().cloned().collect::<Vec<_>>();
        profile_ids.sort();

        profile_ids
            .into_iter()
            .map(|id| {
                let profile = profiles.get(&id).and_then(TomlValue::as_table);
                let provider_id = profile
                    .and_then(|profile| profile.get("model_provider"))
                    .and_then(TomlValue::as_str)
                    .unwrap_or(root_provider_id);
                let (provider_label, base_url) = self.provider_label_and_base_url(provider_id);
                RoutedProfileSummary {
                    route_position: router_state
                        .routes
                        .iter()
                        .position(|route| route.profile_id == id)
                        .map(|index| index + 1),
                    model: profile
                        .and_then(|profile| profile.get("model"))
                        .and_then(TomlValue::as_str)
                        .map(ToOwned::to_owned)
                        .or_else(|| root_model.clone()),
                    id,
                    provider_label,
                    base_url,
                }
            })
            .collect()
    }

    fn provider_label_and_base_url(&self, provider_id: &str) -> (String, Option<String>) {
        if provider_id == self.config.model_provider_id {
            (
                self.config.model_provider.name.clone(),
                self.config.model_provider.base_url.clone(),
            )
        } else if let Some(provider) = self.config.model_providers.get(provider_id) {
            (provider.name.clone(), provider.base_url.clone())
        } else {
            (provider_id.to_string(), None)
        }
    }

    pub(super) async fn close_active_thread_for_profile_reload(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
    ) -> std::result::Result<(), String> {
        self.backtrack.pending_rollback = None;
        app_server
            .thread_unsubscribe(thread_id)
            .await
            .map_err(|err| {
                format!("Failed to unload current session before switching profiles: {err}")
            })?;
        self.clear_active_thread().await;
        self.abort_thread_event_listener(thread_id);
        Ok(())
    }

    pub(super) async fn switch_runtime_profile(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        profile_id: Option<&str>,
    ) -> std::result::Result<(), String> {
        if self.active_profile.as_deref() == profile_id {
            return Ok(());
        }

        let previous_override = self.harness_overrides.config_profile.clone();
        let previous_active_profile = self.active_profile.clone();
        self.harness_overrides.config_profile = profile_id.map(ToOwned::to_owned);

        let current_cwd = self.chat_widget.config_ref().cwd.to_path_buf();
        let mut next_config = match self.rebuild_config_for_cwd(current_cwd).await {
            Ok(config) => config,
            Err(err) => {
                self.harness_overrides.config_profile = previous_override;
                self.active_profile = previous_active_profile;
                return Err(err.to_string());
            }
        };
        self.apply_runtime_policy_overrides(&mut next_config);

        if let Err(err) = self
            .apply_runtime_config_change(
                tui,
                app_server,
                next_config,
                /*reload_live_thread*/ true,
            )
            .await
        {
            self.harness_overrides.config_profile = previous_override;
            self.active_profile = previous_active_profile;
            return Err(err);
        }
        Ok(())
    }

    pub(super) async fn retry_last_user_turn_with_profile_fallback(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        action: ProfileFallbackAction,
        error_message: String,
    ) {
        if !self.chat_widget.has_retryable_user_turn() {
            self.chat_widget.add_error_message(error_message);
            return;
        }

        let generation = self.chat_widget.profile_retry_generation();
        let attempt = self
            .chat_widget
            .profile_retry_attempt_count()
            .saturating_add(1);
        let router_state = self.profile_router_store().load().unwrap_or_default();
        let profile_id = profile_fallback_retry_target(
            action,
            &router_state,
            self.active_profile.as_deref(),
            self.chat_widget.profile_retry_attempted(),
        );
        let target_label = profile_label(profile_id.as_deref()).to_string();
        let history_message = format!("Retrying the last turn with profile `{target_label}`.");
        let delay = profile_fallback_retry_delay(attempt);

        self.chat_widget.finish_failed_turn_for_profile_fallback();

        if delay.is_zero() {
            self.execute_profile_fallback_retry(
                tui,
                app_server,
                generation,
                profile_id,
                history_message,
            )
            .await;
            return;
        }

        self.chat_widget.add_info_message(
            format!(
                "{error_message} Retrying with profile `{target_label}` in {}s.",
                delay.as_secs()
            ),
            /*hint*/ None,
        );
        let app_event_tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            app_event_tx.send(AppEvent::ExecuteProfileFallbackRetry {
                generation,
                profile_id,
                history_message,
            });
        });
    }

    pub(super) async fn execute_profile_fallback_retry(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        generation: u64,
        profile_id: Option<String>,
        history_message: String,
    ) {
        if !self.chat_widget.has_retryable_user_turn()
            || self.chat_widget.profile_retry_generation() != generation
        {
            return;
        }

        if self.active_profile != profile_id {
            if let Err(err) = self
                .switch_runtime_profile(tui, app_server, profile_id.as_deref())
                .await
            {
                let profile_label = profile_label(profile_id.as_deref());
                self.chat_widget.add_error_message(format!(
                    "Failed to switch to fallback profile `{profile_label}`: {err}"
                ));
                return;
            }

            if let Err(err) = self.profile_router_store().update(|state| {
                state.set_runtime_active_profile(profile_id.as_deref());
            }) {
                let profile_label = profile_label(profile_id.as_deref());
                self.chat_widget.add_error_message(format!(
                    "Switched to fallback profile `{profile_label}`, but failed to persist {PROFILE_ROUTER_STATE_RELATIVE_PATH}: {err}"
                ));
            }
        }

        self.chat_widget
            .submit_profile_fallback_retry(history_message);
    }
}

fn profile_management_root_params(
    active_profile: Option<&str>,
    default_profile: &DefaultProfileSummary,
    profiles: &[RoutedProfileSummary],
    router_state: &ProfileRouterState,
    task_running: bool,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    let mut items = vec![
        profile_selection_item(
            "Default Config".to_string(),
            default_profile_description(default_profile),
            active_profile.is_none(),
            task_running,
            RuntimeProfileTarget::Default,
        ),
        SelectionItem {
            name: "Fallback Config".to_string(),
            description: Some(root_fallback_summary(router_state)),
            selected_description: Some(
                "Open your external editor and reorder all named profiles. Saving rewrites the fallback route file from scratch."
                    .to_string(),
            ),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::EditProfileFallbackConfig);
            })],
            dismiss_on_select: false,
            search_value: Some("fallback config route reorder edit".to_string()),
            ..Default::default()
        },
    ];

    if profiles.is_empty() {
        items.push(SelectionItem {
            name: "No named profiles".to_string(),
            description: Some(
                "Add `[profiles.<name>]` entries in config.toml to route API traffic through alternate endpoints."
                    .to_string(),
            ),
            is_disabled: true,
            ..Default::default()
        });
    } else {
        items.extend(profiles.iter().cloned().map(|profile| {
            profile_selection_item(
                profile.id.clone(),
                routed_profile_description(&profile),
                active_profile == Some(profile.id.as_str()),
                task_running,
                RuntimeProfileTarget::Named(profile.id),
            )
        }));
    }

    SelectionViewParams {
        view_id: Some(PROFILE_MANAGEMENT_VIEW_ID),
        title: Some("Profiles".to_string()),
        subtitle: Some(format!(
            "Current runtime: {} · {} named profile(s).",
            active_profile.unwrap_or("default"),
            profiles.len(),
        )),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search profiles".to_string()),
        initial_selected_idx,
        ..Default::default()
    }
}

fn profile_selection_item(
    name: String,
    description: String,
    is_current: bool,
    task_running: bool,
    target: RuntimeProfileTarget,
) -> SelectionItem {
    let (is_disabled, disabled_reason) = if is_current {
        (true, Some("Already active.".to_string()))
    } else if task_running {
        (
            true,
            Some("Wait for the current task to finish before switching profiles.".to_string()),
        )
    } else {
        (false, None)
    };

    SelectionItem {
        name: name.clone(),
        description: Some(description.clone()),
        selected_description: Some(
            "Reload the current session with this profile while preserving input continuity."
                .to_string(),
        ),
        is_current,
        is_disabled,
        disabled_reason,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::SwitchRuntimeProfile {
                target: target.clone(),
            });
        })],
        dismiss_on_select: true,
        search_value: Some(format!("{name} {description}")),
        ..Default::default()
    }
}

fn default_profile_description(profile: &DefaultProfileSummary) -> String {
    let mut parts = vec![format!("provider: {}", profile.provider_label)];
    if let Some(base_url) = &profile.base_url {
        parts.push(base_url.clone());
    }
    if let Some(model) = &profile.model {
        parts.push(format!("model: {model}"));
    }
    parts.push("root config".to_string());
    parts.join(" · ")
}

fn routed_profile_description(profile: &RoutedProfileSummary) -> String {
    let mut parts = vec![profile_endpoint_description(profile)];
    parts.push(
        profile
            .route_position
            .map(|position| format!("fallback #{position}"))
            .unwrap_or_else(|| "not in fallback route".to_string()),
    );
    parts.join(" · ")
}

fn profile_endpoint_description(profile: &RoutedProfileSummary) -> String {
    let mut parts = vec![format!("provider: {}", profile.provider_label)];
    if let Some(base_url) = &profile.base_url {
        parts.push(base_url.clone());
    }
    if let Some(model) = &profile.model {
        parts.push(format!("model: {model}"));
    }
    parts.join(" · ")
}

fn root_fallback_summary(router_state: &ProfileRouterState) -> String {
    if router_state.routes.is_empty() {
        "No profiles in the fallback route.".to_string()
    } else {
        format!(
            "{} profile(s) in route · active fallback: {}",
            router_state.routes.len(),
            router_state.active_profile_id.as_deref().unwrap_or("none")
        )
    }
}

fn fallback_route_editor_seed(
    profiles: &[RoutedProfileSummary],
    router_state: &ProfileRouterState,
) -> String {
    let mut ordered_ids = Vec::with_capacity(profiles.len());
    let current_profile_ids = profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect::<HashSet<_>>();

    for route in &router_state.routes {
        let profile_id = route.profile_id.as_str();
        if current_profile_ids.contains(profile_id)
            && !ordered_ids.iter().any(|id| id == profile_id)
        {
            ordered_ids.push(profile_id.to_string());
        }
    }
    for profile in profiles {
        if !ordered_ids.iter().any(|id| id == &profile.id) {
            ordered_ids.push(profile.id.clone());
        }
    }

    let mut seed = [
        "# Reorder fallback profiles, one id per line.",
        "# Omitted profiles are allowed.",
        "# Keep only profiles currently defined in config.toml, at most once each.",
        "# Blank lines and lines starting with # are ignored.",
    ]
    .join("\n");
    seed.push_str("\n\n");
    seed.push_str(&ordered_ids.join("\n"));
    seed.push('\n');
    seed
}

fn parse_fallback_route_editor_contents(
    contents: &str,
    current_profile_ids: &[String],
) -> Result<Vec<String>, String> {
    let expected_ids = current_profile_ids
        .iter()
        .map(std::string::String::as_str)
        .collect::<HashSet<_>>();
    let mut seen_ids = HashSet::with_capacity(current_profile_ids.len());
    let mut ordered_ids = Vec::with_capacity(current_profile_ids.len());

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !expected_ids.contains(line) {
            return Err(format!(
                "Unknown profile `{line}` in fallback config. Keep only profiles currently defined in config.toml."
            ));
        }
        if !seen_ids.insert(line.to_string()) {
            return Err(format!(
                "Duplicate profile `{line}` in fallback config. Each profile must appear exactly once."
            ));
        }
        ordered_ids.push(line.to_string());
    }

    Ok(ordered_ids)
}

fn rewritten_router_state(
    previous_state: &ProfileRouterState,
    ordered_profile_ids: Vec<String>,
) -> ProfileRouterState {
    let active_profile_id = previous_state
        .active_profile_id
        .as_ref()
        .filter(|profile_id| ordered_profile_ids.iter().any(|id| id == *profile_id))
        .cloned();

    ProfileRouterState {
        active_profile_id,
        routes: ordered_profile_ids
            .into_iter()
            .map(|profile_id| ProfileRouteEntry { profile_id })
            .collect(),
        ..ProfileRouterState::default()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    use super::DefaultProfileSummary;
    use super::RoutedProfileSummary;
    use super::fallback_route_editor_seed;
    use super::parse_fallback_route_editor_contents;
    use super::profile_fallback_retry_delay;
    use super::profile_fallback_retry_target;
    use super::profile_management_root_params;
    use super::rewritten_router_state;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
    use crate::profile_router::ProfileFallbackAction;
    use crate::profile_router::ProfileRouteEntry;
    use crate::profile_router::ProfileRouterState;
    use crate::render::renderable::Renderable;

    fn render_selection_popup(view: &ListSelectionView, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, width, height);
                view.render(area, frame.buffer_mut());
            })
            .expect("draw popup");
        format!("{:?}", terminal.backend())
    }

    fn test_profiles() -> Vec<RoutedProfileSummary> {
        vec![
            RoutedProfileSummary {
                id: "primary".to_string(),
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: Some("https://api.primary.example/v1".to_string()),
                route_position: Some(1),
            },
            RoutedProfileSummary {
                id: "secondary".to_string(),
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: Some("https://api.secondary.example/v1".to_string()),
                route_position: Some(2),
            },
            RoutedProfileSummary {
                id: "tertiary".to_string(),
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: Some("https://api.tertiary.example/v1".to_string()),
                route_position: None,
            },
        ]
    }

    fn test_router_state() -> ProfileRouterState {
        ProfileRouterState {
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
        }
    }

    #[test]
    fn profile_management_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            profile_management_root_params(
                Some("primary"),
                &DefaultProfileSummary {
                    provider_label: "OpenAI".to_string(),
                    model: Some("gpt-5".to_string()),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                },
                &test_profiles(),
                &test_router_state(),
                /*task_running*/ false,
                /*initial_selected_idx*/ None,
            ),
            tx,
        );

        assert_snapshot!(
            "profile_management_popup",
            render_selection_popup(&view, /*width*/ 96, /*height*/ 22)
        );
    }

    #[test]
    fn fallback_route_editor_seed_uses_current_profiles_only() {
        let seed = fallback_route_editor_seed(&test_profiles(), &test_router_state());

        assert_eq!(
            seed,
            concat!(
                "# Reorder fallback profiles, one id per line.\n",
                "# Omitted profiles are allowed.\n",
                "# Keep only profiles currently defined in config.toml, at most once each.\n",
                "# Blank lines and lines starting with # are ignored.\n",
                "\n",
                "primary\n",
                "secondary\n",
                "tertiary\n"
            )
        );
    }

    #[test]
    fn fallback_route_parser_accepts_partial_unique_profile_list() {
        let current_profile_ids = vec![
            "primary".to_string(),
            "secondary".to_string(),
            "tertiary".to_string(),
        ];

        let ordered = parse_fallback_route_editor_contents(
            "# comment\nsecondary\nprimary\ntertiary\n",
            &current_profile_ids,
        )
        .expect("valid fallback route");
        assert_eq!(
            ordered,
            vec![
                "secondary".to_string(),
                "primary".to_string(),
                "tertiary".to_string()
            ]
        );

        let partial =
            parse_fallback_route_editor_contents("secondary\nprimary\n", &current_profile_ids)
                .expect("omitted profiles should be allowed");
        assert_eq!(
            partial,
            vec!["secondary".to_string(), "primary".to_string()]
        );

        let duplicate = parse_fallback_route_editor_contents(
            "secondary\nprimary\nprimary\ntertiary\n",
            &current_profile_ids,
        )
        .expect_err("duplicate profile should fail");
        assert_eq!(
            duplicate,
            "Duplicate profile `primary` in fallback config. Each profile must appear exactly once."
        );

        let unknown = parse_fallback_route_editor_contents(
            "secondary\nunknown\nprimary\ntertiary\n",
            &current_profile_ids,
        )
        .expect_err("unknown profile should fail");
        assert_eq!(
            unknown,
            "Unknown profile `unknown` in fallback config. Keep only profiles currently defined in config.toml."
        );
    }

    #[test]
    fn profile_fallback_retry_delay_uses_exponential_backoff_with_cap() {
        let expected = [
            Duration::ZERO,
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(16),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ];

        for (attempt, delay) in expected.into_iter().enumerate() {
            assert_eq!(profile_fallback_retry_delay(attempt as u32), delay);
        }
    }

    #[test]
    fn retry_same_profile_first_only_rotates_after_same_profile_retry_is_consumed() {
        let router_state = test_router_state();

        assert_eq!(
            profile_fallback_retry_target(
                ProfileFallbackAction::RetrySameProfileFirst,
                &router_state,
                Some("primary"),
                /*same_profile_retry_consumed*/ false,
            ),
            Some("primary".to_string())
        );
        assert_eq!(
            profile_fallback_retry_target(
                ProfileFallbackAction::RetrySameProfileFirst,
                &router_state,
                Some("primary"),
                /*same_profile_retry_consumed*/ true,
            ),
            Some("secondary".to_string())
        );
    }

    #[test]
    fn switch_profile_immediately_uses_next_profile_in_route_order() {
        let router_state = test_router_state();

        assert_eq!(
            profile_fallback_retry_target(
                ProfileFallbackAction::SwitchProfileImmediately,
                &router_state,
                Some("primary"),
                /*same_profile_retry_consumed*/ false,
            ),
            Some("secondary".to_string())
        );
        assert_eq!(
            profile_fallback_retry_target(
                ProfileFallbackAction::SwitchProfileImmediately,
                &router_state,
                None,
                /*same_profile_retry_consumed*/ false,
            ),
            Some("primary".to_string())
        );
    }

    #[test]
    fn rewritten_router_state_drops_stale_entries_and_preserves_active_profile() {
        let state = rewritten_router_state(
            &ProfileRouterState {
                version: 1,
                active_profile_id: Some("secondary".to_string()),
                routes: vec![
                    ProfileRouteEntry {
                        profile_id: "stale".to_string(),
                    },
                    ProfileRouteEntry {
                        profile_id: "primary".to_string(),
                    },
                ],
            },
            vec![
                "tertiary".to_string(),
                "secondary".to_string(),
                "primary".to_string(),
            ],
        );

        assert_eq!(
            state,
            ProfileRouterState {
                version: 1,
                active_profile_id: Some("secondary".to_string()),
                routes: vec![
                    ProfileRouteEntry {
                        profile_id: "tertiary".to_string(),
                    },
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

    #[test]
    fn rewritten_router_state_clears_missing_active_profile() {
        let state = rewritten_router_state(
            &ProfileRouterState {
                version: 1,
                active_profile_id: Some("stale".to_string()),
                routes: Vec::new(),
            },
            vec!["primary".to_string()],
        );

        assert_eq!(
            state,
            ProfileRouterState {
                version: 1,
                active_profile_id: None,
                routes: vec![ProfileRouteEntry {
                    profile_id: "primary".to_string(),
                }],
            }
        );
    }

    #[test]
    fn profile_management_popup_fallback_item_opens_editor_flow() {
        let params = profile_management_root_params(
            Some("primary"),
            &DefaultProfileSummary {
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: Some("https://api.openai.com/v1".to_string()),
            },
            &test_profiles(),
            &test_router_state(),
            /*task_running*/ false,
            /*initial_selected_idx*/ None,
        );

        assert_eq!(params.items[1].name, "Fallback Config");
        assert_eq!(
            params.items[1].selected_description.as_deref(),
            Some(
                "Open your external editor and reorder all named profiles. Saving rewrites the fallback route file from scratch."
            )
        );
        assert_eq!(params.items[1].dismiss_on_select, false);
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        (params.items[1].actions[0])(&AppEventSender::new(tx));
        assert!(matches!(
            rx.try_recv().ok(),
            Some(AppEvent::EditProfileFallbackConfig)
        ));
    }

    #[test]
    fn profile_management_popup_shows_no_named_profiles() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            profile_management_root_params(
                None,
                &DefaultProfileSummary {
                    provider_label: "OpenAI".to_string(),
                    model: Some("gpt-5".to_string()),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                },
                &[],
                &ProfileRouterState::default(),
                /*task_running*/ false,
                /*initial_selected_idx*/ None,
            ),
            tx,
        );

        assert_snapshot!(
            "profile_management_popup_no_named_profiles",
            render_selection_popup(&view, /*width*/ 96, /*height*/ 20)
        );
    }

    #[test]
    fn profile_management_popup_fallback_item_stays_enabled_while_task_running() {
        let params = profile_management_root_params(
            Some("primary"),
            &DefaultProfileSummary {
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: None,
            },
            &[RoutedProfileSummary {
                id: "primary".to_string(),
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: None,
                route_position: Some(1),
            }],
            &ProfileRouterState {
                version: 1,
                active_profile_id: Some("primary".to_string()),
                routes: vec![ProfileRouteEntry {
                    profile_id: "primary".to_string(),
                }],
            },
            /*task_running*/ true,
            /*initial_selected_idx*/ None,
        );

        assert_eq!(params.items[1].is_disabled, false);
    }

    #[test]
    fn profile_management_panel_disables_switches_while_task_running() {
        let params = profile_management_root_params(
            Some("primary"),
            &DefaultProfileSummary {
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: None,
            },
            &[RoutedProfileSummary {
                id: "primary".to_string(),
                provider_label: "OpenAI".to_string(),
                model: Some("gpt-5".to_string()),
                base_url: None,
                route_position: Some(1),
            }],
            &ProfileRouterState {
                version: 1,
                active_profile_id: Some("primary".to_string()),
                routes: vec![ProfileRouteEntry {
                    profile_id: "primary".to_string(),
                }],
            },
            /*task_running*/ true,
            /*initial_selected_idx*/ None,
        );

        assert_eq!(params.items[0].is_disabled, true);
        assert_eq!(params.items[1].is_disabled, false);
        assert_eq!(params.items[2].is_disabled, true);
        assert_eq!(
            params.items[0].disabled_reason.as_deref(),
            Some("Wait for the current task to finish before switching profiles.")
        );
        assert_eq!(
            params.items[2].disabled_reason.as_deref(),
            Some("Already active.")
        );
    }
}
