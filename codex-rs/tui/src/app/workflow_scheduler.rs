use std::collections::HashMap;
use std::collections::VecDeque;
use tokio::task::JoinHandle;

pub(crate) struct BackgroundWorkflowRunState {
    pub(crate) label: String,
    #[cfg(test)]
    pub(crate) is_trigger: bool,
    pub(crate) handle: JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedWorkflowTriggerRun {
    pub(crate) workflow_name: String,
    pub(crate) trigger_id: String,
}

#[derive(Default)]
pub(crate) struct WorkflowSchedulerState {
    running_workflows: HashMap<String, BackgroundWorkflowRunState>,
    queued_trigger_runs: VecDeque<QueuedWorkflowTriggerRun>,
    #[cfg(test)]
    next_background_run_id: u64,
}

impl WorkflowSchedulerState {
    #[cfg(test)]
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

    #[cfg(test)]
    pub(crate) fn register_background_workflow_run(
        &mut self,
        run_id: String,
        label: String,
        is_trigger: bool,
        handle: JoinHandle<()>,
    ) {
        self.running_workflows.insert(
            run_id,
            BackgroundWorkflowRunState {
                label,
                #[cfg(test)]
                is_trigger,
                handle,
            },
        );
    }

    #[cfg(test)]
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

    #[cfg(test)]
    pub(crate) fn has_running_trigger_run(&self) -> bool {
        self.running_workflows.values().any(|run| run.is_trigger)
    }

    #[cfg(test)]
    pub(crate) fn enqueue_trigger_run(&mut self, workflow_name: String, trigger_id: String) {
        self.queued_trigger_runs
            .push_back(QueuedWorkflowTriggerRun {
                workflow_name,
                trigger_id,
            });
    }

    #[cfg(test)]
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
        for run in runs {
            run.handle.abort();
            let _ = run.handle.await;
        }
        self.queued_trigger_runs.clear();
        stopped_count
    }
}
