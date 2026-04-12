use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::App;
use super::workflow_definition::WorkflowTriggerKindDiscriminant;
use super::workflow_definition::load_workflow_registry_for_ui;
use super::workflow_history::workflow_result_cell;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ApprovalsReviewer as AppServerApprovalsReviewer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ServerNotification;
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
use codex_core::config::Config;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use codex_workflow::history::WorkflowReplySource;
use codex_workflow::history::workflow_job_source;
pub(crate) use codex_workflow::runtime::BackgroundWorkflowRunOutcome;
pub(crate) use codex_workflow::runtime::BackgroundWorkflowRunResult;
pub(crate) use codex_workflow::runtime::BackgroundWorkflowRunTarget;
use codex_workflow::runtime::BoxFuture;
pub(crate) use codex_workflow::runtime::OwnedWorkflowPhaseContext;
pub(crate) use codex_workflow::runtime::WorkflowJobRunResult;
pub(crate) use codex_workflow::runtime::WorkflowOutputDelivery;
pub(crate) use codex_workflow::runtime::WorkflowPhaseContext;
use codex_workflow::runtime::WorkflowRuntimeClient;
pub(crate) use codex_workflow::runtime::WorkflowTriggerOverlapBehavior;
use codex_workflow::runtime::WorkflowTurnState;
use codex_workflow::runtime::WorkflowTurnStatus;
use codex_workflow::runtime::run_background_workflow as run_shared_background_workflow;
use codex_workflow::runtime::run_before_turn_workflows as run_shared_before_turn_workflows;
use codex_workflow::runtime::workflow_run_error_message;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const WORKFLOW_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) type WorkflowThreadNotificationChannels =
    Arc<tokio::sync::Mutex<HashMap<ThreadId, mpsc::UnboundedSender<ServerNotification>>>>;

#[derive(Clone)]
pub(crate) struct WorkflowThreadSession {
    thread_id: String,
    cwd: PathBuf,
    notifications: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ServerNotification>>>,
}

pub(crate) struct AppServerWorkflowRuntimeClient {
    request_handle: AppServerRequestHandle,
    workflow_thread_notification_channels: WorkflowThreadNotificationChannels,
    config: Config,
    primary_thread_id: Option<ThreadId>,
    is_remote: bool,
    remote_cwd_override: Option<PathBuf>,
}

impl AppServerWorkflowRuntimeClient {
    pub(crate) fn new(
        app_server: &AppServerSession,
        workflow_thread_notification_channels: WorkflowThreadNotificationChannels,
        config: Config,
        primary_thread_id: Option<ThreadId>,
    ) -> Self {
        Self {
            request_handle: app_server.request_handle(),
            workflow_thread_notification_channels,
            config,
            primary_thread_id,
            is_remote: app_server.is_remote(),
            remote_cwd_override: app_server.remote_cwd_override().map(PathBuf::from),
        }
    }
}

impl WorkflowRuntimeClient for AppServerWorkflowRuntimeClient {
    type Thread = WorkflowThreadSession;

    fn start_workflow_thread(&self) -> BoxFuture<'_, Result<Self::Thread, String>> {
        Box::pin(async move {
            let fork_source_thread_id = if let Some(primary_thread_id) = self.primary_thread_id {
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
                response
                    .thread
                    .path
                    .as_ref()
                    .is_some_and(|path| path.exists())
                    .then_some(primary_thread_id)
            } else {
                None
            };

            if let Some(primary_thread_id) = fork_source_thread_id {
                let response: ThreadForkResponse = self
                    .request_handle
                    .request_typed(ClientRequest::ThreadFork {
                        request_id: request_id(),
                        params: workflow_thread_fork_params(
                            &self.config,
                            primary_thread_id,
                            self.is_remote,
                            self.remote_cwd_override.as_deref(),
                        ),
                    })
                    .await
                    .map_err(|err| format!("failed to fork workflow thread: {err}"))?;
                let thread_id = ThreadId::from_string(&response.thread.id).map_err(|err| {
                    format!(
                        "workflow thread id `{}` is invalid: {err}",
                        response.thread.id
                    )
                })?;
                let (sender, receiver) = mpsc::unbounded_channel();
                self.workflow_thread_notification_channels
                    .lock()
                    .await
                    .insert(thread_id, sender);
                return Ok(WorkflowThreadSession {
                    thread_id: response.thread.id,
                    cwd: response.cwd,
                    notifications: Arc::new(tokio::sync::Mutex::new(receiver)),
                });
            }

            let response: ThreadStartResponse = self
                .request_handle
                .request_typed(ClientRequest::ThreadStart {
                    request_id: request_id(),
                    params: workflow_thread_start_params(
                        &self.config,
                        self.is_remote,
                        self.remote_cwd_override.as_deref(),
                    ),
                })
                .await
                .map_err(|err| format!("failed to start workflow thread: {err}"))?;
            let thread_id = ThreadId::from_string(&response.thread.id).map_err(|err| {
                format!(
                    "workflow thread id `{}` is invalid: {err}",
                    response.thread.id
                )
            })?;
            let (sender, receiver) = mpsc::unbounded_channel();
            self.workflow_thread_notification_channels
                .lock()
                .await
                .insert(thread_id, sender);
            Ok(WorkflowThreadSession {
                thread_id: response.thread.id,
                cwd: response.cwd,
                notifications: Arc::new(tokio::sync::Mutex::new(receiver)),
            })
        })
    }

    fn start_turn<'a>(
        &'a self,
        thread: &'a Self::Thread,
        input: String,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let response: TurnStartResponse = self
                .request_handle
                .request_typed(ClientRequest::TurnStart {
                    request_id: request_id(),
                    params: TurnStartParams {
                        thread_id: thread.thread_id.clone(),
                        input: vec![
                            UserInput::Text {
                                text: input,
                                text_elements: Vec::new(),
                            }
                            .into(),
                        ],
                        cwd: Some(thread.cwd.clone()),
                        approval_policy: Some(
                            self.config.permissions.approval_policy.value().into(),
                        ),
                        approvals_reviewer: Some(self.config.approvals_reviewer.into()),
                        sandbox_policy: Some(
                            self.config.permissions.sandbox_policy.get().clone().into(),
                        ),
                        model: self.config.model.clone(),
                        service_tier: None,
                        effort: self.config.model_reasoning_effort,
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
        thread: &'a Self::Thread,
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
                                return Ok(WorkflowTurnState {
                                    status: workflow_turn_status_from_app_server(
                                        notification.turn.status.clone(),
                                    ),
                                    error: notification
                                        .turn
                                        .error
                                        .clone()
                                        .map(|error| error.message),
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
                let status = workflow_turn_status_from_app_server(turn.status);
                if !matches!(status, WorkflowTurnStatus::InProgress) {
                    return Ok(WorkflowTurnState {
                        status,
                        error: turn.error.map(|error| error.message),
                        last_agent_message,
                    });
                }
                sleep(WORKFLOW_POLL_INTERVAL).await;
            }
        })
    }

    fn interrupt_turn<'a>(
        &'a self,
        thread: &'a Self::Thread,
        turn_id: String,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let _: TurnInterruptResponse = self
                .request_handle
                .request_typed(ClientRequest::TurnInterrupt {
                    request_id: request_id(),
                    params: TurnInterruptParams {
                        thread_id: thread.thread_id.clone(),
                        turn_id,
                    },
                })
                .await
                .map_err(|err| format!("failed to interrupt workflow turn: {err}"))?;
            Ok(())
        })
    }

    fn unsubscribe_thread<'a>(
        &'a self,
        thread: &'a Self::Thread,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let result: Result<ThreadUnsubscribeResponse, String> = self
                .request_handle
                .request_typed(ClientRequest::ThreadUnsubscribe {
                    request_id: request_id(),
                    params: ThreadUnsubscribeParams {
                        thread_id: thread.thread_id.clone(),
                    },
                })
                .await
                .map_err(|err| format!("failed to unsubscribe workflow thread: {err}"));
            if let Ok(parsed_thread_id) = ThreadId::from_string(&thread.thread_id) {
                self.workflow_thread_notification_channels
                    .lock()
                    .await
                    .remove(&parsed_thread_id);
            }
            result.map(|_| ())
        })
    }
}

#[allow(dead_code)]
impl App {
    pub(crate) async fn run_before_turn_workflows(
        &self,
        app_server: &AppServerSession,
        phase_context: WorkflowPhaseContext<'_>,
    ) -> Result<Vec<WorkflowJobRunResult>, String> {
        let registry = load_workflow_registry_for_ui(self.config.cwd.as_path())
            .map_err(|error| format!("failed to load workflows: {error}"))?;
        let client = AppServerWorkflowRuntimeClient::new(
            app_server,
            self.workflow.thread_notification_channels.clone(),
            self.config.clone(),
            self.primary_thread_id,
        );
        run_shared_before_turn_workflows(&client, &registry, phase_context)
            .await
            .map_err(workflow_run_error_message)
    }

    pub(crate) fn start_manual_workflow_trigger_run(
        &mut self,
        app_server: &AppServerSession,
        workflow_name: String,
        trigger_id: String,
    ) -> Arc<dyn HistoryCell> {
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

        let registry = match load_workflow_registry_for_ui(self.config.cwd.as_path()) {
            Ok(registry) => registry,
            Err(error) => {
                self.chat_widget.add_error_message(format!(
                    "Workflow file_watch failed: failed to load workflows: {error}"
                ));
                return Vec::new();
            }
        };

        let mut visible_cells = Vec::new();
        for (workflow, trigger) in
            registry.iter_matching_triggers(WorkflowTriggerKindDiscriminant::FileWatch)
        {
            if !trigger.enabled {
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
        visible_cells
    }

    pub(crate) fn dispatch_next_queued_trigger_run(&mut self, app_server: &AppServerSession) {
        if self.workflow.scheduler.has_running_trigger_run() {
            return;
        }
        let Some(next) = self.workflow.scheduler.dequeue_trigger_run() else {
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
            .workflow
            .scheduler
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
                            WorkflowOutputDelivery::MainThreadInput
                            | WorkflowOutputDelivery::UserFollowup => {
                                if let Some(cell) =
                                    self.queue_workflow_followup_to_primary(message, source)
                                {
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
                .workflow
                .scheduler
                .has_active_trigger_run(&workflow_name, &trigger_id)
                || self
                    .workflow
                    .scheduler
                    .has_queued_trigger_run(&workflow_name, &trigger_id))
        {
            return TriggerRunDispatch::Skipped;
        }

        if self.workflow.scheduler.has_running_trigger_run() {
            self.workflow
                .scheduler
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
            .workflow
            .scheduler
            .next_background_run_id(target.workflow_name(), target.slot_key());
        let runtime_client = AppServerWorkflowRuntimeClient::new(
            app_server,
            self.workflow.thread_notification_channels.clone(),
            self.config.clone(),
            self.primary_thread_id,
        );
        let workflow_cwd = self.config.cwd.to_path_buf();
        let app_event_tx = self.app_event_tx.clone();
        let run_id_for_task = run_id.clone();
        let target_for_task = target.clone();
        let cancellation = CancellationToken::new();
        let cancellation_for_task = cancellation.clone();
        let handle = tokio::spawn(async move {
            let result = run_shared_background_workflow(
                &runtime_client,
                workflow_cwd,
                target_for_task,
                cancellation_for_task,
            )
            .await;
            app_event_tx.send(AppEvent::BackgroundWorkflowRunCompleted {
                run_id: run_id_for_task,
                result: Box::new(result),
            });
        });
        self.workflow.scheduler.register_background_workflow_run(
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

fn workflow_thread_start_params(
    config: &Config,
    is_remote: bool,
    remote_cwd_override: Option<&Path>,
) -> ThreadStartParams {
    ThreadStartParams {
        model: config.model.clone(),
        model_provider: (!is_remote).then_some(config.model_provider_id.clone()),
        cwd: workflow_thread_cwd(config, is_remote, remote_cwd_override),
        approval_policy: Some(config.permissions.approval_policy.value().into()),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(config.approvals_reviewer)),
        sandbox: sandbox_mode_from_policy(config.permissions.sandbox_policy.get().clone()),
        config: config.active_profile.as_ref().map(|profile| {
            HashMap::from([(
                "profile".to_string(),
                serde_json::Value::String(profile.clone()),
            )])
        }),
        ephemeral: Some(true),
        persist_extended_history: true,
        ..ThreadStartParams::default()
    }
}

fn workflow_thread_fork_params(
    config: &Config,
    thread_id: ThreadId,
    is_remote: bool,
    remote_cwd_override: Option<&Path>,
) -> ThreadForkParams {
    ThreadForkParams {
        thread_id: thread_id.to_string(),
        model: config.model.clone(),
        model_provider: (!is_remote).then_some(config.model_provider_id.clone()),
        cwd: workflow_thread_cwd(config, is_remote, remote_cwd_override),
        approval_policy: Some(config.permissions.approval_policy.value().into()),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(config.approvals_reviewer)),
        sandbox: sandbox_mode_from_policy(config.permissions.sandbox_policy.get().clone()),
        config: config.active_profile.as_ref().map(|profile| {
            HashMap::from([(
                "profile".to_string(),
                serde_json::Value::String(profile.clone()),
            )])
        }),
        ephemeral: true,
        persist_extended_history: true,
        ..ThreadForkParams::default()
    }
}

fn workflow_thread_cwd(
    config: &Config,
    is_remote: bool,
    remote_cwd_override: Option<&Path>,
) -> Option<String> {
    if is_remote {
        remote_cwd_override.map(|cwd| cwd.to_string_lossy().to_string())
    } else {
        Some(config.cwd.to_string_lossy().to_string())
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

fn workflow_turn_status_from_app_server(status: TurnStatus) -> WorkflowTurnStatus {
    match status {
        TurnStatus::InProgress => WorkflowTurnStatus::InProgress,
        TurnStatus::Completed => WorkflowTurnStatus::Completed,
        TurnStatus::Interrupted => WorkflowTurnStatus::Interrupted,
        TurnStatus::Failed => WorkflowTurnStatus::Failed,
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use codex_app_server_protocol::TurnCompletedNotification;
    use codex_core::config::ConfigBuilder;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::*;

    async fn build_config(temp_dir: &TempDir) -> Config {
        ConfigBuilder::default()
            .codex_home(temp_dir.path().to_path_buf())
            .build()
            .await
            .expect("config should build")
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
        );
        let (sender, receiver) = mpsc::unbounded_channel();
        let thread = WorkflowThreadSession {
            thread_id: "thr_workflow".to_string(),
            cwd: PathBuf::from("/tmp/workflow"),
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
                        error: None,
                        status: TurnStatus::Completed,
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
                status: WorkflowTurnStatus::Completed,
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
        let app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            workflow_thread_notification_channels.clone(),
            config,
            /*primary_thread_id*/ None,
        );
        let thread = client
            .start_workflow_thread()
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
                        error: None,
                        status: TurnStatus::Completed,
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
                status: WorkflowTurnStatus::Completed,
                error: None,
                last_agent_message: Some("workflow reply".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn start_workflow_thread_starts_fresh_thread_when_primary_thread_is_unmaterialized() {
        let temp_dir = tempdir().expect("tempdir");
        let config = build_config(&temp_dir).await;
        let mut app_server = crate::start_embedded_app_server_for_picker(&config)
            .await
            .expect("embedded app server");
        let primary = app_server
            .start_thread(&config)
            .await
            .expect("start primary thread");
        assert!(
            primary
                .session
                .rollout_path
                .as_ref()
                .is_some_and(|path| !path.exists())
        );
        let workflow_thread_notification_channels =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let client = AppServerWorkflowRuntimeClient::new(
            &app_server,
            workflow_thread_notification_channels.clone(),
            config,
            Some(primary.session.thread_id),
        );
        let workflow_thread = client
            .start_workflow_thread()
            .await
            .expect("start workflow thread");

        assert_ne!(
            workflow_thread.thread_id,
            primary.session.thread_id.to_string()
        );
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
}
