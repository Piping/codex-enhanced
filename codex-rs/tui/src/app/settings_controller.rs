use codex_core::config::edit::ConfigEdit;
use codex_core::config::edit::ConfigEditsBuilder;

use super::App;
#[cfg(target_os = "windows")]
use crate::app_command::AppCommand;
use crate::app_event::AppEvent;
use crate::app_event::RealtimeAudioDeviceKind;
#[cfg(target_os = "windows")]
use crate::app_event::WindowsSandboxEnableMode;
use crate::app_server_session::AppServerSession;
use crate::chatwidget::ExternalEditorState;
use crate::display_preferences::display_preference_edit;
use crate::display_preferences::set_display_preference_in_config;
use crate::history_cell;
use crate::tui;
#[cfg(target_os = "windows")]
use codex_core::windows_sandbox::WindowsSandboxLevelExt;
#[cfg(target_os = "windows")]
use codex_protocol::config_types::WindowsSandboxLevel;
#[cfg(target_os = "windows")]
use ratatui::style::Stylize;
#[cfg(target_os = "windows")]
use ratatui::text::Line;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::time::Instant;

pub(super) struct SettingsController;

impl SettingsController {
    pub(super) async fn handle(
        app: &mut App,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::OpenDisplayPreferencesPanel => {
                app.open_display_preferences_panel();
            }
            AppEvent::UpdateReasoningEffort(effort) => {
                app.on_update_reasoning_effort(effort);
            }
            AppEvent::UpdateModel(model) => {
                app.chat_widget.set_model(&model);
            }
            AppEvent::UpdateCollaborationMode(mask) => {
                app.chat_widget.set_collaboration_mask(mask);
            }
            AppEvent::UpdatePersonality(personality) => {
                app.on_update_personality(personality);
            }
            AppEvent::OpenRealtimeAudioDeviceSelection { kind } => {
                app.chat_widget.open_realtime_audio_device_selection(kind);
            }
            AppEvent::OpenReasoningPopup { model } => {
                app.chat_widget.open_reasoning_popup(model);
            }
            AppEvent::OpenPlanReasoningScopePrompt { model, effort } => {
                app.chat_widget
                    .open_plan_reasoning_scope_prompt(model, effort);
            }
            AppEvent::OpenAllModelsPopup { models } => {
                app.chat_widget.open_all_models_popup(models);
            }
            AppEvent::OpenFullAccessConfirmation {
                preset,
                return_to_permissions,
            } => {
                app.chat_widget
                    .open_full_access_confirmation(preset, return_to_permissions);
            }
            AppEvent::OpenWorldWritableWarningConfirmation {
                preset,
                sample_paths,
                extra_count,
                failed_scan,
            } => {
                app.chat_widget.open_world_writable_warning_confirmation(
                    preset,
                    sample_paths,
                    extra_count,
                    failed_scan,
                );
            }
            AppEvent::OpenFeedbackNote {
                category,
                include_logs,
            } => {
                app.chat_widget.open_feedback_note(category, include_logs);
            }
            AppEvent::OpenFeedbackConsent { category } => {
                app.chat_widget.open_feedback_consent(category);
            }
            AppEvent::SubmitFeedback {
                category,
                reason,
                include_logs,
            } => {
                app.submit_feedback(app_server, category, reason, include_logs);
            }
            AppEvent::FeedbackSubmitted {
                origin_thread_id,
                category,
                include_logs,
                result,
            } => {
                app.handle_feedback_submitted(origin_thread_id, category, include_logs, result)
                    .await;
            }
            AppEvent::LaunchExternalEditor => {
                if app.chat_widget.external_editor_state() == ExternalEditorState::Active {
                    app.launch_external_editor(tui).await;
                }
            }
            AppEvent::OpenWindowsSandboxEnablePrompt { preset } => {
                app.chat_widget.open_windows_sandbox_enable_prompt(preset);
            }
            AppEvent::OpenWindowsSandboxFallbackPrompt { preset } => {
                app.session_telemetry.counter(
                    "codex.windows_sandbox.fallback_prompt_shown",
                    /*inc*/ 1,
                    &[],
                );
                app.chat_widget.clear_windows_sandbox_setup_status();
                if let Some(started_at) = app.windows_sandbox.setup_started_at.take() {
                    app.session_telemetry.record_duration(
                        "codex.windows_sandbox.elevated_setup_duration_ms",
                        started_at.elapsed(),
                        &[("result", "failure")],
                    );
                }
                app.chat_widget.open_windows_sandbox_fallback_prompt(preset);
            }
            AppEvent::BeginWindowsSandboxElevatedSetup { preset } => {
                #[cfg(target_os = "windows")]
                {
                    let policy = preset.sandbox.clone();
                    let policy_cwd = app.config.cwd.clone();
                    let command_cwd = policy_cwd.clone();
                    let env_map: std::collections::HashMap<String, String> =
                        std::env::vars().collect();
                    let codex_home = app.config.codex_home.clone();
                    let tx = app.app_event_tx.clone();

                    if codex_core::windows_sandbox::sandbox_setup_is_complete(codex_home.as_path())
                    {
                        tx.send(AppEvent::EnableWindowsSandboxForAgentMode {
                            preset,
                            mode: WindowsSandboxEnableMode::Elevated,
                        });
                        return;
                    }

                    app.chat_widget.show_windows_sandbox_setup_status();
                    app.windows_sandbox.setup_started_at = Some(Instant::now());
                    let session_telemetry = app.session_telemetry.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = codex_core::windows_sandbox::run_elevated_setup(
                            &policy,
                            policy_cwd.as_path(),
                            command_cwd.as_path(),
                            &env_map,
                            codex_home.as_path(),
                        );
                        let event = match result {
                            Ok(()) => {
                                session_telemetry.counter(
                                    "codex.windows_sandbox.elevated_setup_success",
                                    /*inc*/ 1,
                                    &[],
                                );
                                AppEvent::EnableWindowsSandboxForAgentMode {
                                    preset: preset.clone(),
                                    mode: WindowsSandboxEnableMode::Elevated,
                                }
                            }
                            Err(err) => {
                                let mut code_tag: Option<String> = None;
                                let mut message_tag: Option<String> = None;
                                if let Some((code, message)) =
                                    codex_core::windows_sandbox::elevated_setup_failure_details(
                                        &err,
                                    )
                                {
                                    code_tag = Some(code);
                                    message_tag = Some(message);
                                }
                                let mut tags: Vec<(&str, &str)> = Vec::new();
                                if let Some(code) = code_tag.as_deref() {
                                    tags.push(("code", code));
                                }
                                if let Some(message) = message_tag.as_deref() {
                                    tags.push(("message", message));
                                }
                                session_telemetry.counter(
                                    codex_core::windows_sandbox::elevated_setup_failure_metric_name(
                                        &err,
                                    ),
                                    /*inc*/ 1,
                                    &tags,
                                );
                                tracing::error!(
                                    error = %err,
                                    "failed to run elevated Windows sandbox setup"
                                );
                                AppEvent::OpenWindowsSandboxFallbackPrompt { preset }
                            }
                        };
                        tx.send(event);
                    });
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = preset;
                }
            }
            AppEvent::BeginWindowsSandboxLegacySetup { preset } => {
                #[cfg(target_os = "windows")]
                {
                    let policy = preset.sandbox.clone();
                    let policy_cwd = app.config.cwd.clone();
                    let command_cwd = policy_cwd.clone();
                    let env_map: std::collections::HashMap<String, String> =
                        std::env::vars().collect();
                    let codex_home = app.config.codex_home.clone();
                    let tx = app.app_event_tx.clone();
                    let session_telemetry = app.session_telemetry.clone();

                    app.chat_widget.show_windows_sandbox_setup_status();
                    tokio::task::spawn_blocking(move || {
                        if let Err(err) = codex_core::windows_sandbox::run_legacy_setup_preflight(
                            &policy,
                            policy_cwd.as_path(),
                            command_cwd.as_path(),
                            &env_map,
                            codex_home.as_path(),
                        ) {
                            session_telemetry.counter(
                                "codex.windows_sandbox.legacy_setup_preflight_failed",
                                /*inc*/ 1,
                                &[],
                            );
                            tracing::warn!(
                                error = %err,
                                "failed to preflight non-admin Windows sandbox setup"
                            );
                        }
                        tx.send(AppEvent::EnableWindowsSandboxForAgentMode {
                            preset,
                            mode: WindowsSandboxEnableMode::Legacy,
                        });
                    });
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = preset;
                }
            }
            AppEvent::BeginWindowsSandboxGrantReadRoot { path } => {
                #[cfg(target_os = "windows")]
                {
                    app.chat_widget.add_to_history(history_cell::new_info_event(
                        format!("Granting sandbox read access to {path} ..."),
                        /*hint*/ None,
                    ));

                    let policy = app.config.permissions.sandbox_policy.get().clone();
                    let policy_cwd = app.config.cwd.clone();
                    let command_cwd = app.config.cwd.clone();
                    let env_map: std::collections::HashMap<String, String> =
                        std::env::vars().collect();
                    let codex_home = app.config.codex_home.clone();
                    let tx = app.app_event_tx.clone();

                    tokio::task::spawn_blocking(move || {
                        let requested_path = PathBuf::from(path);
                        let event = match codex_core::windows_sandbox_read_grants::grant_read_root_non_elevated(
                            &policy,
                            policy_cwd.as_path(),
                            command_cwd.as_path(),
                            &env_map,
                            codex_home.as_path(),
                            requested_path.as_path(),
                        ) {
                            Ok(canonical_path) => AppEvent::WindowsSandboxGrantReadRootCompleted {
                                path: canonical_path,
                                error: None,
                            },
                            Err(err) => AppEvent::WindowsSandboxGrantReadRootCompleted {
                                path: requested_path,
                                error: Some(err.to_string()),
                            },
                        };
                        tx.send(event);
                    });
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = path;
                }
            }
            AppEvent::WindowsSandboxGrantReadRootCompleted { path, error } => match error {
                Some(err) => {
                    app.chat_widget
                        .add_to_history(history_cell::new_error_event(format!("Error: {err}")));
                }
                None => {
                    app.chat_widget.add_to_history(history_cell::new_info_event(
                        format!("Sandbox read access granted for {}", path.display()),
                        /*hint*/ None,
                    ));
                }
            },
            AppEvent::EnableWindowsSandboxForAgentMode { preset, mode } => {
                #[cfg(target_os = "windows")]
                {
                    app.chat_widget.clear_windows_sandbox_setup_status();
                    if let Some(started_at) = app.windows_sandbox.setup_started_at.take() {
                        app.session_telemetry.record_duration(
                            "codex.windows_sandbox.elevated_setup_duration_ms",
                            started_at.elapsed(),
                            &[("result", "success")],
                        );
                    }
                    let profile = app.active_profile.as_deref();
                    let elevated_enabled = matches!(mode, WindowsSandboxEnableMode::Elevated);
                    let builder = ConfigEditsBuilder::new(&app.config.codex_home)
                        .with_profile(profile)
                        .set_windows_sandbox_mode(if elevated_enabled {
                            "elevated"
                        } else {
                            "unelevated"
                        })
                        .clear_legacy_windows_sandbox_keys();
                    match builder.apply().await {
                        Ok(()) => {
                            if elevated_enabled {
                                app.config.set_windows_sandbox_enabled(/*value*/ false);
                                app.config
                                    .set_windows_elevated_sandbox_enabled(/*value*/ true);
                            } else {
                                app.config.set_windows_sandbox_enabled(/*value*/ true);
                                app.config
                                    .set_windows_elevated_sandbox_enabled(/*value*/ false);
                            }
                            app.chat_widget.set_windows_sandbox_mode(
                                app.config.permissions.windows_sandbox_mode,
                            );
                            let windows_sandbox_level =
                                WindowsSandboxLevel::from_config(&app.config);
                            if let Some((sample_paths, extra_count, failed_scan)) =
                                app.chat_widget.world_writable_warning_details()
                            {
                                app.app_event_tx.send(AppEvent::CodexOp(
                                    AppCommand::override_turn_context(
                                        /*cwd*/ None,
                                        /*approval_policy*/ None,
                                        /*approvals_reviewer*/ None,
                                        /*sandbox_policy*/ None,
                                        #[cfg(target_os = "windows")]
                                        Some(windows_sandbox_level),
                                        /*model*/ None,
                                        /*effort*/ None,
                                        /*summary*/ None,
                                        /*service_tier*/ None,
                                        /*collaboration_mode*/ None,
                                        /*personality*/ None,
                                    )
                                    .into(),
                                ));
                                app.app_event_tx.send(
                                    AppEvent::OpenWorldWritableWarningConfirmation {
                                        preset: Some(preset.clone()),
                                        sample_paths,
                                        extra_count,
                                        failed_scan,
                                    },
                                );
                            } else {
                                app.app_event_tx.send(AppEvent::CodexOp(
                                    AppCommand::override_turn_context(
                                        /*cwd*/ None,
                                        Some(preset.approval),
                                        Some(app.config.approvals_reviewer),
                                        Some(preset.sandbox.clone()),
                                        #[cfg(target_os = "windows")]
                                        Some(windows_sandbox_level),
                                        /*model*/ None,
                                        /*effort*/ None,
                                        /*summary*/ None,
                                        /*service_tier*/ None,
                                        /*collaboration_mode*/ None,
                                        /*personality*/ None,
                                    )
                                    .into(),
                                ));
                                app.app_event_tx
                                    .send(AppEvent::UpdateAskForApprovalPolicy(preset.approval));
                                app.app_event_tx
                                    .send(AppEvent::UpdateSandboxPolicy(preset.sandbox.clone()));
                                let _ = mode;
                                app.chat_widget.add_plain_history_lines(vec![
                                    Line::from(vec!["• ".dim(), "Sandbox ready".into()]),
                                    Line::from(vec![
                                        "  ".into(),
                                        "Codex can now safely edit files and execute commands in your computer"
                                            .dark_gray(),
                                    ]),
                                ]);
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                error = %err,
                                "failed to enable Windows sandbox feature"
                            );
                            app.chat_widget.add_error_message(format!(
                                "Failed to enable the Windows sandbox feature: {err}"
                            ));
                        }
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = (preset, mode);
                }
            }
            AppEvent::PersistModelSelection { model, effort } => {
                let profile = app.active_profile.as_deref();
                match ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_profile(profile)
                    .set_model(Some(model.as_str()), effort)
                    .apply()
                    .await
                {
                    Ok(()) => {
                        let effort_label = effort
                            .map(|selected_effort| selected_effort.to_string())
                            .unwrap_or_else(|| "default".to_string());
                        tracing::info!("Selected model: {model}, Selected effort: {effort_label}");
                        let mut message = format!("Model changed to {model}");
                        if let Some(label) = App::reasoning_label_for(&model, effort) {
                            message.push(' ');
                            message.push_str(label);
                        }
                        if let Some(profile) = profile {
                            message.push_str(" for ");
                            message.push_str(profile);
                            message.push_str(" profile");
                        }
                        app.chat_widget.add_info_message(message, /*hint*/ None);
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "failed to persist model selection"
                        );
                        if let Some(profile) = profile {
                            app.chat_widget.add_error_message(format!(
                                "Failed to save model for profile `{profile}`: {err}"
                            ));
                        } else {
                            app.chat_widget
                                .add_error_message(format!("Failed to save default model: {err}"));
                        }
                    }
                }
            }
            AppEvent::PersistPersonalitySelection { personality } => {
                let profile = app.active_profile.as_deref();
                match ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_profile(profile)
                    .set_personality(Some(personality))
                    .apply()
                    .await
                {
                    Ok(()) => {
                        let label = App::personality_label(personality);
                        let mut message = format!("Personality set to {label}");
                        if let Some(profile) = profile {
                            message.push_str(" for ");
                            message.push_str(profile);
                            message.push_str(" profile");
                        }
                        app.chat_widget.add_info_message(message, /*hint*/ None);
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "failed to persist personality selection"
                        );
                        if let Some(profile) = profile {
                            app.chat_widget.add_error_message(format!(
                                "Failed to save personality for profile `{profile}`: {err}"
                            ));
                        } else {
                            app.chat_widget.add_error_message(format!(
                                "Failed to save default personality: {err}"
                            ));
                        }
                    }
                }
            }
            AppEvent::PersistServiceTierSelection { service_tier } => {
                app.refresh_status_line();
                let profile = app.active_profile.as_deref();
                match ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_profile(profile)
                    .set_service_tier(service_tier)
                    .apply()
                    .await
                {
                    Ok(()) => {
                        let status = if service_tier.is_some() { "on" } else { "off" };
                        let mut message = format!("Fast mode set to {status}");
                        if let Some(profile) = profile {
                            message.push_str(" for ");
                            message.push_str(profile);
                            message.push_str(" profile");
                        }
                        app.chat_widget.add_info_message(message, /*hint*/ None);
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "failed to persist fast mode selection");
                        if let Some(profile) = profile {
                            app.chat_widget.add_error_message(format!(
                                "Failed to save Fast mode for profile `{profile}`: {err}"
                            ));
                        } else {
                            app.chat_widget.add_error_message(format!(
                                "Failed to save default Fast mode: {err}"
                            ));
                        }
                    }
                }
            }
            AppEvent::PersistRealtimeAudioDeviceSelection { kind, name } => {
                let builder = match kind {
                    RealtimeAudioDeviceKind::Microphone => {
                        ConfigEditsBuilder::new(&app.config.codex_home)
                            .set_realtime_microphone(name.as_deref())
                    }
                    RealtimeAudioDeviceKind::Speaker => {
                        ConfigEditsBuilder::new(&app.config.codex_home)
                            .set_realtime_speaker(name.as_deref())
                    }
                };

                match builder.apply().await {
                    Ok(()) => {
                        match kind {
                            RealtimeAudioDeviceKind::Microphone => {
                                app.config.realtime_audio.microphone = name.clone();
                            }
                            RealtimeAudioDeviceKind::Speaker => {
                                app.config.realtime_audio.speaker = name.clone();
                            }
                        }
                        app.chat_widget
                            .set_realtime_audio_device(kind, name.clone());

                        if app.chat_widget.realtime_conversation_is_live() {
                            app.chat_widget.open_realtime_audio_restart_prompt(kind);
                        } else {
                            let selection = name.unwrap_or_else(|| "System default".to_string());
                            app.chat_widget.add_info_message(
                                format!("Realtime {} set to {selection}", kind.noun()),
                                /*hint*/ None,
                            );
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "failed to persist realtime audio selection"
                        );
                        app.chat_widget.add_error_message(format!(
                            "Failed to save realtime {}: {err}",
                            kind.noun()
                        ));
                    }
                }
            }
            AppEvent::RestartRealtimeAudioDevice { kind } => {
                app.chat_widget.restart_realtime_audio_device(kind);
            }
            AppEvent::UpdateAskForApprovalPolicy(policy) => {
                let mut config = app.config.clone();
                if !app.try_set_approval_policy_on_config(
                    &mut config,
                    policy,
                    "Failed to set approval policy",
                    "failed to set approval policy on app config",
                ) {
                    return;
                }
                app.config = config;
                app.runtime_approval_policy_override =
                    Some(app.config.permissions.approval_policy.value());
                app.chat_widget
                    .set_approval_policy(app.config.permissions.approval_policy.value());
            }
            AppEvent::UpdateSandboxPolicy(policy) => {
                #[cfg(target_os = "windows")]
                let policy_is_workspace_write_or_ro = matches!(
                    &policy,
                    codex_protocol::protocol::SandboxPolicy::WorkspaceWrite { .. }
                        | codex_protocol::protocol::SandboxPolicy::ReadOnly { .. }
                );
                let policy_for_chat = policy.clone();

                let mut config = app.config.clone();
                if !app.try_set_sandbox_policy_on_config(
                    &mut config,
                    policy,
                    "Failed to set sandbox policy",
                    "failed to set sandbox policy on app config",
                ) {
                    return;
                }
                app.config = config;
                if let Err(err) = app.chat_widget.set_sandbox_policy(policy_for_chat) {
                    tracing::warn!(%err, "failed to set sandbox policy on chat config");
                    app.chat_widget
                        .add_error_message(format!("Failed to set sandbox policy: {err}"));
                    return;
                }
                app.runtime_sandbox_policy_override =
                    Some(app.config.permissions.sandbox_policy.get().clone());

                #[cfg(target_os = "windows")]
                {
                    if app.windows_sandbox.skip_world_writable_scan_once {
                        app.windows_sandbox.skip_world_writable_scan_once = false;
                        return;
                    }

                    let should_check = WindowsSandboxLevel::from_config(&app.config)
                        != WindowsSandboxLevel::Disabled
                        && policy_is_workspace_write_or_ro
                        && !app.chat_widget.world_writable_warning_hidden();
                    if should_check {
                        let cwd = app.config.cwd.clone();
                        let env_map: std::collections::HashMap<String, String> =
                            std::env::vars().collect();
                        let tx = app.app_event_tx.clone();
                        let logs_base_dir = app.config.codex_home.clone();
                        let sandbox_policy = app.config.permissions.sandbox_policy.get().clone();
                        App::spawn_world_writable_scan(
                            cwd.to_path_buf(),
                            env_map,
                            logs_base_dir,
                            sandbox_policy,
                            tx,
                        );
                    }
                }
            }
            AppEvent::UpdateApprovalsReviewer(policy) => {
                app.config.approvals_reviewer = policy;
                app.chat_widget.set_approvals_reviewer(policy);
                let profile = app.active_profile.as_deref();
                let segments = if let Some(profile) = profile {
                    vec![
                        "profiles".to_string(),
                        profile.to_string(),
                        "approvals_reviewer".to_string(),
                    ]
                } else {
                    vec!["approvals_reviewer".to_string()]
                };
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_profile(profile)
                    .with_edits([ConfigEdit::SetPath {
                        segments,
                        value: policy.to_string().into(),
                    }])
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        "failed to persist approvals reviewer update"
                    );
                    app.chat_widget
                        .add_error_message(format!("Failed to save approvals reviewer: {err}"));
                }
            }
            AppEvent::UpdateFeatureFlags { updates } => {
                app.update_feature_flags(updates).await;
            }
            AppEvent::ToggleDisplayPreference(key) => {
                let enabled = !app.display_preferences.is_enabled(key);
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_profile(app.active_profile.as_deref())
                    .with_edits([display_preference_edit(key, enabled)])
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        ?key,
                        "failed to persist display preference update"
                    );
                    app.chat_widget
                        .add_error_message(format!("Failed to save UI preference: {err}"));
                } else {
                    app.display_preferences.set_enabled(key, enabled);
                    set_display_preference_in_config(&mut app.config, key, enabled);
                    app.open_display_preferences_panel();
                }
            }
            AppEvent::SkipNextWorldWritableScan => {
                app.windows_sandbox.skip_world_writable_scan_once = true;
            }
            AppEvent::UpdateFullAccessWarningAcknowledged(ack) => {
                app.chat_widget.set_full_access_warning_acknowledged(ack);
            }
            AppEvent::UpdateWorldWritableWarningAcknowledged(ack) => {
                app.chat_widget.set_world_writable_warning_acknowledged(ack);
            }
            AppEvent::UpdateRateLimitSwitchPromptHidden(hidden) => {
                app.chat_widget.set_rate_limit_switch_prompt_hidden(hidden);
            }
            AppEvent::UpdatePlanModeReasoningEffort(effort) => {
                app.config.plan_mode_reasoning_effort = effort;
                app.chat_widget.set_plan_mode_reasoning_effort(effort);
            }
            AppEvent::PersistFullAccessWarningAcknowledged => {
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .set_hide_full_access_warning(/*acknowledged*/ true)
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        "failed to persist full access warning acknowledgement"
                    );
                    app.chat_widget.add_error_message(format!(
                        "Failed to save full access confirmation preference: {err}"
                    ));
                }
            }
            AppEvent::PersistWorldWritableWarningAcknowledged => {
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .set_hide_world_writable_warning(/*acknowledged*/ true)
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        "failed to persist world-writable warning acknowledgement"
                    );
                    app.chat_widget.add_error_message(format!(
                        "Failed to save Agent mode warning preference: {err}"
                    ));
                }
            }
            AppEvent::PersistRateLimitSwitchPromptHidden => {
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .set_hide_rate_limit_model_nudge(/*acknowledged*/ true)
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        "failed to persist rate limit switch prompt preference"
                    );
                    app.chat_widget.add_error_message(format!(
                        "Failed to save rate limit reminder preference: {err}"
                    ));
                }
            }
            AppEvent::PersistPlanModeReasoningEffort(effort) => {
                let profile = app.active_profile.as_deref();
                let segments = if let Some(profile) = profile {
                    vec![
                        "profiles".to_string(),
                        profile.to_string(),
                        "plan_mode_reasoning_effort".to_string(),
                    ]
                } else {
                    vec!["plan_mode_reasoning_effort".to_string()]
                };
                let edit = if let Some(effort) = effort {
                    ConfigEdit::SetPath {
                        segments,
                        value: effort.to_string().into(),
                    }
                } else {
                    ConfigEdit::ClearPath { segments }
                };
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_edits([edit])
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        "failed to persist plan mode reasoning effort"
                    );
                    if let Some(profile) = profile {
                        app.chat_widget.add_error_message(format!(
                            "Failed to save Plan mode reasoning effort for profile `{profile}`: {err}"
                        ));
                    } else {
                        app.chat_widget.add_error_message(format!(
                            "Failed to save Plan mode reasoning effort: {err}"
                        ));
                    }
                }
            }
            AppEvent::PersistModelMigrationPromptAcknowledged {
                from_model,
                to_model,
            } => {
                if let Err(err) = ConfigEditsBuilder::new(&app.config.codex_home)
                    .record_model_migration_seen(from_model.as_str(), to_model.as_str())
                    .apply()
                    .await
                {
                    tracing::error!(
                        error = %err,
                        "failed to persist model migration prompt acknowledgement"
                    );
                    app.chat_widget.add_error_message(format!(
                        "Failed to save model migration prompt preference: {err}"
                    ));
                }
            }
            AppEvent::OpenApprovalsPopup => {
                app.chat_widget.open_approvals_popup();
            }
            AppEvent::OpenPermissionsPopup => {
                app.chat_widget.open_permissions_popup();
            }
            AppEvent::StatusLineSetup { items } => {
                let ids = items.iter().map(ToString::to_string).collect::<Vec<_>>();
                let edit = codex_core::config::edit::status_line_items_edit(&ids);
                let apply_result = ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_edits([edit])
                    .apply()
                    .await;
                match apply_result {
                    Ok(()) => {
                        app.config.tui_status_line = Some(ids.clone());
                        app.chat_widget.setup_status_line(items);
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "failed to persist status line items; keeping previous selection");
                        app.chat_widget
                            .add_error_message(format!("Failed to save status line items: {err}"));
                    }
                }
            }
            AppEvent::StatusLineBranchUpdated { cwd, branch } => {
                app.chat_widget.set_status_line_branch(cwd, branch);
                app.refresh_status_line();
            }
            AppEvent::StatusLineSetupCancelled => {
                app.chat_widget.cancel_status_line_setup();
            }
            AppEvent::TerminalTitleSetup { items } => {
                let ids = items.iter().map(ToString::to_string).collect::<Vec<_>>();
                let edit = codex_core::config::edit::terminal_title_items_edit(&ids);
                let apply_result = ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_edits([edit])
                    .apply()
                    .await;
                match apply_result {
                    Ok(()) => {
                        app.config.tui_terminal_title = Some(ids.clone());
                        app.chat_widget.setup_terminal_title(items);
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "failed to persist terminal title items; keeping previous selection");
                        app.chat_widget.revert_terminal_title_setup_preview();
                        app.chat_widget.add_error_message(format!(
                            "Failed to save terminal title items: {err}"
                        ));
                    }
                }
            }
            AppEvent::TerminalTitleSetupPreview { items } => {
                app.chat_widget.preview_terminal_title(items);
            }
            AppEvent::TerminalTitleSetupCancelled => {
                app.chat_widget.cancel_terminal_title_setup();
            }
            AppEvent::SyntaxThemeSelected { name } => {
                let edit = codex_core::config::edit::syntax_theme_edit(&name);
                let apply_result = ConfigEditsBuilder::new(&app.config.codex_home)
                    .with_edits([edit])
                    .apply()
                    .await;
                match apply_result {
                    Ok(()) => {
                        if let Some(theme) = crate::render::highlight::resolve_theme_by_name(
                            &name,
                            Some(&app.config.codex_home),
                        ) {
                            crate::render::highlight::set_syntax_theme(theme);
                        }
                        app.sync_tui_theme_selection(name);
                    }
                    Err(err) => {
                        app.restore_runtime_theme_from_config();
                        tracing::error!(error = %err, "failed to persist theme selection");
                        app.chat_widget
                            .add_error_message(format!("Failed to save theme: {err}"));
                    }
                }
            }
            _ => unreachable!("non-settings event passed to settings controller"),
        }
    }
}
