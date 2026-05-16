use super::App;
use super::workflow_definition::LoadedWorkflowJob;
use super::workflow_definition::LoadedWorkflowRegistry;
use super::workflow_definition::WorkflowContextStrategy;
use super::workflow_definition::WorkflowExecutionStrategy;
use super::workflow_definition::WorkflowResponseMode;
use super::workflow_definition::WorkflowStep;
use super::workflow_definition::WorkflowTriggerKind;
use super::workflow_definition::load_workflow_registry;
use super::workflow_definition::ordered_jobs_for_roots;
use super::workflow_history::WorkflowReplySource;
use super::workflow_history::workflow_result_cell;
use crate::app_event::AppEvent;
use crate::app_event::WorkflowEvent;
use crate::app_server_session::AppServerSession;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::legacy_core::config::Config;
use crate::session_state::ThreadSessionState;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ApprovalsReviewer as AppServerApprovalsReviewer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadCompactStartParams;
use codex_app_server_protocol::ThreadCompactStartResponse;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::ThreadUnsubscribeParams;
use codex_app_server_protocol::ThreadUnsubscribeResponse;
use codex_app_server_protocol::TurnInterruptParams;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_protocol::ThreadId;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const WORKFLOW_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WORKFLOW_INTERRUPT_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);
const WORKFLOW_STEP_TIMEOUT: Duration = Duration::from_secs(30);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub(crate) type WorkflowThreadNotificationChannels =
    Arc<tokio::sync::Mutex<HashMap<ThreadId, mpsc::UnboundedSender<ServerNotification>>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowTriggerOverlapBehavior {
    Queue,
    Skip,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BackgroundWorkflowRunTarget {
    Trigger {
        workflow_name: String,
        trigger_id: String,
        phase_context: OwnedWorkflowPhaseContext,
        overlap_behavior: WorkflowTriggerOverlapBehavior,
    },
    Job {
        workflow_name: String,
        job_name: String,
    },
}

impl BackgroundWorkflowRunTarget {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn workflow_name(&self) -> &str {
        match self {
            Self::Trigger { workflow_name, .. } | Self::Job { workflow_name, .. } => workflow_name,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn slot_key(&self) -> &str {
        match self {
            Self::Trigger { trigger_id, .. } => trigger_id,
            Self::Job { job_name, .. } => job_name,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn label(&self) -> String {
        format!("{} · {}", self.workflow_name(), self.slot_key())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    #[allow(dead_code)]
    fn started_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger started",
            Self::Job { .. } => "Workflow job started",
        }
    }

    fn completed_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger completed",
            Self::Job { .. } => "Workflow job completed",
        }
    }

    fn stopped_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger stopped",
            Self::Job { .. } => "Workflow job stopped",
        }
    }

    fn failed_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger failed",
            Self::Job { .. } => "Workflow job failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowOutputDelivery {
    MainThreadInput,
    MainThreadCompactInput,
    AssistantCell,
    UserFollowup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowDisabledJobBehavior {
    Skip,
    RunRootJobs,
}

#[derive(Debug, Clone, Copy)]
struct WorkflowRunSpec<'a> {
    workflow_name: &'a str,
    trigger_id: &'a str,
    root_jobs: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkflowPhaseContext<'a> {
    pub(crate) current_user_turn: Option<&'a str>,
    pub(crate) last_assistant_message: Option<&'a str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OwnedWorkflowPhaseContext {
    pub(crate) current_user_turn: Option<String>,
    pub(crate) last_assistant_message: Option<String>,
}

impl OwnedWorkflowPhaseContext {
    fn borrowed(&self) -> WorkflowPhaseContext<'_> {
        WorkflowPhaseContext {
            current_user_turn: self.current_user_turn.as_deref(),
            last_assistant_message: self.last_assistant_message.as_deref(),
        }
    }
}

impl From<WorkflowPhaseContext<'_>> for OwnedWorkflowPhaseContext {
    fn from(value: WorkflowPhaseContext<'_>) -> Self {
        Self {
            current_user_turn: value.current_user_turn.map(str::to_owned),
            last_assistant_message: value.last_assistant_message.map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowJobRunResult {
    pub(crate) delivery: WorkflowOutputDelivery,
    pub(crate) execution_strategy: WorkflowExecutionStrategy,
    pub(crate) workflow_name: String,
    pub(crate) trigger_id: String,
    pub(crate) job_name: String,
    pub(crate) message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BackgroundWorkflowRunOutcome {
    Completed(Vec<WorkflowJobRunResult>),
    Cancelled,
    Failed(String),
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
pub(crate) struct BackgroundWorkflowRunResult {
    #[allow(dead_code)]
    pub(crate) target: BackgroundWorkflowRunTarget,
    pub(crate) outcome: BackgroundWorkflowRunOutcome,
}

#[derive(Clone)]
struct WorkflowThreadSession {
    thread_id: String,
    cwd: PathBuf,
    execution_config: WorkflowExecutionConfig,
    notifications: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ServerNotification>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowExecutionConfig {
    model: String,
    model_provider_id: String,
    service_tier: Option<codex_protocol::config_types::ServiceTier>,
    approval_policy: AskForApproval,
    approvals_reviewer: codex_protocol::config_types::ApprovalsReviewer,
    sandbox_policy: SandboxPolicy,
    cwd: PathBuf,
    reasoning_effort: Option<codex_protocol::openai_models::ReasoningEffort>,
}

#[derive(Clone, Copy)]
struct WorkflowStepExecutionContext<'a> {
    workflow_name: &'a str,
    trigger_id: &'a str,
    job: &'a LoadedWorkflowJob,
    phase_context: WorkflowPhaseContext<'a>,
    cancellation: Option<&'a CancellationToken>,
}

#[derive(Debug, Clone, PartialEq)]
struct WorkflowTurnState {
    status: TurnStatus,
    error: Option<String>,
    last_agent_message: Option<String>,
}

trait WorkflowRuntimeClient: Send + Sync {
    fn start_workflow_thread(
        &self,
        strategy: WorkflowThreadStartStrategy,
        execution_strategy: WorkflowExecutionStrategy,
    ) -> BoxFuture<'_, Result<WorkflowThreadSession, String>>;
    fn start_turn(
        &self,
        thread: &WorkflowThreadSession,
        input: String,
    ) -> BoxFuture<'_, Result<String, String>>;
    fn read_turn<'a>(
        &'a self,
        thread: &'a WorkflowThreadSession,
        turn_id: String,
    ) -> BoxFuture<'a, Result<WorkflowTurnState, String>>;
    fn interrupt_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> BoxFuture<'_, Result<(), String>>;
    fn unsubscribe_thread(&self, thread_id: String) -> BoxFuture<'_, Result<(), String>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowThreadStartStrategy {
    Auto,
    New,
    Fork,
    ForkCompact,
}

impl WorkflowThreadStartStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::New => "new",
            Self::Fork => "fork",
            Self::ForkCompact => "fork_compact",
        }
    }
}

pub(crate) struct AppServerWorkflowRuntimeClient {
    request_handle: AppServerRequestHandle,
    workflow_thread_notification_channels: WorkflowThreadNotificationChannels,
    config: Config,
    primary_thread_id: Option<ThreadId>,
    primary_session_configured: Option<ThreadSessionState>,
    is_remote: bool,
    remote_cwd_override: Option<PathBuf>,
}

impl AppServerWorkflowRuntimeClient {
    pub(crate) fn new(
        app_server: &AppServerSession,
        workflow_thread_notification_channels: WorkflowThreadNotificationChannels,
        config: Config,
        primary_thread_id: Option<ThreadId>,
        primary_session_configured: Option<ThreadSessionState>,
    ) -> Self {
        Self {
            request_handle: app_server.request_handle(),
            workflow_thread_notification_channels,
            config,
            primary_thread_id,
            primary_session_configured,
            is_remote: app_server.is_remote(),
            remote_cwd_override: app_server.remote_cwd_override().map(PathBuf::from),
        }
    }
}

impl WorkflowRuntimeClient for AppServerWorkflowRuntimeClient {
    fn start_workflow_thread(
        &self,
        strategy: WorkflowThreadStartStrategy,
        execution_strategy: WorkflowExecutionStrategy,
    ) -> BoxFuture<'_, Result<WorkflowThreadSession, String>> {
        Box::pin(async move {
            let execution_config = self.resolve_execution_config(execution_strategy)?;
            let fork_source_thread_id = match strategy {
                WorkflowThreadStartStrategy::Auto => self.available_fork_source_thread_id().await?,
                WorkflowThreadStartStrategy::New => None,
                WorkflowThreadStartStrategy::Fork | WorkflowThreadStartStrategy::ForkCompact => {
                    Some(self.available_fork_source_thread_id().await?.ok_or_else(|| {
                        format!(
                            "workflow context strategy requires a materialized primary thread for `{}`",
                            strategy.as_str()
                        )
                    })?)
                }
            };

            let thread = if let Some(primary_thread_id) = fork_source_thread_id {
                let response: ThreadForkResponse = self
                    .request_handle
                    .request_typed(ClientRequest::ThreadFork {
                        request_id: request_id(),
                        params: workflow_thread_fork_params(
                            &execution_config,
                            self.config.active_profile.as_deref(),
                            primary_thread_id,
                            self.is_remote,
                            self.remote_cwd_override.as_deref(),
                        ),
                    })
                    .await
                    .map_err(|err| format!("failed to fork workflow thread: {err}"))?;
                register_workflow_thread(
                    &self.workflow_thread_notification_channels,
                    response.thread.id,
                    response.cwd.to_path_buf(),
                    execution_config.clone(),
                )
                .await?
            } else {
                let response: ThreadStartResponse = self
                    .request_handle
                    .request_typed(ClientRequest::ThreadStart {
                        request_id: request_id(),
                        params: workflow_thread_start_params(
                            &execution_config,
                            self.config.active_profile.as_deref(),
                            self.is_remote,
                            self.remote_cwd_override.as_deref(),
                        ),
                    })
                    .await
                    .map_err(|err| format!("failed to start workflow thread: {err}"))?;
                register_workflow_thread(
                    &self.workflow_thread_notification_channels,
                    response.thread.id,
                    response.cwd.to_path_buf(),
                    execution_config,
                )
                .await?
            };

            if matches!(strategy, WorkflowThreadStartStrategy::ForkCompact) {
                self.compact_workflow_thread(&thread).await?;
            }

            Ok(thread)
        })
    }

    fn start_turn(
        &self,
        thread: &WorkflowThreadSession,
        input: String,
    ) -> BoxFuture<'_, Result<String, String>> {
        let thread_id = thread.thread_id.clone();
        let cwd = thread.cwd.clone();
        let execution_config = thread.execution_config.clone();
        Box::pin(async move {
            let response: TurnStartResponse = self
                .request_handle
                .request_typed(ClientRequest::TurnStart {
                    request_id: request_id(),
                    params: TurnStartParams {
                        thread_id,
                        input: vec![
                            UserInput::Text {
                                text: input,
                                text_elements: Vec::new(),
                            }
                            .into(),
                        ],
                        cwd: Some(cwd),
                        approval_policy: Some(execution_config.approval_policy.into()),
                        approvals_reviewer: Some(execution_config.approvals_reviewer.into()),
                        sandbox_policy: Some(execution_config.sandbox_policy.into()),
                        permissions: None,
                        environments: None,
                        model: Some(execution_config.model),
                        responsesapi_client_metadata: None,
                        service_tier: Some(
                            execution_config
                                .service_tier
                                .map(|service_tier| service_tier.request_value().to_string()),
                        ),
                        effort: execution_config.reasoning_effort,
                        summary: self.config.model_reasoning_summary,
                        personality: None,
                        output_schema: None,
                        collaboration_mode: None,
                    },
                })
                .await
                .map_err(|err| format!("failed to start workflow turn: {err}"))?;
            Ok(response.turn.id)
        })
    }

    fn read_turn<'a>(
        &'a self,
        thread: &'a WorkflowThreadSession,
        turn_id: String,
    ) -> BoxFuture<'a, Result<WorkflowTurnState, String>> {
        Box::pin(async move {
            let mut last_agent_message = None;
            loop {
                {
                    let mut notifications = thread.notifications.lock().await;
                    loop {
                        match notifications.try_recv() {
                            Ok(ServerNotification::ItemCompleted(notification))
                                if notification.thread_id == thread.thread_id
                                    && notification.turn_id == turn_id =>
                            {
                                update_last_workflow_agent_message(
                                    &mut last_agent_message,
                                    &notification,
                                );
                            }
                            Ok(ServerNotification::TurnCompleted(notification))
                                if notification.thread_id == thread.thread_id
                                    && notification.turn.id == turn_id =>
                            {
                                let status = notification.turn.status.clone();
                                let error =
                                    notification.turn.error.clone().map(|error| error.message);
                                return Ok(WorkflowTurnState {
                                    status,
                                    error,
                                    last_agent_message: last_agent_message.or_else(|| {
                                        last_agent_message_for_turn_items(
                                            notification.turn.items.as_slice(),
                                        )
                                    }),
                                });
                            }
                            Ok(_) => {}
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                            | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                        }
                    }
                }

                let response: ThreadReadResponse = match self
                    .request_handle
                    .request_typed(ClientRequest::ThreadRead {
                        request_id: request_id(),
                        params: ThreadReadParams {
                            thread_id: thread.thread_id.clone(),
                            include_turns: true,
                        },
                    })
                    .await
                {
                    Ok(response) => response,
                    Err(err)
                        if {
                            let message = err.to_string();
                            message
                                .contains("includeTurns is unavailable before first user message")
                                || message.contains("ephemeral threads do not support includeTurns")
                        } =>
                    {
                        sleep(WORKFLOW_POLL_INTERVAL).await;
                        continue;
                    }
                    Err(err) => {
                        return Err(format!("failed to read workflow turn `{turn_id}`: {err}"));
                    }
                };
                let Some(turn) = response
                    .thread
                    .turns
                    .into_iter()
                    .find(|turn| turn.id == turn_id)
                else {
                    sleep(WORKFLOW_POLL_INTERVAL).await;
                    continue;
                };
                if let Some(message) = last_agent_message_for_turn_items(turn.items.as_slice()) {
                    last_agent_message = Some(message);
                }
                if !matches!(turn.status, TurnStatus::InProgress) {
                    return Ok(WorkflowTurnState {
                        status: turn.status,
                        error: turn.error.map(|error| error.message),
                        last_agent_message,
                    });
                }
                sleep(WORKFLOW_POLL_INTERVAL).await;
            }
        })
    }

    fn interrupt_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _: TurnInterruptResponse = self
                .request_handle
                .request_typed(ClientRequest::TurnInterrupt {
                    request_id: request_id(),
                    params: TurnInterruptParams { thread_id, turn_id },
                })
                .await
                .map_err(|err| format!("failed to interrupt workflow turn: {err}"))?;
            Ok(())
        })
    }

    fn unsubscribe_thread(&self, thread_id: String) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let result: Result<ThreadUnsubscribeResponse, String> = self
                .request_handle
                .request_typed(ClientRequest::ThreadUnsubscribe {
                    request_id: request_id(),
                    params: ThreadUnsubscribeParams {
                        thread_id: thread_id.clone(),
                    },
                })
                .await
                .map_err(|err| format!("failed to unsubscribe workflow thread: {err}"));
            if let Ok(parsed_thread_id) = ThreadId::from_string(&thread_id) {
                self.workflow_thread_notification_channels
                    .lock()
                    .await
                    .remove(&parsed_thread_id);
            }
            result.map(|_| ())
        })
    }
}

impl AppServerWorkflowRuntimeClient {
    fn resolve_execution_config(
        &self,
        execution_strategy: WorkflowExecutionStrategy,
    ) -> Result<WorkflowExecutionConfig, String> {
        let session = self.primary_session_configured.as_ref().ok_or_else(|| {
            format!(
                "workflow execution strategy `{}` requires a current primary session",
                execution_strategy.as_str()
            )
        })?;

        let mut execution_config = WorkflowExecutionConfig {
            model: session.model.clone(),
            model_provider_id: session.model_provider_id.clone(),
            service_tier: session
                .service_tier
                .as_deref()
                .and_then(codex_protocol::config_types::ServiceTier::from_request_value),
            approval_policy: session.approval_policy.to_core(),
            approvals_reviewer: session.approvals_reviewer,
            sandbox_policy: session
                .permission_profile
                .to_legacy_sandbox_policy(session.cwd.as_path())
                .unwrap_or(SandboxPolicy::DangerFullAccess),
            cwd: session.cwd.to_path_buf(),
            reasoning_effort: session.reasoning_effort,
        };
        if execution_strategy == WorkflowExecutionStrategy::OverrideYolo {
            execution_config.approval_policy = AskForApproval::Never;
            execution_config.approvals_reviewer =
                codex_protocol::config_types::ApprovalsReviewer::User;
            execution_config.sandbox_policy = SandboxPolicy::DangerFullAccess;
        }

        Ok(execution_config)
    }

    async fn available_fork_source_thread_id(&self) -> Result<Option<ThreadId>, String> {
        let Some(primary_thread_id) = self.primary_thread_id else {
            return Ok(None);
        };

        let response: ThreadReadResponse = self
            .request_handle
            .request_typed(ClientRequest::ThreadRead {
                request_id: request_id(),
                params: ThreadReadParams {
                    thread_id: primary_thread_id.to_string(),
                    include_turns: false,
                },
            })
            .await
            .map_err(|err| format!("failed to inspect workflow source thread: {err}"))?;

        Ok(response
            .thread
            .path
            .as_ref()
            .is_some_and(|path| path.exists())
            .then_some(primary_thread_id))
    }

    async fn compact_workflow_thread(&self, thread: &WorkflowThreadSession) -> Result<(), String> {
        let _: ThreadCompactStartResponse = self
            .request_handle
            .request_typed(ClientRequest::ThreadCompactStart {
                request_id: request_id(),
                params: ThreadCompactStartParams {
                    thread_id: thread.thread_id.clone(),
                },
            })
            .await
            .map_err(|err| format!("failed to compact workflow thread: {err}"))?;

        let deadline = tokio::time::Instant::now() + WORKFLOW_STEP_TIMEOUT;
        loop {
            let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now())
            else {
                return Err(format!(
                    "workflow thread compaction timed out after {}",
                    humantime::format_duration(WORKFLOW_STEP_TIMEOUT)
                ));
            };

            {
                let mut notifications = thread.notifications.lock().await;
                loop {
                    match notifications.try_recv() {
                        Ok(ServerNotification::TurnCompleted(notification))
                            if notification.thread_id == thread.thread_id =>
                        {
                            return match notification.turn.status {
                                TurnStatus::Completed => Ok(()),
                                TurnStatus::Interrupted => {
                                    Err("workflow thread compaction was interrupted".to_string())
                                }
                                TurnStatus::Failed => Err(notification
                                    .turn
                                    .error
                                    .map(|error| error.message)
                                    .unwrap_or_else(|| {
                                        "workflow thread compaction failed".to_string()
                                    })),
                                TurnStatus::InProgress => continue,
                            };
                        }
                        Ok(_) => {}
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                        | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }
            }
            sleep(remaining.min(WORKFLOW_POLL_INTERVAL)).await;
        }
    }
}

async fn register_workflow_thread(
    workflow_thread_notification_channels: &WorkflowThreadNotificationChannels,
    thread_id: String,
    cwd: PathBuf,
    execution_config: WorkflowExecutionConfig,
) -> Result<WorkflowThreadSession, String> {
    let parsed_thread_id = ThreadId::from_string(&thread_id)
        .map_err(|err| format!("workflow thread id `{thread_id}` is invalid: {err}"))?;
    let (sender, receiver) = mpsc::unbounded_channel();
    workflow_thread_notification_channels
        .lock()
        .await
        .insert(parsed_thread_id, sender);
    Ok(WorkflowThreadSession {
        thread_id,
        cwd,
        execution_config,
        notifications: Arc::new(tokio::sync::Mutex::new(receiver)),
    })
}

#[allow(dead_code)]
impl App {
    pub(crate) async fn run_before_turn_workflows(
        &self,
        app_server: &AppServerSession,
        phase_context: WorkflowPhaseContext<'_>,
    ) -> Result<Vec<WorkflowJobRunResult>, String> {
        let registry = load_workflow_registry(self.config.cwd.as_path())
            .map_err(|error| format!("failed to load workflows: {error}"))?;
        let client = AppServerWorkflowRuntimeClient::new(
            app_server,
            self.workflow_thread_notification_channels.clone(),
            self.config.clone(),
            self.primary_thread_id,
            self.primary_session_configured.clone(),
        );
        let mut results = Vec::new();
        for workflow in &registry.files {
            for trigger in &workflow.triggers {
                if !trigger.enabled
                    || !trigger
                        .bind_thread
                        .matches_primary_thread_id(self.primary_thread_id)
                    || !matches!(trigger.kind, WorkflowTriggerKind::BeforeTurn)
                {
                    continue;
                }
                results.extend(
                    run_workflow_jobs(
                        &client,
                        &registry,
                        WorkflowRunSpec {
                            workflow_name: &workflow.name,
                            trigger_id: &trigger.id,
                            root_jobs: &trigger.jobs,
                        },
                        phase_context,
                        WorkflowDisabledJobBehavior::Skip,
                        /*cancellation*/ None,
                    )
                    .await
                    .map_err(workflow_run_error_message)?,
                );
            }
        }
        Ok(results)
    }

    pub(crate) fn start_manual_workflow_trigger_run(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        trigger_id: String,
    ) -> Arc<dyn HistoryCell> {
        let registry = match load_workflow_registry(self.config.cwd.as_path()) {
            Ok(registry) => registry,
            Err(error) => {
                return Arc::new(history_cell::new_error_event(format!(
                    "Workflow trigger failed: failed to load workflows: {error}"
                )));
            }
        };
        let Some(workflow) = registry
            .files
            .iter()
            .find(|workflow| workflow.name == workflow_name)
        else {
            return Arc::new(history_cell::new_error_event(format!(
                "Workflow trigger failed: workflow `{workflow_name}` does not exist"
            )));
        };
        let Some(trigger) = workflow
            .triggers
            .iter()
            .find(|trigger| trigger.id == trigger_id)
        else {
            return Arc::new(history_cell::new_error_event(format!(
                "Workflow trigger failed: trigger `{workflow_name}/{trigger_id}` does not exist"
            )));
        };
        if !trigger.enabled {
            return Arc::new(history_cell::new_error_event(format!(
                "Workflow trigger failed: workflow trigger `{workflow_name}/{trigger_id}` is disabled"
            )));
        }
        if !trigger
            .bind_thread
            .matches_primary_thread_id(self.primary_thread_id)
        {
            let current_primary_thread_id = self
                .primary_thread_id
                .map(|thread_id| thread_id.to_string())
                .unwrap_or_else(|| "none".to_string());
            return Arc::new(history_cell::new_error_event(format!(
                "Workflow trigger failed: current primary thread `{current_primary_thread_id}` is not allowed by `bind_thread` for `{workflow_name}/{trigger_id}`"
            )));
        }

        let label = format!("{workflow_name} · {trigger_id}");
        match self.queue_or_start_trigger_run(
            app_server,
            workflow_name,
            trigger_id,
            OwnedWorkflowPhaseContext::default(),
            WorkflowTriggerOverlapBehavior::Queue,
        ) {
            TriggerRunDispatch::Started => Arc::new(history_cell::new_info_event(
                "Workflow trigger started".to_string(),
                Some(label),
            )),
            TriggerRunDispatch::Queued => Arc::new(history_cell::new_info_event(
                "Workflow trigger queued".to_string(),
                Some(label),
            )),
            TriggerRunDispatch::Skipped => Arc::new(history_cell::new_info_event(
                "Workflow trigger skipped".to_string(),
                Some(label),
            )),
        }
    }

    pub(crate) fn start_manual_workflow_job_run(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        job_name: String,
    ) -> Arc<dyn HistoryCell> {
        let target = BackgroundWorkflowRunTarget::Job {
            workflow_name,
            job_name,
        };
        let cell: Arc<dyn HistoryCell> = Arc::new(history_cell::new_info_event(
            target.started_message().to_string(),
            Some(target.label()),
        ));
        self.start_background_workflow_run(app_server, target);
        cell
    }

    pub(crate) fn start_scheduled_workflow_trigger_run(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        trigger_id: String,
        phase_context: WorkflowPhaseContext<'_>,
    ) -> Arc<dyn HistoryCell> {
        let label = format!("{workflow_name} · {trigger_id}");
        match self.queue_or_start_trigger_run(
            app_server,
            workflow_name,
            trigger_id,
            phase_context.into(),
            WorkflowTriggerOverlapBehavior::Queue,
        ) {
            TriggerRunDispatch::Started => Arc::new(history_cell::new_info_event(
                "Workflow trigger started".to_string(),
                Some(label),
            )),
            TriggerRunDispatch::Queued => Arc::new(history_cell::new_info_event(
                "Workflow trigger queued".to_string(),
                Some(label),
            )),
            TriggerRunDispatch::Skipped => Arc::new(history_cell::new_info_event(
                "Workflow trigger skipped".to_string(),
                Some(label),
            )),
        }
    }

    pub(crate) fn start_file_watch_workflow_trigger_run(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        trigger_id: String,
    ) -> Option<Arc<dyn HistoryCell>> {
        let label = format!("{workflow_name} · {trigger_id}");
        match self.queue_or_start_trigger_run(
            app_server,
            workflow_name,
            trigger_id,
            OwnedWorkflowPhaseContext::default(),
            WorkflowTriggerOverlapBehavior::Skip,
        ) {
            TriggerRunDispatch::Started => Some(Arc::new(history_cell::new_info_event(
                "Workflow trigger started".to_string(),
                Some(label),
            ))),
            TriggerRunDispatch::Queued => Some(Arc::new(history_cell::new_info_event(
                "Workflow trigger queued".to_string(),
                Some(label),
            ))),
            TriggerRunDispatch::Skipped => None,
        }
    }

    pub(crate) fn handle_workspace_file_changes_for_workflows(
        &mut self,
        app_server: &AppServerSession,
        changed_paths: &[PathBuf],
    ) -> Vec<Arc<dyn HistoryCell>> {
        let Some(primary_thread_id) = self.primary_thread_id else {
            return Vec::new();
        };
        if changed_paths.is_empty() {
            return Vec::new();
        }

        let registry = match load_workflow_registry(self.config.cwd.as_path()) {
            Ok(registry) => registry,
            Err(error) => {
                self.chat_widget.add_error_message(format!(
                    "Workflow file_watch failed: failed to load workflows: {error}"
                ));
                return Vec::new();
            }
        };

        let mut visible_cells = Vec::new();
        for workflow in &registry.files {
            for trigger in &workflow.triggers {
                if !trigger.enabled
                    || !trigger
                        .bind_thread
                        .matches_primary_thread_id(self.primary_thread_id)
                    || !matches!(trigger.kind, WorkflowTriggerKind::FileWatch)
                {
                    continue;
                }

                let Some(cell) = self.start_file_watch_workflow_trigger_run(
                    app_server,
                    workflow.name.clone(),
                    trigger.id.clone(),
                ) else {
                    continue;
                };
                if let Some(cell) = self.record_workflow_history_cell(primary_thread_id, cell) {
                    visible_cells.push(cell);
                }
            }
        }
        visible_cells
    }

    pub(crate) fn dispatch_next_queued_trigger_run(&mut self, app_server: &AppServerSession) {
        if self.workflow_scheduler.has_running_trigger_run() {
            return;
        }
        let Some(next) = self.workflow_scheduler.dequeue_trigger_run() else {
            return;
        };
        self.start_background_workflow_run(
            app_server,
            BackgroundWorkflowRunTarget::Trigger {
                workflow_name: next.workflow_name,
                trigger_id: next.trigger_id,
                phase_context: next.phase_context,
                overlap_behavior: WorkflowTriggerOverlapBehavior::Queue,
            },
        );
    }

    pub(crate) async fn finish_background_workflow_run(
        &mut self,
        app_server: &AppServerSession,
        run_id: String,
        result: BackgroundWorkflowRunResult,
    ) -> Vec<Arc<dyn HistoryCell>> {
        let Some(run) = self
            .workflow_scheduler
            .take_background_workflow_run(&run_id)
        else {
            return Vec::new();
        };
        let completed_trigger = run.is_trigger;
        let _ = run.handle.await;

        let mut visible_cells = Vec::new();
        if let Some(primary_thread_id) = self.primary_thread_id {
            match result.outcome {
                BackgroundWorkflowRunOutcome::Completed(results) => {
                    for result in results {
                        let source = WorkflowReplySource::new(
                            workflow_job_source(&result),
                            /*action*/ None,
                        );
                        let completed_cell: Arc<dyn HistoryCell> =
                            Arc::new(history_cell::new_info_event(
                                run.target.completed_message().to_string(),
                                Some(source.hint()),
                            ));
                        if let Some(cell) =
                            self.record_workflow_history_cell(primary_thread_id, completed_cell)
                        {
                            visible_cells.push(cell);
                        }

                        let Some(message) =
                            result.message.filter(|message| !message.trim().is_empty())
                        else {
                            continue;
                        };

                        match result.delivery {
                            WorkflowOutputDelivery::AssistantCell => {
                                let assistant_cell: Arc<dyn HistoryCell> = Arc::new(
                                    workflow_result_cell(&message, self.config.cwd.as_path()),
                                );
                                if let Some(cell) = self
                                    .record_workflow_history_cell(primary_thread_id, assistant_cell)
                                {
                                    visible_cells.push(cell);
                                }
                            }
                            WorkflowOutputDelivery::MainThreadCompactInput => {
                                if let Some(cell) = self
                                    .queue_workflow_followup_to_primary_after_compact(
                                        app_server,
                                        message,
                                        source,
                                        result.execution_strategy,
                                    )
                                    .await
                                {
                                    visible_cells.push(cell);
                                }
                            }
                            WorkflowOutputDelivery::MainThreadInput
                            | WorkflowOutputDelivery::UserFollowup => {
                                if let Some(cell) = self.queue_workflow_followup_to_primary(
                                    message,
                                    source,
                                    result.execution_strategy,
                                ) {
                                    visible_cells.push(cell);
                                }
                            }
                        }
                    }
                }
                BackgroundWorkflowRunOutcome::Cancelled => {
                    let cancelled_cell: Arc<dyn HistoryCell> =
                        Arc::new(history_cell::new_info_event(
                            run.target.stopped_message().to_string(),
                            Some(run.target.label()),
                        ));
                    if let Some(cell) =
                        self.record_workflow_history_cell(primary_thread_id, cancelled_cell)
                    {
                        visible_cells.push(cell);
                    }
                }
                BackgroundWorkflowRunOutcome::Failed(error) => {
                    let error_cell: Arc<dyn HistoryCell> = Arc::new(history_cell::new_error_event(
                        format!("{}: {error}", run.target.failed_message()),
                    ));
                    if let Some(cell) =
                        self.record_workflow_history_cell(primary_thread_id, error_cell)
                    {
                        visible_cells.push(cell);
                    }
                }
            }
        }

        if completed_trigger {
            self.dispatch_next_queued_trigger_run(app_server);
        }
        self.sync_background_workflow_status();
        visible_cells
    }

    fn queue_or_start_trigger_run(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        trigger_id: String,
        phase_context: OwnedWorkflowPhaseContext,
        overlap_behavior: WorkflowTriggerOverlapBehavior,
    ) -> TriggerRunDispatch {
        if matches!(overlap_behavior, WorkflowTriggerOverlapBehavior::Skip)
            && (self
                .workflow_scheduler
                .has_active_trigger_run(&workflow_name, &trigger_id)
                || self
                    .workflow_scheduler
                    .has_queued_trigger_run(&workflow_name, &trigger_id))
        {
            return TriggerRunDispatch::Skipped;
        }

        if self.workflow_scheduler.has_running_trigger_run() {
            self.workflow_scheduler
                .enqueue_trigger_run(workflow_name, trigger_id, phase_context);
            self.sync_background_workflow_status();
            return TriggerRunDispatch::Queued;
        }

        self.start_background_workflow_run(
            app_server,
            BackgroundWorkflowRunTarget::Trigger {
                workflow_name,
                trigger_id,
                phase_context,
                overlap_behavior,
            },
        );
        TriggerRunDispatch::Started
    }

    fn start_background_workflow_run(
        &mut self,
        app_server: &AppServerSession,
        target: BackgroundWorkflowRunTarget,
    ) {
        let run_id = self
            .workflow_scheduler
            .next_background_run_id(target.workflow_name(), target.slot_key());
        let runtime_client = AppServerWorkflowRuntimeClient::new(
            app_server,
            self.workflow_thread_notification_channels.clone(),
            self.config.clone(),
            self.primary_thread_id,
            self.primary_session_configured.clone(),
        );
        let workflow_cwd = self.config.cwd.to_path_buf();
        let app_event_tx = self.app_event_tx.clone();
        let run_id_for_task = run_id.clone();
        let target_for_task = target.clone();
        let cancellation = CancellationToken::new();
        let cancellation_for_task = cancellation.clone();
        let handle = tokio::spawn(async move {
            let result = run_background_workflow(
                &runtime_client,
                workflow_cwd,
                target_for_task,
                cancellation_for_task,
            )
            .await;
            app_event_tx.send(AppEvent::Workflow(
                WorkflowEvent::BackgroundWorkflowRunCompleted {
                    run_id: run_id_for_task,
                    result: Box::new(result),
                },
            ));
        });
        self.workflow_scheduler.register_background_workflow_run(
            run_id,
            target,
            cancellation,
            handle,
        );
        self.sync_background_workflow_status();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum TriggerRunDispatch {
    Started,
    Queued,
    Skipped,
}

async fn run_background_workflow(
    client: &dyn WorkflowRuntimeClient,
    workflow_cwd: PathBuf,
    target: BackgroundWorkflowRunTarget,
    cancellation: CancellationToken,
) -> BackgroundWorkflowRunResult {
    let outcome =
        match run_background_workflow_selection(client, workflow_cwd, &target, &cancellation).await
        {
            Ok(results) => BackgroundWorkflowRunOutcome::Completed(results),
            Err(WorkflowRunError::Cancelled) => BackgroundWorkflowRunOutcome::Cancelled,
            Err(error) => BackgroundWorkflowRunOutcome::Failed(workflow_run_error_message(error)),
        };
    BackgroundWorkflowRunResult { target, outcome }
}

async fn run_background_workflow_selection(
    client: &dyn WorkflowRuntimeClient,
    workflow_cwd: PathBuf,
    target: &BackgroundWorkflowRunTarget,
    cancellation: &CancellationToken,
) -> Result<Vec<WorkflowJobRunResult>, WorkflowRunError> {
    let registry = load_workflow_registry(workflow_cwd.as_path())
        .map_err(|error| WorkflowRunError::Failed(error.to_string()))?;
    match target {
        BackgroundWorkflowRunTarget::Trigger {
            workflow_name,
            trigger_id,
            phase_context,
            overlap_behavior: _,
        } => {
            let workflow = registry
                .files
                .iter()
                .find(|workflow| workflow.name == *workflow_name)
                .ok_or_else(|| {
                    WorkflowRunError::Failed(format!("workflow `{workflow_name}` does not exist"))
                })?;
            let trigger = workflow
                .triggers
                .iter()
                .find(|trigger| trigger.id == *trigger_id)
                .ok_or_else(|| {
                    WorkflowRunError::Failed(format!("trigger `{trigger_id}` does not exist"))
                })?;
            if !trigger.enabled {
                return Err(WorkflowRunError::Failed(format!(
                    "workflow trigger `{workflow_name}/{trigger_id}` is disabled"
                )));
            }
            run_workflow_jobs(
                client,
                &registry,
                WorkflowRunSpec {
                    workflow_name,
                    trigger_id,
                    root_jobs: &trigger.jobs,
                },
                phase_context.borrowed(),
                WorkflowDisabledJobBehavior::Skip,
                Some(cancellation),
            )
            .await
        }
        BackgroundWorkflowRunTarget::Job {
            workflow_name,
            job_name,
        } => {
            let job = registry.jobs.get(job_name).ok_or_else(|| {
                WorkflowRunError::Failed(format!("workflow job `{job_name}` does not exist"))
            })?;
            if job.workflow_name != *workflow_name {
                return Err(WorkflowRunError::Failed(format!(
                    "workflow `{workflow_name}` does not define job `{job_name}`"
                )));
            }
            run_workflow_jobs(
                client,
                &registry,
                WorkflowRunSpec {
                    workflow_name,
                    trigger_id: &manual_workflow_job_trigger_id(job_name),
                    root_jobs: std::slice::from_ref(job_name),
                },
                WorkflowPhaseContext {
                    current_user_turn: None,
                    last_assistant_message: None,
                },
                WorkflowDisabledJobBehavior::RunRootJobs,
                Some(cancellation),
            )
            .await
        }
    }
}

async fn run_workflow_jobs(
    client: &dyn WorkflowRuntimeClient,
    registry: &LoadedWorkflowRegistry,
    spec: WorkflowRunSpec<'_>,
    phase_context: WorkflowPhaseContext<'_>,
    disabled_job_behavior: WorkflowDisabledJobBehavior,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<WorkflowJobRunResult>, WorkflowRunError> {
    let ordered = ordered_jobs_for_roots(registry, spec.root_jobs)
        .map_err(|error| WorkflowRunError::Failed(error.to_string()))?;
    let mut results = Vec::new();
    let mut completed = BTreeMap::<String, bool>::new();
    for job_name in ordered {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(WorkflowRunError::Cancelled);
        }
        let job = registry.jobs.get(&job_name).ok_or_else(|| {
            WorkflowRunError::Failed(format!("workflow job `{job_name}` does not exist"))
        })?;
        let should_run_disabled_job =
            matches!(
                disabled_job_behavior,
                WorkflowDisabledJobBehavior::RunRootJobs
            ) && spec.root_jobs.iter().any(|root_job| root_job == &job_name);
        if !job.config.enabled && !should_run_disabled_job {
            completed.insert(job_name, false);
            continue;
        }
        if job
            .config
            .needs
            .iter()
            .any(|dependency| completed.get(dependency) == Some(&false))
        {
            completed.insert(job.name.clone(), false);
            continue;
        }
        let result = run_workflow_job(
            client,
            spec.workflow_name,
            spec.trigger_id,
            job,
            phase_context,
            cancellation,
        )
        .await?;
        completed.insert(job.name.clone(), true);
        results.push(result);
    }
    if results.is_empty() {
        return Err(WorkflowRunError::Failed(format!(
            "workflow `{}/{}` did not run any enabled jobs",
            spec.workflow_name, spec.trigger_id
        )));
    }
    Ok(results)
}

async fn run_workflow_job(
    client: &dyn WorkflowRuntimeClient,
    workflow_name: &str,
    trigger_id: &str,
    job: &LoadedWorkflowJob,
    phase_context: WorkflowPhaseContext<'_>,
    cancellation: Option<&CancellationToken>,
) -> Result<WorkflowJobRunResult, WorkflowRunError> {
    if matches!(
        job.config.context_strategy,
        WorkflowContextStrategy::Embed | WorkflowContextStrategy::EmbedCompact
    ) {
        let prompt = job
            .config
            .steps
            .iter()
            .find_map(|step| match step {
                WorkflowStep::Prompt { prompt, .. } => Some(prompt.clone()),
                WorkflowStep::Run { .. } => None,
            })
            .ok_or_else(|| {
                WorkflowRunError::Failed(format!(
                    "workflow `{workflow_name}` job `{}` uses `context_strategy: {}` but has no prompt step",
                    job.name,
                    job.config.context_strategy.as_str()
                ))
            })?;
        return Ok(WorkflowJobRunResult {
            delivery: match job.config.context_strategy {
                WorkflowContextStrategy::Embed => WorkflowOutputDelivery::MainThreadInput,
                WorkflowContextStrategy::EmbedCompact => {
                    WorkflowOutputDelivery::MainThreadCompactInput
                }
                WorkflowContextStrategy::ThreadAuto
                | WorkflowContextStrategy::ThreadNew
                | WorkflowContextStrategy::ThreadFork
                | WorkflowContextStrategy::ThreadForkCompact => unreachable!(
                    "thread-based context strategy should not return main-thread input"
                ),
            },
            execution_strategy: job.config.execution_strategy,
            workflow_name: workflow_name.to_string(),
            trigger_id: trigger_id.to_string(),
            job_name: job.name.clone(),
            message: Some(prompt),
        });
    }

    let mut thread: Option<WorkflowThreadSession> = None;
    let mut step_outputs = Vec::new();
    let mut last_prompt_response = None;
    for step in &job.config.steps {
        let configured_attempts = step.retry_attempts();
        let step_timeout = step.timeout(WORKFLOW_STEP_TIMEOUT).map_err(|err| {
            WorkflowRunError::Failed(format!(
                "workflow `{workflow_name}` job `{}` has invalid {} step timeout: {err}",
                job.name,
                step.kind()
            ))
        })?;
        let mut attempt = 1;
        let mut used_capacity_retry = false;
        let mut used_timeout_retry = false;
        let step_error = loop {
            if cancellation.is_some_and(CancellationToken::is_cancelled) {
                if let Some(thread) = thread.as_ref() {
                    let _ = client.unsubscribe_thread(thread.thread_id.clone()).await;
                }
                return Err(WorkflowRunError::Cancelled);
            }
            let context = WorkflowStepExecutionContext {
                workflow_name,
                trigger_id,
                job,
                phase_context,
                cancellation,
            };
            let result = execute_workflow_step(
                client,
                &mut thread,
                context,
                step,
                step_timeout,
                &step_outputs,
            )
            .await;
            match result {
                Ok(Some(output)) => {
                    if matches!(step, WorkflowStep::Prompt { .. }) {
                        last_prompt_response = Some(output.clone());
                    }
                    step_outputs.push(output);
                    break None;
                }
                Ok(None) => {
                    break None;
                }
                Err(error) => {
                    let should_retry_capacity =
                        !used_capacity_retry && should_retry_selected_model_capacity_error(&error);
                    let should_retry_timeout =
                        !used_timeout_retry && should_retry_workflow_timeout(&error);
                    let should_retry = !matches!(error, WorkflowRunError::Cancelled)
                        && (attempt < configured_attempts
                            || should_retry_capacity
                            || should_retry_timeout);
                    if should_retry {
                        if attempt >= configured_attempts {
                            if should_retry_capacity {
                                used_capacity_retry = true;
                            }
                            if should_retry_timeout {
                                used_timeout_retry = true;
                            }
                        }
                        sleep(retry_backoff_delay(attempt)).await;
                        attempt = attempt.saturating_add(1);
                        continue;
                    }
                    break Some(error);
                }
            }
        };
        if let Some(error) = step_error {
            if let Some(thread) = thread.as_ref() {
                let _ = client.unsubscribe_thread(thread.thread_id.clone()).await;
            }
            return Err(error);
        }
    }

    if let Some(thread) = thread.as_ref() {
        let _ = client.unsubscribe_thread(thread.thread_id.clone()).await;
    }

    Ok(WorkflowJobRunResult {
        delivery: match job.config.response {
            WorkflowResponseMode::Assistant => WorkflowOutputDelivery::AssistantCell,
            WorkflowResponseMode::User => WorkflowOutputDelivery::UserFollowup,
        },
        execution_strategy: job.config.execution_strategy,
        workflow_name: workflow_name.to_string(),
        trigger_id: trigger_id.to_string(),
        job_name: job.name.clone(),
        message: last_prompt_response
            .or_else(|| (!step_outputs.is_empty()).then(|| step_outputs.join("\n\n"))),
    })
}

async fn execute_workflow_step(
    client: &dyn WorkflowRuntimeClient,
    thread: &mut Option<WorkflowThreadSession>,
    context: WorkflowStepExecutionContext<'_>,
    step: &WorkflowStep,
    step_timeout: Duration,
    step_outputs: &[String],
) -> Result<Option<String>, WorkflowRunError> {
    match step {
        WorkflowStep::Run { run, .. } => {
            run_workflow_command(
                run,
                &context.job.workflow_path,
                step_timeout,
                context.cancellation,
            )
            .await
        }
        WorkflowStep::Prompt { prompt, .. } => {
            let thread = match thread {
                Some(thread) => thread.clone(),
                None => {
                    let strategy = match context.job.config.context_strategy {
                        WorkflowContextStrategy::Embed | WorkflowContextStrategy::EmbedCompact => {
                            unreachable!("embed context strategies should have returned early")
                        }
                        WorkflowContextStrategy::ThreadAuto => WorkflowThreadStartStrategy::Auto,
                        WorkflowContextStrategy::ThreadNew => WorkflowThreadStartStrategy::New,
                        WorkflowContextStrategy::ThreadFork => WorkflowThreadStartStrategy::Fork,
                        WorkflowContextStrategy::ThreadForkCompact => {
                            WorkflowThreadStartStrategy::ForkCompact
                        }
                    };
                    let started = client
                        .start_workflow_thread(strategy, context.job.config.execution_strategy)
                        .await
                        .map_err(WorkflowRunError::Failed)?;
                    *thread = Some(started.clone());
                    started
                }
            };
            let prompt = build_workflow_prompt_input(
                context.workflow_name,
                context.trigger_id,
                &context.job.name,
                prompt,
                context.phase_context,
                step_outputs,
            );
            run_workflow_prompt(client, &thread, prompt, step_timeout, context.cancellation).await
        }
    }
}

async fn run_workflow_prompt(
    client: &dyn WorkflowRuntimeClient,
    thread: &WorkflowThreadSession,
    prompt: String,
    step_timeout: Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<String>, WorkflowRunError> {
    let turn_id = client
        .start_turn(thread, prompt)
        .await
        .map_err(WorkflowRunError::Failed)?;
    let deadline = tokio::time::Instant::now() + step_timeout;
    loop {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            interrupt_active_workflow_turn(client, thread, turn_id.clone()).await;
            return Err(WorkflowRunError::Cancelled);
        }
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            interrupt_active_workflow_turn(client, thread, turn_id.clone()).await;
            return Err(WorkflowRunError::TimedOut(format!(
                "workflow prompt timed out after {}",
                humantime::format_duration(step_timeout)
            )));
        };
        let turn = match tokio::time::timeout(remaining, client.read_turn(thread, turn_id.clone()))
            .await
        {
            Ok(turn) => turn.map_err(WorkflowRunError::Failed)?,
            Err(_) => {
                interrupt_active_workflow_turn(client, thread, turn_id.clone()).await;
                return Err(WorkflowRunError::TimedOut(format!(
                    "workflow prompt timed out after {}",
                    humantime::format_duration(step_timeout)
                )));
            }
        };
        match turn.status {
            TurnStatus::Completed => return Ok(turn.last_agent_message),
            TurnStatus::Interrupted => return Err(WorkflowRunError::Cancelled),
            TurnStatus::Failed => {
                return Err(WorkflowRunError::Failed(
                    turn.error
                        .unwrap_or_else(|| "workflow prompt turn failed".to_string()),
                ));
            }
            TurnStatus::InProgress => {
                if tokio::time::Instant::now() >= deadline {
                    interrupt_active_workflow_turn(client, thread, turn_id.clone()).await;
                    return Err(WorkflowRunError::TimedOut(format!(
                        "workflow prompt timed out after {}",
                        humantime::format_duration(step_timeout)
                    )));
                }
                sleep(WORKFLOW_POLL_INTERVAL).await;
            }
        }
    }
}

async fn interrupt_active_workflow_turn(
    client: &dyn WorkflowRuntimeClient,
    thread: &WorkflowThreadSession,
    turn_id: String,
) {
    let _ = client
        .interrupt_turn(thread.thread_id.clone(), turn_id.clone())
        .await;
    let deadline = tokio::time::Instant::now() + WORKFLOW_INTERRUPT_SETTLE_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        match client.read_turn(thread, turn_id.clone()).await {
            Ok(turn) if !matches!(turn.status, TurnStatus::InProgress) => return,
            Ok(_) | Err(_) => sleep(WORKFLOW_POLL_INTERVAL).await,
        }
    }
}

async fn run_workflow_command(
    command: &str,
    workflow_path: &std::path::Path,
    step_timeout: Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<String>, WorkflowRunError> {
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = Command::new("bash");
        cmd.arg("-lc").arg(command);
        cmd
    };
    cmd.kill_on_drop(true);
    let workflow_dir = workflow_path
        .parent()
        .and_then(|parent| parent.parent())
        .and_then(|parent| parent.parent())
        .unwrap_or(workflow_path);
    let child = cmd
        .current_dir(workflow_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            WorkflowRunError::Failed(format!("failed to run workflow command `{command}`: {err}"))
        })?;
    let wait_with_output = child.wait_with_output();
    tokio::pin!(wait_with_output);
    let output = tokio::select! {
        _ = async {
            if let Some(cancellation) = cancellation {
                cancellation.cancelled().await;
            } else {
                std::future::pending::<()>().await;
            }
        } => return Err(WorkflowRunError::Cancelled),
        output = tokio::time::timeout(step_timeout, &mut wait_with_output) => output
            .map_err(|_| {
                WorkflowRunError::TimedOut(format!(
                    "workflow command `{command}` timed out after {}",
                    humantime::format_duration(step_timeout)
                ))
            })?,
    }
    .map_err(|err| {
        WorkflowRunError::Failed(format!("failed to run workflow command `{command}`: {err}"))
    })?;
    let mut text = String::new();
    if !output.stdout.is_empty() {
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let text = text.trim().to_string();
    if !output.status.success() {
        return Err(WorkflowRunError::Failed(match text.is_empty() {
            true => format!(
                "workflow command `{command}` failed with status {}",
                output.status
            ),
            false => format!(
                "workflow command `{command}` failed with status {}: {text}",
                output.status
            ),
        }));
    }
    Ok((!text.is_empty()).then_some(text))
}

fn build_workflow_prompt_input(
    workflow_name: &str,
    trigger_id: &str,
    job_name: &str,
    prompt: &str,
    phase_context: WorkflowPhaseContext<'_>,
    step_outputs: &[String],
) -> String {
    let mut sections = vec![format!(
        "Workflow: {workflow_name}\nTrigger: {trigger_id}\nJob: {job_name}"
    )];
    if let Some(current_user_turn) = phase_context
        .current_user_turn
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        sections.push(format!(
            "Current main-thread user turn:\n{current_user_turn}"
        ));
    }
    if let Some(last_assistant_message) = phase_context
        .last_assistant_message
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        sections.push(format!(
            "Latest main-thread assistant response:\n{last_assistant_message}"
        ));
    }
    if !step_outputs.is_empty() {
        sections.push(format!(
            "Previous workflow step outputs:\n{}",
            step_outputs.join("\n\n")
        ));
    }
    sections.push(format!("Current workflow prompt:\n{prompt}"));
    sections.join("\n\n")
}

fn workflow_thread_start_params(
    execution_config: &WorkflowExecutionConfig,
    active_profile: Option<&str>,
    is_remote: bool,
    remote_cwd_override: Option<&std::path::Path>,
) -> ThreadStartParams {
    ThreadStartParams {
        model: Some(execution_config.model.clone()),
        model_provider: (!is_remote).then_some(execution_config.model_provider_id.clone()),
        service_tier: Some(
            execution_config
                .service_tier
                .map(|service_tier| service_tier.request_value().to_string()),
        ),
        cwd: workflow_thread_cwd(execution_config, is_remote, remote_cwd_override),
        approval_policy: Some(execution_config.approval_policy.into()),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(
            execution_config.approvals_reviewer,
        )),
        sandbox: sandbox_mode_from_policy(execution_config.sandbox_policy.clone()),
        permissions: None,
        config: active_profile.map(|profile| {
            HashMap::from([(
                "profile".to_string(),
                serde_json::Value::String(profile.to_string()),
            )])
        }),
        environments: None,
        ephemeral: Some(true),
        ..ThreadStartParams::default()
    }
}

fn workflow_thread_fork_params(
    execution_config: &WorkflowExecutionConfig,
    active_profile: Option<&str>,
    thread_id: ThreadId,
    is_remote: bool,
    remote_cwd_override: Option<&std::path::Path>,
) -> ThreadForkParams {
    ThreadForkParams {
        thread_id: thread_id.to_string(),
        model: Some(execution_config.model.clone()),
        model_provider: (!is_remote).then_some(execution_config.model_provider_id.clone()),
        service_tier: Some(
            execution_config
                .service_tier
                .map(|service_tier| service_tier.request_value().to_string()),
        ),
        cwd: workflow_thread_cwd(execution_config, is_remote, remote_cwd_override),
        approval_policy: Some(execution_config.approval_policy.into()),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(
            execution_config.approvals_reviewer,
        )),
        sandbox: sandbox_mode_from_policy(execution_config.sandbox_policy.clone()),
        permissions: None,
        config: active_profile.map(|profile| {
            HashMap::from([(
                "profile".to_string(),
                serde_json::Value::String(profile.to_string()),
            )])
        }),
        ephemeral: true,
        ..ThreadForkParams::default()
    }
}

fn workflow_thread_cwd(
    execution_config: &WorkflowExecutionConfig,
    is_remote: bool,
    remote_cwd_override: Option<&std::path::Path>,
) -> Option<String> {
    if is_remote {
        remote_cwd_override
            .map(|cwd| cwd.to_string_lossy().to_string())
            .or_else(|| Some(execution_config.cwd.to_string_lossy().to_string()))
    } else {
        Some(execution_config.cwd.to_string_lossy().to_string())
    }
}

fn sandbox_mode_from_policy(policy: SandboxPolicy) -> Option<SandboxMode> {
    match policy {
        SandboxPolicy::DangerFullAccess => Some(SandboxMode::DangerFullAccess),
        SandboxPolicy::ReadOnly { .. } => Some(SandboxMode::ReadOnly),
        SandboxPolicy::WorkspaceWrite { .. } => Some(SandboxMode::WorkspaceWrite),
        SandboxPolicy::ExternalSandbox { .. } => None,
    }
}

fn request_id() -> RequestId {
    RequestId::String(format!("workflow-{}", Uuid::new_v4()))
}

fn manual_workflow_job_trigger_id(job_name: &str) -> String {
    format!("job:{job_name}")
}

fn workflow_job_source(result: &WorkflowJobRunResult) -> String {
    format!(
        "{}/{}:{}",
        result.workflow_name, result.trigger_id, result.job_name
    )
}

fn retry_backoff_delay(attempt: u32) -> Duration {
    let seconds = 2u64.saturating_pow(attempt.saturating_sub(1)).min(8);
    Duration::from_secs(seconds.max(1))
}

fn should_retry_selected_model_capacity_error(error: &WorkflowRunError) -> bool {
    matches!(
        error,
        WorkflowRunError::Failed(message)
            if message.contains("Selected model is at capacity. Please try a different model.")
    )
}

fn should_retry_workflow_timeout(error: &WorkflowRunError) -> bool {
    matches!(error, WorkflowRunError::TimedOut(_))
}

fn workflow_run_error_message(error: WorkflowRunError) -> String {
    match error {
        WorkflowRunError::Failed(message) | WorkflowRunError::TimedOut(message) => message,
        WorkflowRunError::Cancelled => "workflow run cancelled".to_string(),
    }
}

fn update_last_workflow_agent_message(
    last_agent_message: &mut Option<String>,
    notification: &ItemCompletedNotification,
) {
    if let ThreadItem::AgentMessage { text, .. } = &notification.item {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            *last_agent_message = Some(trimmed.to_string());
        }
    }
}

fn last_agent_message_for_turn_items(items: &[ThreadItem]) -> Option<String> {
    items.iter().fold(None, |_, item| match item {
        ThreadItem::AgentMessage { text, .. } => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    })
}

#[derive(Debug)]
enum WorkflowRunError {
    Failed(String),
    TimedOut(String),
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_core::config::ConfigBuilder;
    use codex_app_server_protocol::TurnCompletedNotification;
    use pretty_assertions::assert_eq;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use tempfile::TempDir;
    use tempfile::tempdir;
    use tokio::time;

    async fn build_config(temp_dir: &TempDir) -> Config {
        ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config should build")
    }

    fn test_workflow_execution_config() -> WorkflowExecutionConfig {
        WorkflowExecutionConfig {
            model: "gpt-test".to_string(),
            model_provider_id: "test-provider".to_string(),
            service_tier: None,
            approval_policy: AskForApproval::OnRequest,
            approvals_reviewer: codex_protocol::config_types::ApprovalsReviewer::User,
            sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
            cwd: PathBuf::from("/tmp/workflow"),
            reasoning_effort: None,
        }
    }

    #[tokio::test]
    async fn override_yolo_execution_strategy_overrides_session_permissions() {
        let temp_dir = tempdir().expect("tempdir");
        let config = build_config(&temp_dir).await;
        let mut app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let started = app_server
            .start_thread(&config)
            .await
            .expect("start primary thread");
        let session = started.session;
        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            config,
            Some(session.thread_id),
            Some(session.clone()),
        );

        let execution_config = client
            .resolve_execution_config(WorkflowExecutionStrategy::OverrideYolo)
            .expect("resolve execution config");

        assert_eq!(execution_config.cwd, session.cwd.to_path_buf());
        assert_eq!(execution_config.model, session.model);
        assert_eq!(execution_config.approval_policy, AskForApproval::Never);
        assert_eq!(
            execution_config.approvals_reviewer,
            codex_protocol::config_types::ApprovalsReviewer::User
        );
        assert_eq!(
            execution_config.sandbox_policy,
            SandboxPolicy::DangerFullAccess
        );
    }

    struct FakeWorkflowRuntimeClient {
        calls: Mutex<Vec<String>>,
        thread_id: String,
        turn_id: String,
        reads: Mutex<VecDeque<Result<WorkflowTurnState, String>>>,
    }

    impl FakeWorkflowRuntimeClient {
        fn new(reads: Vec<Result<WorkflowTurnState, String>>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                thread_id: "thr_workflow".to_string(),
                turn_id: "turn_workflow".to_string(),
                reads: Mutex::new(reads.into()),
            }
        }
    }

    impl WorkflowRuntimeClient for FakeWorkflowRuntimeClient {
        fn start_workflow_thread(
            &self,
            strategy: WorkflowThreadStartStrategy,
            execution_strategy: WorkflowExecutionStrategy,
        ) -> BoxFuture<'_, Result<WorkflowThreadSession, String>> {
            Box::pin(async move {
                let (_sender, receiver) = mpsc::unbounded_channel();
                self.calls.lock().expect("calls lock").push(format!(
                    "start_workflow_thread:{}:{}",
                    strategy.as_str(),
                    execution_strategy.as_str()
                ));
                Ok(WorkflowThreadSession {
                    thread_id: self.thread_id.clone(),
                    cwd: PathBuf::from("/tmp/workflow"),
                    execution_config: WorkflowExecutionConfig {
                        model: "gpt-test".to_string(),
                        model_provider_id: "test".to_string(),
                        service_tier: None,
                        approval_policy: AskForApproval::OnRequest,
                        approvals_reviewer: codex_protocol::config_types::ApprovalsReviewer::User,
                        sandbox_policy: SandboxPolicy::new_workspace_write_policy(),
                        cwd: PathBuf::from("/tmp/workflow"),
                        reasoning_effort: None,
                    },
                    notifications: Arc::new(tokio::sync::Mutex::new(receiver)),
                })
            })
        }

        fn start_turn(
            &self,
            thread: &WorkflowThreadSession,
            input: String,
        ) -> BoxFuture<'_, Result<String, String>> {
            let thread_id = thread.thread_id.clone();
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("start_turn:{thread_id}:{input}"));
                Ok(self.turn_id.clone())
            })
        }

        fn read_turn<'a>(
            &'a self,
            thread: &'a WorkflowThreadSession,
            turn_id: String,
        ) -> BoxFuture<'a, Result<WorkflowTurnState, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("read_turn:{}:{turn_id}", thread.thread_id));
                self.reads
                    .lock()
                    .expect("reads lock")
                    .pop_front()
                    .unwrap_or({
                        Ok(WorkflowTurnState {
                            status: TurnStatus::InProgress,
                            error: None,
                            last_agent_message: None,
                        })
                    })
            })
        }

        fn interrupt_turn(
            &self,
            thread_id: String,
            turn_id: String,
        ) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("interrupt_turn:{thread_id}:{turn_id}"));
                Ok(())
            })
        }

        fn unsubscribe_thread(&self, thread_id: String) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("unsubscribe_thread:{thread_id}"));
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn prompt_workflow_job_uses_app_server_runtime_sequence() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: manual
    bind_thread: all
    jobs: [review_backlog]

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![
            Ok(WorkflowTurnState {
                status: TurnStatus::InProgress,
                error: None,
                last_agent_message: None,
            }),
            Ok(WorkflowTurnState {
                status: TurnStatus::Completed,
                error: None,
                last_agent_message: Some("workflow reply".to_string()),
            }),
        ]);
        let result = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Job {
                workflow_name: "director".to_string(),
                job_name: "review_backlog".to_string(),
            },
            CancellationToken::new(),
        )
        .await;

        match result.outcome {
            BackgroundWorkflowRunOutcome::Completed(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].message.as_deref(), Some("workflow reply"));
            }
            other => panic!("expected completed run, got {other:?}"),
        }
        assert_eq!(
            client.calls.lock().expect("calls lock").clone(),
            vec![
                "start_workflow_thread:auto:inherit_session".to_string(),
                "start_turn:thr_workflow:Workflow: director\nTrigger: job:review_backlog\nJob: review_backlog\n\nCurrent workflow prompt:\nsummarize the backlog".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "unsubscribe_thread:thr_workflow".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn prompt_workflow_job_retries_selected_model_capacity_once() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![
            Ok(WorkflowTurnState {
                status: TurnStatus::Failed,
                error: Some(
                    "Selected model is at capacity. Please try a different model.".to_string(),
                ),
                last_agent_message: None,
            }),
            Ok(WorkflowTurnState {
                status: TurnStatus::Completed,
                error: None,
                last_agent_message: Some("workflow reply".to_string()),
            }),
        ]);
        let result = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Job {
                workflow_name: "director".to_string(),
                job_name: "review_backlog".to_string(),
            },
            CancellationToken::new(),
        )
        .await;

        match result.outcome {
            BackgroundWorkflowRunOutcome::Completed(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].message.as_deref(), Some("workflow reply"));
            }
            other => panic!("expected completed run, got {other:?}"),
        }
        assert_eq!(
            client.calls.lock().expect("calls lock").clone(),
            vec![
                "start_workflow_thread:auto:inherit_session".to_string(),
                "start_turn:thr_workflow:Workflow: director\nTrigger: job:review_backlog\nJob: review_backlog\n\nCurrent workflow prompt:\nsummarize the backlog".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "start_turn:thr_workflow:Workflow: director\nTrigger: job:review_backlog\nJob: review_backlog\n\nCurrent workflow prompt:\nsummarize the backlog".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "unsubscribe_thread:thr_workflow".to_string(),
            ]
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn prompt_workflow_job_uses_configured_timeout_for_retry() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the backlog
        timeout: 1s
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(Vec::new());
        let run = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Job {
                workflow_name: "director".to_string(),
                job_name: "review_backlog".to_string(),
            },
            CancellationToken::new(),
        );
        tokio::pin!(run);

        let configured_timeout = Duration::from_secs(1);
        tokio::task::yield_now().await;
        time::advance(
            configured_timeout
                + WORKFLOW_INTERRUPT_SETTLE_TIMEOUT
                + retry_backoff_delay(1)
                + configured_timeout
                + WORKFLOW_INTERRUPT_SETTLE_TIMEOUT
                + Duration::from_secs(1),
        )
        .await;

        let result = run.await;
        match result.outcome {
            BackgroundWorkflowRunOutcome::Failed(error) => {
                assert_eq!(error, "workflow prompt timed out after 1s".to_string())
            }
            other => panic!("expected failed timeout run, got {other:?}"),
        }

        let calls = client.calls.lock().expect("calls lock").clone();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("start_turn:"))
                .count(),
            2
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("interrupt_turn:"))
                .count(),
            2
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("unsubscribe_thread:"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn non_manual_trigger_can_run_now_from_workflow_ui() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: after_turn
    id: follow_up
    bind_thread: all
    jobs: [review_backlog]

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![Ok(WorkflowTurnState {
            status: TurnStatus::Completed,
            error: None,
            last_agent_message: Some("workflow reply".to_string()),
        })]);
        let result = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Trigger {
                workflow_name: "director".to_string(),
                trigger_id: "follow_up".to_string(),
                phase_context: OwnedWorkflowPhaseContext::default(),
                overlap_behavior: WorkflowTriggerOverlapBehavior::Queue,
            },
            CancellationToken::new(),
        )
        .await;

        match result.outcome {
            BackgroundWorkflowRunOutcome::Completed(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].message.as_deref(), Some("workflow reply"));
            }
            other => panic!("expected completed run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn manual_job_run_ignores_disabled_root_flag() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    enabled: false
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![Ok(WorkflowTurnState {
            status: TurnStatus::Completed,
            error: None,
            last_agent_message: Some("workflow reply".to_string()),
        })]);
        let result = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Job {
                workflow_name: "director".to_string(),
                job_name: "review_backlog".to_string(),
            },
            CancellationToken::new(),
        )
        .await;

        match result.outcome {
            BackgroundWorkflowRunOutcome::Completed(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].message.as_deref(), Some("workflow reply"));
            }
            other => panic!("expected completed run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn trigger_run_fails_when_all_target_jobs_are_disabled() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: after_turn
    id: follow_up
    bind_thread: all
    jobs: [review_backlog]

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    enabled: false
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(Vec::new());
        let result = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Trigger {
                workflow_name: "director".to_string(),
                trigger_id: "follow_up".to_string(),
                phase_context: OwnedWorkflowPhaseContext::default(),
                overlap_behavior: WorkflowTriggerOverlapBehavior::Queue,
            },
            CancellationToken::new(),
        )
        .await;

        match result.outcome {
            BackgroundWorkflowRunOutcome::Failed(error) => assert_eq!(
                error,
                "workflow `director/follow_up` did not run any enabled jobs".to_string()
            ),
            other => panic!("expected failed run, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_active_workflow_turn() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

jobs:
  review_backlog:
    context_strategy: thread_auto
    execution_strategy: inherit_session
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![
            Ok(WorkflowTurnState {
                status: TurnStatus::InProgress,
                error: None,
                last_agent_message: None,
            }),
            Ok(WorkflowTurnState {
                status: TurnStatus::Interrupted,
                error: None,
                last_agent_message: None,
            }),
        ]);
        let cancellation = CancellationToken::new();
        let run = run_background_workflow(
            &client,
            tempdir.path().to_path_buf(),
            BackgroundWorkflowRunTarget::Job {
                workflow_name: "director".to_string(),
                job_name: "review_backlog".to_string(),
            },
            cancellation.clone(),
        );
        tokio::pin!(run);
        let result = tokio::select! {
            result = &mut run => result,
            _ = sleep(Duration::from_millis(10)) => {
                cancellation.cancel();
                run.await
            }
        };

        assert!(matches!(
            result.outcome,
            BackgroundWorkflowRunOutcome::Cancelled
        ));
        assert_eq!(
            client.calls.lock().expect("calls lock").clone(),
            vec![
                "start_workflow_thread:auto:inherit_session".to_string(),
                "start_turn:thr_workflow:Workflow: director\nTrigger: job:review_backlog\nJob: review_backlog\n\nCurrent workflow prompt:\nsummarize the backlog".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "interrupt_turn:thr_workflow:turn_workflow".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "unsubscribe_thread:thr_workflow".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn read_turn_uses_forwarded_notifications_for_ephemeral_threads() {
        let temp_dir = tempdir().expect("tempdir");
        let config = build_config(&temp_dir).await;
        let app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            config,
            /*primary_thread_id*/ None,
            /*primary_session_configured*/ None,
        );
        let (sender, receiver) = mpsc::unbounded_channel();
        let thread = WorkflowThreadSession {
            thread_id: "thr_workflow".to_string(),
            cwd: PathBuf::from("/tmp/workflow"),
            execution_config: test_workflow_execution_config(),
            notifications: Arc::new(tokio::sync::Mutex::new(receiver)),
        };

        sender
            .send(ServerNotification::ItemCompleted(
                ItemCompletedNotification {
                    item: ThreadItem::AgentMessage {
                        id: "msg-1".to_string(),
                        text: "workflow reply".to_string(),
                        phase: None,
                        memory_citation: None,
                    },
                    thread_id: thread.thread_id.clone(),
                    turn_id: "turn-1".to_string(),
                    completed_at_ms: 0,
                },
            ))
            .expect("item completed notification");
        sender
            .send(ServerNotification::TurnCompleted(
                TurnCompletedNotification {
                    thread_id: thread.thread_id.clone(),
                    turn: codex_app_server_protocol::Turn {
                        id: "turn-1".to_string(),
                        items: Vec::new(),
                        items_view: Default::default(),
                        error: None,
                        status: TurnStatus::Completed,
                        started_at: None,
                        completed_at: None,
                        duration_ms: None,
                    },
                },
            ))
            .expect("turn completed notification");

        let state = client
            .read_turn(&thread, "turn-1".to_string())
            .await
            .expect("read workflow turn");

        assert_eq!(
            state,
            WorkflowTurnState {
                status: TurnStatus::Completed,
                error: None,
                last_agent_message: Some("workflow reply".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn read_turn_retries_include_turns_errors_for_ephemeral_threads() {
        let temp_dir = tempdir().expect("tempdir");
        let config = build_config(&temp_dir).await;
        let workflow_thread_notification_channels =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let mut app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let started = app_server
            .start_thread(&config)
            .await
            .expect("start primary thread");
        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            workflow_thread_notification_channels.clone(),
            config,
            /*primary_thread_id*/ None,
            Some(started.session),
        );
        let thread = client
            .start_workflow_thread(
                WorkflowThreadStartStrategy::Auto,
                WorkflowExecutionStrategy::InheritSession,
            )
            .await
            .expect("start workflow thread");

        let sender = workflow_thread_notification_channels
            .lock()
            .await
            .get(&ThreadId::from_string(&thread.thread_id).expect("workflow thread id"))
            .cloned()
            .expect("workflow notification sender");

        let read_turn = {
            let client = client;
            let thread = thread.clone();
            tokio::spawn(async move { client.read_turn(&thread, "turn-1".to_string()).await })
        };

        sleep(Duration::from_millis(10)).await;
        sender
            .send(ServerNotification::ItemCompleted(
                ItemCompletedNotification {
                    item: ThreadItem::AgentMessage {
                        id: "msg-1".to_string(),
                        text: "workflow reply".to_string(),
                        phase: None,
                        memory_citation: None,
                    },
                    thread_id: thread.thread_id.clone(),
                    turn_id: "turn-1".to_string(),
                    completed_at_ms: 0,
                },
            ))
            .expect("item completed notification");
        sender
            .send(ServerNotification::TurnCompleted(
                TurnCompletedNotification {
                    thread_id: thread.thread_id.clone(),
                    turn: codex_app_server_protocol::Turn {
                        id: "turn-1".to_string(),
                        items: Vec::new(),
                        items_view: Default::default(),
                        error: None,
                        status: TurnStatus::Completed,
                        started_at: None,
                        completed_at: None,
                        duration_ms: None,
                    },
                },
            ))
            .expect("turn completed notification");

        let state = read_turn
            .await
            .expect("join read_turn task")
            .expect("read workflow turn");
        assert_eq!(
            state,
            WorkflowTurnState {
                status: TurnStatus::Completed,
                error: None,
                last_agent_message: Some("workflow reply".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn start_workflow_thread_starts_fresh_thread_when_no_primary_thread_is_available() {
        let temp_dir = tempdir().expect("tempdir");
        let config = build_config(&temp_dir).await;
        let mut app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let started = app_server
            .start_thread(&config)
            .await
            .expect("start primary thread");
        let workflow_thread_notification_channels =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            workflow_thread_notification_channels.clone(),
            config,
            None,
            Some(started.session),
        );
        let workflow_thread = client
            .start_workflow_thread(
                WorkflowThreadStartStrategy::Auto,
                WorkflowExecutionStrategy::InheritSession,
            )
            .await
            .expect("start workflow thread");

        let registered_thread_ids = workflow_thread_notification_channels
            .lock()
            .await
            .keys()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(registered_thread_ids.len(), 1);
        assert_eq!(
            registered_thread_ids[0].to_string(),
            workflow_thread.thread_id
        );
    }

    #[tokio::test]
    async fn start_workflow_thread_rejects_missing_primary_session_for_inherit_session() {
        let temp_dir = tempdir().expect("tempdir");
        let config = build_config(&temp_dir).await;
        let app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            config,
            None,
            None,
        );

        let error = match client
            .start_workflow_thread(
                WorkflowThreadStartStrategy::Auto,
                WorkflowExecutionStrategy::InheritSession,
            )
            .await
        {
            Ok(_) => panic!("missing session should fail"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            "workflow execution strategy `inherit_session` requires a current primary session"
        );
    }
}
