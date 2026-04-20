use super::App;
use super::clawbot_controller::ClawbotController;
use super::profile_controller::ProfileController;
use super::thread_controller::ThreadController;
use super::workflow_controller::WorkflowController;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::tui;

#[derive(Clone, Copy)]
enum FeatureRoute {
    Profile,
    Thread,
    Btw,
    Workflow,
    Clawbot,
}

impl FeatureRoute {
    fn for_event(event: &AppEvent) -> Option<Self> {
        match event {
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
            AppEvent::StartBtwDiscussion { .. } => Some(Self::Btw),
            AppEvent::OpenWorkflowControls
            | AppEvent::OpenWorkflowControlView { .. }
            | AppEvent::CreateDefaultWorkflowTemplate
            | AppEvent::EditWorkflowFile { .. }
            | AppEvent::ToggleWorkflowTriggerEnabled { .. }
            | AppEvent::ToggleWorkflowJobEnabled { .. }
            | AppEvent::CycleWorkflowJobContext { .. }
            | AppEvent::CycleWorkflowJobResponse { .. }
            | AppEvent::EditWorkflowJobField { .. }
            | AppEvent::SetWorkflowTriggerType { .. }
            | AppEvent::EditWorkflowTriggerField { .. }
            | AppEvent::WorkflowWorkspaceFilesChanged { .. }
            | AppEvent::StartManualWorkflowTrigger { .. }
            | AppEvent::StartManualWorkflowJob { .. }
            | AppEvent::ShowWorkflowBackgroundTasks
            | AppEvent::ReplayWorkflowHistory { .. }
            | AppEvent::BackgroundWorkflowRunCompleted { .. } => Some(Self::Workflow),
            AppEvent::ClawbotProviderEvent { .. }
            | AppEvent::ClawbotTurnCompleted { .. }
            | AppEvent::OpenClawbotManagement
            | AppEvent::OpenClawbotManagementView { .. }
            | AppEvent::OpenClawbotFeishuConfigPrompt { .. }
            | AppEvent::SaveClawbotFeishuConfigValue { .. }
            | AppEvent::BindClawbotDiscoveredSession { .. }
            | AppEvent::BindClawbotSessionAndPreempt { .. }
            | AppEvent::ClawbotSetTurnMode { .. }
            | AppEvent::ClawbotSetThreadForwarding { .. }
            | AppEvent::ScanClawbotFeishuSessions
            | AppEvent::ClearClawbotFeishuSessions
            | AppEvent::RetryClawbotFeishuConnection
            | AppEvent::ToggleClawbotForceConnect
            | AppEvent::ClawbotDisconnectThread { .. }
            | AppEvent::EditClawbotStateFile { .. } => Some(Self::Clawbot),
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
    ) -> Option<AppEvent> {
        let Some(route) = FeatureRoute::for_event(&event) else {
            return Some(event);
        };

        match route {
            FeatureRoute::Profile => {
                ProfileController::handle(self, tui, app_server, event).await;
            }
            FeatureRoute::Thread => {
                ThreadController::handle(self, tui, app_server, event).await;
            }
            FeatureRoute::Btw => {
                self.handle_btw_feature_event(app_server, event).await;
            }
            FeatureRoute::Workflow => {
                WorkflowController::handle(self, tui, app_server, event).await;
            }
            FeatureRoute::Clawbot => {
                ClawbotController::handle(self, tui, app_server, event).await;
            }
        }

        None
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
            _ => unreachable!("non-btw event passed to btw feature dispatcher"),
        }
    }
}
