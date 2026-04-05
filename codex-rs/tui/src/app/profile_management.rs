use toml::Value as TomlValue;

use super::App;
use crate::app_event::AppEvent;
use crate::app_event::RuntimeProfileTarget;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::profile_router::ProfileRouterState;

const PROFILE_MANAGEMENT_VIEW_ID: &str = "profile-management";

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
    pub(crate) fn open_profile_management_panel(&mut self) {
        let router_state = self.profile_router_store().load().unwrap_or_default();
        let initial_selected_idx = self
            .chat_widget
            .selected_index_for_active_view(PROFILE_MANAGEMENT_VIEW_ID);
        let params = profile_management_panel_params(
            self.active_profile.as_deref(),
            &self.default_profile_summary(),
            &self.routed_profile_summaries(&router_state),
            router_state.routes.len(),
            self.chat_widget.is_task_running(),
            initial_selected_idx,
        );
        if !self
            .chat_widget
            .replace_selection_view_if_active(PROFILE_MANAGEMENT_VIEW_ID, params)
        {
            self.chat_widget
                .show_selection_view(profile_management_panel_params(
                    self.active_profile.as_deref(),
                    &self.default_profile_summary(),
                    &self.routed_profile_summaries(&router_state),
                    router_state.routes.len(),
                    self.chat_widget.is_task_running(),
                    initial_selected_idx,
                ));
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
}

fn profile_management_panel_params(
    active_profile: Option<&str>,
    default_profile: &DefaultProfileSummary,
    profiles: &[RoutedProfileSummary],
    routed_count: usize,
    task_running: bool,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    let mut items = vec![profile_selection_item(
        "Default Config".to_string(),
        default_profile_description(default_profile),
        active_profile.is_none(),
        task_running,
        RuntimeProfileTarget::Default,
    )];

    if profiles.is_empty() {
        items.push(SelectionItem {
            name: "No named profiles".to_string(),
            description: Some("Add `[profiles.<name>]` entries in config.toml to route API traffic through alternate endpoints.".to_string()),
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
            "Current: {} · {} named profile(s) · {} routed fallback entr{}.",
            active_profile.unwrap_or("default"),
            profiles.len(),
            routed_count,
            if routed_count == 1 { "y" } else { "ies" },
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
    let mut parts = vec![format!("provider: {}", profile.provider_label)];
    if let Some(base_url) = &profile.base_url {
        parts.push(base_url.clone());
    }
    if let Some(model) = &profile.model {
        parts.push(format!("model: {model}"));
    }
    parts.push(
        profile
            .route_position
            .map(|position| format!("fallback #{position}"))
            .unwrap_or_else(|| "not in fallback route".to_string()),
    );
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    use super::DefaultProfileSummary;
    use super::RoutedProfileSummary;
    use super::profile_management_panel_params;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
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

    #[test]
    fn profile_management_popup_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let profiles = vec![
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
        ];
        let view = ListSelectionView::new(
            profile_management_panel_params(
                Some("primary"),
                &DefaultProfileSummary {
                    provider_label: "OpenAI".to_string(),
                    model: Some("gpt-5".to_string()),
                    base_url: Some("https://api.openai.com/v1".to_string()),
                },
                &profiles,
                /*routed_count*/ 2,
                /*task_running*/ false,
                None,
            ),
            tx,
        );

        assert_snapshot!(
            "profile_management_popup",
            render_selection_popup(&view, 96, 22)
        );
    }

    #[test]
    fn profile_management_panel_disables_switches_while_task_running() {
        let params = profile_management_panel_params(
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
            /*routed_count*/ 1,
            /*task_running*/ true,
            None,
        );

        assert_eq!(params.items[0].is_disabled, true);
        assert_eq!(params.items[1].is_disabled, true);
        assert_eq!(
            params.items[0].disabled_reason.as_deref(),
            Some("Wait for the current task to finish before switching profiles.")
        );
        assert_eq!(
            params.items[1].disabled_reason.as_deref(),
            Some("Already active.")
        );
    }
}
