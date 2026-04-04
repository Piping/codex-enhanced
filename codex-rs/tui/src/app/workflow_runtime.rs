use super::App;
use super::workflow_definition::LoadedWorkflowJob;
use super::workflow_definition::LoadedWorkflowRegistry;
use super::workflow_definition::WorkflowContextMode;
use super::workflow_definition::WorkflowResponseMode;
use super::workflow_definition::WorkflowStep;
use super::workflow_definition::WorkflowTriggerKind;
use super::workflow_definition::load_workflow_registry;
use super::workflow_definition::ordered_jobs_for_roots;
use super::workflow_history::WorkflowReplySource;
use super::workflow_history::workflow_result_cell;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ApprovalsReviewer as AppServerApprovalsReviewer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
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
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const WORKFLOW_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WORKFLOW_INTERRUPT_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BackgroundWorkflowRunTarget {
    Trigger {
        workflow_name: String,
        trigger_id: String,
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
    AssistantCell,
    UserFollowup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowRunPhase {
    BeforeTurn,
    AfterTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkflowPhaseContext<'a> {
    pub(crate) current_user_turn: Option<&'a str>,
    pub(crate) last_assistant_message: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowJobRunResult {
    pub(crate) delivery: WorkflowOutputDelivery,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowThreadSession {
    thread_id: String,
    cwd: PathBuf,
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
    fn start_workflow_thread(&self) -> BoxFuture<'_, Result<WorkflowThreadSession, String>>;
    fn start_turn(
        &self,
        thread_id: String,
        cwd: PathBuf,
        input: String,
    ) -> BoxFuture<'_, Result<String, String>>;
    fn read_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> BoxFuture<'_, Result<WorkflowTurnState, String>>;
    fn interrupt_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> BoxFuture<'_, Result<(), String>>;
    fn unsubscribe_thread(&self, thread_id: String) -> BoxFuture<'_, Result<(), String>>;
}

pub(crate) struct AppServerWorkflowRuntimeClient {
    request_handle: AppServerRequestHandle,
    config: Config,
    primary_thread_id: Option<ThreadId>,
    is_remote: bool,
    remote_cwd_override: Option<PathBuf>,
}

impl AppServerWorkflowRuntimeClient {
    pub(crate) fn new(
        app_server: &AppServerSession,
        config: Config,
        primary_thread_id: Option<ThreadId>,
    ) -> Self {
        Self {
            request_handle: app_server.request_handle(),
            config,
            primary_thread_id,
            is_remote: app_server.is_remote(),
            remote_cwd_override: app_server.remote_cwd_override().map(PathBuf::from),
        }
    }
}

impl WorkflowRuntimeClient for AppServerWorkflowRuntimeClient {
    fn start_workflow_thread(&self) -> BoxFuture<'_, Result<WorkflowThreadSession, String>> {
        Box::pin(async move {
            if let Some(primary_thread_id) = self.primary_thread_id {
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
                return Ok(WorkflowThreadSession {
                    thread_id: response.thread.id,
                    cwd: response.cwd,
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
            Ok(WorkflowThreadSession {
                thread_id: response.thread.id,
                cwd: response.cwd,
            })
        })
    }

    fn start_turn(
        &self,
        thread_id: String,
        cwd: PathBuf,
        input: String,
    ) -> BoxFuture<'_, Result<String, String>> {
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

    fn read_turn(
        &self,
        thread_id: String,
        turn_id: String,
    ) -> BoxFuture<'_, Result<WorkflowTurnState, String>> {
        Box::pin(async move {
            let response: ThreadReadResponse = self
                .request_handle
                .request_typed(ClientRequest::ThreadRead {
                    request_id: request_id(),
                    params: ThreadReadParams {
                        thread_id: thread_id.clone(),
                        include_turns: true,
                    },
                })
                .await
                .map_err(|err| format!("failed to read workflow thread: {err}"))?;
            let turn = response
                .thread
                .turns
                .into_iter()
                .find(|turn| turn.id == turn_id)
                .ok_or_else(|| {
                    format!("workflow turn `{turn_id}` is missing from thread `{thread_id}`")
                })?;
            Ok(WorkflowTurnState {
                status: turn.status,
                error: turn.error.map(|error| error.message),
                last_agent_message: turn.items.into_iter().fold(None, |_, item| match item {
                    codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } => {
                        (!text.trim().is_empty()).then_some(text.trim().to_string())
                    }
                    _ => None,
                }),
            })
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
            let _: ThreadUnsubscribeResponse = self
                .request_handle
                .request_typed(ClientRequest::ThreadUnsubscribe {
                    request_id: request_id(),
                    params: ThreadUnsubscribeParams { thread_id },
                })
                .await
                .map_err(|err| format!("failed to unsubscribe workflow thread: {err}"))?;
            Ok(())
        })
    }
}

#[allow(dead_code)]
impl App {
    pub(crate) async fn run_phase_workflows(
        &self,
        app_server: &AppServerSession,
        phase: WorkflowRunPhase,
        phase_context: WorkflowPhaseContext<'_>,
    ) -> Result<Vec<WorkflowJobRunResult>, String> {
        let registry = load_workflow_registry(self.config.cwd.as_path())
            .map_err(|error| format!("failed to load workflows: {error}"))?;
        let client = AppServerWorkflowRuntimeClient::new(
            app_server,
            self.config.clone(),
            self.primary_thread_id,
        );
        let mut results = Vec::new();
        for workflow in &registry.files {
            for trigger in &workflow.triggers {
                if !trigger.enabled || !workflow_trigger_matches_phase(&trigger.kind, phase) {
                    continue;
                }
                results.extend(
                    run_workflow_jobs(
                        &client,
                        &registry,
                        &workflow.name,
                        &trigger.id,
                        &trigger.jobs,
                        phase_context,
                        /*cancellation*/ None,
                    )
                    .await
                    .map_err(|error| match error {
                        WorkflowRunError::Failed(message) => message,
                        WorkflowRunError::Cancelled => "workflow run cancelled".to_string(),
                    })?,
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
        let label = format!("{workflow_name} · {trigger_id}");
        match self.queue_or_start_trigger_run(app_server, workflow_name, trigger_id) {
            TriggerRunDispatch::Started => Arc::new(history_cell::new_info_event(
                "Workflow trigger started".to_string(),
                Some(label),
            )),
            TriggerRunDispatch::Queued => Arc::new(history_cell::new_info_event(
                "Workflow trigger queued".to_string(),
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
    ) {
        let _ = self.queue_or_start_trigger_run(app_server, workflow_name, trigger_id);
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
    ) -> TriggerRunDispatch {
        if self.workflow_scheduler.has_running_trigger_run() {
            self.workflow_scheduler
                .enqueue_trigger_run(workflow_name, trigger_id);
            self.sync_background_workflow_status();
            return TriggerRunDispatch::Queued;
        }

        self.start_background_workflow_run(
            app_server,
            BackgroundWorkflowRunTarget::Trigger {
                workflow_name,
                trigger_id,
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
            let result = run_background_workflow(
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
            Err(WorkflowRunError::Failed(error)) => BackgroundWorkflowRunOutcome::Failed(error),
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
            if !matches!(trigger.kind, WorkflowTriggerKind::Manual) {
                return Err(WorkflowRunError::Failed(format!(
                    "workflow trigger `{workflow_name}/{trigger_id}` is not runnable as a queued manual trigger"
                )));
            }
            run_workflow_jobs(
                client,
                &registry,
                workflow_name,
                trigger_id,
                &trigger.jobs,
                WorkflowPhaseContext {
                    current_user_turn: None,
                    last_assistant_message: None,
                },
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
                workflow_name,
                &manual_workflow_job_trigger_id(job_name),
                std::slice::from_ref(job_name),
                WorkflowPhaseContext {
                    current_user_turn: None,
                    last_assistant_message: None,
                },
                Some(cancellation),
            )
            .await
        }
    }
}

fn workflow_trigger_matches_phase(trigger: &WorkflowTriggerKind, phase: WorkflowRunPhase) -> bool {
    matches!(
        (trigger, phase),
        (
            &WorkflowTriggerKind::BeforeTurn,
            WorkflowRunPhase::BeforeTurn
        ) | (&WorkflowTriggerKind::AfterTurn, WorkflowRunPhase::AfterTurn)
    )
}

async fn run_workflow_jobs(
    client: &dyn WorkflowRuntimeClient,
    registry: &LoadedWorkflowRegistry,
    workflow_name: &str,
    trigger_id: &str,
    root_jobs: &[String],
    phase_context: WorkflowPhaseContext<'_>,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<WorkflowJobRunResult>, WorkflowRunError> {
    let ordered = ordered_jobs_for_roots(registry, root_jobs)
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
        if !job.config.enabled {
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
            workflow_name,
            trigger_id,
            job,
            phase_context,
            cancellation,
        )
        .await?;
        completed.insert(job.name.clone(), true);
        results.push(result);
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
    if matches!(job.config.context, WorkflowContextMode::Embed) {
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
                    "workflow `{workflow_name}` job `{}` uses embed context but has no prompt step",
                    job.name
                ))
            })?;
        return Ok(WorkflowJobRunResult {
            delivery: WorkflowOutputDelivery::MainThreadInput,
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
        let attempts = step.retry_attempts();
        let mut last_error = None;
        for attempt in 1..=attempts {
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
            let result =
                execute_workflow_step(client, &mut thread, context, step, &step_outputs).await;
            match result {
                Ok(Some(output)) => {
                    if matches!(step, WorkflowStep::Prompt { .. }) {
                        last_prompt_response = Some(output.clone());
                    }
                    step_outputs.push(output);
                    last_error = None;
                    break;
                }
                Ok(None) => {
                    last_error = None;
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    if attempt < attempts
                        && !matches!(last_error, Some(WorkflowRunError::Cancelled))
                    {
                        sleep(retry_backoff_delay(attempt)).await;
                    }
                }
            }
        }
        if let Some(error) = last_error {
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
    step_outputs: &[String],
) -> Result<Option<String>, WorkflowRunError> {
    match step {
        WorkflowStep::Run { run, .. } => {
            run_workflow_command(run, &context.job.workflow_path, context.cancellation).await
        }
        WorkflowStep::Prompt { prompt, .. } => {
            let thread = match thread {
                Some(thread) => thread.clone(),
                None => {
                    let started = client
                        .start_workflow_thread()
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
            run_workflow_prompt(client, &thread, prompt, context.cancellation).await
        }
    }
}

async fn run_workflow_prompt(
    client: &dyn WorkflowRuntimeClient,
    thread: &WorkflowThreadSession,
    prompt: String,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<String>, WorkflowRunError> {
    let turn_id = client
        .start_turn(thread.thread_id.clone(), thread.cwd.clone(), prompt)
        .await
        .map_err(WorkflowRunError::Failed)?;
    loop {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            interrupt_active_workflow_turn(client, thread.thread_id.clone(), turn_id.clone()).await;
            return Err(WorkflowRunError::Cancelled);
        }
        let turn = client
            .read_turn(thread.thread_id.clone(), turn_id.clone())
            .await
            .map_err(WorkflowRunError::Failed)?;
        match turn.status {
            TurnStatus::Completed => return Ok(turn.last_agent_message),
            TurnStatus::Interrupted => return Err(WorkflowRunError::Cancelled),
            TurnStatus::Failed => {
                return Err(WorkflowRunError::Failed(
                    turn.error
                        .unwrap_or_else(|| "workflow prompt turn failed".to_string()),
                ));
            }
            TurnStatus::InProgress => sleep(WORKFLOW_POLL_INTERVAL).await,
        }
    }
}

async fn interrupt_active_workflow_turn(
    client: &dyn WorkflowRuntimeClient,
    thread_id: String,
    turn_id: String,
) {
    let _ = client
        .interrupt_turn(thread_id.clone(), turn_id.clone())
        .await;
    let deadline = tokio::time::Instant::now() + WORKFLOW_INTERRUPT_SETTLE_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        match client.read_turn(thread_id.clone(), turn_id.clone()).await {
            Ok(turn) if !matches!(turn.status, TurnStatus::InProgress) => return,
            Ok(_) | Err(_) => sleep(WORKFLOW_POLL_INTERVAL).await,
        }
    }
}

async fn run_workflow_command(
    command: &str,
    workflow_path: &std::path::Path,
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
        output = &mut wait_with_output => output,
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
    config: &Config,
    is_remote: bool,
    remote_cwd_override: Option<&std::path::Path>,
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
    remote_cwd_override: Option<&std::path::Path>,
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
    remote_cwd_override: Option<&std::path::Path>,
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

#[derive(Debug)]
enum WorkflowRunError {
    Failed(String),
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use tempfile::tempdir;

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
        fn start_workflow_thread(&self) -> BoxFuture<'_, Result<WorkflowThreadSession, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push("start_workflow_thread".to_string());
                Ok(WorkflowThreadSession {
                    thread_id: self.thread_id.clone(),
                    cwd: PathBuf::from("/tmp/workflow"),
                })
            })
        }

        fn start_turn(
            &self,
            thread_id: String,
            _cwd: PathBuf,
            input: String,
        ) -> BoxFuture<'_, Result<String, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("start_turn:{thread_id}:{input}"));
                Ok(self.turn_id.clone())
            })
        }

        fn read_turn(
            &self,
            thread_id: String,
            turn_id: String,
        ) -> BoxFuture<'_, Result<WorkflowTurnState, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("read_turn:{thread_id}:{turn_id}"));
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
    jobs: [review_backlog]

jobs:
  review_backlog:
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
                "start_workflow_thread".to_string(),
                "start_turn:thr_workflow:Workflow: director\nTrigger: job:review_backlog\nJob: review_backlog\n\nCurrent workflow prompt:\nsummarize the backlog".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "unsubscribe_thread:thr_workflow".to_string(),
            ]
        );
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
                "start_workflow_thread".to_string(),
                "start_turn:thr_workflow:Workflow: director\nTrigger: job:review_backlog\nJob: review_backlog\n\nCurrent workflow prompt:\nsummarize the backlog".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "interrupt_turn:thr_workflow:turn_workflow".to_string(),
                "read_turn:thr_workflow:turn_workflow".to_string(),
                "unsubscribe_thread:thr_workflow".to_string(),
            ]
        );
    }
}
