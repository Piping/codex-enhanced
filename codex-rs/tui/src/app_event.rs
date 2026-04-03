//! Application-level events used to coordinate UI actions.
//!
//! `AppEvent` is the internal message bus between UI components and the top-level `App` loop.
//! Widgets emit events to request actions that must be handled at the app layer (like opening
//! pickers, persisting configuration, or shutting down the agent), without needing direct access to
//! `App` internals.
//!
//! Exit is modelled explicitly via `AppEvent::Exit(ExitMode)` so callers can request shutdown-first
//! quits without reaching into the app loop or coupling to shutdown/exit sequencing.

use std::path::PathBuf;

use codex_app_server_protocol::PluginInstallResponse;
use codex_app_server_protocol::PluginListResponse;
use codex_app_server_protocol::PluginReadParams;
use codex_app_server_protocol::PluginReadResponse;
use codex_app_server_protocol::PluginUninstallResponse;
use codex_chatgpt::connectors::AppInfo;
use codex_clawbot::ClawbotTurnMode;
use codex_clawbot::ProviderEvent;
use codex_clawbot::ProviderKind;
use codex_clawbot::ProviderSessionRef;
use codex_clawbot::SessionForwardingMode;
use codex_file_search::FileMatch;
use codex_loop::LoopContextMode;
use codex_loop::LoopResponseMode;
use codex_loop::LoopSecurityMode;
use codex_loop::LoopTriggerPhase;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_approval_presets::ApprovalPreset;

use crate::app::loop_timers::after_turn_scheduler::AfterTurnRoundResult;
use crate::bottom_pane::ApprovalRequest;
use crate::bottom_pane::StatusLineItem;
use crate::bottom_pane::TerminalTitleItem;
use crate::display_preferences::DisplayPreferenceKey;
use crate::history_cell::HistoryCell;
use crate::rate_limits::ManagedAccountQuotaUpdate;

use codex_core::config::types::ApprovalsReviewer;
use codex_features::Feature;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::SandboxPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RealtimeAudioDeviceKind {
    Microphone,
    Speaker,
}

impl RealtimeAudioDeviceKind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Microphone => "Microphone",
            Self::Speaker => "Speaker",
        }
    }

    pub(crate) fn noun(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Speaker => "speaker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) enum WindowsSandboxEnableMode {
    Elevated,
    Legacy,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct ConnectorsSnapshot {
    pub(crate) connectors: Vec<AppInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopTimerTriggerSource {
    ScheduledTimer,
    ScheduledIdle,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClawbotFeishuConfigField {
    AppId,
    AppSecret,
    VerificationToken,
    EncryptKey,
    BotOpenId,
    BotUserId,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum AppEvent {
    CodexEvent(Event),
    /// Open the fork-owned control panel.
    OpenControlPanel,
    /// Open the current-thread actions panel inside the control panel flow.
    OpenThreadPanel,
    /// Open the account pool panel inside the control panel flow.
    OpenAccountsPanel,
    /// Open the clawbot runtime panel inside the control panel flow.
    OpenClawbotPanel,
    /// Open the clawbot sessions submenu inside clawbot.
    OpenClawbotSessionsPanel,
    /// Open the Feishu configuration panel inside clawbot.
    OpenClawbotConfigPanel,
    /// Open a prompt to manually bind a Feishu session id to the current thread.
    OpenClawbotManualBindPrompt,
    /// Open the actions panel for a discovered clawbot session.
    OpenClawbotSessionActions {
        session: ProviderSessionRef,
    },
    /// Open a prompt to edit one Feishu config field.
    OpenClawbotFeishuConfigPrompt {
        field: ClawbotFeishuConfigField,
    },
    /// Persist one Feishu config field to the workspace clawbot config.
    SaveClawbotFeishuConfigValue {
        field: ClawbotFeishuConfigField,
        value: String,
    },
    /// Persist a manual Feishu session id binding to the current thread.
    SaveClawbotManualBindSessionId {
        session_id: String,
    },
    /// Persist the turn interaction mode for clawbot-originated turns.
    ClawbotSetTurnMode {
        mode: ClawbotTurnMode,
    },
    /// Bind a clawbot session to the current thread.
    ClawbotConnectCurrentThread {
        session: ProviderSessionRef,
    },
    /// Remove the thread binding for a clawbot session.
    ClawbotDisconnect {
        session: ProviderSessionRef,
    },
    /// Enable or disable one forwarding path for a bound clawbot session.
    ClawbotSetSessionForwarding {
        session: ProviderSessionRef,
        mode: SessionForwardingMode,
    },
    /// Clear cached unread messages for a clawbot session.
    ClawbotFlushCachedMessages {
        session: ProviderSessionRef,
    },
    /// Retry the provider runtime connection and persist the new status.
    ClawbotRetryConnection {
        provider: ProviderKind,
    },
    /// Scan the provider session list and refresh persisted clawbot sessions.
    ClawbotScanSessions {
        provider: ProviderKind,
    },
    /// Clear unbound provider sessions while preserving active bindings.
    ClawbotClearSessions {
        provider: ProviderKind,
    },
    /// Apply one normalized provider event emitted by the clawbot runtime task.
    ClawbotProviderEvent {
        event: Box<ProviderEvent>,
    },
    /// Apply one completed after-turn loop round that was executed off the main UI path.
    PrimaryAfterTurnRoundCompleted {
        result: Result<Box<AfterTurnRoundResult>, String>,
    },
    /// Update the currently running loop label inside an after-turn round.
    PrimaryAfterTurnRoundProgress {
        loop_label: String,
    },
    /// Open the loop timers panel inside the control panel flow.
    OpenLoopTimersPanel,
    /// Open the workspace trigger-queue panel inside Loop Manager.
    OpenLoopTriggerQueuesPanel,
    /// Open the trigger queue entries for a specific phase.
    OpenLoopTriggerQueuePhase {
        phase: LoopTriggerPhase,
    },
    /// Open actions for one trigger queue entry inside a phase.
    OpenLoopTriggerQueueEntryActions {
        phase: LoopTriggerPhase,
        loop_id: String,
        binding_id: String,
    },
    /// Move a loop trigger queue entry earlier or later within a phase.
    MoveLoopTriggerQueueEntry {
        phase: LoopTriggerPhase,
        loop_id: String,
        binding_id: String,
        move_up: bool,
    },
    /// Open the transcript jump panel inside the control panel flow.
    OpenJumpToMessagePanel,
    /// Open the show/hide settings panel inside the control panel flow.
    OpenDisplayPreferencesPanel,
    /// Start a hidden ephemeral `/btw` discussion.
    StartBtwDiscussion {
        prompt: String,
    },
    /// Create a persisted workspace loop timer from `/loop ...`.
    CreateLoopTimer {
        spec: String,
    },
    /// Open the loop creation submenu inside Loop Manager.
    OpenCreateLoopTimerMenu,
    /// Start the step-by-step create flow for a new loop agent.
    StartCreateLoopDraft {
        context_mode: LoopContextMode,
    },
    /// Persist the loop id for the active create draft.
    SaveCreateLoopId {
        id: String,
    },
    /// Persist the prompt for the active create draft.
    SaveCreateLoopPrompt {
        prompt: String,
    },
    /// Open the trigger picker for the active create draft.
    OpenCreateLoopDraftTriggerMenu,
    /// Open the timer-schedule prompt for the active create draft.
    OpenCreateLoopTimerSchedulePrompt,
    /// Open the idle-after prompt for the active create draft.
    OpenCreateLoopIdleAfterPrompt,
    /// Persist a timer trigger schedule for the active create draft.
    SaveCreateLoopTimerSchedule {
        schedule: String,
    },
    /// Persist an idle trigger for the active create draft.
    SaveCreateLoopIdleTrigger {
        after: String,
    },
    /// Persist a before-turn trigger for the active create draft.
    SaveCreateLoopBeforeTurnTrigger,
    /// Persist an after-turn trigger for the active create draft.
    SaveCreateLoopAfterTurnTrigger,
    /// Open the response-mode picker for the active create draft.
    OpenCreateLoopDraftResponseMode,
    /// Persist the response mode for the active create draft.
    SaveCreateLoopResponseMode {
        response_mode: LoopResponseMode,
    },
    /// Persist the security mode for the active create draft.
    SaveCreateLoopSecurityMode {
        security_mode: LoopSecurityMode,
    },
    /// Persist writable directories for the active create draft.
    SaveCreateLoopWritableRoots {
        writable_roots: String,
    },
    /// Execute a due loop timer once.
    TriggerLoopTimer {
        timer_id: String,
        scheduled_for_unix_seconds: i64,
        source: LoopTimerTriggerSource,
    },
    /// Open the actions view for a specific loop timer.
    OpenLoopTimerActions {
        timer_id: String,
    },
    /// Open the triggers view for a specific loop timer.
    OpenLoopTimerTriggers {
        timer_id: String,
    },
    /// Open a create-trigger menu for a loop timer.
    OpenCreateLoopTriggerMenu {
        timer_id: String,
    },
    /// Add a before-turn trigger to a loop timer.
    AddLoopBeforeTurnTrigger {
        timer_id: String,
    },
    /// Add an after-turn trigger to a loop timer.
    AddLoopAfterTurnTrigger {
        timer_id: String,
    },
    /// Open an editor to create an idle trigger for a loop timer.
    OpenCreateLoopIdleTriggerAfter {
        timer_id: String,
    },
    /// Open an editor to create a timer trigger for a loop timer.
    OpenCreateLoopTimerTriggerSchedule {
        timer_id: String,
    },
    /// Persist a newly created idle trigger for a loop timer.
    SaveNewLoopIdleTriggerAfter {
        timer_id: String,
        after: String,
    },
    /// Persist a newly created timer trigger for a loop timer.
    SaveNewLoopTimerTriggerSchedule {
        timer_id: String,
        schedule: String,
    },
    /// Open an editor for an existing idle trigger duration.
    OpenEditLoopTriggerBindingIdleAfter {
        timer_id: String,
        binding_id: String,
    },
    /// Open an editor for an existing timer trigger schedule.
    OpenEditLoopTriggerBindingSchedule {
        timer_id: String,
        binding_id: String,
    },
    /// Open actions for an existing loop trigger binding.
    OpenLoopTriggerBindingActions {
        timer_id: String,
        binding_id: String,
    },
    /// Persist a new schedule for an existing timer trigger.
    SaveLoopTriggerBindingSchedule {
        timer_id: String,
        binding_id: String,
        schedule: String,
    },
    /// Persist a new idle-after duration for an existing idle trigger.
    SaveLoopTriggerBindingIdleAfter {
        timer_id: String,
        binding_id: String,
        after: String,
    },
    /// Enable one loop trigger binding.
    EnableLoopTriggerBinding {
        timer_id: String,
        binding_id: String,
    },
    /// Disable one loop trigger binding.
    DisableLoopTriggerBinding {
        timer_id: String,
        binding_id: String,
    },
    /// Delete one loop trigger binding.
    DeleteLoopTriggerBinding {
        timer_id: String,
        binding_id: String,
    },
    /// Open a prompt editor for a loop timer.
    OpenEditLoopTimerPrompt {
        timer_id: String,
    },
    /// Open an action editor for a loop timer.
    OpenEditLoopTimerAction {
        timer_id: String,
    },
    /// Open a context-mode picker for a loop timer.
    OpenEditLoopTimerContextMode {
        timer_id: String,
    },
    /// Open a response-mode picker for a loop timer.
    OpenEditLoopTimerResponseMode {
        timer_id: String,
    },
    /// Open a security-mode picker for a loop timer.
    OpenEditLoopTimerSecurityMode {
        timer_id: String,
    },
    /// Open execution settings for a loop timer.
    OpenLoopExecutionPanel {
        timer_id: String,
    },
    /// Open an editor for a loop timer's writable directories.
    OpenEditLoopWritableRoots {
        timer_id: String,
    },
    /// Open an editor for a loop timer's working directory.
    OpenEditLoopTimerCwd {
        timer_id: String,
    },
    /// Persist an updated prompt for a loop timer.
    SaveLoopTimerPrompt {
        timer_id: String,
        prompt: String,
    },
    /// Persist an updated action for a loop timer.
    SaveLoopTimerAction {
        timer_id: String,
        action: String,
    },
    /// Persist an updated context mode for a loop timer.
    SaveLoopTimerContextMode {
        timer_id: String,
        context_mode: LoopContextMode,
    },
    /// Persist an updated response mode for a loop timer.
    SaveLoopTimerResponseMode {
        timer_id: String,
        response_mode: LoopResponseMode,
    },
    /// Persist an updated security mode for a loop timer.
    SaveLoopTimerSecurityMode {
        timer_id: String,
        security_mode: LoopSecurityMode,
    },
    /// Persist updated writable directories for a loop timer.
    SaveLoopWritableRoots {
        timer_id: String,
        writable_roots: String,
    },
    /// Persist an updated working directory override for a loop timer.
    SaveLoopTimerCwd {
        timer_id: String,
        cwd: String,
    },
    /// Reset a loop timer's working directory override back to the session default.
    ResetLoopTimerCwd {
        timer_id: String,
    },
    /// Reset a loop timer's writable-directory override back to the session default.
    ResetLoopWritableRoots {
        timer_id: String,
    },
    /// Enable a disabled loop timer.
    EnableLoopTimer {
        timer_id: String,
    },
    /// Disable an enabled loop timer.
    DisableLoopTimer {
        timer_id: String,
    },
    /// Delete a loop timer.
    DeleteLoopTimer {
        timer_id: String,
    },
    /// Final result for a hidden `/loop` execution.
    LoopTimerCompleted {
        timer_id: String,
        prompt: String,
        result: Result<Option<String>, String>,
    },
    /// Final result for a hidden `/btw` discussion.
    BtwCompleted {
        thread_id: ThreadId,
        result: Result<String, String>,
    },
    /// Insert a concise summary from the completed `/btw` answer into the composer.
    BtwInsertSummary,
    /// Insert the full completed `/btw` answer into the composer.
    BtwInsertFull,
    /// Discard and destroy the active `/btw` discussion.
    BtwDiscard,
    /// Restore the latest committed user message into the composer and rollback one turn.
    UndoLastUserMessage,
    /// Open the API profiles submenu inside Accounts.
    OpenApiProfilesPanel,
    /// Open the subscriptions submenu inside Accounts.
    OpenSubscriptionsPanel,
    /// Open the managed-account alias rename submenu.
    OpenManagedAccountRenamePanel,
    /// Open the routed-profile add submenu.
    #[allow(dead_code)]
    OpenProfileRouteAddPanel,
    /// Open the routed-profile reorder submenu.
    #[allow(dead_code)]
    OpenProfileRouteReorderPanel,
    /// Open the routed-profile delete submenu.
    #[allow(dead_code)]
    OpenProfileRouteDeletePanel,
    /// Open actions for one API profile.
    OpenApiProfileActions {
        profile_id: String,
    },
    /// Open the create-API-profile name prompt.
    OpenCreateApiProfileNamePrompt,
    /// Persist the new API profile id in the in-flight draft.
    SaveCreateApiProfileName {
        profile_id: String,
    },
    /// Persist the new API profile endpoint in the in-flight draft.
    SaveCreateApiProfileEndpoint {
        endpoint: String,
    },
    /// Finalize API profile creation from the in-flight draft key input.
    SaveCreateApiProfileKey {
        key: String,
    },
    /// Open the endpoint editor for one API profile.
    OpenEditApiProfileEndpointPrompt {
        profile_id: String,
        current_endpoint: String,
    },
    /// Persist the endpoint for one API profile.
    SaveApiProfileEndpoint {
        profile_id: String,
        endpoint: String,
    },
    /// Open the key editor for one API profile.
    OpenEditApiProfileKeyPrompt {
        profile_id: String,
        current_key: String,
    },
    /// Persist the direct bearer token for one API profile.
    SaveApiProfileKey {
        profile_id: String,
        key: String,
    },
    /// Delete one API profile from config and routing state.
    DeleteApiProfile {
        profile_id: String,
    },
    /// Open the managed-account delete submenu.
    OpenManagedAccountDeletePanel,
    /// Refresh cached quota for the current managed ChatGPT account.
    RefreshManagedAccountQuota,
    /// Refresh cached quota for all managed ChatGPT accounts.
    RefreshAllManagedAccountsQuota,
    /// Add an existing config profile into the routed profile group.
    AddProfileRoute(String),
    /// Remove one profile from the routed profile group.
    DeleteProfileRoute(String),
    /// Move one profile earlier or later inside the routed profile group.
    MoveProfileRoute {
        profile_id: String,
        move_up: bool,
    },
    /// Mark a routed profile as active for future turns.
    SetProfileRouteActive(String),
    /// Mark a managed account as active in the fork-owned registry.
    SetManagedAccountActive(String),
    /// Open an alias editor for a managed account.
    OpenRenameManagedAccountAliasPrompt {
        account_id: String,
        current_alias: String,
    },
    /// Open a delete confirmation view for a managed account.
    OpenDeleteManagedAccountConfirmation {
        account_id: String,
        display_name: String,
    },
    /// Persist a new alias for a managed account.
    SaveManagedAccountAlias {
        account_id: String,
        alias: String,
    },
    /// Delete a managed account from the pool and remove its saved auth snapshot.
    DeleteManagedAccount(String),
    /// Delete all invalid managed accounts from the pool and remove their saved auth snapshots.
    DeleteAllInvalidManagedAccounts,
    /// Retry the last turn using the routed profile fallback policy.
    RetryLastUserTurnWithProfileFallback {
        error_info: CodexErrorInfo,
        error_message: String,
    },
    /// Open the agent picker for switching active threads.
    OpenAgentPicker,
    /// Switch the active thread to the selected agent.
    SelectAgentThread(ThreadId),
    /// Open the transcript overlay and highlight a committed transcript cell.
    JumpToTranscriptCell(usize),
    /// Toggle a local TUI-only display preference.
    ToggleDisplayPreference(DisplayPreferenceKey),
    /// Stop any currently running hidden `/loop` executions.
    StopBackgroundLoopRuns,

    /// Submit an op to the specified thread, regardless of current focus.
    SubmitThreadOp {
        thread_id: ThreadId,
        op: codex_protocol::protocol::Op,
    },

    /// Forward an event from a non-primary thread into the app-level thread router.
    ThreadEvent {
        thread_id: ThreadId,
        event: Event,
    },

    /// Start a new session.
    NewSession,

    /// Clear the terminal UI (screen + scrollback), start a fresh session, and keep the
    /// previous chat resumable.
    ClearUi,

    /// Open the resume picker inside the running TUI session.
    OpenResumePicker,
    /// Open the resume picker across all saved sessions, ignoring cwd filtering.
    OpenResumePickerAll,

    /// Fork the current session into a new thread.
    ForkCurrentSession,

    /// Request to exit the application.
    ///
    /// Use `ShutdownFirst` for user-initiated quits so core cleanup runs and the
    /// UI exits only after `ShutdownComplete`. `Immediate` is a last-resort
    /// escape hatch that skips shutdown and may drop in-flight work (e.g.,
    /// background tasks, rollout flush, or child process cleanup).
    Exit(ExitMode),

    /// Request to exit the application due to a fatal error.
    FatalExitRequest(String),

    /// Forward an `Op` to the Agent. Using an `AppEvent` for this avoids
    /// bubbling channels through layers of widgets.
    CodexOp(codex_protocol::protocol::Op),

    /// Kick off an asynchronous file search for the given query (text after
    /// the `@`). Previous searches may be cancelled by the app layer so there
    /// is at most one in-flight search.
    StartFileSearch(String),

    /// Result of a completed asynchronous file search. The `query` echoes the
    /// original search term so the UI can decide whether the results are
    /// still relevant.
    FileSearchResult {
        query: String,
        matches: Vec<FileMatch>,
    },

    /// Result of refreshing rate limits
    RateLimitSnapshotFetched(RateLimitSnapshot),
    /// Result of explicitly refreshing managed-account quota from the Accounts panel.
    ManagedAccountsQuotaRefreshed(Result<Vec<ManagedAccountQuotaUpdate>, String>),

    /// Result of prefetching connectors.
    ConnectorsLoaded {
        result: Result<ConnectorsSnapshot, String>,
        is_final: bool,
    },

    /// Result of computing a `/diff` command.
    DiffResult(String),

    /// Open the app link view in the bottom pane.
    OpenAppLink {
        app_id: String,
        title: String,
        description: Option<String>,
        instructions: String,
        url: String,
        is_installed: bool,
        is_enabled: bool,
    },

    /// Open the provided URL in the user's browser.
    OpenUrlInBrowser {
        url: String,
    },

    /// Refresh app connector state and mention bindings.
    RefreshConnectors {
        force_refetch: bool,
    },

    /// Fetch plugin marketplace state for the provided working directory.
    FetchPluginsList {
        cwd: PathBuf,
    },

    /// Result of fetching plugin marketplace state.
    PluginsLoaded {
        cwd: PathBuf,
        result: Result<PluginListResponse, String>,
    },

    /// Replace the plugins popup with a plugin-detail loading state.
    OpenPluginDetailLoading {
        plugin_display_name: String,
    },

    /// Fetch detail for a specific plugin from a marketplace.
    FetchPluginDetail {
        cwd: PathBuf,
        params: PluginReadParams,
    },

    /// Result of fetching plugin detail.
    PluginDetailLoaded {
        cwd: PathBuf,
        result: Result<PluginReadResponse, String>,
    },

    /// Replace the plugins popup with an install loading state.
    OpenPluginInstallLoading {
        plugin_display_name: String,
    },

    /// Replace the plugins popup with an uninstall loading state.
    OpenPluginUninstallLoading {
        plugin_display_name: String,
    },

    /// Install a specific plugin from a marketplace.
    FetchPluginInstall {
        cwd: PathBuf,
        marketplace_path: AbsolutePathBuf,
        plugin_name: String,
        plugin_display_name: String,
    },

    /// Result of installing a plugin.
    PluginInstallLoaded {
        cwd: PathBuf,
        marketplace_path: AbsolutePathBuf,
        plugin_name: String,
        plugin_display_name: String,
        result: Result<PluginInstallResponse, String>,
    },

    /// Uninstall a specific plugin by canonical plugin id.
    FetchPluginUninstall {
        cwd: PathBuf,
        plugin_id: String,
        plugin_display_name: String,
    },

    /// Result of uninstalling a plugin.
    PluginUninstallLoaded {
        cwd: PathBuf,
        plugin_id: String,
        plugin_display_name: String,
        result: Result<PluginUninstallResponse, String>,
    },

    /// Advance the post-install plugin app-auth flow.
    PluginInstallAuthAdvance {
        refresh_connectors: bool,
    },

    /// Abandon the post-install plugin app-auth flow.
    PluginInstallAuthAbandon,

    InsertHistoryCell(Box<dyn HistoryCell>),

    /// Apply rollback semantics to local transcript cells.
    ///
    /// This is emitted when rollback was not initiated by the current
    /// backtrack flow so trimming occurs in AppEvent queue order relative to
    /// inserted history cells.
    ApplyThreadRollback {
        num_turns: u32,
    },

    StartCommitAnimation,
    StopCommitAnimation,
    CommitTick,

    /// Update the current reasoning effort in the running app and widget.
    UpdateReasoningEffort(Option<ReasoningEffort>),

    /// Update the current model slug in the running app and widget.
    UpdateModel(String),

    /// Update the active collaboration mask in the running app and widget.
    UpdateCollaborationMode(CollaborationModeMask),

    /// Update the current personality in the running app and widget.
    UpdatePersonality(Personality),

    /// Persist the selected model and reasoning effort to the appropriate config.
    PersistModelSelection {
        model: String,
        effort: Option<ReasoningEffort>,
    },

    /// Persist the selected personality to the appropriate config.
    PersistPersonalitySelection {
        personality: Personality,
    },

    /// Persist the selected service tier to the appropriate config.
    PersistServiceTierSelection {
        service_tier: Option<ServiceTier>,
    },

    /// Open the device picker for a realtime microphone or speaker.
    OpenRealtimeAudioDeviceSelection {
        kind: RealtimeAudioDeviceKind,
    },

    /// Persist the selected realtime microphone or speaker to top-level config.
    #[cfg_attr(
        any(target_os = "linux", not(feature = "voice-input")),
        allow(dead_code)
    )]
    PersistRealtimeAudioDeviceSelection {
        kind: RealtimeAudioDeviceKind,
        name: Option<String>,
    },

    /// Restart the selected realtime microphone or speaker locally.
    RestartRealtimeAudioDevice {
        kind: RealtimeAudioDeviceKind,
    },

    /// Open the reasoning selection popup after picking a model.
    OpenReasoningPopup {
        model: ModelPreset,
    },

    /// Open the Plan-mode reasoning scope prompt for the selected model/effort.
    OpenPlanReasoningScopePrompt {
        model: String,
        effort: Option<ReasoningEffort>,
    },

    /// Open the full model picker (non-auto models).
    OpenAllModelsPopup {
        models: Vec<ModelPreset>,
    },

    /// Open the confirmation prompt before enabling full access mode.
    OpenFullAccessConfirmation {
        preset: ApprovalPreset,
        return_to_permissions: bool,
    },

    /// Open the Windows world-writable directories warning.
    /// If `preset` is `Some`, the confirmation will apply the provided
    /// approval/sandbox configuration on Continue; if `None`, it performs no
    /// policy change and only acknowledges/dismisses the warning.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    OpenWorldWritableWarningConfirmation {
        preset: Option<ApprovalPreset>,
        /// Up to 3 sample world-writable directories to display in the warning.
        sample_paths: Vec<String>,
        /// If there are more than `sample_paths`, this carries the remaining count.
        extra_count: usize,
        /// True when the scan failed (e.g. ACL query error) and protections could not be verified.
        failed_scan: bool,
    },

    /// Prompt to enable the Windows sandbox feature before using Agent mode.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    OpenWindowsSandboxEnablePrompt {
        preset: ApprovalPreset,
    },

    /// Open the Windows sandbox fallback prompt after declining or failing elevation.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    OpenWindowsSandboxFallbackPrompt {
        preset: ApprovalPreset,
    },

    /// Begin the elevated Windows sandbox setup flow.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BeginWindowsSandboxElevatedSetup {
        preset: ApprovalPreset,
    },

    /// Begin the non-elevated Windows sandbox setup flow.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BeginWindowsSandboxLegacySetup {
        preset: ApprovalPreset,
    },

    /// Begin a non-elevated grant of read access for an additional directory.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BeginWindowsSandboxGrantReadRoot {
        path: String,
    },

    /// Result of attempting to grant read access for an additional directory.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    WindowsSandboxGrantReadRootCompleted {
        path: PathBuf,
        error: Option<String>,
    },

    /// Enable the Windows sandbox feature and switch to Agent mode.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    EnableWindowsSandboxForAgentMode {
        preset: ApprovalPreset,
        mode: WindowsSandboxEnableMode,
    },

    /// Update the Windows sandbox feature mode without changing approval presets.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]

    /// Update the current approval policy in the running app and widget.
    UpdateAskForApprovalPolicy(AskForApproval),

    /// Update the current sandbox policy in the running app and widget.
    UpdateSandboxPolicy(SandboxPolicy),

    /// Update the current approvals reviewer in the running app and widget.
    UpdateApprovalsReviewer(ApprovalsReviewer),

    /// Update feature flags and persist them to the top-level config.
    UpdateFeatureFlags {
        updates: Vec<(Feature, bool)>,
    },

    /// Update whether the full access warning prompt has been acknowledged.
    UpdateFullAccessWarningAcknowledged(bool),

    /// Update whether the world-writable directories warning has been acknowledged.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    UpdateWorldWritableWarningAcknowledged(bool),

    /// Update whether the rate limit switch prompt has been acknowledged for the session.
    UpdateRateLimitSwitchPromptHidden(bool),

    /// Update the Plan-mode-specific reasoning effort in memory.
    UpdatePlanModeReasoningEffort(Option<ReasoningEffort>),

    /// Persist the acknowledgement flag for the full access warning prompt.
    PersistFullAccessWarningAcknowledged,

    /// Persist the acknowledgement flag for the world-writable directories warning.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    PersistWorldWritableWarningAcknowledged,

    /// Persist the acknowledgement flag for the rate limit switch prompt.
    PersistRateLimitSwitchPromptHidden,

    /// Persist the Plan-mode-specific reasoning effort.
    PersistPlanModeReasoningEffort(Option<ReasoningEffort>),

    /// Persist the acknowledgement flag for the model migration prompt.
    PersistModelMigrationPromptAcknowledged {
        from_model: String,
        to_model: String,
    },

    /// Skip the next world-writable scan (one-shot) after a user-confirmed continue.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    SkipNextWorldWritableScan,

    /// Re-open the approval presets popup.
    OpenApprovalsPopup,

    /// Open the skills list popup.
    OpenSkillsList,

    /// Open the skills enable/disable picker.
    OpenManageSkillsPopup,

    /// Enable or disable a skill by path.
    SetSkillEnabled {
        path: PathBuf,
        enabled: bool,
    },

    /// Enable or disable an app by connector ID.
    SetAppEnabled {
        id: String,
        enabled: bool,
    },

    /// Notify that the manage skills popup was closed.
    ManageSkillsClosed,

    /// Re-open the permissions presets popup.
    OpenPermissionsPopup,

    /// Live update for the in-progress voice recording placeholder. Carries
    /// the placeholder `id` and the text to display (e.g., an ASCII meter).
    #[cfg(not(target_os = "linux"))]
    UpdateRecordingMeter {
        id: String,
        text: String,
    },

    /// Voice transcription finished for the given placeholder id.
    #[cfg(not(target_os = "linux"))]
    TranscriptionComplete {
        id: String,
        text: String,
    },

    /// Voice transcription failed; remove the placeholder identified by `id`.
    #[cfg(not(target_os = "linux"))]
    TranscriptionFailed {
        id: String,
        #[allow(dead_code)]
        error: String,
    },

    /// Open the branch picker option from the review popup.
    OpenReviewBranchPicker(PathBuf),

    /// Open the commit picker option from the review popup.
    OpenReviewCommitPicker(PathBuf),

    /// Open the custom prompt option from the review popup.
    OpenReviewCustomPrompt,

    /// Submit a user message with an explicit collaboration mask.
    SubmitUserMessageWithMode {
        text: String,
        collaboration_mode: CollaborationModeMask,
    },

    /// Open the approval popup.
    FullScreenApprovalRequest(ApprovalRequest),

    /// Open the feedback note entry overlay after the user selects a category.
    OpenFeedbackNote {
        category: FeedbackCategory,
        include_logs: bool,
    },

    /// Open the upload consent popup for feedback after selecting a category.
    OpenFeedbackConsent {
        category: FeedbackCategory,
    },

    /// Launch the external editor after a normal draw has completed.
    LaunchExternalEditor,

    /// Async update of the current git branch for status line rendering.
    StatusLineBranchUpdated {
        cwd: PathBuf,
        branch: Option<String>,
    },
    /// Apply a user-confirmed status-line item ordering/selection.
    StatusLineSetup {
        items: Vec<StatusLineItem>,
    },
    /// Dismiss the status-line setup UI without changing config.
    StatusLineSetupCancelled,
    /// Apply a user-confirmed terminal-title item ordering/selection.
    TerminalTitleSetup {
        items: Vec<TerminalTitleItem>,
    },
    /// Apply a temporary terminal-title preview while the setup UI is open.
    TerminalTitleSetupPreview {
        items: Vec<TerminalTitleItem>,
    },
    /// Dismiss the terminal-title setup UI without changing config.
    TerminalTitleSetupCancelled,

    /// Apply a user-confirmed syntax theme selection.
    SyntaxThemeSelected {
        name: String,
    },
}

/// The exit strategy requested by the UI layer.
///
/// Most user-initiated exits should use `ShutdownFirst` so core cleanup runs and the UI exits only
/// after core acknowledges completion. `Immediate` is an escape hatch for cases where shutdown has
/// already completed (or is being bypassed) and the UI loop should terminate right away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitMode {
    /// Shutdown core and exit after completion.
    ShutdownFirst,
    /// Exit the UI loop immediately without waiting for shutdown.
    ///
    /// This skips `Op::Shutdown`, so any in-flight work may be dropped and
    /// cleanup that normally runs before `ShutdownComplete` can be missed.
    Immediate,
    /// Exit the UI loop immediately and ask the launcher to restart into the
    /// current session.
    RespawnImmediate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedbackCategory {
    BadResult,
    GoodResult,
    Bug,
    SafetyCheck,
    Other,
}
