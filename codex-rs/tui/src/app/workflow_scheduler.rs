#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::App;
use super::workflow_runtime::BackgroundWorkflowRunTarget;
use super::workflow_runtime::OwnedWorkflowPhaseContext;

pub(crate) struct BackgroundWorkflowRunState {
    pub(crate) label: String,
    pub(crate) is_trigger: bool,
    pub(crate) target: BackgroundWorkflowRunTarget,
    pub(crate) cancellation: CancellationToken,
    pub(crate) handle: JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedWorkflowTriggerRun {
    pub(crate) workflow_name: String,
    pub(crate) trigger_id: String,
    pub(crate) phase_context: OwnedWorkflowPhaseContext,
}

#[derive(Default)]
pub(crate) struct WorkflowSchedulerState {
    running_workflows: HashMap<String, BackgroundWorkflowRunState>,
    queued_trigger_runs: VecDeque<QueuedWorkflowTriggerRun>,
    next_background_run_id: u64,
}

impl WorkflowSchedulerState {
    pub(crate) fn next_background_run_id(
        &mut self,
        workflow_name: &str,
        target_name: &str,
    ) -> String {
        self.next_background_run_id = self.next_background_run_id.saturating_add(1);
        format!(
            "{workflow_name}/{target_name}#{}",
            self.next_background_run_id
        )
    }

    pub(crate) fn register_background_workflow_run(
        &mut self,
        run_id: String,
        target: BackgroundWorkflowRunTarget,
        cancellation: CancellationToken,
        handle: JoinHandle<()>,
    ) {
        let label = target.label();
        let is_trigger = matches!(target, BackgroundWorkflowRunTarget::Trigger { .. });
        self.running_workflows.insert(
            run_id,
            BackgroundWorkflowRunState {
                label,
                is_trigger,
                target,
                cancellation,
                handle,
            },
        );
    }

    pub(crate) fn take_background_workflow_run(
        &mut self,
        run_id: &str,
    ) -> Option<BackgroundWorkflowRunState> {
        self.running_workflows.remove(run_id)
    }

    pub(crate) fn background_workflow_labels(&self) -> Vec<String> {
        let mut labels = self
            .running_workflows
            .values()
            .map(|run| run.label.clone())
            .collect::<Vec<_>>();
        labels.sort();
        labels
    }

    pub(crate) fn queued_trigger_labels(&self) -> Vec<String> {
        self.queued_trigger_runs
            .iter()
            .map(|run| format!("{} · {}", run.workflow_name, run.trigger_id))
            .collect()
    }

    pub(crate) fn has_running_trigger_run(&self) -> bool {
        self.running_workflows.values().any(|run| run.is_trigger)
    }

    pub(crate) fn has_active_trigger_run(&self, workflow_name: &str, trigger_id: &str) -> bool {
        self.running_workflows.values().any(|run| {
            matches!(
                &run.target,
                BackgroundWorkflowRunTarget::Trigger {
                    workflow_name: active_workflow_name,
                    trigger_id: active_trigger_id,
                    ..
                } if active_workflow_name == workflow_name && active_trigger_id == trigger_id
            )
        })
    }

    pub(crate) fn has_queued_trigger_run(&self, workflow_name: &str, trigger_id: &str) -> bool {
        self.queued_trigger_runs
            .iter()
            .any(|run| run.workflow_name == workflow_name && run.trigger_id == trigger_id)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn enqueue_trigger_run(
        &mut self,
        workflow_name: String,
        trigger_id: String,
        phase_context: OwnedWorkflowPhaseContext,
    ) {
        self.queued_trigger_runs
            .push_back(QueuedWorkflowTriggerRun {
                workflow_name,
                trigger_id,
                phase_context,
            });
    }

    pub(crate) fn dequeue_trigger_run(&mut self) -> Option<QueuedWorkflowTriggerRun> {
        self.queued_trigger_runs.pop_front()
    }

    pub(crate) async fn stop_active_workflow_runs(&mut self) -> usize {
        let runs = self
            .running_workflows
            .drain()
            .map(|(_, run)| run)
            .collect::<Vec<_>>();
        let stopped_count = runs.len();
        for mut run in runs {
            run.cancellation.cancel();
            if timeout(Duration::from_secs(1), &mut run.handle)
                .await
                .is_err()
            {
                run.handle.abort();
                let _ = run.handle.await;
            }
        }
        self.queued_trigger_runs.clear();
        stopped_count
    }
}

impl App {
    pub(crate) fn background_workflow_labels(&self) -> Vec<String> {
        self.workflow_scheduler.background_workflow_labels()
    }

    pub(crate) fn queued_trigger_labels(&self) -> Vec<String> {
        self.workflow_scheduler.queued_trigger_labels()
    }

    pub(crate) fn sync_background_workflow_status(&mut self) {
        self.chat_widget.sync_background_workflow_status(
            self.background_workflow_labels(),
            self.queued_trigger_labels(),
        );
        self.refresh_workflow_controls_if_active();
    }
}
