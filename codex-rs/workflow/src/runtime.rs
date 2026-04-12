use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::definition::LoadedWorkflowJob;
use crate::definition::LoadedWorkflowRegistry;
use crate::definition::WorkflowContextMode;
use crate::definition::WorkflowResponseMode;
use crate::definition::WorkflowStep;
use crate::definition::WorkflowTriggerKindDiscriminant;
use crate::definition::load_workflow_registry;
use crate::definition::ordered_jobs_for_roots;

const WORKFLOW_POLL_INTERVAL: Duration = Duration::from_millis(50);
const WORKFLOW_INTERRUPT_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);
const WORKFLOW_STEP_TIMEOUT: Duration = Duration::from_secs(30);

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// App-specific bridge used by the shared workflow runtime to create threads,
/// drive prompt turns, and observe their completion state.
///
/// Implementations own transport and thread-session details; the runtime only
/// assumes that a started thread can be passed back into subsequent calls until
/// it is unsubscribed.
pub trait WorkflowRuntimeClient: Send + Sync {
    type Thread: Clone + Send + Sync;

    fn start_workflow_thread(&self) -> BoxFuture<'_, Result<Self::Thread, String>>;
    fn start_turn<'a>(
        &'a self,
        thread: &'a Self::Thread,
        input: String,
    ) -> BoxFuture<'a, Result<String, String>>;
    fn read_turn<'a>(
        &'a self,
        thread: &'a Self::Thread,
        turn_id: String,
    ) -> BoxFuture<'a, Result<WorkflowTurnState, String>>;
    fn interrupt_turn<'a>(
        &'a self,
        thread: &'a Self::Thread,
        turn_id: String,
    ) -> BoxFuture<'a, Result<(), String>>;
    fn unsubscribe_thread<'a>(
        &'a self,
        thread: &'a Self::Thread,
    ) -> BoxFuture<'a, Result<(), String>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTriggerOverlapBehavior {
    Queue,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowOutputDelivery {
    MainThreadInput,
    AssistantCell,
    UserFollowup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowPhaseContext<'a> {
    pub current_user_turn: Option<&'a str>,
    pub last_assistant_message: Option<&'a str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedWorkflowPhaseContext {
    pub current_user_turn: Option<String>,
    pub last_assistant_message: Option<String>,
}

impl OwnedWorkflowPhaseContext {
    pub fn borrowed(&self) -> WorkflowPhaseContext<'_> {
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

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundWorkflowRunTarget {
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
    pub fn workflow_name(&self) -> &str {
        match self {
            Self::Trigger { workflow_name, .. } | Self::Job { workflow_name, .. } => workflow_name,
        }
    }

    pub fn slot_key(&self) -> &str {
        match self {
            Self::Trigger { trigger_id, .. } => trigger_id,
            Self::Job { job_name, .. } => job_name,
        }
    }

    pub fn label(&self) -> String {
        format!("{} · {}", self.workflow_name(), self.slot_key())
    }

    pub fn started_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger started",
            Self::Job { .. } => "Workflow job started",
        }
    }

    pub fn completed_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger completed",
            Self::Job { .. } => "Workflow job completed",
        }
    }

    pub fn stopped_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger stopped",
            Self::Job { .. } => "Workflow job stopped",
        }
    }

    pub fn failed_message(&self) -> &'static str {
        match self {
            Self::Trigger { .. } => "Workflow trigger failed",
            Self::Job { .. } => "Workflow job failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowJobRunResult {
    pub delivery: WorkflowOutputDelivery,
    pub workflow_name: String,
    pub trigger_id: String,
    pub job_name: String,
    pub message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BackgroundWorkflowRunOutcome {
    Completed(Vec<WorkflowJobRunResult>),
    Cancelled,
    Failed(String),
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
pub struct BackgroundWorkflowRunResult {
    pub target: BackgroundWorkflowRunTarget,
    pub outcome: BackgroundWorkflowRunOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTurnState {
    pub status: WorkflowTurnStatus,
    pub error: Option<String>,
    pub last_agent_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug)]
pub enum WorkflowRunError {
    Failed(String),
    TimedOut(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowDisabledJobBehavior {
    Skip,
    RunRootJobs,
}

#[derive(Clone, Copy)]
struct WorkflowStepExecutionContext<'a> {
    workflow_name: &'a str,
    trigger_id: &'a str,
    job: &'a LoadedWorkflowJob,
    phase_context: WorkflowPhaseContext<'a>,
    cancellation: Option<&'a CancellationToken>,
}

pub async fn run_before_turn_workflows<C: WorkflowRuntimeClient>(
    client: &C,
    registry: &LoadedWorkflowRegistry,
    phase_context: WorkflowPhaseContext<'_>,
) -> Result<Vec<WorkflowJobRunResult>, WorkflowRunError> {
    let mut results = Vec::new();
    for (workflow, trigger) in
        registry.iter_matching_triggers(WorkflowTriggerKindDiscriminant::BeforeTurn)
    {
        if !trigger.enabled {
            continue;
        }
        results.extend(
            run_workflow_jobs(
                client,
                registry,
                &workflow.name,
                &trigger.id,
                &trigger.jobs,
                phase_context,
                WorkflowDisabledJobBehavior::Skip,
                /*cancellation*/ None,
            )
            .await?,
        );
    }
    Ok(results)
}

pub async fn run_background_workflow<C: WorkflowRuntimeClient>(
    client: &C,
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

pub fn manual_workflow_job_trigger_id(job_name: &str) -> String {
    format!("job:{job_name}")
}

pub fn workflow_run_error_message(error: WorkflowRunError) -> String {
    match error {
        WorkflowRunError::Failed(message) | WorkflowRunError::TimedOut(message) => message,
        WorkflowRunError::Cancelled => "workflow run cancelled".to_string(),
    }
}

async fn run_background_workflow_selection<C: WorkflowRuntimeClient>(
    client: &C,
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
            let workflow = registry.workflow(workflow_name).ok_or_else(|| {
                WorkflowRunError::Failed(format!("workflow `{workflow_name}` does not exist"))
            })?;
            let trigger = registry.trigger(workflow_name, trigger_id).ok_or_else(|| {
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
                &workflow.name,
                &trigger.id,
                &trigger.jobs,
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
            let job = registry.job(job_name).ok_or_else(|| {
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
                WorkflowDisabledJobBehavior::RunRootJobs,
                Some(cancellation),
            )
            .await
        }
    }
}

async fn run_workflow_jobs<C: WorkflowRuntimeClient>(
    client: &C,
    registry: &LoadedWorkflowRegistry,
    workflow_name: &str,
    trigger_id: &str,
    root_jobs: &[String],
    phase_context: WorkflowPhaseContext<'_>,
    disabled_job_behavior: WorkflowDisabledJobBehavior,
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
        let job = registry.job(&job_name).ok_or_else(|| {
            WorkflowRunError::Failed(format!("workflow job `{job_name}` does not exist"))
        })?;
        let should_run_disabled_job = matches!(
            disabled_job_behavior,
            WorkflowDisabledJobBehavior::RunRootJobs
        ) && root_jobs.iter().any(|root_job| root_job == &job_name);
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
    if results.is_empty() {
        return Err(WorkflowRunError::Failed(format!(
            "workflow `{workflow_name}/{trigger_id}` did not run any enabled jobs"
        )));
    }
    Ok(results)
}

async fn run_workflow_job<C: WorkflowRuntimeClient>(
    client: &C,
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

    let mut thread: Option<C::Thread> = None;
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
                    let _ = client.unsubscribe_thread(thread).await;
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
                Ok(None) => break None,
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
                let _ = client.unsubscribe_thread(thread).await;
            }
            return Err(error);
        }
    }

    if let Some(thread) = thread.as_ref() {
        let _ = client.unsubscribe_thread(thread).await;
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

async fn execute_workflow_step<C: WorkflowRuntimeClient>(
    client: &C,
    thread: &mut Option<C::Thread>,
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
            run_workflow_prompt(client, &thread, prompt, step_timeout, context.cancellation).await
        }
    }
}

async fn run_workflow_prompt<C: WorkflowRuntimeClient>(
    client: &C,
    thread: &C::Thread,
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
            WorkflowTurnStatus::Completed => return Ok(turn.last_agent_message),
            WorkflowTurnStatus::Interrupted => return Err(WorkflowRunError::Cancelled),
            WorkflowTurnStatus::Failed => {
                return Err(WorkflowRunError::Failed(
                    turn.error
                        .unwrap_or_else(|| "workflow prompt turn failed".to_string()),
                ));
            }
            WorkflowTurnStatus::InProgress => {
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

async fn interrupt_active_workflow_turn<C: WorkflowRuntimeClient>(
    client: &C,
    thread: &C::Thread,
    turn_id: String,
) {
    let _ = client.interrupt_turn(thread, turn_id.clone()).await;
    let deadline = tokio::time::Instant::now() + WORKFLOW_INTERRUPT_SETTLE_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        match client.read_turn(thread, turn_id.clone()).await {
            Ok(turn) if !matches!(turn.status, WorkflowTurnStatus::InProgress) => return,
            Ok(_) | Err(_) => sleep(WORKFLOW_POLL_INTERVAL).await,
        }
    }
}

async fn run_workflow_command(
    command: &str,
    workflow_path: &Path,
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use tokio::time;

    use super::*;

    #[derive(Clone)]
    struct FakeThread {
        thread_id: String,
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
        type Thread = FakeThread;

        fn start_workflow_thread(&self) -> BoxFuture<'_, Result<Self::Thread, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push("start_workflow_thread".to_string());
                Ok(FakeThread {
                    thread_id: self.thread_id.clone(),
                })
            })
        }

        fn start_turn<'a>(
            &'a self,
            thread: &'a Self::Thread,
            input: String,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("start_turn:{}:{input}", thread.thread_id));
                Ok(self.turn_id.clone())
            })
        }

        fn read_turn<'a>(
            &'a self,
            thread: &'a Self::Thread,
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
                    .unwrap_or(Ok(WorkflowTurnState {
                        status: WorkflowTurnStatus::InProgress,
                        error: None,
                        last_agent_message: None,
                    }))
            })
        }

        fn interrupt_turn<'a>(
            &'a self,
            thread: &'a Self::Thread,
            turn_id: String,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("interrupt_turn:{}:{turn_id}", thread.thread_id));
                Ok(())
            })
        }

        fn unsubscribe_thread<'a>(
            &'a self,
            thread: &'a Self::Thread,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("unsubscribe_thread:{}", thread.thread_id));
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn prompt_workflow_job_uses_runtime_sequence() {
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
                status: WorkflowTurnStatus::InProgress,
                error: None,
                last_agent_message: None,
            }),
            Ok(WorkflowTurnState {
                status: WorkflowTurnStatus::Completed,
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
    async fn prompt_workflow_job_retries_selected_model_capacity_once() {
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
                status: WorkflowTurnStatus::Failed,
                error: Some(
                    "Selected model is at capacity. Please try a different model.".to_string(),
                ),
                last_agent_message: None,
            }),
            Ok(WorkflowTurnState {
                status: WorkflowTurnStatus::Completed,
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
    async fn non_manual_trigger_can_run_now() {
        let tempdir = tempdir().expect("tempdir");
        let workflows_dir = tempdir.path().join(".codex/workflows");
        std::fs::create_dir_all(&workflows_dir).expect("workflow dir");
        std::fs::write(
            workflows_dir.join("manual.yaml"),
            r#"name: director

triggers:
  - type: after_turn
    id: follow_up
    jobs: [review_backlog]

jobs:
  review_backlog:
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![Ok(WorkflowTurnState {
            status: WorkflowTurnStatus::Completed,
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
    enabled: false
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![Ok(WorkflowTurnState {
            status: WorkflowTurnStatus::Completed,
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
    jobs: [review_backlog]

jobs:
  review_backlog:
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
    steps:
      - prompt: summarize the backlog
"#,
        )
        .expect("workflow fixture");
        let client = FakeWorkflowRuntimeClient::new(vec![
            Ok(WorkflowTurnState {
                status: WorkflowTurnStatus::InProgress,
                error: None,
                last_agent_message: None,
            }),
            Ok(WorkflowTurnState {
                status: WorkflowTurnStatus::Interrupted,
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
