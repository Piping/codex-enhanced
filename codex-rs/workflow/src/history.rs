use crate::runtime::WorkflowJobRunResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowReplySource {
    workflow_id: String,
    action: Option<String>,
}

impl WorkflowReplySource {
    pub fn new(workflow_id: String, action: Option<String>) -> Self {
        Self {
            workflow_id,
            action,
        }
    }

    pub fn hint(&self) -> String {
        match self
            .action
            .as_deref()
            .map(str::trim)
            .filter(|action| !action.is_empty())
        {
            Some(action) => format!("{} · {}", self.workflow_id, workflow_prompt_prefix(action)),
            None => self.workflow_id.clone(),
        }
    }
}

pub fn workflow_job_source(result: &WorkflowJobRunResult) -> String {
    format!(
        "{}/{}:{}",
        result.workflow_name, result.trigger_id, result.job_name
    )
}

pub fn workflow_prompt_prefix(prompt: &str) -> String {
    let prefix = prompt.chars().take(48).collect::<String>();
    if prompt.chars().count() > 48 {
        format!("{prefix}...")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::WorkflowReplySource;

    #[test]
    fn workflow_reply_source_hint_prefers_action_when_present() {
        let source = WorkflowReplySource::new(
            "director/review:summary".to_string(),
            Some("summarize it".to_string()),
        );
        assert_eq!(
            source.hint(),
            "director/review:summary · summarize it".to_string()
        );
    }
}
