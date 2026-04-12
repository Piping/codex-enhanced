use std::future::Future;
use std::pin::Pin;

use color_eyre::eyre::Result;

use super::App;
use super::AppRunControl;
use super::clawbot_controller;
use super::dream_controller;
use super::integration_controller::IntegrationController;
use super::profile_controller::ProfileController;
use super::session_controller::SessionController;
use super::settings_controller::SettingsController;
use super::thread_controller::ThreadController;
use super::workflow_controller;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::tui;

pub(super) enum FeatureDispatchOutcome {
    Unhandled(Box<AppEvent>),
    Handled,
    Return(AppRunControl),
}

pub(super) type FeatureDispatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FeatureDispatchOutcome>> + 'a>>;

type FeatureDispatchFn = for<'a> fn(
    &'a mut App,
    &'a mut tui::Tui,
    &'a mut AppServerSession,
    AppEvent,
) -> FeatureDispatchFuture<'a>;

struct RecentFeatureDispatcher {
    matches: fn(&AppEvent) -> bool,
    dispatch: FeatureDispatchFn,
}

const RECENT_FEATURE_DISPATCHERS: &[RecentFeatureDispatcher] = &[
    RecentFeatureDispatcher {
        matches: dream_controller::matches_event,
        dispatch: dream_controller::dispatch,
    },
    RecentFeatureDispatcher {
        matches: workflow_controller::matches_event,
        dispatch: workflow_controller::dispatch,
    },
    RecentFeatureDispatcher {
        matches: clawbot_controller::matches_event,
        dispatch: clawbot_controller::dispatch,
    },
];

#[derive(Clone, Copy)]
enum LegacyFeatureRoute {
    Session,
    Profile,
    Thread,
    Btw,
    Integration,
    Settings,
}

impl LegacyFeatureRoute {
    fn for_event(event: &AppEvent) -> Option<Self> {
        match event {
            AppEvent::NewSession | AppEvent::ClearUi | AppEvent::OpenResumePicker => {
                Some(Self::Session)
            }
            AppEvent::OpenProfileManagementPanel
            | AppEvent::EditProfileFallbackConfig
            | AppEvent::SwitchRuntimeProfile { .. }
            | AppEvent::RetryLastUserTurnWithProfileFallback { .. }
            | AppEvent::ExecuteProfileFallbackRetry { .. } => Some(Self::Profile),
            AppEvent::OpenThreadPanel
            | AppEvent::OpenJumpToMessagePanel
            | AppEvent::JumpToTranscriptCell { .. }
            | AppEvent::ForkCurrentSession
            | AppEvent::UndoLastUserMessage => Some(Self::Thread),
            AppEvent::StartBtwDiscussion { .. }
            | AppEvent::BtwCompleted { .. }
            | AppEvent::BtwInsertSummary
            | AppEvent::BtwInsertFull
            | AppEvent::BtwDiscard => Some(Self::Btw),
            AppEvent::OpenAppLink { .. }
            | AppEvent::OpenUrlInBrowser { .. }
            | AppEvent::RefreshConnectors { .. }
            | AppEvent::PluginInstallAuthAdvance { .. }
            | AppEvent::PluginInstallAuthAbandon
            | AppEvent::FetchPluginsList { .. }
            | AppEvent::OpenPluginDetailLoading { .. }
            | AppEvent::OpenPluginInstallLoading { .. }
            | AppEvent::OpenPluginUninstallLoading { .. }
            | AppEvent::PluginsLoaded { .. }
            | AppEvent::FetchPluginDetail { .. }
            | AppEvent::PluginDetailLoaded { .. }
            | AppEvent::FetchPluginInstall { .. }
            | AppEvent::FetchPluginUninstall { .. }
            | AppEvent::PluginInstallLoaded { .. }
            | AppEvent::PluginUninstallLoaded { .. }
            | AppEvent::FetchMcpInventory
            | AppEvent::McpInventoryLoaded { .. }
            | AppEvent::StartFileSearch(_)
            | AppEvent::FileSearchResult { .. }
            | AppEvent::RefreshRateLimits { .. }
            | AppEvent::RateLimitsLoaded { .. }
            | AppEvent::ConnectorsLoaded { .. }
            | AppEvent::OpenSkillsList
            | AppEvent::OpenManageSkillsPopup
            | AppEvent::SetSkillEnabled { .. }
            | AppEvent::SetAppEnabled { .. }
            | AppEvent::ManageSkillsClosed => Some(Self::Integration),
            AppEvent::OpenDisplayPreferencesPanel
            | AppEvent::UpdateReasoningEffort(_)
            | AppEvent::UpdateModel(_)
            | AppEvent::UpdateCollaborationMode(_)
            | AppEvent::UpdatePersonality(_)
            | AppEvent::OpenRealtimeAudioDeviceSelection { .. }
            | AppEvent::OpenReasoningPopup { .. }
            | AppEvent::OpenPlanReasoningScopePrompt { .. }
            | AppEvent::OpenAllModelsPopup { .. }
            | AppEvent::OpenFullAccessConfirmation { .. }
            | AppEvent::OpenWorldWritableWarningConfirmation { .. }
            | AppEvent::OpenFeedbackNote { .. }
            | AppEvent::OpenFeedbackConsent { .. }
            | AppEvent::SubmitFeedback { .. }
            | AppEvent::FeedbackSubmitted { .. }
            | AppEvent::LaunchExternalEditor
            | AppEvent::OpenWindowsSandboxEnablePrompt { .. }
            | AppEvent::OpenWindowsSandboxFallbackPrompt { .. }
            | AppEvent::BeginWindowsSandboxElevatedSetup { .. }
            | AppEvent::BeginWindowsSandboxLegacySetup { .. }
            | AppEvent::BeginWindowsSandboxGrantReadRoot { .. }
            | AppEvent::WindowsSandboxGrantReadRootCompleted { .. }
            | AppEvent::EnableWindowsSandboxForAgentMode { .. }
            | AppEvent::PersistModelSelection { .. }
            | AppEvent::PersistPersonalitySelection { .. }
            | AppEvent::PersistServiceTierSelection { .. }
            | AppEvent::PersistRealtimeAudioDeviceSelection { .. }
            | AppEvent::RestartRealtimeAudioDevice { .. }
            | AppEvent::UpdateAskForApprovalPolicy(_)
            | AppEvent::UpdateSandboxPolicy(_)
            | AppEvent::UpdateApprovalsReviewer(_)
            | AppEvent::UpdateFeatureFlags { .. }
            | AppEvent::ToggleDisplayPreference(_)
            | AppEvent::SkipNextWorldWritableScan
            | AppEvent::UpdateFullAccessWarningAcknowledged(_)
            | AppEvent::UpdateWorldWritableWarningAcknowledged(_)
            | AppEvent::UpdateRateLimitSwitchPromptHidden(_)
            | AppEvent::UpdatePlanModeReasoningEffort(_)
            | AppEvent::PersistFullAccessWarningAcknowledged
            | AppEvent::PersistWorldWritableWarningAcknowledged
            | AppEvent::PersistRateLimitSwitchPromptHidden
            | AppEvent::PersistPlanModeReasoningEffort(_)
            | AppEvent::PersistModelMigrationPromptAcknowledged { .. }
            | AppEvent::OpenApprovalsPopup
            | AppEvent::OpenPermissionsPopup
            | AppEvent::StatusLineSetup { .. }
            | AppEvent::StatusLineBranchUpdated { .. }
            | AppEvent::StatusLineSetupCancelled
            | AppEvent::TerminalTitleSetup { .. }
            | AppEvent::TerminalTitleSetupPreview { .. }
            | AppEvent::TerminalTitleSetupCancelled
            | AppEvent::SyntaxThemeSelected { .. } => Some(Self::Settings),
            _ => None,
        }
    }
}

impl App {
    pub(super) async fn dispatch_feature_event(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) -> Result<FeatureDispatchOutcome> {
        let event = match self
            .dispatch_recent_feature_event(tui, app_server, event)
            .await?
        {
            FeatureDispatchOutcome::Unhandled(event) => *event,
            handled => return Ok(handled),
        };

        let Some(route) = LegacyFeatureRoute::for_event(&event) else {
            return Ok(FeatureDispatchOutcome::Unhandled(Box::new(event)));
        };

        match route {
            LegacyFeatureRoute::Session => {
                if let Some(control) =
                    SessionController::handle(self, tui, app_server, event).await?
                {
                    return Ok(FeatureDispatchOutcome::Return(control));
                }
            }
            LegacyFeatureRoute::Profile => {
                ProfileController::handle(self, tui, app_server, event).await;
            }
            LegacyFeatureRoute::Thread => {
                ThreadController::handle(self, tui, app_server, event).await;
            }
            LegacyFeatureRoute::Btw => {
                self.handle_btw_feature_event(app_server, event).await;
            }
            LegacyFeatureRoute::Integration => {
                IntegrationController::handle(self, app_server, event).await;
            }
            LegacyFeatureRoute::Settings => {
                SettingsController::handle(self, tui, app_server, event).await;
            }
        }

        Ok(FeatureDispatchOutcome::Handled)
    }

    async fn dispatch_recent_feature_event(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) -> Result<FeatureDispatchOutcome> {
        for dispatcher in RECENT_FEATURE_DISPATCHERS {
            if (dispatcher.matches)(&event) {
                return (dispatcher.dispatch)(self, tui, app_server, event).await;
            }
        }

        Ok(FeatureDispatchOutcome::Unhandled(Box::new(event)))
    }

    pub(super) async fn handle_btw_feature_event(
        &mut self,
        app_server: &mut AppServerSession,
        event: AppEvent,
    ) {
        match event {
            AppEvent::StartBtwDiscussion { prompt } => {
                self.start_btw_discussion(app_server, prompt).await;
            }
            AppEvent::BtwCompleted { thread_id, result } => {
                let is_error = result.is_err();
                self.finish_btw_discussion(thread_id, result);
                if is_error {
                    self.close_btw_session(app_server).await;
                }
            }
            AppEvent::BtwInsertSummary => {
                self.insert_btw_summary(app_server).await;
            }
            AppEvent::BtwInsertFull => {
                self.insert_btw_full(app_server).await;
            }
            AppEvent::BtwDiscard => {
                self.discard_btw_session(app_server).await;
            }
            _ => unreachable!("non-btw event passed to btw feature dispatcher"),
        }
    }
}
