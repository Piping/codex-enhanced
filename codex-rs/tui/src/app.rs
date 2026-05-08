//! Top-level TUI application state and runtime wiring.
//!
//! This module owns the `App` struct, shared imports, and the high-level run loop that coordinates
//! the focused app submodules.

use crate::app_backtrack::BacktrackState;
use crate::app_command::AppCommand;
use crate::app_command::AppCommandView;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotControlsDestination;
use crate::app_event::ExitMode;
use crate::app_event::FeedbackCategory;
use crate::app_event::HistoryLookupResponse;
use crate::app_event::RateLimitRefreshOrigin;
use crate::app_event::RealtimeAudioDeviceKind;
#[cfg(target_os = "windows")]
use crate::app_event::WindowsSandboxEnableMode;
use crate::app_event_sender::AppEventSender;
use crate::app_server_session::AppServerSession;
use crate::app_server_session::AppServerStartedThread;
use crate::app_server_session::app_server_rate_limit_snapshots;
use crate::bottom_pane::AppLinkViewParams;
use crate::bottom_pane::ApprovalRequest;
use crate::bottom_pane::FeedbackAudience;
use crate::bottom_pane::McpServerElicitationFormRequest;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::chatwidget::ChatWidget;
use crate::chatwidget::ExternalEditorState;
use crate::chatwidget::ReplayKind;
use crate::chatwidget::ThreadInputState;
use crate::cwd_prompt::CwdPromptAction;
use crate::diff_render::DiffSummary;
use crate::display_preferences::DisplayPreferences;
use crate::display_preferences::display_preference_edit;
use crate::display_preferences::set_display_preference_in_config;
use crate::display_preferences_menu::DISPLAY_PREFERENCES_SELECTION_VIEW_ID;
use crate::display_preferences_menu::display_preferences_panel_params;
use crate::exec_command::split_command_string;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::external_agent_config_migration_startup::ExternalAgentConfigMigrationStartupOutcome;
use crate::external_agent_config_migration_startup::handle_external_agent_config_migration_prompt_if_needed;
use crate::external_editor;
use crate::file_search::FileSearchManager;
use crate::history_cell;
use crate::history_cell::HistoryCell;
#[cfg(not(debug_assertions))]
use crate::history_cell::UpdateAvailableHistoryCell;
use crate::insert_history::ScrollbackWrapMode;
use crate::key_hint::KeyBindingListExt;
use crate::keymap::RuntimeKeymap;
use crate::legacy_core::config::Config;
use crate::legacy_core::config::ConfigBuilder;
use crate::legacy_core::config::ConfigOverrides;
use crate::legacy_core::config::edit::ConfigEdit;
use crate::legacy_core::config::edit::ConfigEditsBuilder;
#[cfg(target_os = "windows")]
use crate::legacy_core::windows_sandbox::WindowsSandboxLevelExt;
use crate::model_catalog::ModelCatalog;
use crate::model_migration::ModelMigrationOutcome;
use crate::model_migration::migration_copy_for_models;
use crate::model_migration::run_model_migration_prompt;
use crate::multi_agents::agent_picker_status_dot_spans;
use crate::multi_agents::format_agent_picker_item_name;
use crate::pager_overlay::Overlay;
use crate::render::highlight::highlight_bash_to_lines;
use crate::render::renderable::Renderable;
use crate::resume_picker::SessionSelection;
use crate::resume_picker::SessionTarget;
use crate::session_resume::cwds_differ;
use crate::session_resume::read_session_model;
use crate::session_state::ThreadSessionState;
#[cfg(test)]
use crate::test_support::PathBufExt;
#[cfg(test)]
use crate::test_support::test_path_buf;
#[cfg(test)]
use crate::test_support::test_path_display;
use crate::token_usage::TokenUsage;
use crate::transcript_reflow::TranscriptReflowState;
use crate::tui;
use crate::tui::TuiEvent;
use crate::update_action::UpdateAction;
use crate::version::CODEX_CLI_VERSION;
use crate::workspace_command::AppServerWorkspaceCommandRunner;
use crate::workspace_command::WorkspaceCommandRunner;
use codex_ansi_escape::ansi_escape_line;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::AddCreditsNudgeCreditType;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::CodexErrorInfo as AppServerCodexErrorInfo;
use codex_app_server_protocol::ConfigBatchWriteParams;
use codex_app_server_protocol::ConfigLayerSource;
use codex_app_server_protocol::ConfigValueWriteParams;
use codex_app_server_protocol::ConfigWriteResponse;
use codex_app_server_protocol::FeedbackUploadParams;
use codex_app_server_protocol::FeedbackUploadResponse;
use codex_app_server_protocol::GetAccountRateLimitsResponse;
use codex_app_server_protocol::HooksListParams;
use codex_app_server_protocol::HooksListResponse;
use codex_app_server_protocol::ListMcpServerStatusParams;
use codex_app_server_protocol::ListMcpServerStatusResponse;
#[cfg(test)]
use codex_app_server_protocol::McpAuthStatus;
use codex_app_server_protocol::McpServerStatus;
use codex_app_server_protocol::McpServerStatusDetail;
use codex_app_server_protocol::MergeStrategy;
use codex_app_server_protocol::PluginInstallParams;
use codex_app_server_protocol::PluginInstallResponse;
use codex_app_server_protocol::PluginListParams;
use codex_app_server_protocol::PluginListResponse;
use codex_app_server_protocol::PluginReadParams;
use codex_app_server_protocol::PluginReadResponse;
use codex_app_server_protocol::PluginUninstallParams;
use codex_app_server_protocol::PluginUninstallResponse;
use codex_app_server_protocol::RateLimitSnapshot;
use codex_app_server_protocol::SendAddCreditsNudgeEmailParams;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::SkillErrorInfo;
use codex_app_server_protocol::SkillsListParams;
use codex_app_server_protocol::SkillsListResponse;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadListParams;
use codex_app_server_protocol::ThreadLoadedListParams;
use codex_app_server_protocol::ThreadMemoryMode;
use codex_app_server_protocol::ThreadRollbackResponse;
use codex_app_server_protocol::ThreadSortKey as AppServerThreadSortKey;
use codex_app_server_protocol::ThreadSourceKind;
use codex_app_server_protocol::ThreadStartSource;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnError as AppServerTurnError;
use codex_app_server_protocol::TurnStatus;
use codex_clawbot::PendingClawbotTurn;
#[cfg(test)]
use codex_clawbot::ProviderOutboundReaction;
use codex_clawbot::ProviderOutboundTextMessage;
use codex_config::ConfigLayerStackOrdering;
use codex_config::types::ApprovalsReviewer;
use codex_config::types::ModelAvailabilityNuxConfig;
use codex_core_plugins::PluginsManager;
use codex_exec_server::EnvironmentManager;
use codex_features::Feature;
use codex_model_provider::create_model_provider;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager::model_presets::HIDE_GPT_5_1_CODEX_MAX_MIGRATION_PROMPT_CONFIG;
use codex_models_manager::model_presets::HIDE_GPT5_1_MIGRATION_PROMPT_CONFIG;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::config_types::Personality;
#[cfg(target_os = "windows")]
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::openai_models::ModelAvailabilityNux;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelUpgrade;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
#[cfg(target_os = "windows")]
use codex_protocol::permissions::FileSystemSandboxKind;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::FinalOutput;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_rollout::ARCHIVED_SESSIONS_SUBDIR;
use codex_rollout::StateDbHandle;
use codex_terminal_detection::user_agent;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path as path_utils;
use color_eyre::eyre::Result;
use color_eyre::eyre::WrapErr;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::backend::Backend;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tokio::select;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::unbounded_channel;
use tokio::task::JoinHandle;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
use toml::Value as TomlValue;
use uuid::Uuid;
mod agent_delete;
mod agent_navigation;
mod app_server_event_targets;
mod app_server_events;
pub(crate) mod app_server_requests;
mod background_requests;
mod btw;
mod clawbot;
mod clawbot_controller;
mod clawbot_controls;
mod config_persistence;
mod editor_helpers;
mod event_dispatch;
mod feature_dispatch;
mod history_ui;
mod input;
mod jump_navigation;
mod key_chord;
mod loaded_threads;
mod pending_interactive_replay;
mod platform_actions;
mod popup_helpers;
mod profile_controller;
mod profile_management;
mod replay_filter;
mod resize_reflow;
mod run;
mod session_lifecycle;
mod side;
mod startup_prompts;
mod thread_controller;
mod thread_events;
mod thread_goal_actions;
mod thread_menu;
mod thread_routing;
mod thread_session_state;
mod workflow_controller;
mod workflow_controls;
mod workflow_definition;
mod workflow_editor;
mod workflow_file_watch;
mod workflow_history;
pub(crate) mod workflow_runtime;
mod workflow_scheduler;
mod workflow_yaml;

use self::agent_navigation::AgentNavigationState;
use self::app_server_requests::PendingAppServerRequests;
use self::btw::BtwSessionState;
use self::key_chord::KeyChordAction;
use self::key_chord::KeyChordResolution;
use self::key_chord::KeyChordState;
use self::loaded_threads::find_loaded_subagent_threads_for_primary;
use self::pending_interactive_replay::PendingInteractiveReplayState;
use self::platform_actions::*;
use self::side::SideParentStatus;
use self::side::SideParentStatusChange;
use self::side::SideThreadState;
use self::startup_prompts::*;
use self::thread_events::*;
use self::workflow_file_watch::WorkflowFileWatchState;
use self::workflow_history::WorkflowHistoryState;
use self::workflow_scheduler::WorkflowSchedulerState;

const EXTERNAL_EDITOR_HINT: &str = "Save and close external editor to continue.";
const THREAD_EVENT_CHANNEL_CAPACITY: usize = 32768;
#[derive(Clone, Copy)]
enum ThreadLivenessRefreshMode {
    Picker,
    Selection,
}

enum ThreadInteractiveRequest {
    AppLink(AppLinkViewParams),
    Approval(ApprovalRequest),
    McpServerElicitation(McpServerElicitationFormRequest),
}

/// Extracts `receiver_thread_ids` from collab agent tool-call notifications.
///
/// Only `ItemStarted` and `ItemCompleted` notifications with a `CollabAgentToolCall` item carry
/// receiver thread ids. All other notification variants return `None`.
fn collab_receiver_thread_ids(notification: &ServerNotification) -> Option<&[String]> {
    match notification {
        ServerNotification::ItemStarted(notification) => match &notification.item {
            ThreadItem::CollabAgentToolCall {
                receiver_thread_ids,
                ..
            } => Some(receiver_thread_ids),
            _ => None,
        },
        ServerNotification::ItemCompleted(notification) => match &notification.item {
            ThreadItem::CollabAgentToolCall {
                receiver_thread_ids,
                ..
            } => Some(receiver_thread_ids),
            _ => None,
        },
        _ => None,
    }
}

fn workflow_after_turn_last_agent_message(
    primary_thread_id: Option<ThreadId>,
    thread_id: ThreadId,
    notification: &ServerNotification,
) -> Option<AfterTurnContext> {
    if primary_thread_id != Some(thread_id) {
        return None;
    }
    let ServerNotification::TurnCompleted(notification) = notification else {
        return None;
    };
    if !matches!(
        notification.turn.status,
        TurnStatus::Completed | TurnStatus::Failed
    ) {
        return None;
    }
    Some(AfterTurnContext {
        last_agent_message: last_agent_message_for_turn(&notification.turn),
        status: notification.turn.status.clone(),
    })
}

fn last_agent_message_for_turn(turn: &Turn) -> Option<String> {
    turn.items.iter().fold(None, |_, item| match item {
        ThreadItem::AgentMessage { text, .. } => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AfterTurnContext {
    last_agent_message: Option<String>,
    status: TurnStatus,
}

fn default_exec_approval_decisions(
    network_approval_context: Option<&codex_app_server_protocol::NetworkApprovalContext>,
    proposed_execpolicy_amendment: Option<&codex_app_server_protocol::ExecPolicyAmendment>,
    proposed_network_policy_amendments: Option<
        &[codex_app_server_protocol::NetworkPolicyAmendment],
    >,
    additional_permissions: Option<&codex_app_server_protocol::AdditionalPermissionProfile>,
) -> Vec<codex_app_server_protocol::CommandExecutionApprovalDecision> {
    use codex_app_server_protocol::CommandExecutionApprovalDecision;
    use codex_app_server_protocol::NetworkPolicyRuleAction;

    if network_approval_context.is_some() {
        let mut decisions = vec![
            CommandExecutionApprovalDecision::Accept,
            CommandExecutionApprovalDecision::AcceptForSession,
        ];
        if let Some(amendment) = proposed_network_policy_amendments.and_then(|amendments| {
            amendments
                .iter()
                .find(|amendment| amendment.action == NetworkPolicyRuleAction::Allow)
        }) {
            decisions.push(
                CommandExecutionApprovalDecision::ApplyNetworkPolicyAmendment {
                    network_policy_amendment: amendment.clone(),
                },
            );
        }
        decisions.push(CommandExecutionApprovalDecision::Cancel);
        return decisions;
    }

    if additional_permissions.is_some() {
        return vec![
            CommandExecutionApprovalDecision::Accept,
            CommandExecutionApprovalDecision::Cancel,
        ];
    }

    let mut decisions = vec![CommandExecutionApprovalDecision::Accept];
    if let Some(execpolicy_amendment) = proposed_execpolicy_amendment {
        decisions.push(
            CommandExecutionApprovalDecision::AcceptWithExecpolicyAmendment {
                execpolicy_amendment: execpolicy_amendment.clone(),
            },
        );
    }
    decisions.push(CommandExecutionApprovalDecision::Cancel);
    decisions
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutoReviewMode {
    approval_policy: AskForApproval,
    approvals_reviewer: ApprovalsReviewer,
    permission_profile: PermissionProfile,
}

/// Enabling the Auto-review experiment in the TUI should also switch the
/// current `/permissions` settings to the matching Auto-review mode. Users
/// can still change `/permissions` afterward; this just assumes that opting into
/// the experiment means they want Auto-review enabled immediately.
fn auto_review_mode() -> AutoReviewMode {
    AutoReviewMode {
        approval_policy: AskForApproval::OnRequest,
        approvals_reviewer: ApprovalsReviewer::AutoReview,
        permission_profile: PermissionProfile::workspace_write(),
    }
}

#[cfg(target_os = "windows")]
fn managed_filesystem_sandbox_is_restricted(permission_profile: &PermissionProfile) -> bool {
    matches!(
        permission_profile.file_system_sandbox_policy().kind,
        FileSystemSandboxKind::Restricted
    )
}

fn should_respawn_with_yolo(config: &Config) -> bool {
    config.permissions.approval_policy.value() == AskForApproval::Never
        && config
            .permissions
            .permission_profile()
            .to_legacy_sandbox_policy(config.cwd.as_path())
            .is_ok_and(|sandbox_policy| sandbox_policy == SandboxPolicy::DangerFullAccess)
}

#[cfg(unix)]
fn spawn_respawn_signal_listener(app_event_tx: AppEventSender) -> std::io::Result<()> {
    use tokio::signal::unix::SignalKind;

    let mut signal = tokio::signal::unix::signal(SignalKind::user_defined1())?;
    tokio::spawn(async move {
        if signal.recv().await.is_some() {
            app_event_tx.send(AppEvent::Exit(ExitMode::RespawnImmediate));
        }
    });
    Ok(())
}

#[cfg(not(unix))]
fn spawn_respawn_signal_listener(_app_event_tx: AppEventSender) -> std::io::Result<()> {
    Ok(())
}

/// Baseline cadence for periodic stream commit animation ticks.
///
/// Smooth-mode streaming drains one line per tick, so this interval controls
/// perceived typing speed for non-backlogged output.
const COMMIT_ANIMATION_TICK: Duration = tui::TARGET_FRAME_INTERVAL;

#[derive(Debug, Clone)]
pub struct AppExitInfo {
    pub token_usage: TokenUsage,
    pub thread_id: Option<ThreadId>,
    pub thread_name: Option<String>,
    pub respawn_target: Option<String>,
    pub update_action: Option<UpdateAction>,
    pub respawn_with_yolo: bool,
    pub exit_reason: ExitReason,
}

impl AppExitInfo {
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            token_usage: TokenUsage::default(),
            thread_id: None,
            thread_name: None,
            respawn_target: None,
            update_action: None,
            respawn_with_yolo: false,
            exit_reason: ExitReason::Fatal(message.into()),
        }
    }
}

#[derive(Debug)]
pub(crate) enum AppRunControl {
    Continue,
    Exit(ExitReason),
}

#[derive(Debug, Clone)]
pub enum ExitReason {
    UserRequested,
    RespawnRequested,
    Fatal(String),
}

fn session_summary(
    token_usage: TokenUsage,
    thread_id: Option<ThreadId>,
    thread_name: Option<String>,
    rollout_path: Option<&Path>,
) -> Option<SessionSummary> {
    let usage_line = (!token_usage.is_zero()).then(|| token_usage.to_string());
    let thread_id =
        resumable_thread(thread_id, thread_name, rollout_path).map(|thread| thread.thread_id);
    let resume_command =
        crate::legacy_core::util::resume_command(/*thread_name*/ None, thread_id);

    if usage_line.is_none() && resume_command.is_none() {
        return None;
    }

    Some(SessionSummary {
        usage_line,
        resume_command,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumableThread {
    thread_id: ThreadId,
    thread_name: Option<String>,
}

fn resumable_thread(
    thread_id: Option<ThreadId>,
    thread_name: Option<String>,
    rollout_path: Option<&Path>,
) -> Option<ResumableThread> {
    let thread_id = thread_id?;
    let rollout_path = rollout_path?;
    rollout_path_is_resumable(rollout_path).then_some(ResumableThread {
        thread_id,
        thread_name,
    })
}

fn rollout_path_is_resumable(rollout_path: &Path) -> bool {
    std::fs::metadata(rollout_path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

fn errors_for_cwd(cwd: &Path, response: &SkillsListResponse) -> Vec<SkillErrorInfo> {
    response
        .data
        .iter()
        .find(|entry| entry.cwd.as_path() == cwd)
        .map(|entry| entry.errors.clone())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionSummary {
    usage_line: Option<String>,
    resume_command: Option<String>,
}

#[derive(Debug, Default)]
struct InitialHistoryReplayBuffer {
    retained_lines: VecDeque<Line<'static>>,
    render_from_transcript_tail: bool,
}

fn should_show_model_migration_prompt(
    current_model: &str,
    target_model: &str,
    seen_migrations: &BTreeMap<String, String>,
    available_models: &[ModelPreset],
) -> bool {
    if target_model == current_model {
        return false;
    }

    if let Some(seen_target) = seen_migrations.get(current_model)
        && seen_target == target_model
    {
        return false;
    }

    if !available_models
        .iter()
        .any(|preset| preset.model == target_model && preset.show_in_picker)
    {
        return false;
    }

    if available_models
        .iter()
        .any(|preset| preset.model == current_model && preset.upgrade.is_some())
    {
        return true;
    }

    if available_models
        .iter()
        .any(|preset| preset.upgrade.as_ref().map(|u| u.id.as_str()) == Some(target_model))
    {
        return true;
    }

    false
}

fn migration_prompt_hidden(config: &Config, migration_config_key: &str) -> bool {
    match migration_config_key {
        HIDE_GPT_5_1_CODEX_MAX_MIGRATION_PROMPT_CONFIG => config
            .notices
            .hide_gpt_5_1_codex_max_migration_prompt
            .unwrap_or(false),
        HIDE_GPT5_1_MIGRATION_PROMPT_CONFIG => {
            config.notices.hide_gpt5_1_migration_prompt.unwrap_or(false)
        }
        _ => false,
    }
}

fn target_preset_for_upgrade<'a>(
    available_models: &'a [ModelPreset],
    target_model: &str,
) -> Option<&'a ModelPreset> {
    available_models
        .iter()
        .find(|preset| preset.model == target_model && preset.show_in_picker)
}

const MODEL_AVAILABILITY_NUX_MAX_SHOW_COUNT: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupTooltipOverride {
    model_slug: String,
    message: String,
}

fn select_model_availability_nux(
    available_models: &[ModelPreset],
    nux_config: &ModelAvailabilityNuxConfig,
) -> Option<StartupTooltipOverride> {
    available_models.iter().find_map(|preset| {
        let ModelAvailabilityNux { message } = preset.availability_nux.as_ref()?;
        let shown_count = nux_config
            .shown_count
            .get(&preset.model)
            .copied()
            .unwrap_or_default();
        (shown_count < MODEL_AVAILABILITY_NUX_MAX_SHOW_COUNT).then(|| StartupTooltipOverride {
            model_slug: preset.model.clone(),
            message: message.clone(),
        })
    })
}

async fn prepare_startup_tooltip_override(
    config: &mut Config,
    available_models: &[ModelPreset],
    is_first_run: bool,
) -> Option<String> {
    if is_first_run || !config.show_tooltips {
        return None;
    }

    let tooltip_override =
        select_model_availability_nux(available_models, &config.model_availability_nux)?;

    let shown_count = config
        .model_availability_nux
        .shown_count
        .get(&tooltip_override.model_slug)
        .copied()
        .unwrap_or_default();
    let next_count = shown_count.saturating_add(1);
    let mut updated_shown_count = config.model_availability_nux.shown_count.clone();
    updated_shown_count.insert(tooltip_override.model_slug.clone(), next_count);

    if let Err(err) = ConfigEditsBuilder::new(&config.codex_home)
        .set_model_availability_nux_count(&updated_shown_count)
        .apply()
        .await
    {
        tracing::error!(
            error = %err,
            model = %tooltip_override.model_slug,
            "failed to persist model availability nux count"
        );
        return Some(tooltip_override.message);
    }

    config.model_availability_nux.shown_count = updated_shown_count;
    Some(tooltip_override.message)
}

async fn handle_model_migration_prompt_if_needed(
    tui: &mut tui::Tui,
    config: &mut Config,
    model: &str,
    app_event_tx: &AppEventSender,
    available_models: &[ModelPreset],
) -> Option<AppExitInfo> {
    let upgrade = available_models
        .iter()
        .find(|preset| preset.model == model)
        .and_then(|preset| preset.upgrade.as_ref());

    if let Some(ModelUpgrade {
        id: target_model,
        reasoning_effort_mapping,
        migration_config_key,
        model_link,
        upgrade_copy,
        migration_markdown,
    }) = upgrade
    {
        if migration_prompt_hidden(config, migration_config_key.as_str()) {
            return None;
        }

        let target_model = target_model.to_string();
        if !should_show_model_migration_prompt(
            model,
            &target_model,
            &config.notices.model_migrations,
            available_models,
        ) {
            return None;
        }

        let current_preset = available_models.iter().find(|preset| preset.model == model);
        let target_preset = target_preset_for_upgrade(available_models, &target_model);
        let target_preset = target_preset?;
        let target_display_name = target_preset.display_name.clone();
        let heading_label = if target_display_name == model {
            target_model.clone()
        } else {
            target_display_name.clone()
        };
        let target_description =
            (!target_preset.description.is_empty()).then(|| target_preset.description.clone());
        let can_opt_out = current_preset.is_some();
        let prompt_copy = migration_copy_for_models(
            model,
            &target_model,
            model_link.clone(),
            upgrade_copy.clone(),
            migration_markdown.clone(),
            heading_label,
            target_description,
            can_opt_out,
        );
        match run_model_migration_prompt(tui, prompt_copy).await {
            ModelMigrationOutcome::Accepted => {
                app_event_tx.send(AppEvent::PersistModelMigrationPromptAcknowledged {
                    from_model: model.to_string(),
                    to_model: target_model.clone(),
                });

                let mapped_effort = if let Some(reasoning_effort_mapping) = reasoning_effort_mapping
                    && let Some(reasoning_effort) = config.model_reasoning_effort
                {
                    reasoning_effort_mapping
                        .get(&reasoning_effort)
                        .cloned()
                        .or(config.model_reasoning_effort)
                } else {
                    config.model_reasoning_effort
                };

                config.model = Some(target_model.clone());
                config.model_reasoning_effort = mapped_effort;
                app_event_tx.send(AppEvent::UpdateModel(target_model.clone()));
                app_event_tx.send(AppEvent::UpdateReasoningEffort(mapped_effort));
                app_event_tx.send(AppEvent::PersistModelSelection {
                    model: target_model.clone(),
                    effort: mapped_effort,
                });
            }
            ModelMigrationOutcome::Rejected => {
                app_event_tx.send(AppEvent::PersistModelMigrationPromptAcknowledged {
                    from_model: model.to_string(),
                    to_model: target_model.clone(),
                });
            }
            ModelMigrationOutcome::Exit => {
                return Some(AppExitInfo {
                    token_usage: TokenUsage::default(),
                    thread_id: None,
                    thread_name: None,
                    respawn_target: None,
                    update_action: None,
                    respawn_with_yolo: false,
                    exit_reason: ExitReason::UserRequested,
                });
            }
        }
    }

    None
}

pub(crate) struct App {
    model_catalog: Arc<ModelCatalog>,
    pub(crate) session_telemetry: SessionTelemetry,
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) chat_widget: ChatWidget,
    workspace_command_runner: Option<WorkspaceCommandRunner>,
    /// Config is stored here so we can recreate ChatWidgets as needed.
    pub(crate) config: Config,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) active_profile: Option<String>,
    cli_kv_overrides: Vec<(String, TomlValue)>,
    harness_overrides: ConfigOverrides,
    runtime_approval_policy_override: Option<AskForApproval>,
    runtime_permission_profile_override: Option<PermissionProfile>,

    pub(crate) file_search: FileSearchManager,

    pub(crate) transcript_cells: Vec<Arc<dyn HistoryCell>>,

    // Pager overlay state (Transcript or Static like Diff)
    pub(crate) overlay: Option<Overlay>,
    pub(crate) deferred_history_lines: Vec<(Vec<Line<'static>>, ScrollbackWrapMode)>,
    has_emitted_history_lines: bool,
    transcript_reflow: TranscriptReflowState,
    initial_history_replay_buffer: Option<InitialHistoryReplayBuffer>,

    pub(crate) enhanced_keys_supported: bool,
    pub(crate) keymap: RuntimeKeymap,

    /// Controls the animation thread that sends CommitTick events.
    pub(crate) commit_anim_running: Arc<AtomicBool>,
    // Shared across ChatWidget instances so invalid status-line config warnings only emit once.
    status_line_invalid_items_warned: Arc<AtomicBool>,
    // Shared across ChatWidget instances so invalid terminal-title config warnings only emit once.
    terminal_title_invalid_items_warned: Arc<AtomicBool>,

    // Esc-backtracking state grouped
    pub(crate) backtrack: crate::app_backtrack::BacktrackState,
    key_chord: KeyChordState,
    display_preferences: DisplayPreferences,
    /// When set, the next draw re-renders the transcript into terminal scrollback once.
    ///
    /// This is used after a confirmed thread rollback to ensure scrollback reflects the trimmed
    /// transcript cells.
    pub(crate) backtrack_render_pending: bool,
    pub(crate) feedback: codex_feedback::CodexFeedback,
    feedback_audience: FeedbackAudience,
    environment_manager: Arc<EnvironmentManager>,
    remote_app_server_url: Option<String>,
    remote_app_server_auth_token: Option<String>,
    /// Set when the user confirms an update; propagated on exit.
    pub(crate) pending_update_action: Option<UpdateAction>,

    /// Tracks the thread we intentionally shut down while exiting the app.
    ///
    /// When this matches the active thread, its `ShutdownComplete` should lead to
    /// process exit instead of being treated as an unexpected sub-agent death that
    /// triggers failover to the primary thread.
    ///
    /// This is thread-scoped state (`Option<ThreadId>`) instead of a global bool
    /// so shutdown events from other threads still take the normal failover path.
    pending_shutdown_exit_thread_id: Option<ThreadId>,

    windows_sandbox: WindowsSandboxState,

    thread_event_channels: HashMap<ThreadId, ThreadEventChannel>,
    thread_event_listener_tasks: HashMap<ThreadId, JoinHandle<()>>,
    agent_navigation: AgentNavigationState,
    side_threads: HashMap<ThreadId, SideThreadState>,
    active_thread_id: Option<ThreadId>,
    active_thread_rx: Option<mpsc::Receiver<ThreadBufferedEvent>>,
    primary_thread_id: Option<ThreadId>,
    last_subagent_backfill_attempt: Option<ThreadId>,
    primary_session_configured: Option<ThreadSessionState>,
    pending_primary_events: VecDeque<ThreadBufferedEvent>,
    pending_workflow_compact_followups: VecDeque<PendingWorkflowCompactFollowup>,
    pending_app_server_requests: PendingAppServerRequests,
    // Serialize plugin enablement writes per plugin so stale completions cannot
    // overwrite a newer toggle, even if the plugin is toggled from different
    // cwd contexts.
    pending_plugin_enabled_writes: HashMap<String, Option<bool>>,
    // Serialize hook enablement writes per hook so stale completions cannot
    // persist an older toggle after a newer one.
    pending_hook_enabled_writes: HashMap<String, Option<bool>>,
    workflow_thread_notification_channels: workflow_runtime::WorkflowThreadNotificationChannels,
    workflow_file_watch: Option<WorkflowFileWatchState>,
    workflow_scheduler: WorkflowSchedulerState,
    workflow_history: WorkflowHistoryState,
    btw_session: Option<BtwSessionState>,
    clawbot_controls_destination: ClawbotControlsDestination,
    clawbot_workspace_root: Option<PathBuf>,
    clawbot_provider_task: Option<JoinHandle<()>>,
    clawbot_pending_turns: HashMap<ThreadId, VecDeque<PendingClawbotTurn>>,
    #[cfg(test)]
    clawbot_outbound_messages: Vec<ProviderOutboundTextMessage>,
    #[cfg(test)]
    clawbot_outbound_reactions: Vec<ProviderOutboundReaction>,
    #[cfg(test)]
    clawbot_removed_outbound_reactions: Vec<ProviderOutboundReaction>,
}

#[derive(Default)]
struct WindowsSandboxState {
    setup_started_at: Option<Instant>,
    // One-shot suppression of the next world-writable scan after user confirmation.
    skip_world_writable_scan_once: bool,
}

#[derive(Debug)]
struct PendingWorkflowCompactFollowup {
    thread_id: ThreadId,
    op: Op,
}

fn normalize_harness_overrides_for_cwd(
    mut overrides: ConfigOverrides,
    base_cwd: &AbsolutePathBuf,
) -> Result<ConfigOverrides> {
    if overrides.additional_writable_roots.is_empty() {
        return Ok(overrides);
    }

    let mut normalized = Vec::with_capacity(overrides.additional_writable_roots.len());
    for root in overrides.additional_writable_roots.drain(..) {
        let absolute = base_cwd.join(root);
        normalized.push(absolute.into_path_buf());
    }
    overrides.additional_writable_roots = normalized;
    Ok(overrides)
}

fn active_turn_not_steerable_turn_error(error: &TypedRequestError) -> Option<AppServerTurnError> {
    let TypedRequestError::Server { source, .. } = error else {
        return None;
    };
    let turn_error: AppServerTurnError = serde_json::from_value(source.data.clone()?).ok()?;
    matches!(
        turn_error.codex_error_info,
        Some(AppServerCodexErrorInfo::ActiveTurnNotSteerable { .. })
    )
    .then_some(turn_error)
}

async fn resolve_runtime_model_provider_base_url(provider: &ModelProviderInfo) -> Option<String> {
    let provider = create_model_provider(provider.clone(), /*auth_manager*/ None);
    match provider.runtime_base_url().await {
        Ok(base_url) => base_url,
        Err(err) => {
            tracing::warn!(%err, "failed to resolve runtime model provider base URL for status");
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveTurnSteerRace {
    Missing,
    ExpectedTurnMismatch { actual_turn_id: String },
}

fn active_turn_steer_race(error: &TypedRequestError) -> Option<ActiveTurnSteerRace> {
    let TypedRequestError::Server { method, source } = error else {
        return None;
    };
    if method != "turn/steer" {
        return None;
    }
    if source.message == "no active turn to steer" {
        return Some(ActiveTurnSteerRace::Missing);
    }

    // App-server steer mismatches mean our cached active turn id is stale, but the response
    // includes the server's current active turn so we can resynchronize and retry once.
    let mismatch_prefix = "expected active turn id `";
    let mismatch_separator = "` but found `";
    let actual_turn_id = source
        .message
        .strip_prefix(mismatch_prefix)?
        .split_once(mismatch_separator)?
        .1
        .strip_suffix('`')?
        .to_string();
    Some(ActiveTurnSteerRace::ExpectedTurnMismatch { actual_turn_id })
}

impl App {
    pub fn chatwidget_init_for_forked_or_resumed_thread(
        &self,
        tui: &mut tui::Tui,
        cfg: crate::legacy_core::config::Config,
        initial_user_message: Option<crate::chatwidget::UserMessage>,
    ) -> crate::chatwidget::ChatWidgetInit {
        crate::chatwidget::ChatWidgetInit {
            config: cfg,
            display_preferences: self.display_preferences.clone(),
            frame_requester: tui.frame_requester(),
            app_event_tx: self.app_event_tx.clone(),
            workspace_command_runner: self.workspace_command_runner.clone(),
            initial_user_message,
            enhanced_keys_supported: self.enhanced_keys_supported,
            has_chatgpt_account: self.chat_widget.has_chatgpt_account(),
            model_catalog: self.model_catalog.clone(),
            feedback: self.feedback.clone(),
            is_first_run: false,
            status_account_display: self.chat_widget.status_account_display().cloned(),
            runtime_model_provider_base_url: self
                .chat_widget
                .runtime_model_provider_base_url()
                .map(str::to_string),
            initial_plan_type: self.chat_widget.current_plan_type(),
            model: Some(self.chat_widget.current_model().to_string()),
            startup_tooltip_override: None,
            status_line_invalid_items_warned: self.status_line_invalid_items_warned.clone(),
            terminal_title_invalid_items_warned: self.terminal_title_invalid_items_warned.clone(),
            session_telemetry: self.session_telemetry.clone(),
        }
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

#[cfg(any())]
mod tests {
    use super::*;
    mod btw_tests;
    mod clawbot_tests;

    use crate::app_backtrack::BacktrackSelection;
    use crate::app_backtrack::BacktrackState;
    use crate::app_backtrack::user_count;

    use crate::chatwidget::ChatWidgetInit;
    use crate::chatwidget::create_initial_user_message;
    use crate::chatwidget::tests::make_chatwidget_manual_with_sender;
    use crate::chatwidget::tests::render_bottom_popup;
    use crate::chatwidget::tests::set_chatgpt_auth;
    use crate::chatwidget::tests::set_fast_mode_test_catalog;
    use crate::file_search::FileSearchManager;
    use crate::history_cell::AgentMessageCell;
    use crate::history_cell::HistoryCell;
    use crate::history_cell::UserHistoryCell;
    use crate::history_cell::new_session_info;
    use crate::multi_agents::AgentPickerThreadEntry;
    use assert_matches::assert_matches;
    use codex_app_server_client::AppServerEvent;

    use crate::app_event::ClawbotForwardingChannel;
    use crate::legacy_core::config::ConfigBuilder;
    use crate::legacy_core::config::ConfigOverrides;
    use crate::render::renderable::Renderable;
    use codex_app_server_protocol::AdditionalFileSystemPermissions;
    use codex_app_server_protocol::AdditionalNetworkPermissions;
    use codex_app_server_protocol::AdditionalPermissionProfile;
    use codex_app_server_protocol::AgentMessageDeltaNotification;
    use codex_app_server_protocol::CommandExecutionRequestApprovalParams;
    use codex_app_server_protocol::ConfigWarningNotification;
    use codex_app_server_protocol::HookCompletedNotification;
    use codex_app_server_protocol::HookEventName as AppServerHookEventName;
    use codex_app_server_protocol::HookExecutionMode as AppServerHookExecutionMode;
    use codex_app_server_protocol::HookHandlerType as AppServerHookHandlerType;
    use codex_app_server_protocol::HookOutputEntry as AppServerHookOutputEntry;
    use codex_app_server_protocol::HookOutputEntryKind as AppServerHookOutputEntryKind;
    use codex_app_server_protocol::HookRunStatus as AppServerHookRunStatus;
    use codex_app_server_protocol::HookRunSummary as AppServerHookRunSummary;
    use codex_app_server_protocol::HookScope as AppServerHookScope;
    use codex_app_server_protocol::HookStartedNotification;
    use codex_app_server_protocol::ItemCompletedNotification;
    use codex_app_server_protocol::JSONRPCErrorError;
    use codex_app_server_protocol::NetworkApprovalContext as AppServerNetworkApprovalContext;
    use codex_app_server_protocol::NetworkApprovalProtocol as AppServerNetworkApprovalProtocol;
    use codex_app_server_protocol::NetworkPolicyAmendment as AppServerNetworkPolicyAmendment;
    use codex_app_server_protocol::NetworkPolicyRuleAction as AppServerNetworkPolicyRuleAction;
    use codex_app_server_protocol::NonSteerableTurnKind as AppServerNonSteerableTurnKind;
    use codex_app_server_protocol::PermissionsRequestApprovalParams;
    use codex_app_server_protocol::PluginMarketplaceEntry;
    use codex_app_server_protocol::RequestId as AppServerRequestId;
    use codex_app_server_protocol::ServerNotification;
    use codex_app_server_protocol::ServerRequest;
    use codex_app_server_protocol::Thread;
    use codex_app_server_protocol::ThreadArchivedNotification;
    use codex_app_server_protocol::ThreadClosedNotification;
    use codex_app_server_protocol::ThreadItem;
    use codex_app_server_protocol::ThreadStartedNotification;
    use codex_app_server_protocol::ThreadTokenUsage;
    use codex_app_server_protocol::ThreadTokenUsageUpdatedNotification;
    use codex_app_server_protocol::TokenUsageBreakdown;
    use codex_app_server_protocol::ToolRequestUserInputParams;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnCompletedNotification;
    use codex_app_server_protocol::TurnError as AppServerTurnError;
    use codex_app_server_protocol::TurnStartedNotification;
    use codex_app_server_protocol::TurnStatus;
    use codex_app_server_protocol::UserInput as AppServerUserInput;
    use codex_config::types::ModelAvailabilityNuxConfig;
    use codex_otel::SessionTelemetry;
    use codex_protocol::ThreadId;
    use codex_protocol::config_types::CollaborationMode;
    use codex_protocol::config_types::CollaborationModeMask;
    use codex_protocol::config_types::ModeKind;
    use codex_protocol::config_types::Settings;
    use codex_protocol::mcp::Tool;
    use codex_protocol::models::FileSystemPermissions;
    use codex_protocol::models::NetworkPermissions;
    use codex_protocol::models::PermissionProfile;
    use codex_protocol::openai_models::ModelAvailabilityNux;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::Event;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::McpAuthStatus;
    use codex_protocol::protocol::NetworkApprovalContext;
    use codex_protocol::protocol::NetworkApprovalProtocol;
    use codex_protocol::protocol::RolloutItem;
    use codex_protocol::protocol::RolloutLine;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::protocol::SessionConfiguredEvent;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::protocol::TurnContextItem;
    use codex_protocol::request_permissions::RequestPermissionProfile;
    use codex_protocol::user_input::TextElement;
    use codex_protocol::user_input::UserInput;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use crossterm::event::KeyModifiers;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::prelude::Line;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;
    use tokio::time;

    fn test_absolute_path(path: &str) -> AbsolutePathBuf {
        AbsolutePathBuf::try_from(PathBuf::from(path)).expect("absolute test path")
    }

    #[test]
    fn hide_cli_only_plugin_marketplaces_removes_openai_bundled() {
        let mut response = PluginListResponse {
            marketplaces: vec![
                PluginMarketplaceEntry {
                    name: "openai-bundled".to_string(),
                    path: test_absolute_path("/marketplaces/openai-bundled"),
                    interface: None,
                    plugins: Vec::new(),
                },
                PluginMarketplaceEntry {
                    name: "openai-curated".to_string(),
                    path: test_absolute_path("/marketplaces/openai-curated"),
                    interface: None,
                    plugins: Vec::new(),
                },
            ],
            marketplace_load_errors: Vec::new(),
            remote_sync_error: None,
            featured_plugin_ids: Vec::new(),
        };

        hide_cli_only_plugin_marketplaces(&mut response);

        assert_eq!(
            response.marketplaces,
            vec![PluginMarketplaceEntry {
                name: "openai-curated".to_string(),
                path: test_absolute_path("/marketplaces/openai-curated"),
                interface: None,
                plugins: Vec::new(),
            }]
        );
    }

    #[test]
    fn normalize_harness_overrides_resolves_relative_add_dirs() -> Result<()> {
        let temp_dir = tempdir()?;
        let base_cwd = temp_dir.path().join("base").abs();
        std::fs::create_dir_all(base_cwd.as_path())?;

        let overrides = ConfigOverrides {
            additional_writable_roots: vec![PathBuf::from("rel")],
            ..Default::default()
        };
        let normalized = normalize_harness_overrides_for_cwd(overrides, &base_cwd)?;

        assert_eq!(
            normalized.additional_writable_roots,
            vec![base_cwd.join("rel").into_path_buf()]
        );
        Ok(())
    }

    #[test]
    fn mcp_inventory_maps_prefix_tool_names_by_server() {
        let statuses = vec![
            McpServerStatus {
                name: "docs".to_string(),
                tools: HashMap::from([(
                    "list".to_string(),
                    Tool {
                        description: None,
                        name: "list".to_string(),
                        title: None,
                        input_schema: serde_json::json!({"type": "object"}),
                        output_schema: None,
                        annotations: None,
                        icons: None,
                        meta: None,
                    },
                )]),
                resources: Vec::new(),
                resource_templates: Vec::new(),
                auth_status: codex_app_server_protocol::McpAuthStatus::Unsupported,
            },
            McpServerStatus {
                name: "disabled".to_string(),
                tools: HashMap::new(),
                resources: Vec::new(),
                resource_templates: Vec::new(),
                auth_status: codex_app_server_protocol::McpAuthStatus::Unsupported,
            },
        ];

        let (tools, resources, resource_templates, auth_statuses) =
            mcp_inventory_maps_from_statuses(statuses);
        let mut resource_names = resources.keys().cloned().collect::<Vec<_>>();
        resource_names.sort();
        let mut template_names = resource_templates.keys().cloned().collect::<Vec<_>>();
        template_names.sort();

        assert_eq!(
            tools.keys().cloned().collect::<Vec<_>>(),
            vec!["mcp__docs__list".to_string()]
        );
        assert_eq!(resource_names, vec!["disabled", "docs"]);
        assert_eq!(template_names, vec!["disabled", "docs"]);
        assert_eq!(
            auth_statuses.get("disabled"),
            Some(&McpAuthStatus::Unsupported)
        );
    }

    #[tokio::test]
    async fn handle_mcp_inventory_result_clears_committed_loading_cell() {
        let mut app = make_test_app().await;
        app.transcript_cells
            .push(Arc::new(history_cell::new_mcp_inventory_loading(
                /*animations_enabled*/ false,
            )));

        app.handle_mcp_inventory_result(Ok(vec![McpServerStatus {
            name: "docs".to_string(),
            tools: HashMap::new(),
            resources: Vec::new(),
            resource_templates: Vec::new(),
            auth_status: codex_app_server_protocol::McpAuthStatus::Unsupported,
        }]));

        assert_eq!(app.transcript_cells.len(), 0);
    }

    #[test]
    fn startup_waiting_gate_is_only_for_fresh_or_exit_session_selection() {
        assert_eq!(
            App::should_wait_for_initial_session(&SessionSelection::StartFresh),
            true
        );
        assert_eq!(
            App::should_wait_for_initial_session(&SessionSelection::Exit),
            true
        );
        assert_eq!(
            App::should_wait_for_initial_session(&SessionSelection::Resume(
                crate::resume_picker::SessionTarget {
                    path: Some(PathBuf::from("/tmp/restore")),
                    thread_id: ThreadId::new(),
                }
            )),
            false
        );
        assert_eq!(
            App::should_wait_for_initial_session(&SessionSelection::Fork(
                crate::resume_picker::SessionTarget {
                    path: Some(PathBuf::from("/tmp/fork")),
                    thread_id: ThreadId::new(),
                }
            )),
            false
        );
    }

    #[test]
    fn startup_waiting_gate_holds_active_thread_events_until_primary_thread_configured() {
        let mut wait_for_initial_session =
            App::should_wait_for_initial_session(&SessionSelection::StartFresh);
        assert_eq!(wait_for_initial_session, true);
        assert_eq!(
            App::should_handle_active_thread_events(
                wait_for_initial_session,
                /*has_active_thread_receiver*/ true
            ),
            false
        );

        assert_eq!(
            App::should_stop_waiting_for_initial_session(
                wait_for_initial_session,
                /*primary_thread_id*/ None
            ),
            false
        );
        if App::should_stop_waiting_for_initial_session(
            wait_for_initial_session,
            Some(ThreadId::new()),
        ) {
            wait_for_initial_session = false;
        }
        assert_eq!(wait_for_initial_session, false);

        assert_eq!(
            App::should_handle_active_thread_events(
                wait_for_initial_session,
                /*has_active_thread_receiver*/ true
            ),
            true
        );
    }

    #[test]
    fn startup_waiting_gate_not_applied_for_resume_or_fork_session_selection() {
        let wait_for_resume = App::should_wait_for_initial_session(&SessionSelection::Resume(
            crate::resume_picker::SessionTarget {
                path: Some(PathBuf::from("/tmp/restore")),
                thread_id: ThreadId::new(),
            },
        ));
        assert_eq!(
            App::should_handle_active_thread_events(
                wait_for_resume,
                /*has_active_thread_receiver*/ true
            ),
            true
        );
        let wait_for_fork = App::should_wait_for_initial_session(&SessionSelection::Fork(
            crate::resume_picker::SessionTarget {
                path: Some(PathBuf::from("/tmp/fork")),
                thread_id: ThreadId::new(),
            },
        ));
        assert_eq!(
            App::should_handle_active_thread_events(
                wait_for_fork,
                /*has_active_thread_receiver*/ true
            ),
            true
        );
    }

    #[tokio::test]
    async fn ignore_same_thread_resume_reports_noop_for_current_thread() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.thread_event_channels.insert(
            thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                session,
                Vec::new(),
            ),
        );
        app.activate_thread_channel(thread_id).await;
        while app_event_rx.try_recv().is_ok() {}

        let ignored = app.ignore_same_thread_resume(&crate::resume_picker::SessionTarget {
            path: Some(test_path_buf("/tmp/project")),
            thread_id,
        });

        assert!(ignored);
        let cell = match app_event_rx.try_recv() {
            Ok(AppEvent::InsertHistoryCell(cell)) => cell,
            other => panic!("expected info message after same-thread resume, saw {other:?}"),
        };
        let rendered = lines_to_single_string(&cell.display_lines(/*width*/ 80));
        assert!(rendered.contains(&format!(
            "Already viewing {}.",
            test_path_display("/tmp/project")
        )));
    }

    #[tokio::test]
    async fn ignore_same_thread_resume_allows_reattaching_displayed_inactive_thread() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session);

        let ignored = app.ignore_same_thread_resume(&crate::resume_picker::SessionTarget {
            path: Some(test_path_buf("/tmp/project")),
            thread_id,
        });

        assert!(!ignored);
        assert!(app.transcript_cells.is_empty());
    }

    #[tokio::test]
    async fn enqueue_primary_thread_session_replays_buffered_approval_after_attach() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let approval_request =
            exec_approval_request(thread_id, "turn-1", "call-1", /*approval_id*/ None);

        assert_eq!(
            app.pending_app_server_requests
                .note_server_request(&approval_request),
            None
        );
        app.enqueue_primary_thread_request(approval_request).await?;
        app.enqueue_primary_thread_session(
            test_thread_session(thread_id, test_path_buf("/tmp/project")),
            Vec::new(),
        )
        .await?;

        let rx = app
            .active_thread_rx
            .as_mut()
            .expect("primary thread receiver should be active");
        let event = time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("timed out waiting for buffered approval event")
            .expect("channel closed unexpectedly");

        assert!(matches!(
            &event,
            ThreadBufferedEvent::Request(ServerRequest::CommandExecutionRequestApproval {
                params,
                ..
            }) if params.turn_id == "turn-1"
        ));

        app.handle_thread_event_now(event);
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        while let Ok(app_event) = app_event_rx.try_recv() {
            if let AppEvent::SubmitThreadOp {
                thread_id: op_thread_id,
                ..
            } = app_event
            {
                assert_eq!(op_thread_id, thread_id);
                return Ok(());
            }
        }

        panic!("expected approval action to submit a thread-scoped op");
    }

    #[tokio::test]
    async fn resolved_buffered_approval_does_not_become_actionable_after_drain() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let approval_request =
            exec_approval_request(thread_id, "turn-1", "call-1", /*approval_id*/ None);

        app.enqueue_primary_thread_session(
            test_thread_session(thread_id, test_path_buf("/tmp/project")),
            Vec::new(),
        )
        .await?;
        while app_event_rx.try_recv().is_ok() {}

        assert_eq!(
            app.pending_app_server_requests
                .note_server_request(&approval_request),
            None
        );
        app.enqueue_thread_request(thread_id, approval_request)
            .await?;

        let resolved = app
            .pending_app_server_requests
            .resolve_notification(&AppServerRequestId::Integer(1))
            .expect("matching app-server request should resolve");
        app.chat_widget.dismiss_app_server_request(&resolved);
        while app_event_rx.try_recv().is_ok() {}

        let rx = app
            .active_thread_rx
            .as_mut()
            .expect("primary thread receiver should be active");
        let event = time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("timed out waiting for buffered approval event")
            .expect("channel closed unexpectedly");

        assert!(matches!(
            &event,
            ThreadBufferedEvent::Request(ServerRequest::CommandExecutionRequestApproval {
                params,
                ..
            }) if params.turn_id == "turn-1"
        ));

        app.handle_thread_event_now(event);
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        while let Ok(app_event) = app_event_rx.try_recv() {
            assert!(
                !matches!(app_event, AppEvent::SubmitThreadOp { .. }),
                "resolved buffered approval should not become actionable"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn enqueue_primary_thread_session_replays_turns_before_initial_prompt_submit()
    -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let initial_prompt = "follow-up after replay".to_string();
        let config = app.config.clone();
        let model = crate::legacy_core::test_support::get_model_offline(config.model.as_deref());
        app.chat_widget = ChatWidget::new_with_app_event(ChatWidgetInit {
            config,
            display_preferences: app.display_preferences.clone(),
            frame_requester: crate::tui::FrameRequester::test_dummy(),
            app_event_tx: app.app_event_tx.clone(),
            initial_user_message: create_initial_user_message(
                Some(initial_prompt.clone()),
                Vec::new(),
                Vec::new(),
            ),
            enhanced_keys_supported: false,
            has_chatgpt_account: false,
            model_catalog: app.model_catalog.clone(),
            feedback: codex_feedback::CodexFeedback::new(),
            is_first_run: false,
            status_account_display: None,
            initial_plan_type: None,
            model: Some(model),
            startup_tooltip_override: None,
            status_line_invalid_items_warned: app.status_line_invalid_items_warned.clone(),
            terminal_title_invalid_items_warned: app.terminal_title_invalid_items_warned.clone(),
            session_telemetry: app.session_telemetry.clone(),
        });

        app.enqueue_primary_thread_session(
            test_thread_session(thread_id, test_path_buf("/tmp/project")),
            vec![test_turn(
                "turn-1",
                TurnStatus::Completed,
                vec![ThreadItem::UserMessage {
                    id: "user-1".to_string(),
                    content: vec![AppServerUserInput::Text {
                        text: "earlier prompt".to_string(),
                        text_elements: Vec::new(),
                    }],
                }],
            )],
        )
        .await?;

        let mut saw_replayed_answer = false;
        let mut submitted_items = None;
        while let Ok(event) = app_event_rx.try_recv() {
            match event {
                AppEvent::InsertHistoryCell(cell) => {
                    let transcript = lines_to_single_string(&cell.transcript_lines(/*width*/ 80));
                    saw_replayed_answer |= transcript.contains("earlier prompt");
                }
                AppEvent::SubmitThreadOp {
                    thread_id: op_thread_id,
                    op: Op::UserTurn { items, .. },
                } => {
                    assert_eq!(op_thread_id, thread_id);
                    submitted_items = Some(items);
                }
                AppEvent::CodexOp(Op::UserTurn { items, .. }) => {
                    submitted_items = Some(items);
                }
                _ => {}
            }
        }
        assert!(
            saw_replayed_answer,
            "expected replayed history before initial prompt submit"
        );
        assert_eq!(
            submitted_items,
            Some(vec![UserInput::Text {
                text: initial_prompt,
                text_elements: Vec::new(),
            }])
        );

        Ok(())
    }

    #[tokio::test]
    async fn reset_thread_event_state_aborts_listener_tasks() {
        struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

        impl Drop for NotifyOnDrop {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }

        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _notify_on_drop = NotifyOnDrop(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        app.thread_event_listener_tasks.insert(thread_id, handle);
        started_rx
            .await
            .expect("listener task should report it started");

        app.reset_thread_event_state();

        assert_eq!(app.thread_event_listener_tasks.is_empty(), true);
        time::timeout(Duration::from_millis(50), dropped_rx)
            .await
            .expect("timed out waiting for listener task abort")
            .expect("listener task drop notification should succeed");
    }

    #[tokio::test]
    async fn enqueue_thread_event_does_not_block_when_channel_full() -> Result<()> {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        app.set_thread_active(thread_id, /*active*/ true).await;

        let event = thread_closed_notification(thread_id);

        app.enqueue_thread_notification(thread_id, event.clone())
            .await?;
        time::timeout(
            Duration::from_millis(50),
            app.enqueue_thread_notification(thread_id, event),
        )
        .await
        .expect("enqueue_thread_notification blocked on a full channel")?;

        let mut rx = app
            .thread_event_channels
            .get_mut(&thread_id)
            .expect("missing thread channel")
            .receiver
            .take()
            .expect("missing receiver");

        time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("timed out waiting for first event")
            .expect("channel closed unexpectedly");
        time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .expect("timed out waiting for second event")
            .expect("channel closed unexpectedly");

        Ok(())
    }

    #[tokio::test]
    async fn replay_thread_snapshot_restores_draft_and_queued_input() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.thread_event_channels.insert(
            thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                session.clone(),
                Vec::new(),
            ),
        );
        app.activate_thread_channel(thread_id).await;
        app.chat_widget.handle_thread_session(session.clone());

        app.chat_widget
            .apply_external_edit("draft prompt".to_string());
        app.chat_widget.submit_user_message_with_mode(
            "queued follow-up".to_string(),
            CollaborationModeMask {
                name: "Default".to_string(),
                mode: None,
                model: None,
                reasoning_effort: None,
                developer_instructions: None,
            },
        );
        let expected_input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected thread input state");

        app.store_active_thread_receiver().await;

        let snapshot = {
            let channel = app
                .thread_event_channels
                .get(&thread_id)
                .expect("thread channel should exist");
            let store = channel.store.lock().await;
            assert_eq!(store.input_state, Some(expected_input_state));
            store.snapshot()
        };

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;

        app.replay_thread_snapshot(snapshot, /*resume_restored_queue*/ true);

        assert_eq!(app.chat_widget.composer_text_with_pending(), "draft prompt");
        assert!(app.chat_widget.queued_user_message_texts().is_empty());
        while let Ok(op) = new_op_rx.try_recv() {
            assert!(
                !matches!(op, Op::UserTurn { .. }),
                "draft-only replay should not auto-submit queued input"
            );
        }
    }

    #[tokio::test]
    async fn active_turn_id_for_thread_uses_snapshot_turns() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.thread_event_channels.insert(
            thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                session,
                vec![test_turn("turn-1", TurnStatus::InProgress, Vec::new())],
            ),
        );

        assert_eq!(
            app.active_turn_id_for_thread(thread_id).await,
            Some("turn-1".to_string())
        );
    }

    #[tokio::test]
    async fn replayed_turn_complete_submits_restored_queued_follow_up() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget.handle_server_notification(
            turn_started_notification(thread_id, "turn-1"),
            /*replay_kind*/ None,
        );
        app.chat_widget.handle_server_notification(
            agent_message_delta_notification(thread_id, "turn-1", "agent-1", "streaming"),
            /*replay_kind*/ None,
        );
        app.chat_widget
            .apply_external_edit("queued follow-up".to_string());
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected queued follow-up state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        while new_op_rx.try_recv().is_ok() {}
        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![ThreadBufferedEvent::Notification(
                    turn_completed_notification(thread_id, "turn-1", TurnStatus::Completed),
                )],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );

        match next_user_turn_op(&mut new_op_rx) {
            Op::UserTurn { items, .. } => assert_eq!(
                items,
                vec![UserInput::Text {
                    text: "queued follow-up".to_string(),
                    text_elements: Vec::new(),
                }]
            ),
            other => panic!("expected queued follow-up submission, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_only_thread_keeps_restored_queue_visible() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget.handle_server_notification(
            turn_started_notification(thread_id, "turn-1"),
            /*replay_kind*/ None,
        );
        app.chat_widget.handle_server_notification(
            agent_message_delta_notification(thread_id, "turn-1", "agent-1", "streaming"),
            /*replay_kind*/ None,
        );
        app.chat_widget
            .apply_external_edit("queued follow-up".to_string());
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected queued follow-up state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        while new_op_rx.try_recv().is_ok() {}

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![ThreadBufferedEvent::Notification(
                    turn_completed_notification(thread_id, "turn-1", TurnStatus::Completed),
                )],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ false,
        );

        assert_eq!(
            app.chat_widget.queued_user_message_texts(),
            vec!["queued follow-up".to_string()]
        );
        assert!(
            new_op_rx.try_recv().is_err(),
            "replay-only threads should not auto-submit restored queue"
        );
    }

    #[tokio::test]
    async fn replay_thread_snapshot_keeps_queue_when_running_state_only_comes_from_snapshot() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget.handle_server_notification(
            turn_started_notification(thread_id, "turn-1"),
            /*replay_kind*/ None,
        );
        app.chat_widget.handle_server_notification(
            agent_message_delta_notification(thread_id, "turn-1", "agent-1", "streaming"),
            /*replay_kind*/ None,
        );
        app.chat_widget
            .apply_external_edit("queued follow-up".to_string());
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected queued follow-up state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        while new_op_rx.try_recv().is_ok() {}

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );

        assert_eq!(
            app.chat_widget.queued_user_message_texts(),
            vec!["queued follow-up".to_string()]
        );
        assert!(
            new_op_rx.try_recv().is_err(),
            "restored queue should stay queued when replay did not prove the turn finished"
        );
    }

    #[tokio::test]
    async fn replay_thread_snapshot_in_progress_turn_restores_running_queue_state() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget.handle_server_notification(
            turn_started_notification(thread_id, "turn-1"),
            /*replay_kind*/ None,
        );
        app.chat_widget.handle_server_notification(
            agent_message_delta_notification(thread_id, "turn-1", "agent-1", "streaming"),
            /*replay_kind*/ None,
        );
        app.chat_widget
            .apply_external_edit("queued follow-up".to_string());
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected queued follow-up state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        while new_op_rx.try_recv().is_ok() {}

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: vec![test_turn("turn-1", TurnStatus::InProgress, Vec::new())],
                events: Vec::new(),
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );

        assert_eq!(
            app.chat_widget.queued_user_message_texts(),
            vec!["queued follow-up".to_string()]
        );
        assert!(
            new_op_rx.try_recv().is_err(),
            "restored queue should stay queued while replayed turn is still running"
        );
    }

    #[tokio::test]
    async fn replay_thread_snapshot_in_progress_turn_restores_running_state_without_input_state() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        let (chat_widget, _app_event_tx, _rx, _new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session);

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: vec![test_turn("turn-1", TurnStatus::InProgress, Vec::new())],
                events: Vec::new(),
                input_state: None,
            },
            /*resume_restored_queue*/ false,
        );

        assert!(app.chat_widget.is_task_running_for_test());
    }

    #[tokio::test]
    async fn replay_thread_snapshot_does_not_submit_queue_before_replay_catches_up() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget.handle_server_notification(
            turn_started_notification(thread_id, "turn-1"),
            /*replay_kind*/ None,
        );
        app.chat_widget.handle_server_notification(
            agent_message_delta_notification(thread_id, "turn-1", "agent-1", "streaming"),
            /*replay_kind*/ None,
        );
        app.chat_widget
            .apply_external_edit("queued follow-up".to_string());
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected queued follow-up state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        while new_op_rx.try_recv().is_ok() {}

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![
                    ThreadBufferedEvent::Notification(turn_completed_notification(
                        thread_id,
                        "turn-0",
                        TurnStatus::Completed,
                    )),
                    ThreadBufferedEvent::Notification(turn_started_notification(
                        thread_id, "turn-1",
                    )),
                ],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );

        assert!(
            new_op_rx.try_recv().is_err(),
            "queued follow-up should stay queued until the latest turn completes"
        );
        assert_eq!(
            app.chat_widget.queued_user_message_texts(),
            vec!["queued follow-up".to_string()]
        );

        app.chat_widget.handle_server_notification(
            turn_completed_notification(thread_id, "turn-1", TurnStatus::Completed),
            /*replay_kind*/ None,
        );

        match next_user_turn_op(&mut new_op_rx) {
            Op::UserTurn { items, .. } => assert_eq!(
                items,
                vec![UserInput::Text {
                    text: "queued follow-up".to_string(),
                    text_elements: Vec::new(),
                }]
            ),
            other => panic!("expected queued follow-up submission, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_thread_snapshot_restores_pending_pastes_for_submit() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.thread_event_channels.insert(
            thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                session.clone(),
                Vec::new(),
            ),
        );
        app.activate_thread_channel(thread_id).await;
        app.chat_widget.handle_thread_session(session);

        let large = "x".repeat(1005);
        app.chat_widget.handle_paste(large.clone());
        let expected_input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected thread input state");

        app.store_active_thread_receiver().await;

        let snapshot = {
            let channel = app
                .thread_event_channels
                .get(&thread_id)
                .expect("thread channel should exist");
            let store = channel.store.lock().await;
            assert_eq!(store.input_state, Some(expected_input_state));
            store.snapshot()
        };

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.replay_thread_snapshot(snapshot, /*resume_restored_queue*/ true);

        assert_eq!(app.chat_widget.composer_text_with_pending(), large);

        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        match next_user_turn_op(&mut new_op_rx) {
            Op::UserTurn { items, .. } => assert_eq!(
                items,
                vec![UserInput::Text {
                    text: large,
                    text_elements: Vec::new(),
                }]
            ),
            other => panic!("expected restored paste submission, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_thread_snapshot_restores_collaboration_mode_for_draft_submit() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::High));
        app.chat_widget
            .set_collaboration_mask(CollaborationModeMask {
                name: "Plan".to_string(),
                mode: Some(ModeKind::Plan),
                model: Some("gpt-restored".to_string()),
                reasoning_effort: Some(Some(ReasoningEffortConfig::High)),
                developer_instructions: None,
            });
        app.chat_widget
            .apply_external_edit("draft prompt".to_string());
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected draft input state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::Low));
        app.chat_widget
            .set_collaboration_mask(CollaborationModeMask {
                name: "Default".to_string(),
                mode: Some(ModeKind::Default),
                model: Some("gpt-replacement".to_string()),
                reasoning_effort: Some(Some(ReasoningEffortConfig::Low)),
                developer_instructions: None,
            });
        while new_op_rx.try_recv().is_ok() {}

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        match next_user_turn_op(&mut new_op_rx) {
            Op::UserTurn {
                items,
                model,
                effort,
                collaboration_mode,
                ..
            } => {
                assert_eq!(
                    items,
                    vec![UserInput::Text {
                        text: "draft prompt".to_string(),
                        text_elements: Vec::new(),
                    }]
                );
                assert_eq!(model, "gpt-restored".to_string());
                assert_eq!(effort, Some(ReasoningEffortConfig::High));
                assert_eq!(
                    collaboration_mode,
                    Some(CollaborationMode {
                        mode: ModeKind::Plan,
                        settings: Settings {
                            model: "gpt-restored".to_string(),
                            reasoning_effort: Some(ReasoningEffortConfig::High),
                            developer_instructions: None,
                        },
                    })
                );
            }
            other => panic!("expected restored draft submission, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_thread_snapshot_restores_collaboration_mode_without_input() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::High));
        app.chat_widget
            .set_collaboration_mask(CollaborationModeMask {
                name: "Plan".to_string(),
                mode: Some(ModeKind::Plan),
                model: Some("gpt-restored".to_string()),
                reasoning_effort: Some(Some(ReasoningEffortConfig::High)),
                developer_instructions: None,
            });
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected collaboration-only input state");

        let (chat_widget, _app_event_tx, _rx, _new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::Low));
        app.chat_widget
            .set_collaboration_mask(CollaborationModeMask {
                name: "Default".to_string(),
                mode: Some(ModeKind::Default),
                model: Some("gpt-replacement".to_string()),
                reasoning_effort: Some(Some(ReasoningEffortConfig::Low)),
                developer_instructions: None,
            });

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );

        assert_eq!(
            app.chat_widget.active_collaboration_mode_kind(),
            ModeKind::Plan
        );
        assert_eq!(app.chat_widget.current_model(), "gpt-restored");
        assert_eq!(
            app.chat_widget.current_reasoning_effort(),
            Some(ReasoningEffortConfig::High)
        );
    }

    #[tokio::test]
    async fn replayed_interrupted_turn_restores_queued_input_to_composer() {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        app.chat_widget.handle_thread_session(session.clone());
        app.chat_widget.handle_server_notification(
            turn_started_notification(thread_id, "turn-1"),
            /*replay_kind*/ None,
        );
        app.chat_widget.handle_server_notification(
            agent_message_delta_notification(thread_id, "turn-1", "agent-1", "streaming"),
            /*replay_kind*/ None,
        );
        app.chat_widget
            .apply_external_edit("queued follow-up".to_string());
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let input_state = app
            .chat_widget
            .capture_thread_input_state()
            .expect("expected queued follow-up state");

        let (chat_widget, _app_event_tx, _rx, mut new_op_rx) =
            make_chatwidget_manual_with_sender().await;
        app.chat_widget = chat_widget;
        app.chat_widget.handle_thread_session(session.clone());
        while new_op_rx.try_recv().is_ok() {}

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![ThreadBufferedEvent::Notification(
                    turn_completed_notification(thread_id, "turn-1", TurnStatus::Interrupted),
                )],
                input_state: Some(input_state),
            },
            /*resume_restored_queue*/ true,
        );

        assert_eq!(
            app.chat_widget.composer_text_with_pending(),
            "queued follow-up"
        );
        assert!(app.chat_widget.queued_user_message_texts().is_empty());
        assert!(
            new_op_rx.try_recv().is_err(),
            "replayed interrupted turns should restore queued input for editing, not submit it"
        );
    }

    #[tokio::test]
    async fn token_usage_update_refreshes_status_line_with_runtime_context_window() {
        let mut app = make_test_app().await;
        app.chat_widget
            .setup_status_line(vec![crate::bottom_pane::StatusLineItem::ContextWindowSize]);

        assert_eq!(app.chat_widget.status_line_text(), None);

        app.handle_thread_event_now(ThreadBufferedEvent::Notification(token_usage_notification(
            ThreadId::new(),
            "turn-1",
            Some(950_000),
        )));

        assert_eq!(
            app.chat_widget.status_line_text(),
            Some("950K window".into())
        );
    }

    #[tokio::test]
    async fn open_agent_picker_keeps_missing_threads_for_replay() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let thread_id = ThreadId::new();
        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));

        app.open_agent_picker(&mut app_server).await;

        assert_eq!(app.thread_event_channels.contains_key(&thread_id), true);
        assert_eq!(
            app.agent_navigation.get(&thread_id),
            Some(&AgentPickerThreadEntry {
                agent_nickname: None,
                agent_role: None,
                is_closed: true,
            })
        );
        assert_eq!(app.agent_navigation.ordered_thread_ids(), vec![thread_id]);
        Ok(())
    }

    #[tokio::test]
    async fn open_agent_picker_preserves_cached_metadata_for_replay_threads() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let thread_id = ThreadId::new();
        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        app.agent_navigation.upsert(
            thread_id,
            Some("Robie".to_string()),
            Some("explorer".to_string()),
            /*is_closed*/ true,
        );

        app.open_agent_picker(&mut app_server).await;

        assert_eq!(app.thread_event_channels.contains_key(&thread_id), true);
        assert_eq!(
            app.agent_navigation.get(&thread_id),
            Some(&AgentPickerThreadEntry {
                agent_nickname: Some("Robie".to_string()),
                agent_role: Some("explorer".to_string()),
                is_closed: true,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn open_agent_picker_prunes_terminal_metadata_only_threads() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let thread_id = ThreadId::new();
        app.agent_navigation.upsert(
            thread_id,
            Some("Ghost".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );

        app.open_agent_picker(&mut app_server).await;

        assert_eq!(app.agent_navigation.get(&thread_id), None);
        assert!(app.agent_navigation.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn open_agent_picker_marks_terminal_read_errors_closed() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let thread_id = ThreadId::new();
        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        app.agent_navigation.upsert(
            thread_id,
            Some("Robie".to_string()),
            Some("explorer".to_string()),
            /*is_closed*/ false,
        );

        app.open_agent_picker(&mut app_server).await;

        assert_eq!(
            app.agent_navigation.get(&thread_id),
            Some(&AgentPickerThreadEntry {
                agent_nickname: Some("Robie".to_string()),
                agent_role: Some("explorer".to_string()),
                is_closed: true,
            })
        );
        Ok(())
    }

    #[test]
    fn terminal_thread_read_error_detection_matches_not_loaded_errors() {
        let err = color_eyre::eyre::eyre!(
            "thread/read failed during TUI session lookup: thread/read failed: thread not loaded: thr_123"
        );

        assert!(App::is_terminal_thread_read_error(&err));
    }

    #[test]
    fn terminal_thread_read_error_detection_ignores_transient_failures() {
        let err = color_eyre::eyre::eyre!(
            "thread/read failed during TUI session lookup: thread/read transport error: broken pipe"
        );

        assert!(!App::is_terminal_thread_read_error(&err));
    }

    #[test]
    fn closed_state_for_thread_read_error_preserves_live_state_without_cache_on_transient_error() {
        let err = color_eyre::eyre::eyre!(
            "thread/read failed during TUI session lookup: thread/read transport error: broken pipe"
        );

        assert!(!App::closed_state_for_thread_read_error(
            &err, /*existing_is_closed*/ None
        ));
    }

    #[test]
    fn closed_state_for_thread_read_error_marks_terminal_uncached_threads_closed() {
        let err = color_eyre::eyre::eyre!(
            "thread/read failed during TUI session lookup: thread/read failed: thread not loaded: thr_123"
        );

        assert!(App::closed_state_for_thread_read_error(
            &err, /*existing_is_closed*/ None
        ));
    }

    #[test]
    fn closed_state_for_thread_read_status_marks_not_loaded_picker_threads_closed() {
        assert!(App::closed_state_for_thread_read_status(
            &codex_app_server_protocol::ThreadStatus::NotLoaded,
            ThreadLivenessRefreshMode::Picker,
        ));
    }

    #[test]
    fn closed_state_for_thread_read_status_keeps_not_loaded_selection_threads_open() {
        assert!(!App::closed_state_for_thread_read_status(
            &codex_app_server_protocol::ThreadStatus::NotLoaded,
            ThreadLivenessRefreshMode::Selection,
        ));
    }

    #[test]
    fn include_turns_fallback_detection_handles_unmaterialized_and_ephemeral_threads() {
        let unmaterialized = color_eyre::eyre::eyre!(
            "thread/read failed during TUI session lookup: thread/read failed: thread thr_123 is not materialized yet; includeTurns is unavailable before first user message"
        );
        let ephemeral = color_eyre::eyre::eyre!(
            "thread/read failed during TUI session lookup: thread/read failed: ephemeral threads do not support includeTurns"
        );

        assert!(App::can_fallback_from_include_turns_error(&unmaterialized));
        assert!(App::can_fallback_from_include_turns_error(&ephemeral));
    }

    #[tokio::test]
    async fn open_agent_picker_marks_loaded_threads_open() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        let thread_id = started.session.thread_id;
        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));

        app.open_agent_picker(&mut app_server).await;

        assert_eq!(
            app.agent_navigation.get(&thread_id),
            Some(&AgentPickerThreadEntry {
                agent_nickname: None,
                agent_role: None,
                is_closed: false,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn archived_thread_notifications_do_not_recreate_live_subagent_state() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let primary_thread_id = ThreadId::new();
        let archived_thread_id = ThreadId::new();
        app.primary_thread_id = Some(primary_thread_id);
        app.active_thread_id = Some(archived_thread_id);
        app.thread_event_channels
            .insert(archived_thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        app.agent_navigation.upsert(
            archived_thread_id,
            Some("Ghost".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );

        app.enqueue_thread_notification(
            archived_thread_id,
            ServerNotification::ThreadArchived(ThreadArchivedNotification {
                thread_id: archived_thread_id.to_string(),
            }),
        )
        .await?;

        assert_eq!(app.agent_navigation.get(&archived_thread_id), None);
        assert!(!app.thread_event_channels.contains_key(&archived_thread_id));
        assert_matches!(
            app_event_rx.try_recv(),
            Ok(AppEvent::SelectAgentThread(thread_id)) if thread_id == primary_thread_id
        );
        Ok(())
    }

    #[tokio::test]
    async fn archived_thread_rollout_path_detection_matches_archived_sessions_root() {
        let app = make_test_app().await;
        let archived_path = app
            .config
            .codex_home
            .join(codex_rollout::ARCHIVED_SESSIONS_SUBDIR)
            .join("2026/04/21/rollout-test.jsonl");
        let live_path = app
            .config
            .codex_home
            .join("sessions/2026/04/21/rollout-test.jsonl");

        assert!(app.is_archived_thread_rollout_path(Some(archived_path.as_path())));
        assert!(!app.is_archived_thread_rollout_path(Some(live_path.as_path())));
        assert!(!app.is_archived_thread_rollout_path(None));
    }

    #[tokio::test]
    async fn attach_live_thread_for_selection_rejects_empty_non_ephemeral_fallback_threads()
    -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        let thread_id = started.session.thread_id;
        app.agent_navigation.upsert(
            thread_id,
            Some("Scout".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );

        let err = app
            .attach_live_thread_for_selection(&mut app_server, thread_id)
            .await
            .expect_err("empty fallback should not attach as a blank replay-only thread");

        assert_eq!(
            err.to_string(),
            format!("Agent thread {thread_id} is not yet available for replay or live attach.")
        );
        assert!(!app.thread_event_channels.contains_key(&thread_id));
        Ok(())
    }

    #[tokio::test]
    async fn attach_live_thread_for_selection_rejects_unmaterialized_fallback_threads() -> Result<()>
    {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let mut ephemeral_config = app.chat_widget.config_ref().clone();
        ephemeral_config.ephemeral = true;
        let started = app_server.start_thread(&ephemeral_config).await?;
        let thread_id = started.session.thread_id;
        app.agent_navigation.upsert(
            thread_id,
            Some("Scout".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );

        let err = app
            .attach_live_thread_for_selection(&mut app_server, thread_id)
            .await
            .expect_err("ephemeral fallback should not attach as a blank live thread");

        assert_eq!(
            err.to_string(),
            format!("Agent thread {thread_id} is not yet available for replay or live attach.")
        );
        assert!(!app.thread_event_channels.contains_key(&thread_id));
        Ok(())
    }

    #[tokio::test]
    async fn should_attach_live_thread_for_selection_skips_closed_metadata_only_threads() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        app.agent_navigation.upsert(
            thread_id,
            Some("Ghost".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ true,
        );

        assert!(!app.should_attach_live_thread_for_selection(thread_id));

        app.agent_navigation.upsert(
            thread_id,
            Some("Ghost".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );
        assert!(app.should_attach_live_thread_for_selection(thread_id));

        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        assert!(!app.should_attach_live_thread_for_selection(thread_id));
    }

    #[tokio::test]
    async fn refresh_agent_picker_thread_liveness_prunes_closed_metadata_only_threads() -> Result<()>
    {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let thread_id = ThreadId::new();
        app.agent_navigation.upsert(
            thread_id,
            Some("Ghost".to_string()),
            Some("worker".to_string()),
            /*is_closed*/ false,
        );

        let is_available = app
            .refresh_agent_picker_thread_liveness(
                &mut app_server,
                thread_id,
                ThreadLivenessRefreshMode::Picker,
            )
            .await;

        assert!(!is_available);
        assert_eq!(app.agent_navigation.get(&thread_id), None);
        assert!(!app.thread_event_channels.contains_key(&thread_id));
        Ok(())
    }

    #[tokio::test]
    async fn refresh_agent_picker_thread_liveness_keeps_unloaded_selection_targets_attachable()
    -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        let thread_id = started.session.thread_id;
        app_server.thread_unsubscribe(thread_id).await?;

        let is_available = app
            .refresh_agent_picker_thread_liveness(
                &mut app_server,
                thread_id,
                ThreadLivenessRefreshMode::Selection,
            )
            .await;

        assert!(is_available);
        assert!(app.agent_navigation.get(&thread_id).is_none());
        assert!(app.should_attach_live_thread_for_selection(thread_id));
        Ok(())
    }

    #[tokio::test]
    async fn open_agent_picker_prompts_to_enable_multi_agent_when_disabled() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let _ = app.config.features.disable(Feature::Collab);

        app.open_agent_picker(&mut app_server).await;
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_matches!(
            app_event_rx.try_recv(),
            Ok(AppEvent::UpdateFeatureFlags { updates }) if updates == vec![(Feature::Collab, true)]
        );
        let cell = match app_event_rx.try_recv() {
            Ok(AppEvent::InsertHistoryCell(cell)) => cell,
            other => panic!("expected InsertHistoryCell event, got {other:?}"),
        };
        let rendered = cell
            .display_lines(/*width*/ 120)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Subagents will be enabled in the next session."));
        Ok(())
    }

    #[tokio::test]
    async fn update_memory_settings_persists_and_updates_widget_config() -> Result<()> {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let mut app_server = crate::start_embedded_app_server_for_picker(&app.config).await?;

        app.update_memory_settings_with_app_server(
            &mut app_server,
            /*use_memories*/ false,
            /*generate_memories*/ false,
        )
        .await;

        assert!(!app.config.memories.use_memories);
        assert!(!app.config.memories.generate_memories);
        assert!(!app.chat_widget.config_ref().memories.use_memories);
        assert!(!app.chat_widget.config_ref().memories.generate_memories);

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        let config_value = toml::from_str::<TomlValue>(&config)?;
        let memories = config_value
            .as_table()
            .and_then(|table| table.get("memories"))
            .and_then(TomlValue::as_table)
            .expect("memories table should exist");
        assert_eq!(
            memories.get("use_memories"),
            Some(&TomlValue::Boolean(false))
        );
        assert_eq!(
            memories.get("generate_memories"),
            Some(&TomlValue::Boolean(false))
        );
        assert!(
            !memories.contains_key("no_memories_if_mcp_or_web_search"),
            "the TUI menu should not write the MCP pollution setting"
        );
        app_server.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn update_memory_settings_updates_current_thread_memory_mode() -> Result<()> {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        // Seed the previous setting so this test exercises the thread-mode update path.
        app.config.memories.generate_memories = true;

        let mut app_server = crate::start_embedded_app_server_for_picker(&app.config).await?;
        let started = app_server.start_thread(&app.config).await?;
        let thread_id = started.session.thread_id;
        app.active_thread_id = Some(thread_id);

        app.update_memory_settings_with_app_server(
            &mut app_server,
            /*use_memories*/ true,
            /*generate_memories*/ false,
        )
        .await;

        let state_db = codex_state::StateRuntime::init(
            codex_home.path().to_path_buf(),
            app.config.model_provider_id.clone(),
        )
        .await
        .expect("state db should initialize");
        let memory_mode = state_db
            .get_thread_memory_mode(thread_id)
            .await
            .expect("thread memory mode should be readable");
        assert_eq!(memory_mode.as_deref(), Some("disabled"));

        app_server.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn reset_memories_clears_local_memory_directories() -> Result<()> {
        let (mut app, _app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        app.config.sqlite_home = codex_home.path().to_path_buf();

        let memory_root = codex_home.path().join("memories");
        let extensions_root = codex_home.path().join("memories_extensions");
        std::fs::create_dir_all(memory_root.join("rollout_summaries"))?;
        std::fs::create_dir_all(&extensions_root)?;
        std::fs::write(memory_root.join("MEMORY.md"), "stale memory\n")?;
        std::fs::write(
            memory_root.join("rollout_summaries").join("stale.md"),
            "stale summary\n",
        )?;
        std::fs::write(extensions_root.join("stale.txt"), "stale extension\n")?;

        let mut app_server = crate::start_embedded_app_server_for_picker(&app.config).await?;

        app.reset_memories_with_app_server(&mut app_server).await;

        assert_eq!(std::fs::read_dir(&memory_root)?.count(), 0);
        assert_eq!(std::fs::read_dir(&extensions_root)?.count(), 0);

        app_server.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_enabling_guardian_selects_guardian_approvals() -> Result<()> {
        let (mut app, mut app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let guardian_approvals = guardian_approvals_mode();

        app.update_feature_flags(vec![(Feature::GuardianApproval, true)])
            .await;

        assert!(app.config.features.enabled(Feature::GuardianApproval));
        assert!(
            app.chat_widget
                .config_ref()
                .features
                .enabled(Feature::GuardianApproval)
        );
        assert_eq!(
            app.config.approvals_reviewer,
            guardian_approvals.approvals_reviewer
        );
        assert_eq!(
            app.config.permissions.approval_policy.value(),
            guardian_approvals.approval_policy
        );
        assert_eq!(
            app.chat_widget
                .config_ref()
                .permissions
                .approval_policy
                .value(),
            guardian_approvals.approval_policy
        );
        assert_eq!(
            app.chat_widget
                .config_ref()
                .permissions
                .sandbox_policy
                .get(),
            &guardian_approvals.sandbox_policy
        );
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            guardian_approvals.approvals_reviewer
        );
        assert_eq!(app.runtime_approval_policy_override, None);
        assert_eq!(app.runtime_sandbox_policy_override, None);
        assert_eq!(
            op_rx.try_recv(),
            Ok(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: Some(guardian_approvals.approval_policy),
                approvals_reviewer: Some(guardian_approvals.approvals_reviewer),
                sandbox_policy: Some(guardian_approvals.sandbox_policy.clone()),
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            })
        );
        let cell = match app_event_rx.try_recv() {
            Ok(AppEvent::InsertHistoryCell(cell)) => cell,
            other => panic!("expected InsertHistoryCell event, got {other:?}"),
        };
        let rendered = cell
            .display_lines(/*width*/ 120)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Permissions updated to Guardian Approvals"));

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        assert!(config.contains("guardian_approval = true"));
        assert!(config.contains("approvals_reviewer = \"guardian_subagent\""));
        assert!(config.contains("approval_policy = \"on-request\""));
        assert!(config.contains("sandbox_mode = \"workspace-write\""));
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_disabling_guardian_clears_review_policy_and_restores_default()
    -> Result<()> {
        let (mut app, mut app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let config_toml_path = codex_home.path().join("config.toml").abs();
        let config_toml = "approvals_reviewer = \"guardian_subagent\"\napproval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n\n[features]\nguardian_approval = true\n";
        std::fs::write(config_toml_path.as_path(), config_toml)?;
        let user_config = toml::from_str::<TomlValue>(config_toml)?;
        app.config.config_layer_stack = app
            .config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        app.config
            .features
            .set_enabled(Feature::GuardianApproval, /*enabled*/ true)?;
        app.chat_widget
            .set_feature_enabled(Feature::GuardianApproval, /*enabled*/ true);
        app.config.approvals_reviewer = ApprovalsReviewer::GuardianSubagent;
        app.chat_widget
            .set_approvals_reviewer(ApprovalsReviewer::GuardianSubagent);
        app.config
            .permissions
            .approval_policy
            .set(AskForApproval::OnRequest)?;
        app.config
            .permissions
            .sandbox_policy
            .set(SandboxPolicy::new_workspace_write_policy())?;
        app.chat_widget
            .set_approval_policy(AskForApproval::OnRequest);
        app.chat_widget
            .set_sandbox_policy(SandboxPolicy::new_workspace_write_policy())?;

        app.update_feature_flags(vec![(Feature::GuardianApproval, false)])
            .await;

        assert!(!app.config.features.enabled(Feature::GuardianApproval));
        assert!(
            !app.chat_widget
                .config_ref()
                .features
                .enabled(Feature::GuardianApproval)
        );
        assert_eq!(app.config.approvals_reviewer, ApprovalsReviewer::User);
        assert_eq!(
            app.config.permissions.approval_policy.value(),
            AskForApproval::OnRequest
        );
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            ApprovalsReviewer::User
        );
        assert_eq!(app.runtime_approval_policy_override, None);
        assert_eq!(
            op_rx.try_recv(),
            Ok(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                approvals_reviewer: Some(ApprovalsReviewer::User),
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            })
        );
        let cell = match app_event_rx.try_recv() {
            Ok(AppEvent::InsertHistoryCell(cell)) => cell,
            other => panic!("expected InsertHistoryCell event, got {other:?}"),
        };
        let rendered = cell
            .display_lines(/*width*/ 120)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Permissions updated to Default"));

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        assert!(!config.contains("guardian_approval = true"));
        assert!(!config.contains("approvals_reviewer ="));
        assert!(config.contains("approval_policy = \"on-request\""));
        assert!(config.contains("sandbox_mode = \"workspace-write\""));
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_enabling_guardian_overrides_explicit_manual_review_policy()
    -> Result<()> {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let guardian_approvals = guardian_approvals_mode();
        let config_toml_path = codex_home.path().join("config.toml").abs();
        let config_toml = "approvals_reviewer = \"user\"\n";
        std::fs::write(config_toml_path.as_path(), config_toml)?;
        let user_config = toml::from_str::<TomlValue>(config_toml)?;
        app.config.config_layer_stack = app
            .config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        app.config.approvals_reviewer = ApprovalsReviewer::User;
        app.chat_widget
            .set_approvals_reviewer(ApprovalsReviewer::User);

        app.update_feature_flags(vec![(Feature::GuardianApproval, true)])
            .await;

        assert!(app.config.features.enabled(Feature::GuardianApproval));
        assert_eq!(
            app.config.approvals_reviewer,
            guardian_approvals.approvals_reviewer
        );
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            guardian_approvals.approvals_reviewer
        );
        assert_eq!(
            app.config.permissions.approval_policy.value(),
            guardian_approvals.approval_policy
        );
        assert_eq!(
            app.chat_widget
                .config_ref()
                .permissions
                .sandbox_policy
                .get(),
            &guardian_approvals.sandbox_policy
        );
        assert_eq!(
            op_rx.try_recv(),
            Ok(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: Some(guardian_approvals.approval_policy),
                approvals_reviewer: Some(guardian_approvals.approvals_reviewer),
                sandbox_policy: Some(guardian_approvals.sandbox_policy.clone()),
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            })
        );

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        assert!(config.contains("approvals_reviewer = \"guardian_subagent\""));
        assert!(config.contains("guardian_approval = true"));
        assert!(config.contains("approval_policy = \"on-request\""));
        assert!(config.contains("sandbox_mode = \"workspace-write\""));
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_disabling_guardian_clears_manual_review_policy_without_history()
    -> Result<()> {
        let (mut app, mut app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let config_toml_path = codex_home.path().join("config.toml").abs();
        let config_toml = "approvals_reviewer = \"user\"\napproval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n\n[features]\nguardian_approval = true\n";
        std::fs::write(config_toml_path.as_path(), config_toml)?;
        let user_config = toml::from_str::<TomlValue>(config_toml)?;
        app.config.config_layer_stack = app
            .config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        app.config
            .features
            .set_enabled(Feature::GuardianApproval, /*enabled*/ true)?;
        app.chat_widget
            .set_feature_enabled(Feature::GuardianApproval, /*enabled*/ true);
        app.config.approvals_reviewer = ApprovalsReviewer::User;
        app.chat_widget
            .set_approvals_reviewer(ApprovalsReviewer::User);

        app.update_feature_flags(vec![(Feature::GuardianApproval, false)])
            .await;

        assert!(!app.config.features.enabled(Feature::GuardianApproval));
        assert_eq!(app.config.approvals_reviewer, ApprovalsReviewer::User);
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            ApprovalsReviewer::User
        );
        assert_eq!(
            op_rx.try_recv(),
            Ok(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                approvals_reviewer: Some(ApprovalsReviewer::User),
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            })
        );
        assert!(
            app_event_rx.try_recv().is_err(),
            "manual review should not emit a permissions history update when the effective state stays default"
        );

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        assert!(!config.contains("guardian_approval = true"));
        assert!(!config.contains("approvals_reviewer ="));
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_enabling_guardian_in_profile_sets_profile_auto_review_policy()
    -> Result<()> {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let guardian_approvals = guardian_approvals_mode();
        app.active_profile = Some("guardian".to_string());
        let config_toml_path = codex_home.path().join("config.toml").abs();
        let config_toml = "profile = \"guardian\"\napprovals_reviewer = \"user\"\n";
        std::fs::write(config_toml_path.as_path(), config_toml)?;
        let user_config = toml::from_str::<TomlValue>(config_toml)?;
        app.config.config_layer_stack = app
            .config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        app.config.approvals_reviewer = ApprovalsReviewer::User;
        app.chat_widget
            .set_approvals_reviewer(ApprovalsReviewer::User);

        app.update_feature_flags(vec![(Feature::GuardianApproval, true)])
            .await;

        assert!(app.config.features.enabled(Feature::GuardianApproval));
        assert_eq!(
            app.config.approvals_reviewer,
            guardian_approvals.approvals_reviewer
        );
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            guardian_approvals.approvals_reviewer
        );
        assert_eq!(
            op_rx.try_recv(),
            Ok(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: Some(guardian_approvals.approval_policy),
                approvals_reviewer: Some(guardian_approvals.approvals_reviewer),
                sandbox_policy: Some(guardian_approvals.sandbox_policy.clone()),
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            })
        );

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        let config_value = toml::from_str::<TomlValue>(&config)?;
        let profile_config = config_value
            .as_table()
            .and_then(|table| table.get("profiles"))
            .and_then(TomlValue::as_table)
            .and_then(|profiles| profiles.get("guardian"))
            .and_then(TomlValue::as_table)
            .expect("guardian profile should exist");
        assert_eq!(
            config_value
                .as_table()
                .and_then(|table| table.get("approvals_reviewer")),
            Some(&TomlValue::String("user".to_string()))
        );
        assert_eq!(
            profile_config.get("approvals_reviewer"),
            Some(&TomlValue::String("guardian_subagent".to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_disabling_guardian_in_profile_allows_inherited_user_reviewer()
    -> Result<()> {
        let (mut app, mut app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        app.active_profile = Some("guardian".to_string());
        let config_toml_path = codex_home.path().join("config.toml").abs();
        let config_toml = r#"
profile = "guardian"
approvals_reviewer = "user"

[profiles.guardian]
approvals_reviewer = "guardian_subagent"

[profiles.guardian.features]
guardian_approval = true
"#;
        std::fs::write(config_toml_path.as_path(), config_toml)?;
        let user_config = toml::from_str::<TomlValue>(config_toml)?;
        app.config.config_layer_stack = app
            .config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        app.config
            .features
            .set_enabled(Feature::GuardianApproval, /*enabled*/ true)?;
        app.chat_widget
            .set_feature_enabled(Feature::GuardianApproval, /*enabled*/ true);
        app.config.approvals_reviewer = ApprovalsReviewer::GuardianSubagent;
        app.chat_widget
            .set_approvals_reviewer(ApprovalsReviewer::GuardianSubagent);

        app.update_feature_flags(vec![(Feature::GuardianApproval, false)])
            .await;

        assert!(!app.config.features.enabled(Feature::GuardianApproval));
        assert!(
            !app.chat_widget
                .config_ref()
                .features
                .enabled(Feature::GuardianApproval)
        );
        assert_eq!(app.config.approvals_reviewer, ApprovalsReviewer::User);
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            ApprovalsReviewer::User
        );
        assert_eq!(
            op_rx.try_recv(),
            Ok(Op::OverrideTurnContext {
                cwd: None,
                approval_policy: None,
                approvals_reviewer: Some(ApprovalsReviewer::User),
                sandbox_policy: None,
                windows_sandbox_level: None,
                model: None,
                effort: None,
                summary: None,
                service_tier: None,
                collaboration_mode: None,
                personality: None,
            })
        );
        let cell = match app_event_rx.try_recv() {
            Ok(AppEvent::InsertHistoryCell(cell)) => cell,
            other => panic!("expected InsertHistoryCell event, got {other:?}"),
        };
        let rendered = cell
            .display_lines(/*width*/ 120)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Permissions updated to Default"));

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        assert!(!config.contains("guardian_approval = true"));
        assert!(!config.contains("guardian_subagent"));
        assert_eq!(
            toml::from_str::<TomlValue>(&config)?
                .as_table()
                .and_then(|table| table.get("approvals_reviewer")),
            Some(&TomlValue::String("user".to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn update_feature_flags_disabling_guardian_in_profile_keeps_inherited_non_user_reviewer_enabled()
    -> Result<()> {
        let (mut app, mut app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        app.active_profile = Some("guardian".to_string());
        let config_toml_path = codex_home.path().join("config.toml").abs();
        let config_toml = "profile = \"guardian\"\napprovals_reviewer = \"guardian_subagent\"\n\n[features]\nguardian_approval = true\n";
        std::fs::write(config_toml_path.as_path(), config_toml)?;
        let user_config = toml::from_str::<TomlValue>(config_toml)?;
        app.config.config_layer_stack = app
            .config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        app.config
            .features
            .set_enabled(Feature::GuardianApproval, /*enabled*/ true)?;
        app.chat_widget
            .set_feature_enabled(Feature::GuardianApproval, /*enabled*/ true);
        app.config.approvals_reviewer = ApprovalsReviewer::GuardianSubagent;
        app.chat_widget
            .set_approvals_reviewer(ApprovalsReviewer::GuardianSubagent);

        app.update_feature_flags(vec![(Feature::GuardianApproval, false)])
            .await;

        assert!(app.config.features.enabled(Feature::GuardianApproval));
        assert!(
            app.chat_widget
                .config_ref()
                .features
                .enabled(Feature::GuardianApproval)
        );
        assert_eq!(
            app.config.approvals_reviewer,
            ApprovalsReviewer::GuardianSubagent
        );
        assert_eq!(
            app.chat_widget.config_ref().approvals_reviewer,
            ApprovalsReviewer::GuardianSubagent
        );
        assert!(
            op_rx.try_recv().is_err(),
            "disabling an inherited non-user reviewer should not patch the active session"
        );
        let app_events = std::iter::from_fn(|| app_event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(
            !app_events.iter().any(|event| match event {
                AppEvent::InsertHistoryCell(cell) => cell
                    .display_lines(/*width*/ 120)
                    .iter()
                    .any(|line| line.to_string().contains("Permissions updated to")),
                _ => false,
            }),
            "blocking disable with inherited guardian review should not emit a permissions history update: {app_events:?}"
        );

        let config = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
        assert!(config.contains("guardian_approval = true"));
        assert_eq!(
            toml::from_str::<TomlValue>(&config)?
                .as_table()
                .and_then(|table| table.get("approvals_reviewer")),
            Some(&TomlValue::String("guardian_subagent".to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn open_agent_picker_allows_existing_agent_threads_when_feature_is_disabled() -> Result<()>
    {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let thread_id = ThreadId::new();
        app.thread_event_channels
            .insert(thread_id, ThreadEventChannel::new(/*capacity*/ 1));

        app.open_agent_picker(&mut app_server).await;
        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_matches!(
            app_event_rx.try_recv(),
            Ok(AppEvent::SelectAgentThread(selected_thread_id)) if selected_thread_id == thread_id
        );
        Ok(())
    }

    #[tokio::test]
    async fn refresh_pending_thread_approvals_only_lists_inactive_threads() {
        let mut app = make_test_app().await;
        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000002").expect("valid thread");

        app.primary_thread_id = Some(main_thread_id);
        app.active_thread_id = Some(main_thread_id);
        app.thread_event_channels
            .insert(main_thread_id, ThreadEventChannel::new(/*capacity*/ 1));

        let agent_channel = ThreadEventChannel::new(/*capacity*/ 1);
        {
            let mut store = agent_channel.store.lock().await;
            store.push_request(exec_approval_request(
                agent_thread_id,
                "turn-1",
                "call-1",
                /*approval_id*/ None,
            ));
        }
        app.thread_event_channels
            .insert(agent_thread_id, agent_channel);
        app.agent_navigation.upsert(
            agent_thread_id,
            Some("Robie".to_string()),
            Some("explorer".to_string()),
            /*is_closed*/ false,
        );

        app.refresh_pending_thread_approvals().await;
        assert_eq!(
            app.chat_widget.pending_thread_approvals(),
            &["Robie [explorer]".to_string()]
        );

        app.active_thread_id = Some(agent_thread_id);
        app.refresh_pending_thread_approvals().await;
        assert!(app.chat_widget.pending_thread_approvals().is_empty());
    }

    #[tokio::test]
    async fn inactive_thread_approval_bubbles_into_active_view() -> Result<()> {
        let mut app = make_test_app().await;
        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000011").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000022").expect("valid thread");

        app.primary_thread_id = Some(main_thread_id);
        app.active_thread_id = Some(main_thread_id);
        app.thread_event_channels
            .insert(main_thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        app.thread_event_channels.insert(
            agent_thread_id,
            ThreadEventChannel::new_with_session(
                /*capacity*/ 1,
                ThreadSessionState {
                    approval_policy: AskForApproval::OnRequest,
                    sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
                    rollout_path: Some(test_path_buf("/tmp/agent-rollout.jsonl")),
                    ..test_thread_session(agent_thread_id, test_path_buf("/tmp/agent"))
                },
                Vec::new(),
            ),
        );
        app.agent_navigation.upsert(
            agent_thread_id,
            Some("Robie".to_string()),
            Some("explorer".to_string()),
            /*is_closed*/ false,
        );

        app.enqueue_thread_request(
            agent_thread_id,
            exec_approval_request(
                agent_thread_id,
                "turn-approval",
                "call-approval",
                /*approval_id*/ None,
            ),
        )
        .await?;

        assert_eq!(app.chat_widget.has_active_view(), true);
        assert_eq!(
            app.chat_widget.pending_thread_approvals(),
            &["Robie [explorer]".to_string()]
        );

        Ok(())
    }

    #[tokio::test]
    async fn inactive_thread_exec_approval_preserves_context() {
        let app = make_test_app().await;
        let thread_id = ThreadId::new();
        let mut request = exec_approval_request(
            thread_id,
            "turn-approval",
            "call-approval",
            /*approval_id*/ None,
        );
        let ServerRequest::CommandExecutionRequestApproval { params, .. } = &mut request else {
            panic!("expected exec approval request");
        };
        params.network_approval_context = Some(AppServerNetworkApprovalContext {
            host: "example.com".to_string(),
            protocol: AppServerNetworkApprovalProtocol::Socks5Tcp,
        });
        params.additional_permissions = Some(AdditionalPermissionProfile {
            network: Some(AdditionalNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(AdditionalFileSystemPermissions {
                read: Some(vec![test_absolute_path("/tmp/read-only")]),
                write: Some(vec![test_absolute_path("/tmp/write")]),
            }),
        });
        params.proposed_network_policy_amendments = Some(vec![AppServerNetworkPolicyAmendment {
            host: "example.com".to_string(),
            action: AppServerNetworkPolicyRuleAction::Allow,
        }]);

        let Some(ThreadInteractiveRequest::Approval(ApprovalRequest::Exec {
            available_decisions,
            network_approval_context,
            additional_permissions,
            ..
        })) = app
            .interactive_request_for_thread_request(thread_id, &request)
            .await
        else {
            panic!("expected exec approval request");
        };

        assert_eq!(
            network_approval_context,
            Some(NetworkApprovalContext {
                host: "example.com".to_string(),
                protocol: NetworkApprovalProtocol::Socks5Tcp,
            })
        );
        assert_eq!(
            additional_permissions,
            Some(PermissionProfile {
                network: Some(NetworkPermissions {
                    enabled: Some(true),
                }),
                file_system: Some(FileSystemPermissions {
                    read: Some(vec![test_absolute_path("/tmp/read-only")]),
                    write: Some(vec![test_absolute_path("/tmp/write")]),
                }),
            })
        );
        assert_eq!(
            available_decisions,
            vec![
                codex_protocol::protocol::ReviewDecision::Approved,
                codex_protocol::protocol::ReviewDecision::ApprovedForSession,
                codex_protocol::protocol::ReviewDecision::NetworkPolicyAmendment {
                    network_policy_amendment: codex_protocol::approvals::NetworkPolicyAmendment {
                        host: "example.com".to_string(),
                        action: codex_protocol::approvals::NetworkPolicyRuleAction::Allow,
                    },
                },
                codex_protocol::protocol::ReviewDecision::Abort,
            ]
        );
    }

    #[tokio::test]
    async fn inactive_thread_exec_approval_splits_shell_wrapped_command() {
        let app = make_test_app().await;
        let thread_id = ThreadId::new();
        let script = r#"python3 -c 'print("Hello, world!")'"#;
        let mut request = exec_approval_request(
            thread_id,
            "turn-approval",
            "call-approval",
            /*approval_id*/ None,
        );
        let ServerRequest::CommandExecutionRequestApproval { params, .. } = &mut request else {
            panic!("expected exec approval request");
        };
        params.command = Some(
            shlex::try_join(["/bin/zsh", "-lc", script]).expect("round-trippable shell wrapper"),
        );

        let Some(ThreadInteractiveRequest::Approval(ApprovalRequest::Exec { command, .. })) = app
            .interactive_request_for_thread_request(thread_id, &request)
            .await
        else {
            panic!("expected exec approval request");
        };

        assert_eq!(
            command,
            vec![
                "/bin/zsh".to_string(),
                "-lc".to_string(),
                script.to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn inactive_thread_permissions_approval_preserves_file_system_permissions() {
        let app = make_test_app().await;
        let thread_id = ThreadId::new();
        let request = ServerRequest::PermissionsRequestApproval {
            request_id: AppServerRequestId::Integer(7),
            params: PermissionsRequestApprovalParams {
                thread_id: thread_id.to_string(),
                turn_id: "turn-approval".to_string(),
                item_id: "call-approval".to_string(),
                reason: Some("Need access to .git".to_string()),
                permissions: codex_app_server_protocol::RequestPermissionProfile {
                    network: Some(AdditionalNetworkPermissions {
                        enabled: Some(true),
                    }),
                    file_system: Some(AdditionalFileSystemPermissions {
                        read: Some(vec![test_absolute_path("/tmp/read-only")]),
                        write: Some(vec![test_absolute_path("/tmp/write")]),
                    }),
                },
            },
        };

        let Some(ThreadInteractiveRequest::Approval(ApprovalRequest::Permissions {
            permissions,
            ..
        })) = app
            .interactive_request_for_thread_request(thread_id, &request)
            .await
        else {
            panic!("expected permissions approval request");
        };

        assert_eq!(
            permissions,
            RequestPermissionProfile {
                network: Some(NetworkPermissions {
                    enabled: Some(true),
                }),
                file_system: Some(FileSystemPermissions {
                    read: Some(vec![test_absolute_path("/tmp/read-only")]),
                    write: Some(vec![test_absolute_path("/tmp/write")]),
                }),
            }
        );
    }

    #[tokio::test]
    async fn inactive_thread_approval_badge_clears_after_turn_completion_notification() -> Result<()>
    {
        let mut app = make_test_app().await;
        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000101").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000202").expect("valid thread");

        app.primary_thread_id = Some(main_thread_id);
        app.active_thread_id = Some(main_thread_id);
        app.thread_event_channels
            .insert(main_thread_id, ThreadEventChannel::new(/*capacity*/ 1));
        app.thread_event_channels.insert(
            agent_thread_id,
            ThreadEventChannel::new_with_session(
                /*capacity*/ 4,
                ThreadSessionState {
                    approval_policy: AskForApproval::OnRequest,
                    sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
                    rollout_path: Some(test_path_buf("/tmp/agent-rollout.jsonl")),
                    ..test_thread_session(agent_thread_id, test_path_buf("/tmp/agent"))
                },
                Vec::new(),
            ),
        );
        app.agent_navigation.upsert(
            agent_thread_id,
            Some("Robie".to_string()),
            Some("explorer".to_string()),
            /*is_closed*/ false,
        );

        app.enqueue_thread_request(
            agent_thread_id,
            exec_approval_request(
                agent_thread_id,
                "turn-approval",
                "call-approval",
                /*approval_id*/ None,
            ),
        )
        .await?;
        assert_eq!(
            app.chat_widget.pending_thread_approvals(),
            &["Robie [explorer]".to_string()]
        );

        app.enqueue_thread_notification(
            agent_thread_id,
            turn_completed_notification(agent_thread_id, "turn-approval", TurnStatus::Completed),
        )
        .await?;

        assert!(
            app.chat_widget.pending_thread_approvals().is_empty(),
            "turn completion should clear inactive-thread approval badge immediately"
        );

        Ok(())
    }

    #[tokio::test]
    async fn inactive_thread_started_notification_initializes_replay_session() -> Result<()> {
        let mut app = make_test_app().await;
        let temp_dir = tempdir()?;
        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000101").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000202").expect("valid thread");
        let primary_session = ThreadSessionState {
            approval_policy: AskForApproval::OnRequest,
            sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
            ..test_thread_session(main_thread_id, test_path_buf("/tmp/main"))
        };

        app.primary_thread_id = Some(main_thread_id);
        app.active_thread_id = Some(main_thread_id);
        app.primary_session_configured = Some(primary_session.clone());
        app.thread_event_channels.insert(
            main_thread_id,
            ThreadEventChannel::new_with_session(
                /*capacity*/ 4,
                primary_session.clone(),
                Vec::new(),
            ),
        );

        let rollout_path = temp_dir.path().join("agent-rollout.jsonl");
        let turn_context = TurnContextItem {
            turn_id: None,
            trace_id: None,
            cwd: test_path_buf("/tmp/agent"),
            current_date: None,
            timezone: None,
            approval_policy: primary_session.approval_policy,
            sandbox_policy: primary_session.sandbox_policy.clone(),
            network: None,
            file_system_sandbox_policy: None,
            model: "gpt-agent".to_string(),
            personality: None,
            collaboration_mode: None,
            realtime_active: Some(false),
            effort: primary_session.reasoning_effort,
            summary: app.config.model_reasoning_summary.unwrap_or_default(),
            user_instructions: None,
            developer_instructions: None,
            final_output_json_schema: None,
            truncation_policy: None,
        };
        let rollout = RolloutLine {
            timestamp: "t0".to_string(),
            item: RolloutItem::TurnContext(turn_context),
        };
        std::fs::write(
            &rollout_path,
            format!("{}\n", serde_json::to_string(&rollout)?),
        )?;
        app.enqueue_thread_notification(
            agent_thread_id,
            ServerNotification::ThreadStarted(ThreadStartedNotification {
                thread: Thread {
                    id: agent_thread_id.to_string(),
                    forked_from_id: None,
                    preview: "agent thread".to_string(),
                    ephemeral: false,
                    model_provider: "agent-provider".to_string(),
                    created_at: 1,
                    updated_at: 2,
                    status: codex_app_server_protocol::ThreadStatus::Idle,
                    path: Some(rollout_path.clone()),
                    cwd: test_path_buf("/tmp/agent").abs(),
                    cli_version: "0.0.0".to_string(),
                    source: codex_app_server_protocol::SessionSource::Unknown,
                    agent_nickname: Some("Robie".to_string()),
                    agent_role: Some("explorer".to_string()),
                    git_info: None,
                    name: Some("agent thread".to_string()),
                    turns: Vec::new(),
                },
            }),
        )
        .await?;

        let store = app
            .thread_event_channels
            .get(&agent_thread_id)
            .expect("agent thread channel")
            .store
            .lock()
            .await;
        let session = store.session.clone().expect("inferred session");
        drop(store);

        assert_eq!(session.thread_id, agent_thread_id);
        assert_eq!(session.thread_name, Some("agent thread".to_string()));
        assert_eq!(session.model, "gpt-agent");
        assert_eq!(session.model_provider_id, "agent-provider");
        assert_eq!(session.approval_policy, primary_session.approval_policy);
        assert_eq!(session.cwd.as_path(), test_path_buf("/tmp/agent").as_path());
        assert_eq!(session.rollout_path, Some(rollout_path));
        assert_eq!(
            app.agent_navigation.get(&agent_thread_id),
            Some(&AgentPickerThreadEntry {
                agent_nickname: Some("Robie".to_string()),
                agent_role: Some("explorer".to_string()),
                is_closed: false,
            })
        );

        Ok(())
    }

    #[tokio::test]
    async fn inactive_thread_started_notification_preserves_primary_model_when_path_missing()
    -> Result<()> {
        let mut app = make_test_app().await;
        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000301").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000302").expect("valid thread");
        let primary_session = ThreadSessionState {
            approval_policy: AskForApproval::OnRequest,
            sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
            ..test_thread_session(main_thread_id, test_path_buf("/tmp/main"))
        };

        app.primary_thread_id = Some(main_thread_id);
        app.active_thread_id = Some(main_thread_id);
        app.primary_session_configured = Some(primary_session.clone());
        app.thread_event_channels.insert(
            main_thread_id,
            ThreadEventChannel::new_with_session(
                /*capacity*/ 4,
                primary_session.clone(),
                Vec::new(),
            ),
        );

        app.enqueue_thread_notification(
            agent_thread_id,
            ServerNotification::ThreadStarted(ThreadStartedNotification {
                thread: Thread {
                    id: agent_thread_id.to_string(),
                    forked_from_id: None,
                    preview: "agent thread".to_string(),
                    ephemeral: false,
                    model_provider: "agent-provider".to_string(),
                    created_at: 1,
                    updated_at: 2,
                    status: codex_app_server_protocol::ThreadStatus::Idle,
                    path: None,
                    cwd: test_path_buf("/tmp/agent").abs(),
                    cli_version: "0.0.0".to_string(),
                    source: codex_app_server_protocol::SessionSource::Unknown,
                    agent_nickname: Some("Robie".to_string()),
                    agent_role: Some("explorer".to_string()),
                    git_info: None,
                    name: Some("agent thread".to_string()),
                    turns: Vec::new(),
                },
            }),
        )
        .await?;

        let store = app
            .thread_event_channels
            .get(&agent_thread_id)
            .expect("agent thread channel")
            .store
            .lock()
            .await;
        let session = store.session.clone().expect("inferred session");

        assert_eq!(session.model, primary_session.model);

        Ok(())
    }

    #[test]
    fn agent_picker_item_name_snapshot() {
        let thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000123").expect("valid thread id");
        let snapshot = [
            format!(
                "{} | {}",
                format_agent_picker_item_name(
                    Some("Robie"),
                    Some("explorer"),
                    /*is_primary*/ true
                ),
                thread_id
            ),
            format!(
                "{} | {}",
                format_agent_picker_item_name(
                    Some("Robie"),
                    Some("explorer"),
                    /*is_primary*/ false
                ),
                thread_id
            ),
            format!(
                "{} | {}",
                format_agent_picker_item_name(
                    Some("Robie"),
                    /*agent_role*/ None,
                    /*is_primary*/ false
                ),
                thread_id
            ),
            format!(
                "{} | {}",
                format_agent_picker_item_name(
                    /*agent_nickname*/ None,
                    Some("explorer"),
                    /*is_primary*/ false
                ),
                thread_id
            ),
            format!(
                "{} | {}",
                format_agent_picker_item_name(
                    /*agent_nickname*/ None, /*agent_role*/ None,
                    /*is_primary*/ false
                ),
                thread_id
            ),
        ]
        .join("\n");
        assert_snapshot!("agent_picker_item_name", snapshot);
    }

    #[tokio::test]
    async fn active_non_primary_shutdown_target_returns_none_for_non_shutdown_event() -> Result<()>
    {
        let mut app = make_test_app().await;
        app.active_thread_id = Some(ThreadId::new());
        app.primary_thread_id = Some(ThreadId::new());

        assert_eq!(
            app.active_non_primary_shutdown_target(&ServerNotification::SkillsChanged(
                codex_app_server_protocol::SkillsChangedNotification {},
            )),
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_non_primary_shutdown_target_returns_none_for_primary_thread_shutdown()
    -> Result<()> {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        app.active_thread_id = Some(thread_id);
        app.primary_thread_id = Some(thread_id);

        assert_eq!(
            app.active_non_primary_shutdown_target(&thread_closed_notification(thread_id)),
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_non_primary_shutdown_target_returns_ids_for_non_primary_shutdown() -> Result<()>
    {
        let mut app = make_test_app().await;
        let active_thread_id = ThreadId::new();
        let primary_thread_id = ThreadId::new();
        app.active_thread_id = Some(active_thread_id);
        app.primary_thread_id = Some(primary_thread_id);

        assert_eq!(
            app.active_non_primary_shutdown_target(&thread_closed_notification(active_thread_id)),
            Some((active_thread_id, primary_thread_id))
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_non_primary_shutdown_target_returns_none_when_shutdown_exit_is_pending()
    -> Result<()> {
        let mut app = make_test_app().await;
        let active_thread_id = ThreadId::new();
        let primary_thread_id = ThreadId::new();
        app.active_thread_id = Some(active_thread_id);
        app.primary_thread_id = Some(primary_thread_id);
        app.pending_shutdown_exit_thread_id = Some(active_thread_id);

        assert_eq!(
            app.active_non_primary_shutdown_target(&thread_closed_notification(active_thread_id)),
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_non_primary_shutdown_target_still_switches_for_other_pending_exit_thread()
    -> Result<()> {
        let mut app = make_test_app().await;
        let active_thread_id = ThreadId::new();
        let primary_thread_id = ThreadId::new();
        app.active_thread_id = Some(active_thread_id);
        app.primary_thread_id = Some(primary_thread_id);
        app.pending_shutdown_exit_thread_id = Some(ThreadId::new());

        assert_eq!(
            app.active_non_primary_shutdown_target(&thread_closed_notification(active_thread_id)),
            Some((active_thread_id, primary_thread_id))
        );
        Ok(())
    }

    async fn render_clear_ui_header_after_long_transcript_for_snapshot() -> String {
        let mut app = make_test_app().await;
        app.config.cwd = test_path_buf("/tmp/project").abs();
        app.chat_widget.set_model("gpt-test");
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::High));
        let story_part_one = "In the cliffside town of Bracken Ferry, the lighthouse had been dark for \
            nineteen years, and the children were told it was because the sea no longer wanted a \
            guide. Mara, who repaired clocks for a living, found that hard to believe. Every dawn she \
            heard the gulls circling the empty tower, and every dusk she watched ships hesitate at the \
            mouth of the bay as if listening for a signal that never came. When an old brass key fell \
            out of a cracked parcel in her workshop, tagged only with the words 'for the lamp room,' \
            she decided to climb the hill and see what the town had forgotten.";
        let story_part_two = "Inside the lighthouse she found gears wrapped in oilcloth, logbooks filled \
            with weather notes, and a lens shrouded beneath salt-stiff canvas. The mechanism was not \
            broken, only unfinished. Someone had removed the governor spring and hidden it in a false \
            drawer, along with a letter from the last keeper admitting he had darkened the light on \
            purpose after smugglers threatened his family. Mara spent the night rebuilding the clockwork \
            from spare watch parts, her fingers blackened with soot and grease, while a storm gathered \
            over the water and the harbor bells began to ring.";
        let story_part_three = "At midnight the first squall hit, and the fishing boats returned early, \
            blind in sheets of rain. Mara wound the mechanism, set the teeth by hand, and watched the \
            great lens begin to turn in slow, certain arcs. The beam swept across the bay, caught the \
            whitecaps, and reached the boats just as they were drifting toward the rocks below the \
            eastern cliffs. In the morning the town square was crowded with wet sailors, angry elders, \
            and wide-eyed children, but when the oldest captain placed the keeper's log on the fountain \
            and thanked Mara for relighting the coast, nobody argued. By sunset, Bracken Ferry had a \
            lighthouse again, and Mara had more clocks to mend than ever because everyone wanted \
            something in town to keep better time.";

        let user_cell = |text: &str| -> Arc<dyn HistoryCell> {
            Arc::new(UserHistoryCell {
                message: text.to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn HistoryCell>
        };
        let agent_cell = |text: &str| -> Arc<dyn HistoryCell> {
            Arc::new(AgentMessageCell::new(
                vec![Line::from(text.to_string())],
                /*is_first_line*/ true,
            )) as Arc<dyn HistoryCell>
        };
        let make_header = |is_first| -> Arc<dyn HistoryCell> {
            let event = SessionConfiguredEvent {
                session_id: ThreadId::new(),
                forked_from_id: None,
                thread_name: None,
                model: "gpt-test".to_string(),
                model_provider_id: "test-provider".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                cwd: test_path_buf("/tmp/project").abs(),
                reasoning_effort: Some(ReasoningEffortConfig::High),
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(PathBuf::new()),
            };
            Arc::new(new_session_info(
                app.chat_widget.config_ref(),
                app.chat_widget.current_model(),
                event,
                is_first,
                /*tooltip_override*/ None,
                /*auth_plan*/ None,
                /*show_fast_status*/ false,
            )) as Arc<dyn HistoryCell>
        };

        app.transcript_cells = vec![
            make_header(true),
            Arc::new(crate::history_cell::new_info_event(
                "startup tip that used to replay".to_string(),
                /*hint*/ None,
            )) as Arc<dyn HistoryCell>,
            user_cell("Tell me a long story about a town with a dark lighthouse."),
            agent_cell(story_part_one),
            user_cell("Continue the story and reveal why the light went out."),
            agent_cell(story_part_two),
            user_cell("Finish the story with a storm and a resolution."),
            agent_cell(story_part_three),
        ];
        app.has_emitted_history_lines = true;

        let rendered = app
            .clear_ui_header_lines_with_version(/*width*/ 80, "<VERSION>")
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !rendered.contains("startup tip that used to replay"),
            "clear header should not replay startup notices"
        );
        assert!(
            !rendered.contains("Bracken Ferry"),
            "clear header should not replay prior conversation turns"
        );
        rendered
    }

    #[tokio::test]
    #[cfg_attr(
        target_os = "windows",
        ignore = "snapshot path rendering differs on Windows"
    )]
    async fn clear_ui_after_long_transcript_snapshots_fresh_header_only() {
        let rendered = render_clear_ui_header_after_long_transcript_for_snapshot().await;
        assert_snapshot!("clear_ui_after_long_transcript_fresh_header_only", rendered);
    }

    #[tokio::test]
    #[cfg_attr(
        target_os = "windows",
        ignore = "snapshot path rendering differs on Windows"
    )]
    async fn ctrl_l_clear_ui_after_long_transcript_reuses_clear_header_snapshot() {
        let rendered = render_clear_ui_header_after_long_transcript_for_snapshot().await;
        assert_snapshot!("clear_ui_after_long_transcript_fresh_header_only", rendered);
    }

    #[tokio::test]
    #[cfg_attr(
        target_os = "windows",
        ignore = "snapshot path rendering differs on Windows"
    )]
    async fn clear_ui_header_shows_fast_status_for_fast_capable_models() {
        let mut app = make_test_app().await;
        app.config.cwd = test_path_buf("/tmp/project").abs();
        app.chat_widget.set_model("gpt-5.4");
        set_fast_mode_test_catalog(&mut app.chat_widget);
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::XHigh));
        app.chat_widget
            .set_service_tier(Some(codex_protocol::config_types::ServiceTier::Fast));
        set_chatgpt_auth(&mut app.chat_widget);
        set_fast_mode_test_catalog(&mut app.chat_widget);

        let rendered = app
            .clear_ui_header_lines_with_version(/*width*/ 80, "<VERSION>")
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_snapshot!("clear_ui_header_fast_status_fast_capable_models", rendered);
    }

    pub(super) async fn make_test_app() -> App {
        let (chat_widget, app_event_tx, _rx, _op_rx) = make_chatwidget_manual_with_sender().await;
        let config = chat_widget.config_ref().clone();
        let display_preferences = DisplayPreferences::from_config(&config);
        let file_search = FileSearchManager::new(config.cwd.to_path_buf(), app_event_tx.clone());
        let model = crate::legacy_core::test_support::get_model_offline(config.model.as_deref());
        let session_telemetry = test_session_telemetry(&config, model.as_str());

        App {
            model_catalog: chat_widget.model_catalog(),
            session_telemetry,
            app_event_tx,
            chat_widget,
            config,
            display_preferences,
            active_profile: None,
            cli_kv_overrides: Vec::new(),
            harness_overrides: ConfigOverrides::default(),
            runtime_approval_policy_override: None,
            runtime_sandbox_policy_override: None,
            file_search,
            transcript_cells: Vec::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            enhanced_keys_supported: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            status_line_invalid_items_warned: Arc::new(AtomicBool::new(false)),
            terminal_title_invalid_items_warned: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            key_chord: KeyChordState::default(),
            backtrack_render_pending: false,
            feedback: codex_feedback::CodexFeedback::new(),
            feedback_audience: FeedbackAudience::External,
            environment_manager: Arc::new(EnvironmentManager::new(/*exec_server_url*/ None)),
            remote_app_server_url: None,
            remote_app_server_auth_token: None,
            pending_update_action: None,
            pending_shutdown_exit_thread_id: None,
            windows_sandbox: WindowsSandboxState::default(),
            thread_event_channels: HashMap::new(),
            thread_event_listener_tasks: HashMap::new(),
            agent_navigation: AgentNavigationState::default(),
            active_thread_id: None,
            active_thread_rx: None,
            primary_thread_id: None,
            last_subagent_backfill_attempt: None,
            primary_session_configured: None,
            pending_primary_events: VecDeque::new(),
            pending_workflow_compact_followups: VecDeque::new(),
            pending_app_server_requests: PendingAppServerRequests::default(),
            pending_plugin_enabled_writes: HashMap::new(),
            pending_hook_enabled_writes: HashMap::new(),
            workflow_thread_notification_channels: Arc::new(
                tokio::sync::Mutex::new(HashMap::new()),
            ),
            workflow_file_watch: None,
            workflow_scheduler: WorkflowSchedulerState::default(),
            workflow_history: WorkflowHistoryState::default(),
            btw_session: None,
            clawbot_controls_destination: ClawbotControlsDestination::Root,
            clawbot_workspace_root: None,
            clawbot_provider_task: None,
            clawbot_pending_turns: HashMap::new(),
            #[cfg(test)]
            clawbot_outbound_messages: Vec::new(),
            #[cfg(test)]
            clawbot_outbound_reactions: Vec::new(),
            #[cfg(test)]
            clawbot_removed_outbound_reactions: Vec::new(),
        }
    }

    async fn make_test_app_with_channels() -> (
        App,
        tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
        tokio::sync::mpsc::UnboundedReceiver<Op>,
    ) {
        let (chat_widget, app_event_tx, rx, op_rx) = make_chatwidget_manual_with_sender().await;
        let config = chat_widget.config_ref().clone();
        let display_preferences = DisplayPreferences::from_config(&config);
        let file_search = FileSearchManager::new(config.cwd.to_path_buf(), app_event_tx.clone());
        let model = crate::legacy_core::test_support::get_model_offline(config.model.as_deref());
        let session_telemetry = test_session_telemetry(&config, model.as_str());

        (
            App {
                model_catalog: chat_widget.model_catalog(),
                session_telemetry,
                app_event_tx,
                chat_widget,
                config,
                display_preferences,
                active_profile: None,
                cli_kv_overrides: Vec::new(),
                harness_overrides: ConfigOverrides::default(),
                runtime_approval_policy_override: None,
                runtime_sandbox_policy_override: None,
                file_search,
                transcript_cells: Vec::new(),
                overlay: None,
                deferred_history_lines: Vec::new(),
                has_emitted_history_lines: false,
                enhanced_keys_supported: false,
                commit_anim_running: Arc::new(AtomicBool::new(false)),
                status_line_invalid_items_warned: Arc::new(AtomicBool::new(false)),
                terminal_title_invalid_items_warned: Arc::new(AtomicBool::new(false)),
                backtrack: BacktrackState::default(),
                key_chord: KeyChordState::default(),
                backtrack_render_pending: false,
                feedback: codex_feedback::CodexFeedback::new(),
                feedback_audience: FeedbackAudience::External,
                environment_manager: Arc::new(EnvironmentManager::new(
                    /*exec_server_url*/ None,
                )),
                remote_app_server_url: None,
                remote_app_server_auth_token: None,
                pending_update_action: None,
                pending_shutdown_exit_thread_id: None,
                windows_sandbox: WindowsSandboxState::default(),
                thread_event_channels: HashMap::new(),
                thread_event_listener_tasks: HashMap::new(),
                agent_navigation: AgentNavigationState::default(),
                active_thread_id: None,
                active_thread_rx: None,
                primary_thread_id: None,
                last_subagent_backfill_attempt: None,
                primary_session_configured: None,
                pending_primary_events: VecDeque::new(),
                pending_workflow_compact_followups: VecDeque::new(),
                pending_app_server_requests: PendingAppServerRequests::default(),
                pending_plugin_enabled_writes: HashMap::new(),
                pending_hook_enabled_writes: HashMap::new(),
                workflow_thread_notification_channels: Arc::new(tokio::sync::Mutex::new(
                    HashMap::new(),
                )),
                workflow_file_watch: None,
                workflow_scheduler: WorkflowSchedulerState::default(),
                workflow_history: WorkflowHistoryState::default(),
                btw_session: None,
                clawbot_controls_destination: ClawbotControlsDestination::Root,
                clawbot_workspace_root: None,
                clawbot_provider_task: None,
                clawbot_pending_turns: HashMap::new(),
                #[cfg(test)]
                clawbot_outbound_messages: Vec::new(),
                #[cfg(test)]
                clawbot_outbound_reactions: Vec::new(),
                #[cfg(test)]
                clawbot_removed_outbound_reactions: Vec::new(),
            },
            rx,
            op_rx,
        )
    }

    fn write_test_before_turn_workflow(workspace_cwd: &Path) -> Result<()> {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir)?;
        std::fs::write(
            workflows_dir.join("before_turn.yaml"),
            r#"name: director

triggers:
  - type: before_turn
    bind_thread: all
    jobs: [augment]

jobs:
  augment:
    context_strategy: embed
    execution_strategy: inherit_session
    steps:
      - prompt: |
          added by before_turn
"#,
        )?;
        Ok(())
    }

    fn write_test_after_turn_workflow(workspace_cwd: &Path) -> Result<()> {
        write_test_after_turn_workflow_with_condition(workspace_cwd, None)
    }

    fn write_test_after_turn_workflow_with_condition(
        workspace_cwd: &Path,
        condition: Option<&str>,
    ) -> Result<()> {
        write_test_after_turn_workflow_with_condition_and_bind_thread(
            workspace_cwd,
            condition,
            "all",
        )
    }

    fn write_test_after_turn_workflow_with_condition_and_bind_thread(
        workspace_cwd: &Path,
        condition: Option<&str>,
        bind_thread: &str,
    ) -> Result<()> {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir)?;
        let condition = condition
            .map(|condition| format!("    condition: {condition}\n"))
            .unwrap_or_default();
        std::fs::write(
            workflows_dir.join("after_turn.yaml"),
            format!(
                r#"name: director

triggers:
  - type: after_turn
    id: followup
    bind_thread: {bind_thread}
{condition}    jobs: [followup]

jobs:
  followup:
    context_strategy: embed
    execution_strategy: inherit_session
    steps:
      - prompt: |
          follow up from workflow
"#,
            ),
        )?;
        Ok(())
    }

    fn write_test_manual_workflow_with_bind_thread(
        workspace_cwd: &Path,
        bind_thread: &str,
    ) -> Result<()> {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir)?;
        std::fs::write(
            workflows_dir.join("manual_restricted.yaml"),
            format!(
                r#"name: restricted

triggers:
  - type: manual
    id: review_backlog
    bind_thread: {bind_thread}
    jobs: [summarize]

jobs:
  summarize:
    context_strategy: embed
    execution_strategy: inherit_session
    steps:
      - prompt: |
          summarize the backlog
"#
            ),
        )?;
        Ok(())
    }

    fn write_test_manual_workflow(workspace_cwd: &Path) -> Result<()> {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir)?;
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: manual
    id: review_backlog
    bind_thread: all
    jobs: [summarize]
  - type: manual
    id: triage
    bind_thread: all
    jobs: [notify]
  - type: after_turn
    id: followup
    bind_thread: all
    jobs: [notify]

jobs:
  summarize:
    context_strategy: embed
    execution_strategy: inherit_session
    steps:
      - prompt: |
          summarize the backlog
  notify:
    context_strategy: embed
    execution_strategy: inherit_session
    response: user
    steps:
      - prompt: |
          send workflow update
"#,
        )?;
        Ok(())
    }

    fn write_test_file_watch_workflow(workspace_cwd: &Path) -> Result<()> {
        let workflows_dir = workspace_cwd.join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir)?;
        std::fs::write(
            workflows_dir.join("file_watch.yaml"),
            r#"name: watcher

triggers:
  - type: file_watch
    id: refresh
    bind_thread: all
    jobs: [summarize]

jobs:
  summarize:
    context_strategy: embed
    execution_strategy: inherit_session
    steps:
      - prompt: |
          summarize the latest file changes
"#,
        )?;
        Ok(())
    }
    fn test_thread_session(thread_id: ThreadId, cwd: PathBuf) -> ThreadSessionState {
        ThreadSessionState {
            thread_id,
            forked_from_id: None,
            thread_name: None,
            model: "gpt-test".to_string(),
            model_provider_id: "test-provider".to_string(),
            service_tier: None,
            approval_policy: AskForApproval::Never,
            approvals_reviewer: ApprovalsReviewer::User,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            cwd: cwd.abs(),
            instruction_source_paths: Vec::new(),
            reasoning_effort: None,
            history_log_id: 0,
            history_entry_count: 0,
            network_proxy: None,
            rollout_path: Some(PathBuf::new()),
        }
    }

    fn test_turn(turn_id: &str, status: TurnStatus, items: Vec<ThreadItem>) -> Turn {
        Turn {
            id: turn_id.to_string(),
            items,
            status,
            error: None,
            started_at: None,
            completed_at: None,
            duration_ms: None,
        }
    }

    fn turn_started_notification(thread_id: ThreadId, turn_id: &str) -> ServerNotification {
        ServerNotification::TurnStarted(TurnStartedNotification {
            thread_id: thread_id.to_string(),
            turn: Turn {
                started_at: Some(0),
                ..test_turn(turn_id, TurnStatus::InProgress, Vec::new())
            },
        })
    }

    fn turn_completed_notification(
        thread_id: ThreadId,
        turn_id: &str,
        status: TurnStatus,
    ) -> ServerNotification {
        ServerNotification::TurnCompleted(TurnCompletedNotification {
            thread_id: thread_id.to_string(),
            turn: Turn {
                completed_at: Some(0),
                duration_ms: Some(1),
                ..test_turn(turn_id, status, Vec::new())
            },
        })
    }

    fn turn_completed_notification_with_agent_message(
        thread_id: ThreadId,
        turn_id: &str,
        status: TurnStatus,
        message: &str,
    ) -> ServerNotification {
        ServerNotification::TurnCompleted(TurnCompletedNotification {
            thread_id: thread_id.to_string(),
            turn: test_turn(
                turn_id,
                status,
                vec![ThreadItem::AgentMessage {
                    id: "agent-1".to_string(),
                    text: message.to_string(),
                    phase: None,
                    memory_citation: None,
                }],
            ),
        })
    }

    fn thread_closed_notification(thread_id: ThreadId) -> ServerNotification {
        ServerNotification::ThreadClosed(ThreadClosedNotification {
            thread_id: thread_id.to_string(),
        })
    }

    fn token_usage_notification(
        thread_id: ThreadId,
        turn_id: &str,
        model_context_window: Option<i64>,
    ) -> ServerNotification {
        ServerNotification::ThreadTokenUsageUpdated(ThreadTokenUsageUpdatedNotification {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            token_usage: ThreadTokenUsage {
                total: TokenUsageBreakdown {
                    total_tokens: 10,
                    input_tokens: 4,
                    cached_input_tokens: 1,
                    output_tokens: 5,
                    reasoning_output_tokens: 0,
                },
                last: TokenUsageBreakdown {
                    total_tokens: 10,
                    input_tokens: 4,
                    cached_input_tokens: 1,
                    output_tokens: 5,
                    reasoning_output_tokens: 0,
                },
                model_context_window,
            },
        })
    }

    fn hook_started_notification(thread_id: ThreadId, turn_id: &str) -> ServerNotification {
        ServerNotification::HookStarted(HookStartedNotification {
            thread_id: thread_id.to_string(),
            turn_id: Some(turn_id.to_string()),
            run: AppServerHookRunSummary {
                id: "user-prompt-submit:0:/tmp/hooks.json".to_string(),
                event_name: AppServerHookEventName::UserPromptSubmit,
                handler_type: AppServerHookHandlerType::Command,
                execution_mode: AppServerHookExecutionMode::Sync,
                scope: AppServerHookScope::Turn,
                source_path: test_path_buf("/tmp/hooks.json").abs(),
                source: codex_app_server_protocol::HookSource::User,
                display_order: 0,
                status: AppServerHookRunStatus::Running,
                status_message: Some("checking go-workflow input policy".to_string()),
                started_at: 1,
                completed_at: None,
                duration_ms: None,
                entries: Vec::new(),
            },
        })
    }

    fn hook_completed_notification(thread_id: ThreadId, turn_id: &str) -> ServerNotification {
        ServerNotification::HookCompleted(HookCompletedNotification {
            thread_id: thread_id.to_string(),
            turn_id: Some(turn_id.to_string()),
            run: AppServerHookRunSummary {
                id: "user-prompt-submit:0:/tmp/hooks.json".to_string(),
                event_name: AppServerHookEventName::UserPromptSubmit,
                handler_type: AppServerHookHandlerType::Command,
                execution_mode: AppServerHookExecutionMode::Sync,
                scope: AppServerHookScope::Turn,
                source_path: test_path_buf("/tmp/hooks.json").abs(),
                source: codex_app_server_protocol::HookSource::User,
                display_order: 0,
                status: AppServerHookRunStatus::Stopped,
                status_message: Some("checking go-workflow input policy".to_string()),
                started_at: 1,
                completed_at: Some(11),
                duration_ms: Some(10),
                entries: vec![
                    AppServerHookOutputEntry {
                        kind: AppServerHookOutputEntryKind::Warning,
                        text: "go-workflow must start from PlanMode".to_string(),
                    },
                    AppServerHookOutputEntry {
                        kind: AppServerHookOutputEntryKind::Stop,
                        text: "prompt blocked".to_string(),
                    },
                ],
            },
        })
    }

    fn agent_message_delta_notification(
        thread_id: ThreadId,
        turn_id: &str,
        item_id: &str,
        delta: &str,
    ) -> ServerNotification {
        ServerNotification::AgentMessageDelta(AgentMessageDeltaNotification {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            item_id: item_id.to_string(),
            delta: delta.to_string(),
        })
    }

    fn exec_approval_request(
        thread_id: ThreadId,
        turn_id: &str,
        item_id: &str,
        approval_id: Option<&str>,
    ) -> ServerRequest {
        ServerRequest::CommandExecutionRequestApproval {
            request_id: AppServerRequestId::Integer(1),
            params: CommandExecutionRequestApprovalParams {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                item_id: item_id.to_string(),
                approval_id: approval_id.map(str::to_string),
                reason: Some("needs approval".to_string()),
                network_approval_context: None,
                command: Some("echo hello".to_string()),
                cwd: Some(test_path_buf("/tmp/project").abs()),
                command_actions: None,
                additional_permissions: None,
                proposed_execpolicy_amendment: None,
                proposed_network_policy_amendments: None,
                available_decisions: None,
            },
        }
    }

    fn request_user_input_request(
        thread_id: ThreadId,
        turn_id: &str,
        item_id: &str,
    ) -> ServerRequest {
        ServerRequest::ToolRequestUserInput {
            request_id: AppServerRequestId::Integer(99),
            params: ToolRequestUserInputParams {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.to_string(),
                item_id: item_id.to_string(),
                questions: Vec::new(),
            },
        }
    }

    #[test]
    fn thread_event_store_tracks_active_turn_lifecycle() {
        let mut store = ThreadEventStore::new(/*capacity*/ 8);
        assert_eq!(store.active_turn_id(), None);

        let thread_id = ThreadId::new();
        store.push_notification(turn_started_notification(thread_id, "turn-1"));
        assert_eq!(store.active_turn_id(), Some("turn-1"));

        store.push_notification(turn_completed_notification(
            thread_id,
            "turn-2",
            TurnStatus::Completed,
        ));
        assert_eq!(store.active_turn_id(), Some("turn-1"));

        store.push_notification(turn_completed_notification(
            thread_id,
            "turn-1",
            TurnStatus::Interrupted,
        ));
        assert_eq!(store.active_turn_id(), None);
    }

    #[test]
    fn thread_event_store_restores_active_turn_from_snapshot_turns() {
        let thread_id = ThreadId::new();
        let session = test_thread_session(thread_id, test_path_buf("/tmp/project"));
        let turns = vec![
            test_turn("turn-1", TurnStatus::Completed, Vec::new()),
            test_turn("turn-2", TurnStatus::InProgress, Vec::new()),
        ];

        let store =
            ThreadEventStore::new_with_session(/*capacity*/ 8, session.clone(), turns.clone());
        assert_eq!(store.active_turn_id(), Some("turn-2"));

        let mut refreshed_store = ThreadEventStore::new(/*capacity*/ 8);
        refreshed_store.set_session(session, turns);
        assert_eq!(refreshed_store.active_turn_id(), Some("turn-2"));
    }

    #[test]
    fn thread_event_store_clear_active_turn_id_resets_cached_turn() {
        let mut store = ThreadEventStore::new(/*capacity*/ 8);
        let thread_id = ThreadId::new();
        store.push_notification(turn_started_notification(thread_id, "turn-1"));

        store.clear_active_turn_id();

        assert_eq!(store.active_turn_id(), None);
    }

    #[test]
    fn thread_event_store_rebase_preserves_resolved_request_state() {
        let thread_id = ThreadId::new();
        let mut store = ThreadEventStore::new(/*capacity*/ 8);
        store.push_request(exec_approval_request(
            thread_id,
            "turn-approval",
            "call-approval",
            /*approval_id*/ None,
        ));
        store.push_notification(ServerNotification::ServerRequestResolved(
            codex_app_server_protocol::ServerRequestResolvedNotification {
                request_id: AppServerRequestId::Integer(1),
                thread_id: thread_id.to_string(),
            },
        ));

        store.rebase_buffer_after_session_refresh();

        let snapshot = store.snapshot();
        assert!(snapshot.events.is_empty());
        assert_eq!(store.has_pending_thread_approvals(), false);
    }

    #[test]
    fn thread_event_store_rebase_preserves_hook_notifications() {
        let thread_id = ThreadId::new();
        let mut store = ThreadEventStore::new(/*capacity*/ 8);
        store.push_notification(hook_started_notification(thread_id, "turn-hook"));
        store.push_notification(hook_completed_notification(thread_id, "turn-hook"));

        store.rebase_buffer_after_session_refresh();

        let snapshot = store.snapshot();
        let hook_notifications = snapshot
            .events
            .into_iter()
            .map(|event| match event {
                ThreadBufferedEvent::Notification(notification) => {
                    serde_json::to_value(notification).expect("hook notification should serialize")
                }
                other => panic!("expected buffered hook notification, saw: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            hook_notifications,
            vec![
                serde_json::to_value(hook_started_notification(thread_id, "turn-hook"))
                    .expect("hook notification should serialize"),
                serde_json::to_value(hook_completed_notification(thread_id, "turn-hook"))
                    .expect("hook notification should serialize"),
            ]
        );
    }

    #[test]
    fn build_feedback_upload_params_includes_thread_id_and_rollout_path() {
        let thread_id = ThreadId::new();
        let rollout_path = PathBuf::from("/tmp/rollout.jsonl");

        let params = build_feedback_upload_params(
            Some(thread_id),
            Some(rollout_path.clone()),
            FeedbackCategory::SafetyCheck,
            Some("needs follow-up".to_string()),
            Some("turn-123".to_string()),
            /*include_logs*/ true,
        );

        assert_eq!(params.classification, "safety_check");
        assert_eq!(params.reason, Some("needs follow-up".to_string()));
        assert_eq!(params.thread_id, Some(thread_id.to_string()));
        assert_eq!(
            params
                .tags
                .as_ref()
                .and_then(|tags| tags.get("turn_id"))
                .map(String::as_str),
            Some("turn-123")
        );
        assert_eq!(params.include_logs, true);
        assert_eq!(params.extra_log_files, Some(vec![rollout_path]));
    }

    #[test]
    fn build_feedback_upload_params_omits_rollout_path_without_logs() {
        let params = build_feedback_upload_params(
            /*origin_thread_id*/ None,
            Some(PathBuf::from("/tmp/rollout.jsonl")),
            FeedbackCategory::GoodResult,
            /*reason*/ None,
            /*turn_id*/ None,
            /*include_logs*/ false,
        );

        assert_eq!(params.classification, "good_result");
        assert_eq!(params.reason, None);
        assert_eq!(params.thread_id, None);
        assert_eq!(params.tags, None);
        assert_eq!(params.include_logs, false);
        assert_eq!(params.extra_log_files, None);
    }

    #[tokio::test]
    async fn feedback_submission_without_thread_emits_error_history_cell() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;

        app.handle_feedback_submitted(
            /*origin_thread_id*/ None,
            FeedbackCategory::Bug,
            /*include_logs*/ true,
            Err("boom".to_string()),
        )
        .await;

        let cell = match app_event_rx.try_recv() {
            Ok(AppEvent::InsertHistoryCell(cell)) => cell,
            other => panic!("expected feedback error history cell, saw {other:?}"),
        };
        assert_eq!(
            lines_to_single_string(&cell.display_lines(/*width*/ 120)),
            "■ Failed to upload feedback: boom"
        );
    }

    #[tokio::test]
    async fn feedback_submission_for_inactive_thread_replays_into_origin_thread() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let origin_thread_id = ThreadId::new();
        let active_thread_id = ThreadId::new();
        let origin_session = test_thread_session(origin_thread_id, test_path_buf("/tmp/origin"));
        let active_session = test_thread_session(active_thread_id, test_path_buf("/tmp/active"));
        app.thread_event_channels.insert(
            origin_thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                origin_session.clone(),
                Vec::new(),
            ),
        );
        app.thread_event_channels.insert(
            active_thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                active_session.clone(),
                Vec::new(),
            ),
        );
        app.activate_thread_channel(active_thread_id).await;
        app.chat_widget.handle_thread_session(active_session);
        while app_event_rx.try_recv().is_ok() {}

        app.handle_feedback_submitted(
            Some(origin_thread_id),
            FeedbackCategory::Bug,
            /*include_logs*/ true,
            Ok("uploaded-thread".to_string()),
        )
        .await;

        assert_matches!(
            app_event_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        );

        let snapshot = {
            let channel = app
                .thread_event_channels
                .get(&origin_thread_id)
                .expect("origin thread channel should exist");
            let store = channel.store.lock().await;
            assert!(matches!(
                store.buffer.back(),
                Some(ThreadBufferedEvent::FeedbackSubmission(_))
            ));
            store.snapshot()
        };

        app.replay_thread_snapshot(snapshot, /*resume_restored_queue*/ false);

        let mut rendered_cells = Vec::new();
        while let Ok(event) = app_event_rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = event {
                rendered_cells.push(lines_to_single_string(&cell.display_lines(/*width*/ 120)));
            }
        }
        assert!(rendered_cells.iter().any(|cell| {
            cell.contains("• Feedback uploaded. Please open an issue using the following URL:")
                && cell.contains("uploaded-thread")
        }));
    }

    fn next_user_turn_op(op_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Op>) -> Op {
        let mut seen = Vec::new();
        while let Ok(op) = op_rx.try_recv() {
            if matches!(op, Op::UserTurn { .. }) {
                return op;
            }
            seen.push(format!("{op:?}"));
        }
        panic!("expected UserTurn op, saw: {seen:?}");
    }

    fn lines_to_single_string(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn test_session_telemetry(config: &Config, model: &str) -> SessionTelemetry {
        let model_info =
            crate::legacy_core::test_support::construct_model_info_offline(model, config);
        SessionTelemetry::new(
            ThreadId::new(),
            model,
            model_info.slug.as_str(),
            /*account_id*/ None,
            /*account_email*/ None,
            /*auth_mode*/ None,
            "test_originator".to_string(),
            /*log_user_prompts*/ false,
            "test".to_string(),
            SessionSource::Cli,
        )
    }

    fn app_enabled_in_effective_config(config: &Config, app_id: &str) -> Option<bool> {
        config
            .config_layer_stack
            .effective_config()
            .as_table()
            .and_then(|table| table.get("apps"))
            .and_then(TomlValue::as_table)
            .and_then(|apps| apps.get(app_id))
            .and_then(TomlValue::as_table)
            .and_then(|app| app.get("enabled"))
            .and_then(TomlValue::as_bool)
    }

    fn all_model_presets() -> Vec<ModelPreset> {
        crate::legacy_core::test_support::all_model_presets().clone()
    }

    fn model_availability_nux_config(shown_count: &[(&str, u32)]) -> ModelAvailabilityNuxConfig {
        ModelAvailabilityNuxConfig {
            shown_count: shown_count
                .iter()
                .map(|(model, count)| ((*model).to_string(), *count))
                .collect(),
        }
    }

    fn model_migration_copy_to_plain_text(
        copy: &crate::model_migration::ModelMigrationCopy,
    ) -> String {
        if let Some(markdown) = copy.markdown.as_ref() {
            return markdown.clone();
        }
        let mut s = String::new();
        for span in &copy.heading {
            s.push_str(&span.content);
        }
        s.push('\n');
        s.push('\n');
        for line in &copy.content {
            for span in &line.spans {
                s.push_str(&span.content);
            }
            s.push('\n');
        }
        s
    }

    #[tokio::test]
    async fn model_migration_prompt_only_shows_for_deprecated_models() {
        let seen = BTreeMap::new();
        assert!(should_show_model_migration_prompt(
            "gpt-5",
            "gpt-5.2-codex",
            &seen,
            &all_model_presets()
        ));
        assert!(should_show_model_migration_prompt(
            "gpt-5-codex",
            "gpt-5.2-codex",
            &seen,
            &all_model_presets()
        ));
        assert!(should_show_model_migration_prompt(
            "gpt-5-codex-mini",
            "gpt-5.2-codex",
            &seen,
            &all_model_presets()
        ));
        assert!(should_show_model_migration_prompt(
            "gpt-5.1-codex",
            "gpt-5.2-codex",
            &seen,
            &all_model_presets()
        ));
        assert!(!should_show_model_migration_prompt(
            "gpt-5.1-codex",
            "gpt-5.1-codex",
            &seen,
            &all_model_presets()
        ));
    }

    #[test]
    fn select_model_availability_nux_picks_only_eligible_model() {
        let mut presets = all_model_presets();
        presets.iter_mut().for_each(|preset| {
            preset.availability_nux = None;
        });
        let target = presets
            .iter_mut()
            .find(|preset| preset.model == "gpt-5")
            .expect("target preset present");
        target.availability_nux = Some(ModelAvailabilityNux {
            message: "gpt-5 is available".to_string(),
        });

        let selected = select_model_availability_nux(&presets, &model_availability_nux_config(&[]));

        assert_eq!(
            selected,
            Some(StartupTooltipOverride {
                model_slug: "gpt-5".to_string(),
                message: "gpt-5 is available".to_string(),
            })
        );
    }

    #[test]
    fn select_model_availability_nux_skips_missing_and_exhausted_models() {
        let mut presets = all_model_presets();
        presets.iter_mut().for_each(|preset| {
            preset.availability_nux = None;
        });
        let gpt_5 = presets
            .iter_mut()
            .find(|preset| preset.model == "gpt-5")
            .expect("gpt-5 preset present");
        gpt_5.availability_nux = Some(ModelAvailabilityNux {
            message: "gpt-5 is available".to_string(),
        });
        let gpt_5_2 = presets
            .iter_mut()
            .find(|preset| preset.model == "gpt-5.2")
            .expect("gpt-5.2 preset present");
        gpt_5_2.availability_nux = Some(ModelAvailabilityNux {
            message: "gpt-5.2 is available".to_string(),
        });

        let selected = select_model_availability_nux(
            &presets,
            &model_availability_nux_config(&[("gpt-5", MODEL_AVAILABILITY_NUX_MAX_SHOW_COUNT)]),
        );

        assert_eq!(
            selected,
            Some(StartupTooltipOverride {
                model_slug: "gpt-5.2".to_string(),
                message: "gpt-5.2 is available".to_string(),
            })
        );
    }

    #[test]
    fn active_turn_not_steerable_turn_error_extracts_structured_server_error() {
        let turn_error = AppServerTurnError {
            message: "cannot steer a review turn".to_string(),
            codex_error_info: Some(AppServerCodexErrorInfo::ActiveTurnNotSteerable {
                turn_kind: AppServerNonSteerableTurnKind::Review,
            }),
            additional_details: None,
        };
        let error = TypedRequestError::Server {
            method: "turn/steer".to_string(),
            source: JSONRPCErrorError {
                code: -32602,
                message: turn_error.message.clone(),
                data: Some(serde_json::to_value(&turn_error).expect("turn error should serialize")),
            },
        };

        assert_eq!(
            active_turn_not_steerable_turn_error(&error),
            Some(turn_error)
        );
    }

    #[test]
    fn active_turn_steer_race_detects_missing_active_turn() {
        let error = TypedRequestError::Server {
            method: "turn/steer".to_string(),
            source: JSONRPCErrorError {
                code: -32602,
                message: "no active turn to steer".to_string(),
                data: None,
            },
        };

        assert_eq!(
            active_turn_steer_race(&error),
            Some(ActiveTurnSteerRace::Missing)
        );
        assert_eq!(active_turn_not_steerable_turn_error(&error), None);
    }

    #[test]
    fn active_turn_steer_race_extracts_actual_turn_id_from_mismatch() {
        let error = TypedRequestError::Server {
            method: "turn/steer".to_string(),
            source: JSONRPCErrorError {
                code: -32602,
                message: "expected active turn id `turn-expected` but found `turn-actual`"
                    .to_string(),
                data: None,
            },
        };

        assert_eq!(
            active_turn_steer_race(&error),
            Some(ActiveTurnSteerRace::ExpectedTurnMismatch {
                actual_turn_id: "turn-actual".to_string(),
            })
        );
    }

    #[test]
    fn select_model_availability_nux_uses_existing_model_order_as_priority() {
        let mut presets = all_model_presets();
        presets.iter_mut().for_each(|preset| {
            preset.availability_nux = None;
        });
        let first = presets
            .iter_mut()
            .find(|preset| preset.model == "gpt-5")
            .expect("gpt-5 preset present");
        first.availability_nux = Some(ModelAvailabilityNux {
            message: "first".to_string(),
        });
        let second = presets
            .iter_mut()
            .find(|preset| preset.model == "gpt-5.2")
            .expect("gpt-5.2 preset present");
        second.availability_nux = Some(ModelAvailabilityNux {
            message: "second".to_string(),
        });

        let selected = select_model_availability_nux(&presets, &model_availability_nux_config(&[]));

        assert_eq!(
            selected,
            Some(StartupTooltipOverride {
                model_slug: "gpt-5.2".to_string(),
                message: "second".to_string(),
            })
        );
    }

    #[test]
    fn select_model_availability_nux_returns_none_when_all_models_are_exhausted() {
        let mut presets = all_model_presets();
        presets.iter_mut().for_each(|preset| {
            preset.availability_nux = None;
        });
        let target = presets
            .iter_mut()
            .find(|preset| preset.model == "gpt-5")
            .expect("target preset present");
        target.availability_nux = Some(ModelAvailabilityNux {
            message: "gpt-5 is available".to_string(),
        });

        let selected = select_model_availability_nux(
            &presets,
            &model_availability_nux_config(&[("gpt-5", MODEL_AVAILABILITY_NUX_MAX_SHOW_COUNT)]),
        );

        assert_eq!(selected, None);
    }

    #[tokio::test]
    async fn model_migration_prompt_respects_hide_flag_and_self_target() {
        let mut seen = BTreeMap::new();
        seen.insert("gpt-5".to_string(), "gpt-5.1".to_string());
        assert!(!should_show_model_migration_prompt(
            "gpt-5",
            "gpt-5.1",
            &seen,
            &all_model_presets()
        ));
        assert!(!should_show_model_migration_prompt(
            "gpt-5.1",
            "gpt-5.1",
            &seen,
            &all_model_presets()
        ));
    }

    #[tokio::test]
    async fn model_migration_prompt_skips_when_target_missing_or_hidden() {
        let mut available = all_model_presets();
        let mut current = available
            .iter()
            .find(|preset| preset.model == "gpt-5-codex")
            .cloned()
            .expect("preset present");
        current.upgrade = Some(ModelUpgrade {
            id: "missing-target".to_string(),
            reasoning_effort_mapping: None,
            migration_config_key: HIDE_GPT5_1_MIGRATION_PROMPT_CONFIG.to_string(),
            model_link: None,
            upgrade_copy: None,
            migration_markdown: None,
        });
        available.retain(|preset| preset.model != "gpt-5-codex");
        available.push(current.clone());

        assert!(!should_show_model_migration_prompt(
            &current.model,
            "missing-target",
            &BTreeMap::new(),
            &available,
        ));

        assert!(target_preset_for_upgrade(&available, "missing-target").is_none());

        let mut with_hidden_target = all_model_presets();
        let target = with_hidden_target
            .iter_mut()
            .find(|preset| preset.model == "gpt-5.2-codex")
            .expect("target preset present");
        target.show_in_picker = false;

        assert!(!should_show_model_migration_prompt(
            "gpt-5-codex",
            "gpt-5.2-codex",
            &BTreeMap::new(),
            &with_hidden_target,
        ));
        assert!(target_preset_for_upgrade(&with_hidden_target, "gpt-5.2-codex").is_none());
    }

    #[tokio::test]
    async fn model_migration_prompt_shows_for_hidden_model() {
        let codex_home = tempdir().expect("temp codex home");
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let mut available_models = all_model_presets();
        let current = available_models
            .iter()
            .find(|preset| preset.model == "gpt-5.1-codex")
            .cloned()
            .expect("gpt-5.1-codex preset present");
        assert!(
            !current.show_in_picker,
            "expected gpt-5.1-codex to be hidden from picker for this test"
        );

        let upgrade = current.upgrade.as_ref().expect("upgrade configured");
        // Test "hidden current model still prompts" even if bundled
        // catalog data changes the target model's picker visibility.
        available_models
            .iter_mut()
            .find(|preset| preset.model == upgrade.id)
            .expect("upgrade target present")
            .show_in_picker = true;
        assert!(
            should_show_model_migration_prompt(
                &current.model,
                &upgrade.id,
                &config.notices.model_migrations,
                &available_models,
            ),
            "expected migration prompt to be eligible for hidden model"
        );

        let target = target_preset_for_upgrade(&available_models, &upgrade.id)
            .expect("upgrade target present");
        let target_description =
            (!target.description.is_empty()).then(|| target.description.clone());
        let can_opt_out = true;
        let copy = migration_copy_for_models(
            &current.model,
            &upgrade.id,
            upgrade.model_link.clone(),
            upgrade.upgrade_copy.clone(),
            upgrade.migration_markdown.clone(),
            target.display_name.clone(),
            target_description,
            can_opt_out,
        );

        // Snapshot the copy we would show; rendering is covered by model_migration snapshots.
        assert_snapshot!(
            "model_migration_prompt_shows_for_hidden_model",
            model_migration_copy_to_plain_text(&copy)
        );
    }

    #[tokio::test]
    async fn update_reasoning_effort_updates_collaboration_mode() {
        let mut app = make_test_app().await;
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::Medium));

        app.on_update_reasoning_effort(Some(ReasoningEffortConfig::High));

        assert_eq!(
            app.chat_widget.current_reasoning_effort(),
            Some(ReasoningEffortConfig::High)
        );
        assert_eq!(
            app.config.model_reasoning_effort,
            Some(ReasoningEffortConfig::High)
        );
    }

    #[tokio::test]
    async fn refresh_in_memory_config_from_disk_loads_latest_apps_state() -> Result<()> {
        let mut app = make_test_app().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        let app_id = "unit_test_refresh_in_memory_config_connector".to_string();

        assert_eq!(app_enabled_in_effective_config(&app.config, &app_id), None);

        ConfigEditsBuilder::new(&app.config.codex_home)
            .with_edits([
                ConfigEdit::SetPath {
                    segments: vec!["apps".to_string(), app_id.clone(), "enabled".to_string()],
                    value: false.into(),
                },
                ConfigEdit::SetPath {
                    segments: vec![
                        "apps".to_string(),
                        app_id.clone(),
                        "disabled_reason".to_string(),
                    ],
                    value: "user".into(),
                },
            ])
            .apply()
            .await
            .expect("persist app toggle");

        assert_eq!(app_enabled_in_effective_config(&app.config, &app_id), None);

        app.refresh_in_memory_config_from_disk().await?;

        assert_eq!(
            app_enabled_in_effective_config(&app.config, &app_id),
            Some(false)
        );
        Ok(())
    }

    #[tokio::test]
    async fn refresh_in_memory_config_from_disk_best_effort_keeps_current_config_on_error()
    -> Result<()> {
        let mut app = make_test_app().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        std::fs::write(codex_home.path().join("config.toml"), "[broken")?;
        let original_config = app.config.clone();

        app.refresh_in_memory_config_from_disk_best_effort("starting a new thread")
            .await;

        assert_eq!(app.config, original_config);
        Ok(())
    }

    #[tokio::test]
    async fn refresh_in_memory_config_from_disk_uses_active_chat_widget_cwd() -> Result<()> {
        let mut app = make_test_app().await;
        let original_cwd = app.config.cwd.clone();
        let next_cwd_tmp = tempdir()?;
        let next_cwd = next_cwd_tmp.path().to_path_buf();

        app.chat_widget.handle_codex_event(Event {
            id: String::new(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: ThreadId::new(),
                forked_from_id: None,
                thread_name: None,
                model: "gpt-test".to_string(),
                model_provider_id: "test-provider".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                cwd: next_cwd.clone().abs(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(PathBuf::new()),
            }),
        });

        assert_eq!(app.chat_widget.config_ref().cwd.to_path_buf(), next_cwd);
        assert_eq!(app.config.cwd, original_cwd);

        app.refresh_in_memory_config_from_disk().await?;

        assert_eq!(app.config.cwd, app.chat_widget.config_ref().cwd);
        Ok(())
    }

    #[tokio::test]
    async fn refresh_in_memory_config_from_disk_syncs_active_profile() -> Result<()> {
        let mut app = make_test_app().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        app.active_profile = Some("stale".to_string());

        std::fs::write(
            codex_home.path().join("config.toml"),
            r#"
profile = "fresh"

[profiles.fresh]
model = "gpt-5.2"
"#,
        )?;

        app.refresh_in_memory_config_from_disk().await?;

        assert_eq!(app.config.active_profile.as_deref(), Some("fresh"));
        assert_eq!(app.active_profile.as_deref(), Some("fresh"));
        Ok(())
    }

    #[tokio::test]
    async fn routed_profile_runtime_changed_detects_profile_and_provider_reload_inputs() {
        let app = make_test_app().await;
        let current_config = app.config.clone();

        let mut next_profile = current_config.clone();
        next_profile.active_profile = Some("secondary".to_string());
        assert!(App::routed_profile_runtime_changed(
            &current_config,
            &next_profile
        ));

        let mut next_provider = current_config.clone();
        next_provider.model_provider.base_url = Some("https://example.com/v1".to_string());
        assert!(App::routed_profile_runtime_changed(
            &current_config,
            &next_provider
        ));

        let mut next_chatgpt_base_url = current_config.clone();
        next_chatgpt_base_url.chatgpt_base_url =
            "https://chatgpt.example.com/backend-api/".to_string();
        assert!(App::routed_profile_runtime_changed(
            &current_config,
            &next_chatgpt_base_url
        ));

        assert!(!App::routed_profile_runtime_changed(
            &current_config,
            &current_config
        ));
    }

    #[tokio::test]
    async fn rebuild_config_for_resume_or_fallback_uses_current_config_on_same_cwd_error()
    -> Result<()> {
        let mut app = make_test_app().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        std::fs::write(codex_home.path().join("config.toml"), "[broken")?;
        let current_config = app.config.clone();
        let current_cwd = current_config.cwd.clone();

        let resume_config = app
            .rebuild_config_for_resume_or_fallback(&current_cwd, current_cwd.to_path_buf())
            .await?;

        assert_eq!(resume_config, current_config);
        Ok(())
    }

    #[tokio::test]
    async fn rebuild_config_for_resume_or_fallback_errors_when_cwd_changes() -> Result<()> {
        let mut app = make_test_app().await;
        let codex_home = tempdir()?;
        app.config.codex_home = codex_home.path().to_path_buf().abs();
        std::fs::write(codex_home.path().join("config.toml"), "[broken")?;
        let current_cwd = app.config.cwd.clone();
        let next_cwd_tmp = tempdir()?;
        let next_cwd = next_cwd_tmp.path().to_path_buf();

        let result = app
            .rebuild_config_for_resume_or_fallback(&current_cwd, next_cwd)
            .await;

        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn sync_tui_theme_selection_updates_chat_widget_config_copy() {
        let mut app = make_test_app().await;

        app.sync_tui_theme_selection("dracula".to_string());

        assert_eq!(app.config.tui_theme.as_deref(), Some("dracula"));
        assert_eq!(
            app.chat_widget.config_ref().tui_theme.as_deref(),
            Some("dracula")
        );
    }

    #[tokio::test]
    async fn fresh_session_config_uses_current_service_tier() {
        let mut app = make_test_app().await;
        app.chat_widget
            .set_service_tier(Some(codex_protocol::config_types::ServiceTier::Fast));

        let config = app.fresh_session_config();

        assert_eq!(
            config.service_tier,
            Some(codex_protocol::config_types::ServiceTier::Fast)
        );
    }

    #[tokio::test]
    async fn backtrack_selection_with_duplicate_history_targets_unique_turn() {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;

        let user_cell = |text: &str,
                         text_elements: Vec<TextElement>,
                         local_image_paths: Vec<PathBuf>,
                         remote_image_urls: Vec<String>|
         -> Arc<dyn HistoryCell> {
            Arc::new(UserHistoryCell {
                message: text.to_string(),
                text_elements,
                local_image_paths,
                remote_image_urls,
            }) as Arc<dyn HistoryCell>
        };
        let agent_cell = |text: &str| -> Arc<dyn HistoryCell> {
            Arc::new(AgentMessageCell::new(
                vec![Line::from(text.to_string())],
                /*is_first_line*/ true,
            )) as Arc<dyn HistoryCell>
        };

        let make_header = |is_first| {
            let event = SessionConfiguredEvent {
                session_id: ThreadId::new(),
                forked_from_id: None,
                thread_name: None,
                model: "gpt-test".to_string(),
                model_provider_id: "test-provider".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                cwd: test_path_buf("/home/user/project").abs(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(PathBuf::new()),
            };
            Arc::new(new_session_info(
                app.chat_widget.config_ref(),
                app.chat_widget.current_model(),
                event,
                is_first,
                /*tooltip_override*/ None,
                /*auth_plan*/ None,
                /*show_fast_status*/ false,
            )) as Arc<dyn HistoryCell>
        };

        let placeholder = "[Image #1]";
        let edited_text = format!("follow-up (edited) {placeholder}");
        let edited_range = edited_text.len().saturating_sub(placeholder.len())..edited_text.len();
        let edited_text_elements = vec![TextElement::new(
            edited_range.into(),
            /*placeholder*/ None,
        )];
        let edited_local_image_paths = vec![PathBuf::from("/tmp/fake-image.png")];

        // Simulate a transcript with duplicated history (e.g., from prior backtracks)
        // and an edited turn appended after a session header boundary.
        app.transcript_cells = vec![
            make_header(true),
            user_cell("first question", Vec::new(), Vec::new(), Vec::new()),
            agent_cell("answer first"),
            user_cell("follow-up", Vec::new(), Vec::new(), Vec::new()),
            agent_cell("answer follow-up"),
            make_header(false),
            user_cell("first question", Vec::new(), Vec::new(), Vec::new()),
            agent_cell("answer first"),
            user_cell(
                &edited_text,
                edited_text_elements.clone(),
                edited_local_image_paths.clone(),
                vec!["https://example.com/backtrack.png".to_string()],
            ),
            agent_cell("answer edited"),
        ];

        assert_eq!(user_count(&app.transcript_cells), 2);

        let base_id = ThreadId::new();
        app.chat_widget.handle_codex_event(Event {
            id: String::new(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: base_id,
                forked_from_id: None,
                thread_name: None,
                model: "gpt-test".to_string(),
                model_provider_id: "test-provider".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                cwd: test_path_buf("/home/user/project").abs(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(PathBuf::new()),
            }),
        });

        app.backtrack.base_id = Some(base_id);
        app.backtrack.primed = true;
        app.backtrack.nth_user_message = user_count(&app.transcript_cells).saturating_sub(1);

        let selection = app
            .confirm_backtrack_from_main()
            .expect("backtrack selection");
        assert_eq!(selection.nth_user_message, 1);
        assert_eq!(selection.prefill, edited_text);
        assert_eq!(selection.text_elements, edited_text_elements);
        assert_eq!(selection.local_image_paths, edited_local_image_paths);
        assert_eq!(
            selection.remote_image_urls,
            vec!["https://example.com/backtrack.png".to_string()]
        );

        app.apply_backtrack_rollback(selection);
        assert_eq!(
            app.chat_widget.remote_image_urls(),
            vec!["https://example.com/backtrack.png".to_string()]
        );

        let mut rollback_turns = None;
        while let Ok(op) = op_rx.try_recv() {
            if let Op::ThreadRollback { num_turns } = op {
                rollback_turns = Some(num_turns);
            }
        }

        assert_eq!(rollback_turns, Some(1));
    }

    #[tokio::test]
    async fn backtrack_remote_image_only_selection_clears_existing_composer_draft() {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;

        app.transcript_cells = vec![Arc::new(UserHistoryCell {
            message: "original".to_string(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: Vec::new(),
        }) as Arc<dyn HistoryCell>];
        app.chat_widget
            .set_composer_text("stale draft".to_string(), Vec::new(), Vec::new());

        let remote_image_url = "https://example.com/remote-only.png".to_string();
        app.apply_backtrack_rollback(BacktrackSelection {
            nth_user_message: 0,
            prefill: String::new(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: vec![remote_image_url.clone()],
        });

        assert_eq!(app.chat_widget.composer_text_with_pending(), "");
        assert_eq!(app.chat_widget.remote_image_urls(), vec![remote_image_url]);

        let mut rollback_turns = None;
        while let Ok(op) = op_rx.try_recv() {
            if let Op::ThreadRollback { num_turns } = op {
                rollback_turns = Some(num_turns);
            }
        }
        assert_eq!(rollback_turns, Some(1));
    }

    #[tokio::test]
    async fn backtrack_resubmit_preserves_data_image_urls_in_user_turn() {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;

        let thread_id = ThreadId::new();
        app.chat_widget.handle_codex_event(Event {
            id: String::new(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: thread_id,
                forked_from_id: None,
                thread_name: None,
                model: "gpt-test".to_string(),
                model_provider_id: "test-provider".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                cwd: test_path_buf("/home/user/project").abs(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(PathBuf::new()),
            }),
        });

        let data_image_url = "data:image/png;base64,abc123".to_string();
        app.transcript_cells = vec![Arc::new(UserHistoryCell {
            message: "please inspect this".to_string(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: vec![data_image_url.clone()],
        }) as Arc<dyn HistoryCell>];

        app.apply_backtrack_rollback(BacktrackSelection {
            nth_user_message: 0,
            prefill: "please inspect this".to_string(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: vec![data_image_url.clone()],
        });

        app.chat_widget
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let mut saw_rollback = false;
        let mut submitted_items: Option<Vec<UserInput>> = None;
        while let Ok(op) = op_rx.try_recv() {
            match op {
                Op::ThreadRollback { .. } => saw_rollback = true,
                Op::UserTurn { items, .. } => submitted_items = Some(items),
                _ => {}
            }
        }

        assert!(saw_rollback);
        let items = submitted_items.expect("expected user turn after backtrack resubmit");
        assert!(items.iter().any(|item| {
            matches!(
                item,
                UserInput::Image { image_url } if image_url == &data_image_url
            )
        }));
    }

    #[tokio::test]
    async fn undo_last_user_message_restores_latest_user_input_and_rolls_back_one_turn() {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;

        let remote_image_url = "https://example.com/latest.png".to_string();
        app.transcript_cells = vec![
            Arc::new(UserHistoryCell {
                message: "first".to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn HistoryCell>,
            Arc::new(UserHistoryCell {
                message: "latest".to_string(),
                text_elements: vec![TextElement::new(
                    codex_protocol::user_input::ByteRange { start: 0, end: 6 },
                    Some("latest".to_string()),
                )],
                local_image_paths: Vec::new(),
                remote_image_urls: vec![remote_image_url.clone()],
            }) as Arc<dyn HistoryCell>,
        ];
        app.chat_widget
            .set_composer_text("stale draft".to_string(), Vec::new(), Vec::new());

        assert!(app.undo_last_user_message());
        assert_eq!(app.chat_widget.composer_text_with_pending(), "latest");
        assert_eq!(app.chat_widget.remote_image_urls(), vec![remote_image_url]);

        let mut rollback_turns = None;
        while let Ok(op) = op_rx.try_recv() {
            if let Op::ThreadRollback { num_turns } = op {
                rollback_turns = Some(num_turns);
            }
        }

        assert_eq!(rollback_turns, Some(1));
    }

    #[tokio::test]
    async fn ctrl_x_digit_switches_to_agent_slot() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;

        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000002").expect("valid thread");
        app.primary_thread_id = Some(main_thread_id);
        app.upsert_agent_picker_thread(
            main_thread_id,
            Some("main".to_string()),
            Some("default".to_string()),
            /*is_closed*/ false,
        );
        app.upsert_agent_picker_thread(
            agent_thread_id,
            Some("btw".to_string()),
            Some("btw".to_string()),
            /*is_closed*/ false,
        );
        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE,)),
            None
        );
        assert_matches!(
            app_event_rx.try_recv(),
            Ok(AppEvent::SelectAgentThread(selected_thread_id))
                if selected_thread_id == agent_thread_id
        );
        assert_eq!(app.chat_widget.composer_text_with_pending(), "");
    }

    #[tokio::test]
    async fn ctrl_x_unknown_second_key_falls_through_to_composer_input() {
        let mut app = make_test_app().await;

        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        let forwarded =
            app.handle_key_chord_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(
            forwarded,
            Some(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        );
    }

    #[tokio::test]
    async fn ctrl_x_ctrl_digit_does_not_switch_threads() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;

        let main_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid thread");
        let agent_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000002").expect("valid thread");
        app.primary_thread_id = Some(main_thread_id);
        app.upsert_agent_picker_thread(
            main_thread_id,
            Some("main".to_string()),
            Some("default".to_string()),
            /*is_closed*/ false,
        );
        app.upsert_agent_picker_thread(
            agent_thread_id,
            Some("btw".to_string()),
            Some("btw".to_string()),
            /*is_closed*/ false,
        );

        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('2'),
                KeyModifiers::CONTROL,
            )),
            Some(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::CONTROL))
        );
        assert!(app_event_rx.try_recv().is_err());
        assert_eq!(app.chat_widget.composer_text_with_pending(), "");
    }

    #[tokio::test]
    async fn ctrl_x_ctrl_r_requests_respawn() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;

        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        assert_matches!(app_event_rx.try_recv(), Ok(AppEvent::RespawnRequested));
    }

    #[tokio::test]
    async fn ctrl_x_ctrl_u_requests_undo_last_user_message() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;

        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        assert_eq!(
            app.handle_key_chord_key_event(KeyEvent::new(
                KeyCode::Char('u'),
                KeyModifiers::CONTROL,
            )),
            None
        );
        assert_matches!(app_event_rx.try_recv(), Ok(AppEvent::UndoLastUserMessage));
    }

    #[tokio::test]
    async fn replay_thread_snapshot_replays_turn_history_in_order() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: Some(test_thread_session(
                    thread_id,
                    test_path_buf("/home/user/project"),
                )),
                turns: vec![
                    Turn {
                        id: "turn-1".to_string(),
                        items: vec![ThreadItem::UserMessage {
                            id: "user-1".to_string(),
                            content: vec![AppServerUserInput::Text {
                                text: "first prompt".to_string(),
                                text_elements: Vec::new(),
                            }],
                        }],
                        status: TurnStatus::Completed,
                        error: None,
                        started_at: None,
                        completed_at: None,
                        duration_ms: None,
                    },
                    Turn {
                        id: "turn-2".to_string(),
                        items: vec![
                            ThreadItem::UserMessage {
                                id: "user-2".to_string(),
                                content: vec![AppServerUserInput::Text {
                                    text: "third prompt".to_string(),
                                    text_elements: Vec::new(),
                                }],
                            },
                            ThreadItem::AgentMessage {
                                id: "assistant-2".to_string(),
                                text: "done".to_string(),
                                phase: None,
                                memory_citation: None,
                            },
                        ],
                        status: TurnStatus::Completed,
                        error: None,
                        started_at: None,
                        completed_at: None,
                        duration_ms: None,
                    },
                ],
                events: Vec::new(),
                input_state: None,
            },
            /*resume_restored_queue*/ false,
        );

        while let Ok(event) = app_event_rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = event {
                let cell: Arc<dyn HistoryCell> = cell.into();
                app.transcript_cells.push(cell);
            }
        }

        let user_messages: Vec<String> = app
            .transcript_cells
            .iter()
            .filter_map(|cell| {
                cell.as_any()
                    .downcast_ref::<UserHistoryCell>()
                    .map(|cell| cell.message.clone())
            })
            .collect();
        assert_eq!(
            user_messages,
            vec!["first prompt".to_string(), "third prompt".to_string()]
        );
    }

    #[tokio::test]
    async fn replay_thread_snapshot_queues_workflow_history_after_turn_replay() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        app.active_thread_id = Some(thread_id);
        let stored_cell: Arc<dyn HistoryCell> = Arc::new(history_cell::new_info_event(
            "Workflow reply".to_string(),
            Some("director/review_backlog".to_string()),
        ));
        let _ = app.record_workflow_history_cell(thread_id, stored_cell);

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: Some(test_thread_session(
                    thread_id,
                    PathBuf::from("/home/user/project"),
                )),
                turns: vec![test_turn(
                    "turn-1",
                    TurnStatus::Completed,
                    vec![ThreadItem::UserMessage {
                        id: "user-1".to_string(),
                        content: vec![AppServerUserInput::Text {
                            text: "first prompt".to_string(),
                            text_elements: Vec::new(),
                        }],
                    }],
                )],
                events: Vec::new(),
                input_state: None,
            },
            /*resume_restored_queue*/ false,
        );

        let mut replay_order = Vec::new();
        while let Ok(event) = app_event_rx.try_recv() {
            match event {
                AppEvent::InsertHistoryCell(cell) => {
                    let cell: Arc<dyn HistoryCell> = cell.into();
                    let transcript = lines_to_single_string(&cell.transcript_lines(/*width*/ 80));
                    if transcript.contains("first prompt") {
                        replay_order.push("turn");
                    }
                    app.transcript_cells.push(cell);
                }
                AppEvent::ReplayWorkflowHistory {
                    thread_id: replay_thread_id,
                } => {
                    assert_eq!(replay_thread_id, thread_id);
                    replay_order.push("workflow");
                    let lines = app.replay_workflow_history_cells_for_thread(
                        replay_thread_id,
                        /*width*/ 80,
                    );
                    assert!(lines_to_single_string(&lines).contains("Workflow reply"));
                }
                _ => {}
            }
        }

        assert_eq!(replay_order, vec!["turn", "workflow"]);
        let transcript: Vec<String> = app
            .transcript_cells
            .iter()
            .map(|cell| lines_to_single_string(&cell.transcript_lines(/*width*/ 80)))
            .collect();
        assert!(transcript.iter().any(|cell| cell.contains("first prompt")));
        assert!(
            transcript
                .last()
                .is_some_and(|cell| cell.contains("Workflow reply"))
        );
    }

    #[tokio::test]
    async fn enqueue_primary_thread_session_queues_workflow_history_after_turn_replay() -> Result<()>
    {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        let stored_cell: Arc<dyn HistoryCell> = Arc::new(history_cell::new_info_event(
            "Workflow reply".to_string(),
            Some("director/review_backlog".to_string()),
        ));
        let _ = app.record_workflow_history_cell(thread_id, stored_cell);

        app.enqueue_primary_thread_session(
            test_thread_session(thread_id, PathBuf::from("/tmp/project")),
            vec![test_turn(
                "turn-1",
                TurnStatus::Completed,
                vec![ThreadItem::UserMessage {
                    id: "user-1".to_string(),
                    content: vec![AppServerUserInput::Text {
                        text: "earlier prompt".to_string(),
                        text_elements: Vec::new(),
                    }],
                }],
            )],
        )
        .await?;

        let mut replay_order = Vec::new();
        while let Ok(event) = app_event_rx.try_recv() {
            match event {
                AppEvent::InsertHistoryCell(cell) => {
                    let transcript = lines_to_single_string(&cell.transcript_lines(/*width*/ 80));
                    if transcript.contains("earlier prompt") {
                        replay_order.push("turn");
                    }
                }
                AppEvent::ReplayWorkflowHistory {
                    thread_id: replay_thread_id,
                } => {
                    assert_eq!(replay_thread_id, thread_id);
                    replay_order.push("workflow");
                }
                _ => {}
            }
        }

        assert_eq!(replay_order, vec!["turn", "workflow"]);
        Ok(())
    }

    #[tokio::test]
    async fn replace_chat_widget_reseeds_collab_agent_metadata_for_replay() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let receiver_thread_id =
            ThreadId::from_string("019cff70-2599-75e2-af72-b958ce5dc1cc").expect("valid thread");
        app.agent_navigation.upsert(
            receiver_thread_id,
            Some("Robie".to_string()),
            Some("explorer".to_string()),
            /*is_closed*/ false,
        );

        let replacement = ChatWidget::new_with_app_event(ChatWidgetInit {
            config: app.config.clone(),
            display_preferences: app.display_preferences.clone(),
            frame_requester: crate::tui::FrameRequester::test_dummy(),
            app_event_tx: app.app_event_tx.clone(),
            initial_user_message: None,
            enhanced_keys_supported: app.enhanced_keys_supported,
            has_chatgpt_account: app.chat_widget.has_chatgpt_account(),
            model_catalog: app.model_catalog.clone(),
            feedback: app.feedback.clone(),
            is_first_run: false,
            status_account_display: app.chat_widget.status_account_display().cloned(),
            initial_plan_type: app.chat_widget.current_plan_type(),
            model: Some(app.chat_widget.current_model().to_string()),
            startup_tooltip_override: None,
            status_line_invalid_items_warned: app.status_line_invalid_items_warned.clone(),
            terminal_title_invalid_items_warned: app.terminal_title_invalid_items_warned.clone(),
            session_telemetry: app.session_telemetry.clone(),
        });
        app.replace_chat_widget(replacement);

        app.replay_thread_snapshot(
            ThreadEventSnapshot {
                session: None,
                turns: Vec::new(),
                events: vec![ThreadBufferedEvent::Notification(
                    ServerNotification::ItemStarted(codex_app_server_protocol::ItemStartedNotification {
                        thread_id: "thread-1".to_string(),
                        turn_id: "turn-1".to_string(),
                        item: ThreadItem::CollabAgentToolCall {
                            id: "wait-1".to_string(),
                            tool: codex_app_server_protocol::CollabAgentTool::Wait,
                            status: codex_app_server_protocol::CollabAgentToolCallStatus::InProgress,
                            sender_thread_id: ThreadId::new().to_string(),
                            receiver_thread_ids: vec![receiver_thread_id.to_string()],
                            prompt: None,
                            model: None,
                            reasoning_effort: None,
                            agents_states: HashMap::new(),
                        },
                    }),
                )],
                input_state: None,
            },
            /*resume_restored_queue*/ false,
        );

        let mut saw_named_wait = false;
        while let Ok(event) = app_event_rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = event {
                let transcript = lines_to_single_string(&cell.transcript_lines(/*width*/ 80));
                saw_named_wait |= transcript.contains("Robie [explorer]");
            }
        }

        assert!(
            saw_named_wait,
            "expected replayed wait item to keep agent name"
        );
    }

    #[tokio::test]
    async fn refreshed_snapshot_session_persists_resumed_turns() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let initial_session = test_thread_session(thread_id, test_path_buf("/tmp/original"));
        app.thread_event_channels.insert(
            thread_id,
            ThreadEventChannel::new_with_session(
                /*capacity*/ 4,
                initial_session.clone(),
                Vec::new(),
            ),
        );

        let resumed_turns = vec![test_turn(
            "turn-1",
            TurnStatus::Completed,
            vec![ThreadItem::UserMessage {
                id: "user-1".to_string(),
                content: vec![AppServerUserInput::Text {
                    text: "restored prompt".to_string(),
                    text_elements: Vec::new(),
                }],
            }],
        )];
        let resumed_session = ThreadSessionState {
            cwd: test_path_buf("/tmp/refreshed").abs(),
            ..initial_session.clone()
        };
        let mut snapshot = ThreadEventSnapshot {
            session: Some(initial_session),
            turns: Vec::new(),
            events: Vec::new(),
            input_state: None,
        };

        app.apply_refreshed_snapshot_thread(
            thread_id,
            AppServerStartedThread {
                session: resumed_session.clone(),
                turns: resumed_turns.clone(),
            },
            &mut snapshot,
        )
        .await;

        assert_eq!(snapshot.session, Some(resumed_session.clone()));
        assert_eq!(snapshot.turns, resumed_turns);

        let store = app
            .thread_event_channels
            .get(&thread_id)
            .expect("thread channel")
            .store
            .lock()
            .await;
        let store_snapshot = store.snapshot();
        assert_eq!(store_snapshot.session, Some(resumed_session));
        assert_eq!(store_snapshot.turns, snapshot.turns);
    }

    #[tokio::test]
    async fn queued_rollback_syncs_overlay_and_clears_deferred_history() {
        let mut app = make_test_app().await;
        app.transcript_cells = vec![
            Arc::new(UserHistoryCell {
                message: "first".to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn HistoryCell>,
            Arc::new(AgentMessageCell::new(
                vec![Line::from("after first")],
                /*is_first_line*/ false,
            )) as Arc<dyn HistoryCell>,
            Arc::new(UserHistoryCell {
                message: "second".to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn HistoryCell>,
            Arc::new(AgentMessageCell::new(
                vec![Line::from("after second")],
                /*is_first_line*/ false,
            )) as Arc<dyn HistoryCell>,
        ];
        app.overlay = Some(Overlay::new_transcript(app.transcript_cells.clone()));
        app.deferred_history_lines = vec![(
            vec![Line::from("stale buffered line")],
            ScrollbackWrapMode::Adaptive,
        )];
        app.backtrack.overlay_preview_active = true;
        app.backtrack.nth_user_message = 1;

        let changed = app.apply_non_pending_thread_rollback(/*num_turns*/ 1);

        assert!(changed);
        assert!(app.backtrack_render_pending);
        assert!(app.deferred_history_lines.is_empty());
        assert_eq!(app.backtrack.nth_user_message, 0);
        let user_messages: Vec<String> = app
            .transcript_cells
            .iter()
            .filter_map(|cell| {
                cell.as_any()
                    .downcast_ref::<UserHistoryCell>()
                    .map(|cell| cell.message.clone())
            })
            .collect();
        assert_eq!(user_messages, vec!["first".to_string()]);
        let overlay_cell_count = match app.overlay.as_ref() {
            Some(Overlay::Transcript(t)) => t.committed_cell_count(),
            _ => panic!("expected transcript overlay"),
        };
        assert_eq!(overlay_cell_count, app.transcript_cells.len());
    }

    #[tokio::test]
    async fn thread_rollback_response_discards_queued_active_thread_events() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        let (tx, rx) = mpsc::channel(8);
        app.active_thread_id = Some(thread_id);
        app.active_thread_rx = Some(rx);
        tx.send(ThreadBufferedEvent::Notification(
            ServerNotification::ConfigWarning(ConfigWarningNotification {
                summary: "stale warning".to_string(),
                details: None,
                path: None,
                range: None,
            }),
        ))
        .await
        .expect("event should queue");

        app.handle_thread_rollback_response(
            thread_id,
            /*num_turns*/ 1,
            &ThreadRollbackResponse {
                thread: Thread {
                    id: thread_id.to_string(),
                    forked_from_id: None,
                    preview: String::new(),
                    ephemeral: false,
                    model_provider: "openai".to_string(),
                    created_at: 0,
                    updated_at: 0,
                    status: codex_app_server_protocol::ThreadStatus::Idle,
                    path: None,
                    cwd: test_path_buf("/tmp/project").abs(),
                    cli_version: "0.0.0".to_string(),
                    source: SessionSource::Cli.into(),
                    agent_nickname: None,
                    agent_role: None,
                    git_info: None,
                    name: None,
                    turns: Vec::new(),
                },
            },
        )
        .await;

        let rx = app
            .active_thread_rx
            .as_mut()
            .expect("active receiver should remain attached");
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn new_session_requests_shutdown_for_previous_conversation() {
        let (mut app, mut app_event_rx, mut op_rx) = make_test_app_with_channels().await;

        let thread_id = ThreadId::new();
        let event = SessionConfiguredEvent {
            session_id: thread_id,
            forked_from_id: None,
            thread_name: None,
            model: "gpt-test".to_string(),
            model_provider_id: "test-provider".to_string(),
            service_tier: None,
            approval_policy: AskForApproval::Never,
            approvals_reviewer: ApprovalsReviewer::User,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            cwd: test_path_buf("/home/user/project").abs(),
            reasoning_effort: None,
            history_log_id: 0,
            history_entry_count: 0,
            initial_messages: None,
            network_proxy: None,
            rollout_path: Some(PathBuf::new()),
        };

        app.chat_widget.handle_codex_event(Event {
            id: String::new(),
            msg: EventMsg::SessionConfigured(event),
        });

        while app_event_rx.try_recv().is_ok() {}
        while op_rx.try_recv().is_ok() {}

        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        app.shutdown_current_thread(&mut app_server).await;

        assert!(
            op_rx.try_recv().is_err(),
            "shutdown should not submit Op::Shutdown"
        );
    }

    #[tokio::test]
    async fn shutting_down_primary_thread_stops_background_workflow_runs() {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.primary_thread_id = Some(thread_id);
        app.chat_widget.handle_codex_event(Event {
            id: String::new(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: thread_id,
                forked_from_id: started.session.forked_from_id,
                thread_name: started.session.thread_name.clone(),
                model: started.session.model.clone(),
                model_provider_id: started.session.model_provider_id.clone(),
                service_tier: started.session.service_tier,
                approval_policy: started.session.approval_policy,
                approvals_reviewer: started.session.approvals_reviewer,
                sandbox_policy: started.session.sandbox_policy.clone(),
                cwd: started.session.cwd.clone(),
                reasoning_effort: started.session.reasoning_effort,
                history_log_id: started.session.history_log_id,
                history_entry_count: usize::try_from(started.session.history_entry_count)
                    .expect("history entry count fits usize"),
                initial_messages: None,
                network_proxy: started.session.network_proxy.clone(),
                rollout_path: started.session.rollout_path.clone(),
            }),
        });

        let _run_id = app.start_test_background_workflow_run(
            "director".to_string(),
            "review_backlog".to_string(),
            /*is_trigger*/ false,
        );
        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · review_backlog".to_string()]
        );

        app.shutdown_current_thread(&mut app_server).await;

        assert!(app.background_workflow_labels().is_empty());
        assert!(app.queued_trigger_labels().is_empty());
    }

    #[tokio::test]
    async fn before_turn_workflow_augments_primary_user_turn() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_before_turn_workflow(app.config.cwd.as_path())?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.primary_thread_id = Some(thread_id);

        let (op, cells) = app
            .augment_primary_thread_op_with_before_turn_workflows(
                &app_server,
                thread_id,
                AppCommand::from_core(Op::UserTurn {
                    items: vec![UserInput::Text {
                        text: "original prompt".to_string(),
                        text_elements: Vec::new(),
                    }],
                    cwd: app.config.cwd.to_path_buf(),
                    approval_policy: AskForApproval::Never,
                    approvals_reviewer: Some(ApprovalsReviewer::User),
                    sandbox_policy: SandboxPolicy::new_read_only_policy(),
                    model: "gpt-test".to_string(),
                    effort: None,
                    summary: None,
                    service_tier: None,
                    final_output_json_schema: None,
                    collaboration_mode: None,
                    personality: None,
                }),
            )
            .await;

        let AppCommandView::UserTurn { items, .. } = op.view() else {
            panic!("expected user turn");
        };
        assert_eq!(
            items,
            &[
                UserInput::Text {
                    text: "original prompt".to_string(),
                    text_elements: Vec::new(),
                },
                UserInput::Text {
                    text: "added by before_turn".to_string(),
                    text_elements: Vec::new(),
                }
            ]
        );
        let rendered_cells: Vec<String> = cells
            .iter()
            .map(|cell| lines_to_single_string(&cell.transcript_lines(/*width*/ 80)))
            .collect();
        assert_eq!(rendered_cells.len(), 1);
        assert!(rendered_cells[0].contains("Workflow job completed"));
        Ok(())
    }
    #[tokio::test]
    async fn active_primary_turn_complete_waits_for_consumption_before_after_turn() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_after_turn_workflow(app.config.cwd.as_path())?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.enqueue_primary_thread_session(started.session, Vec::new())
            .await?;
        while app_event_rx.try_recv().is_ok() {}

        app.enqueue_thread_notification(
            thread_id,
            turn_completed_notification_with_agent_message(
                thread_id,
                "turn-1",
                TurnStatus::Completed,
                "final reply",
            ),
        )
        .await?;

        assert!(
            app_event_rx.try_recv().is_err(),
            "after_turn should not run before the active thread consumes TurnCompleted"
        );

        let queued_event = app
            .active_thread_rx
            .as_mut()
            .expect("active thread receiver")
            .recv()
            .await
            .expect("queued active-thread event");
        app.handle_thread_event_now(queued_event);

        while let Ok(event) = app_event_rx.try_recv() {
            match event {
                AppEvent::ClawbotTurnCompleted { .. }
                | AppEvent::InsertHistoryCell(_)
                | AppEvent::ReplayWorkflowHistory { .. } => {
                    continue;
                }
                other => {
                    panic!(
                        "after_turn should not run before event consumption finishes, got {other:?}"
                    );
                }
            }
        }

        let visible_cells = app
            .handle_primary_thread_turn_complete_for_workflows(
                &app_server,
                AfterTurnContext {
                    last_agent_message: Some("final reply".to_string()),
                    status: TurnStatus::Completed,
                },
            )
            .await;
        assert_eq!(visible_cells.len(), 1);
        let rendered_cells: Vec<String> = visible_cells
            .iter()
            .map(|cell| lines_to_single_string(&cell.transcript_lines(/*width*/ 80)))
            .collect();
        assert!(rendered_cells[0].contains("Workflow trigger started"));
        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · followup".to_string()]
        );

        let (run_id, result) = loop {
            match tokio::time::timeout(Duration::from_secs(1), app_event_rx.recv())
                .await?
                .expect("expected background workflow event")
            {
                AppEvent::BackgroundWorkflowRunCompleted { run_id, result } => {
                    break (run_id, result);
                }
                other => panic!("expected background workflow completion event, got {other:?}"),
            }
        };
        let completion_cells = app
            .finish_background_workflow_run(&app_server, run_id, *result)
            .await;
        let rendered_completion: Vec<String> = completion_cells
            .iter()
            .map(|cell| lines_to_single_string(&cell.transcript_lines(/*width*/ 80)))
            .collect();
        assert_eq!(rendered_completion.len(), 2);
        assert!(rendered_completion[0].contains("Workflow trigger completed"));
        assert!(rendered_completion[1].contains("Workflow reply"));

        match app_event_rx
            .try_recv()
            .expect("expected workflow follow-up submission")
        {
            AppEvent::SubmitWorkflowFollowup {
                thread_id: submit_thread_id,
                op: Op::UserTurn { items, .. },
            } => {
                assert_eq!(submit_thread_id, thread_id);
                assert_eq!(
                    items,
                    vec![UserInput::Text {
                        text: "follow up from workflow".to_string(),
                        text_elements: Vec::new(),
                    }]
                );
            }
            other => panic!("expected workflow follow-up submission, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_primary_turn_skips_after_turn_by_default() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_after_turn_workflow(app.config.cwd.as_path())?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.enqueue_primary_thread_session(started.session, Vec::new())
            .await?;
        app.active_thread_id = Some(ThreadId::new());
        while app_event_rx.try_recv().is_ok() {}

        app.handle_app_server_event(
            &app_server,
            AppServerEvent::ServerNotification(turn_completed_notification_with_agent_message(
                thread_id,
                "turn-failed",
                TurnStatus::Failed,
                "failed reply",
            )),
        )
        .await;

        assert!(app.background_workflow_labels().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn failed_primary_turn_runs_after_turn_when_condition_allows_any_result() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_after_turn_workflow_with_condition(
            app.config.cwd.as_path(),
            Some("turn_finished"),
        )?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.enqueue_primary_thread_session(started.session, Vec::new())
            .await?;
        app.active_thread_id = Some(ThreadId::new());
        while app_event_rx.try_recv().is_ok() {}

        app.handle_app_server_event(
            &app_server,
            AppServerEvent::ServerNotification(turn_completed_notification_with_agent_message(
                thread_id,
                "turn-failed",
                TurnStatus::Failed,
                "failed reply",
            )),
        )
        .await;

        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · followup".to_string()]
        );
        Ok(())
    }

    #[tokio::test]
    async fn after_turn_skips_when_primary_thread_is_not_bound() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();

        let allowed_thread_id = ThreadId::new();
        write_test_after_turn_workflow_with_condition_and_bind_thread(
            app.config.cwd.as_path(),
            None,
            format!("[\"{allowed_thread_id}\"]").as_str(),
        )?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        app.enqueue_primary_thread_session(started.session, Vec::new())
            .await?;
        app.active_thread_id = Some(ThreadId::new());
        while app_event_rx.try_recv().is_ok() {}

        app.handle_app_server_event(
            &app_server,
            AppServerEvent::ServerNotification(turn_completed_notification_with_agent_message(
                app.primary_thread_id.expect("primary thread"),
                "turn-1",
                TurnStatus::Completed,
                "final reply",
            )),
        )
        .await;

        assert!(app.background_workflow_labels().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn inactive_primary_turn_complete_still_runs_after_turn_continuity() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_after_turn_workflow(app.config.cwd.as_path())?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.enqueue_primary_thread_session(started.session, Vec::new())
            .await?;
        let agent_thread_id = ThreadId::new();
        app.active_thread_id = Some(agent_thread_id);
        while app_event_rx.try_recv().is_ok() {}

        app.handle_app_server_event(
            &app_server,
            AppServerEvent::ServerNotification(turn_completed_notification_with_agent_message(
                thread_id,
                "turn-1",
                TurnStatus::Completed,
                "final reply",
            )),
        )
        .await;

        let (run_id, result) = loop {
            match tokio::time::timeout(Duration::from_secs(1), app_event_rx.recv())
                .await?
                .expect("expected inactive primary background workflow event")
            {
                AppEvent::BackgroundWorkflowRunCompleted { run_id, result } => {
                    break (run_id, result);
                }
                AppEvent::ClawbotTurnCompleted { .. }
                | AppEvent::InsertHistoryCell(_)
                | AppEvent::ReplayWorkflowHistory { .. } => {
                    continue;
                }
                other => panic!("expected background workflow completion event, got {other:?}"),
            }
        };
        let visible_cells = app
            .finish_background_workflow_run(&app_server, run_id, *result)
            .await;
        assert!(
            visible_cells.is_empty(),
            "inactive primary thread should record workflow cells without rendering them immediately"
        );

        match app_event_rx
            .try_recv()
            .expect("expected inactive primary workflow follow-up submission")
        {
            AppEvent::SubmitWorkflowFollowup {
                thread_id: submit_thread_id,
                op: Op::UserTurn { items, .. },
            } => {
                assert_eq!(submit_thread_id, thread_id);
                assert_eq!(
                    items,
                    vec![UserInput::Text {
                        text: "follow up from workflow".to_string(),
                        text_elements: Vec::new(),
                    }]
                );
            }
            other => panic!("expected workflow follow-up submission, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn workflow_ui_popup_snapshot() -> Result<()> {
        let mut app = make_test_app().await;
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path())?;

        let slow_run = app
            .start_test_manual_workflow_trigger_run(
                "director".to_string(),
                "review_backlog".to_string(),
            )
            .expect("running manual trigger");
        let queued_run = app
            .start_test_manual_workflow_trigger_run("director".to_string(), "triage".to_string());
        assert!(queued_run.is_none());

        app.open_workflow_controls_popup();

        let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
        assert_snapshot!("workflow_controls_popup", popup);

        app.finish_test_background_workflow_run(slow_run).await;
        Ok(())
    }

    #[tokio::test]
    async fn workflow_ui_manual_trigger_action_updates_scheduler_status() -> Result<()> {
        let mut app = make_test_app().await;
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_manual_workflow(app.config.cwd.as_path())?;

        let started_app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref()).await?;
        let cell = app.start_manual_workflow_trigger_from_ui(
            &started_app_server,
            "director".to_string(),
            "review_backlog".to_string(),
        );
        let rendered = cell
            .display_lines(/*width*/ 100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Workflow trigger started"));
        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · review_backlog".to_string()]
        );

        let stopped = app.workflow_scheduler.stop_active_workflow_runs().await;
        assert_eq!(stopped, 1);
        app.sync_background_workflow_status();
        Ok(())
    }

    #[tokio::test]
    async fn workflow_ui_manual_trigger_action_rejects_unbound_primary_thread() -> Result<()> {
        let mut app = make_test_app().await;
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();

        let allowed_thread_id = ThreadId::new();
        write_test_manual_workflow_with_bind_thread(
            app.config.cwd.as_path(),
            format!("[\"{allowed_thread_id}\"]").as_str(),
        )?;

        let mut started_app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref()).await?;
        let started = started_app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        app.primary_thread_id = Some(started.session.thread_id);
        app.primary_session_configured = Some(started.session);
        app.active_thread_id = app.primary_thread_id;

        let cell = app.start_manual_workflow_trigger_from_ui(
            &started_app_server,
            "restricted".to_string(),
            "review_backlog".to_string(),
        );
        let rendered = cell
            .display_lines(/*width*/ 100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Workflow trigger failed"));
        assert!(rendered.contains("not allowed by `bind_thread`"));
        assert!(app.background_workflow_labels().is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn clean_background_terminals_stops_background_workflows() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref()).await?;
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        let thread_id = started.session.thread_id;
        app.primary_thread_id = Some(thread_id);
        app.primary_session_configured = Some(started.session);
        app.active_thread_id = Some(thread_id);

        let run_id = app
            .workflow_scheduler
            .next_background_run_id("director", "review_backlog");
        let cancellation = CancellationToken::new();
        let cancellation_for_task = cancellation.clone();
        app.workflow_scheduler.register_background_workflow_run(
            run_id,
            workflow_runtime::BackgroundWorkflowRunTarget::Job {
                workflow_name: "director".to_string(),
                job_name: "review_backlog".to_string(),
            },
            cancellation,
            tokio::spawn(async move {
                cancellation_for_task.cancelled().await;
            }),
        );
        app.workflow_scheduler.enqueue_trigger_run(
            "director".to_string(),
            "triage".to_string(),
            workflow_runtime::OwnedWorkflowPhaseContext::default(),
        );
        app.sync_background_workflow_status();

        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · review_backlog".to_string()]
        );
        assert_eq!(
            app.queued_trigger_labels(),
            vec!["director · triage".to_string()]
        );

        let handled = app
            .try_submit_active_thread_op_via_app_server(
                &mut app_server,
                thread_id,
                &AppCommand::clean_background_terminals(),
            )
            .await?;

        assert!(handled);
        assert!(app.background_workflow_labels().is_empty());
        assert!(app.queued_trigger_labels().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn manual_triggers_use_a_global_fifo_queue() {
        let mut app = make_test_app().await;

        let slow_run =
            app.start_test_manual_workflow_trigger_run("director".to_string(), "slow".to_string());
        let fast_run =
            app.start_test_manual_workflow_trigger_run("director".to_string(), "fast".to_string());

        assert!(slow_run.is_some());
        assert!(fast_run.is_none());
        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · slow".to_string()]
        );
        assert_eq!(
            app.queued_trigger_labels(),
            vec!["director · fast".to_string()]
        );

        app.finish_test_background_workflow_run(slow_run.expect("slow run id"))
            .await;

        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · fast".to_string()]
        );
        assert!(app.queued_trigger_labels().is_empty());
    }

    #[tokio::test]
    async fn file_watch_triggers_share_the_global_queue_and_skip_duplicates() -> Result<()> {
        let mut app = make_test_app().await;
        write_test_file_watch_workflow(app.config.cwd.as_path())?;

        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref()).await?;
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        let thread_id = started.session.thread_id;
        app.primary_thread_id = Some(thread_id);
        app.primary_session_configured = Some(started.session);
        app.active_thread_id = Some(thread_id);

        let running =
            app.start_test_manual_workflow_trigger_run("director".to_string(), "slow".to_string());
        assert!(running.is_some());

        let first = app.handle_workspace_file_changes_for_workflows(
            &app_server,
            &[app.config.cwd.as_path().join("src")],
        );
        let second = app.handle_workspace_file_changes_for_workflows(
            &app_server,
            &[app.config.cwd.as_path().join("src/lib.rs")],
        );

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · slow".to_string()]
        );
        assert_eq!(
            app.queued_trigger_labels(),
            vec!["watcher · refresh".to_string()]
        );
        Ok(())
    }

    #[tokio::test]
    async fn workflow_follow_up_completion_submits_to_primary_thread() {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.primary_thread_id = Some(thread_id);
        app.primary_session_configured = Some(started.session);
        app.active_thread_id = Some(thread_id);

        let run_id = app
            .workflow_scheduler
            .next_background_run_id("director", "review_backlog");
        let target = workflow_runtime::BackgroundWorkflowRunTarget::Job {
            workflow_name: "director".to_string(),
            job_name: "review_backlog".to_string(),
        };
        app.workflow_scheduler.register_background_workflow_run(
            run_id.clone(),
            target.clone(),
            CancellationToken::new(),
            tokio::spawn(async {}),
        );

        let cells = app
            .finish_background_workflow_run(
                &app_server,
                run_id,
                workflow_runtime::BackgroundWorkflowRunResult {
                    target,
                    outcome: workflow_runtime::BackgroundWorkflowRunOutcome::Completed(vec![
                        workflow_runtime::WorkflowJobRunResult {
                            delivery: workflow_runtime::WorkflowOutputDelivery::UserFollowup,
                            execution_strategy:
                                workflow_definition::WorkflowExecutionStrategy::InheritSession,
                            workflow_name: "director".to_string(),
                            trigger_id: "job:review_backlog".to_string(),
                            job_name: "review_backlog".to_string(),
                            message: Some("workflow follow-up".to_string()),
                        },
                    ]),
                },
            )
            .await;

        let rendered_cells: Vec<String> = cells
            .iter()
            .map(|cell| lines_to_single_string(&cell.transcript_lines(/*width*/ 80)))
            .collect();
        assert_eq!(rendered_cells.len(), 2);
        assert!(rendered_cells[0].contains("Workflow job completed"));
        assert!(rendered_cells[1].contains("Workflow reply"));

        match app_event_rx
            .try_recv()
            .expect("expected queued workflow follow-up")
        {
            AppEvent::SubmitWorkflowFollowup {
                thread_id: submit_thread_id,
                op: Op::UserTurn { items, .. },
            } => {
                assert_eq!(submit_thread_id, thread_id);
                assert_eq!(
                    items,
                    vec![UserInput::Text {
                        text: "workflow follow-up".to_string(),
                        text_elements: Vec::new(),
                    }]
                );
            }
            other => panic!("expected workflow follow-up submission, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn workflow_followup_completion_retriggers_after_turn() -> Result<()> {
        let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let tempdir = tempdir()?;
        app.config.cwd = tempdir.path().to_path_buf().abs();
        write_test_after_turn_workflow(app.config.cwd.as_path())?;

        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.enqueue_primary_thread_session(started.session, Vec::new())
            .await?;
        app.active_thread_id = Some(ThreadId::new());
        while app_event_rx.try_recv().is_ok() {}

        app.handle_app_server_event(
            &app_server,
            AppServerEvent::ServerNotification(turn_completed_notification_with_agent_message(
                thread_id,
                "turn-followup",
                TurnStatus::Completed,
                "workflow-originated follow-up reply",
            )),
        )
        .await;

        assert_eq!(
            app.background_workflow_labels(),
            vec!["director · followup".to_string()]
        );
        while let Ok(event) = app_event_rx.try_recv() {
            match event {
                AppEvent::ClawbotTurnCompleted { .. }
                | AppEvent::InsertHistoryCell(_)
                | AppEvent::ReplayWorkflowHistory { .. } => {}
                other => panic!("unexpected event after workflow follow-up completion: {other:?}"),
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn app_server_notifications_forward_to_workflow_thread_receivers() -> Result<()> {
        let mut app = make_test_app().await;
        let app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
            .await
            .expect("embedded app server");
        let thread_id = ThreadId::new();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        app.workflow_thread_notification_channels
            .lock()
            .await
            .insert(thread_id, sender);

        let notification = ServerNotification::ItemCompleted(ItemCompletedNotification {
            item: ThreadItem::AgentMessage {
                id: "msg-1".to_string(),
                text: "workflow reply".to_string(),
                phase: None,
                memory_citation: None,
            },
            thread_id: thread_id.to_string(),
            turn_id: "turn-1".to_string(),
        });

        app.handle_app_server_event(
            &app_server,
            AppServerEvent::ServerNotification(notification.clone()),
        )
        .await;

        match receiver.recv().await {
            Some(ServerNotification::ItemCompleted(received)) => {
                assert_eq!(received.thread_id, thread_id.to_string());
                assert_eq!(received.turn_id, "turn-1");
                let ThreadItem::AgentMessage { text, .. } = received.item else {
                    panic!("expected forwarded workflow agent message");
                };
                assert_eq!(text, "workflow reply");
            }
            other => panic!("expected forwarded workflow notification, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_first_exit_returns_immediate_exit_when_shutdown_submit_fails() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        app.active_thread_id = Some(thread_id);

        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let control = app
            .handle_exit_mode(&mut app_server, ExitMode::ShutdownFirst)
            .await;

        assert_eq!(app.pending_shutdown_exit_thread_id, None);
        assert!(matches!(
            control,
            AppRunControl::Exit(ExitReason::UserRequested)
        ));
    }

    #[tokio::test]
    async fn shutdown_first_exit_uses_app_server_shutdown_without_submitting_op() {
        let (mut app, _app_event_rx, mut op_rx) = make_test_app_with_channels().await;
        let thread_id = ThreadId::new();
        app.active_thread_id = Some(thread_id);

        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let control = app
            .handle_exit_mode(&mut app_server, ExitMode::ShutdownFirst)
            .await;

        assert_eq!(app.pending_shutdown_exit_thread_id, None);
        assert!(matches!(
            control,
            AppRunControl::Exit(ExitReason::UserRequested)
        ));
        assert!(
            op_rx.try_recv().is_err(),
            "shutdown should not submit Op::Shutdown"
        );
    }

    #[tokio::test]
    async fn respawn_immediate_exit_returns_respawn_requested() {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");

        let control = app
            .handle_exit_mode(&mut app_server, ExitMode::RespawnImmediate)
            .await;

        assert_eq!(app.pending_shutdown_exit_thread_id, None);
        assert!(matches!(
            control,
            AppRunControl::Exit(ExitReason::RespawnRequested)
        ));
    }

    #[tokio::test]
    async fn active_btw_thread_id_for_respawn_only_reattaches_visible_btw_thread() {
        let mut app = make_test_app().await;
        let main_thread_id = ThreadId::new();
        let btw_thread_id = ThreadId::new();

        app.primary_thread_id = Some(main_thread_id);
        app.active_thread_id = Some(btw_thread_id);
        app.btw_session = Some(BtwSessionState {
            thread_id: btw_thread_id,
        });
        assert_eq!(app.active_btw_thread_id_for_respawn(), Some(btw_thread_id));

        app.chat_widget.handle_thread_session(test_thread_session(
            main_thread_id,
            test_path_buf("/tmp/main"),
        ));
        app.active_thread_id = Some(main_thread_id);
        assert_eq!(app.active_btw_thread_id_for_respawn(), None);

        app.active_thread_id = Some(btw_thread_id);
        assert_eq!(app.active_btw_thread_id_for_respawn(), None);
    }

    #[tokio::test]
    async fn active_btw_thread_id_for_respawn_skips_hidden_btw_thread_without_visible_context() {
        let mut app = make_test_app().await;
        let btw_thread_id = ThreadId::new();

        app.btw_session = Some(BtwSessionState {
            thread_id: btw_thread_id,
        });

        assert_eq!(app.active_btw_thread_id_for_respawn(), None);
    }

    #[tokio::test]
    async fn active_btw_thread_id_for_respawn_uses_visible_btw_role_after_restore() {
        let mut app = make_test_app().await;
        let main_thread_id = ThreadId::new();
        let btw_thread_id = ThreadId::new();

        app.primary_thread_id = Some(main_thread_id);
        app.upsert_agent_picker_thread(
            main_thread_id,
            Some("main".to_string()),
            Some("default".to_string()),
            /*is_closed*/ false,
        );
        app.upsert_agent_picker_thread(
            btw_thread_id,
            Some("btw scratch".to_string()),
            Some("btw".to_string()),
            /*is_closed*/ false,
        );
        app.active_thread_id = Some(btw_thread_id);
        app.chat_widget.handle_thread_session(test_thread_session(
            btw_thread_id,
            test_path_buf("/tmp/btw"),
        ));

        assert_eq!(app.btw_session, None);
        assert_eq!(app.active_btw_thread_id_for_respawn(), Some(btw_thread_id));
    }

    #[tokio::test]
    async fn current_displayed_thread_respawn_target_ignores_stale_btw_name() {
        let mut app = make_test_app().await;
        let main_thread_id = ThreadId::new();
        let btw_thread_id = ThreadId::new();

        let main_session = test_thread_session(main_thread_id, test_path_buf("/tmp/main"));
        let stale_name_main_session = ThreadSessionState {
            thread_name: Some("btw scratch".to_string()),
            ..main_session.clone()
        };
        let btw_session = ThreadSessionState {
            thread_name: Some("btw scratch".to_string()),
            ..test_thread_session(btw_thread_id, test_path_buf("/tmp/btw"))
        };

        app.thread_event_channels.insert(
            main_thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                main_session,
                Vec::new(),
            ),
        );
        app.thread_event_channels.insert(
            btw_thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                btw_session.clone(),
                Vec::new(),
            ),
        );
        app.active_thread_id = Some(main_thread_id);
        app.chat_widget
            .handle_thread_session(stale_name_main_session);

        assert_eq!(
            app.chat_widget.thread_name().as_deref(),
            Some("btw scratch")
        );
        assert_eq!(
            app.current_displayed_thread_respawn_target().await,
            Some(main_thread_id.to_string())
        );
    }

    #[tokio::test]
    async fn current_displayed_thread_respawn_target_uses_visible_btw_thread_name() {
        let mut app = make_test_app().await;
        let btw_thread_id = ThreadId::new();
        let btw_session = ThreadSessionState {
            thread_name: Some("btw scratch".to_string()),
            ..test_thread_session(btw_thread_id, test_path_buf("/tmp/btw"))
        };

        app.thread_event_channels.insert(
            btw_thread_id,
            ThreadEventChannel::new_with_session(
                THREAD_EVENT_CHANNEL_CAPACITY,
                btw_session.clone(),
                Vec::new(),
            ),
        );
        app.active_thread_id = Some(btw_thread_id);
        app.chat_widget.handle_thread_session(btw_session);

        assert_eq!(
            app.current_displayed_thread_respawn_target().await,
            Some(btw_thread_id.to_string())
        );
    }

    #[tokio::test]
    async fn current_displayed_thread_respawn_target_prefers_thread_id_over_stale_visible_name() {
        let mut app = make_test_app().await;
        let main_thread_id = ThreadId::new();
        let stale_name_session = ThreadSessionState {
            thread_name: Some("btw scratch".to_string()),
            ..test_thread_session(main_thread_id, test_path_buf("/tmp/main"))
        };

        app.chat_widget.handle_thread_session(stale_name_session);

        assert_eq!(
            app.current_displayed_thread_respawn_target().await,
            Some(main_thread_id.to_string())
        );
    }

    #[tokio::test]
    async fn current_displayed_thread_respawn_target_prefers_visible_thread_over_stale_active_id() {
        let mut app = make_test_app().await;
        let main_thread_id = ThreadId::new();
        let btw_thread_id = ThreadId::new();

        app.active_thread_id = Some(btw_thread_id);
        app.chat_widget.handle_thread_session(test_thread_session(
            main_thread_id,
            test_path_buf("/tmp/main"),
        ));

        assert_eq!(
            app.current_displayed_thread_respawn_target().await,
            Some(main_thread_id.to_string())
        );
    }

    #[tokio::test]
    async fn interrupt_without_active_turn_is_treated_as_handled() {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("thread/start should succeed");
        let thread_id = started.session.thread_id;
        app.enqueue_primary_thread_session(started.session, started.turns)
            .await
            .expect("primary thread should be registered");
        let op = AppCommand::interrupt();

        let handled = app
            .try_submit_active_thread_op_via_app_server(&mut app_server, thread_id, &op)
            .await
            .expect("interrupt submission should not fail");

        assert_eq!(handled, true);
    }

    #[tokio::test]
    async fn closing_active_thread_for_profile_reload_allows_fresh_start() -> Result<()> {
        let mut app = make_test_app().await;
        let mut app_server =
            crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
                .await
                .expect("embedded app server");
        let started = app_server
            .start_thread(app.chat_widget.config_ref())
            .await
            .expect("start thread");
        let thread_id = started.session.thread_id;
        app.primary_thread_id = Some(thread_id);
        app.active_thread_id = Some(thread_id);

        app.close_active_thread_for_profile_reload(&mut app_server, thread_id)
            .await
            .map_err(|err| color_eyre::eyre::eyre!(err))?;

        assert_eq!(app.active_thread_id, None);

        let restarted = app_server
            .start_thread(app.chat_widget.config_ref())
            .await?;
        assert_ne!(restarted.session.thread_id, thread_id);
        Ok(())
    }

    #[tokio::test]
    async fn clear_only_ui_reset_preserves_chat_session_state() {
        let mut app = make_test_app().await;
        let thread_id = ThreadId::new();
        app.chat_widget.handle_codex_event(Event {
            id: String::new(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: thread_id,
                forked_from_id: None,
                thread_name: Some("keep me".to_string()),
                model: "gpt-test".to_string(),
                model_provider_id: "test-provider".to_string(),
                service_tier: None,
                approval_policy: AskForApproval::Never,
                approvals_reviewer: ApprovalsReviewer::User,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                cwd: test_path_buf("/tmp/project").abs(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                network_proxy: None,
                rollout_path: Some(PathBuf::new()),
            }),
        });
        app.chat_widget
            .apply_external_edit("draft prompt".to_string());
        app.transcript_cells = vec![Arc::new(UserHistoryCell {
            message: "old message".to_string(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: Vec::new(),
        }) as Arc<dyn HistoryCell>];
        app.overlay = Some(Overlay::new_transcript(app.transcript_cells.clone()));
        app.deferred_history_lines = vec![(
            vec![Line::from("stale buffered line")],
            ScrollbackWrapMode::Adaptive,
        )];
        app.has_emitted_history_lines = true;
        app.backtrack.primed = true;
        app.backtrack.overlay_preview_active = true;
        app.backtrack.nth_user_message = 0;
        app.backtrack_render_pending = true;

        app.reset_app_ui_state_after_clear();

        assert!(app.overlay.is_none());
        assert!(app.transcript_cells.is_empty());
        assert!(app.deferred_history_lines.is_empty());
        assert!(!app.has_emitted_history_lines);
        assert!(!app.backtrack.primed);
        assert!(!app.backtrack.overlay_preview_active);
        assert!(app.backtrack.pending_rollback.is_none());
        assert!(!app.backtrack_render_pending);
        assert_eq!(app.chat_widget.thread_id(), Some(thread_id));
        assert_eq!(app.chat_widget.composer_text_with_pending(), "draft prompt");
    }

    #[tokio::test]
    async fn session_summary_skips_when_no_usage_or_resume_hint() {
        assert!(
            session_summary(
                TokenUsage::default(),
                /*thread_id*/ None,
                /*thread_name*/ None,
                /*rollout_path*/ None,
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn session_summary_skips_resume_hint_until_rollout_exists() {
        let usage = TokenUsage::default();
        let conversation = ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let temp_dir = tempdir().expect("temp dir");
        let rollout_path = temp_dir.path().join("rollout.jsonl");

        assert!(
            session_summary(
                usage,
                Some(conversation),
                /*thread_name*/ None,
                Some(&rollout_path),
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn session_summary_includes_resume_hint_for_persisted_rollout() {
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            total_tokens: 12,
            ..Default::default()
        };
        let conversation = ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let temp_dir = tempdir().expect("temp dir");
        let rollout_path = temp_dir.path().join("rollout.jsonl");
        std::fs::write(&rollout_path, "{}\n").expect("write rollout");

        let summary = session_summary(
            usage,
            Some(conversation),
            /*thread_name*/ None,
            Some(&rollout_path),
        )
        .expect("summary");
        assert_eq!(
            summary.usage_line,
            Some("Token usage: total=12 input=10 output=2".to_string())
        );
        assert_eq!(
            summary.resume_command,
            Some("codex resume 123e4567-e89b-12d3-a456-426614174000".to_string())
        );
    }

    #[tokio::test]
    async fn session_summary_prefers_name_over_id() {
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            total_tokens: 12,
            ..Default::default()
        };
        let conversation = ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let temp_dir = tempdir().expect("temp dir");
        let rollout_path = temp_dir.path().join("rollout.jsonl");
        std::fs::write(&rollout_path, "{}\n").expect("write rollout");

        let summary = session_summary(
            usage,
            Some(conversation),
            Some("my-session".to_string()),
            Some(&rollout_path),
        )
        .expect("summary");
        assert_eq!(
            summary.resume_command,
            Some("codex resume my-session".to_string())
        );
    }
}
