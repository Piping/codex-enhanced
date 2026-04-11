use super::App;
use crate::app_event::AppEvent;
use crate::app_event::RuntimeProfileTarget;
use crate::app_server_session::AppServerSession;
use crate::profile_router::PROFILE_ROUTER_STATE_RELATIVE_PATH;
use crate::tui;

pub(super) struct ProfileController;

impl ProfileController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::OpenProfileManagementPanel => {
                app.open_profile_management_panel();
            }
            AppEvent::EditProfileFallbackConfig => {
                app.edit_profile_fallback_config_from_ui(tui).await;
            }
            AppEvent::SwitchRuntimeProfile { target } => {
                let is_default_target = matches!(&target, RuntimeProfileTarget::Default);
                let target_profile = match &target {
                    RuntimeProfileTarget::Default => None,
                    RuntimeProfileTarget::Named(profile_id) => Some(profile_id.as_str()),
                };
                let target_label = target_profile.unwrap_or("default");
                if let Err(err) = app
                    .switch_runtime_profile(tui, app_server, target_profile)
                    .await
                {
                    app.chat_widget.add_error_message(format!(
                        "Failed to switch to profile `{target_label}`: {err}"
                    ));
                } else if let Err(err) = app.profile_router_store().update(|state| {
                    state.set_runtime_active_profile(target_profile);
                }) {
                    app.chat_widget.add_error_message(format!(
                        "Switched to profile `{target_label}`, but failed to persist {PROFILE_ROUTER_STATE_RELATIVE_PATH}: {err}"
                    ));
                } else if is_default_target {
                    app.chat_widget.add_info_message(
                        "Switched to the default config profile.".to_string(),
                        /*hint*/ None,
                    );
                } else {
                    app.chat_widget.add_info_message(
                        format!("Switched to profile `{target_label}`."),
                        /*hint*/ None,
                    );
                }
            }
            AppEvent::RetryLastUserTurnWithProfileFallback {
                action,
                error_message,
            } => {
                app.retry_last_user_turn_with_profile_fallback(
                    tui,
                    app_server,
                    action,
                    error_message,
                )
                .await;
            }
            _ => unreachable!("non-profile event passed to profile controller"),
        }
    }
}
