use std::collections::HashMap;
use std::path::PathBuf;

use codex_app_server_protocol::ApprovalsReviewer as AppServerApprovalsReviewer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::SubAgentSpawnParams;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::UserInput;
use codex_protocol::ThreadId;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::SandboxPolicy;
use uuid::Uuid;

use super::App;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use codex_app_server_client::AppServerRequestHandle;

const BTW_DEVELOPER_INSTRUCTIONS: &str = concat!(
    "This is a `/btw` agent thread. ",
    "Treat it as a dedicated follow-up agent with the same approval and sandbox settings as the ",
    "source thread. ",
    "Use the thread directly and answer the user's prompt in that thread."
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BtwSessionState {
    pub(crate) thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BtwPermissions {
    pub(super) approval_policy: codex_app_server_protocol::AskForApproval,
    pub(super) approvals_reviewer: codex_protocol::config_types::ApprovalsReviewer,
    pub(super) permission_profile: PermissionProfile,
    pub(super) active_permission_profile: Option<ActivePermissionProfile>,
    pub(super) sandbox_policy: SandboxPolicy,
}

impl App {
    pub(crate) async fn start_btw_discussion(
        &mut self,
        app_server: &mut AppServerSession,
        prompt: String,
    ) {
        let trimmed_prompt = prompt.trim();
        if trimmed_prompt.is_empty() {
            self.chat_widget
                .add_error_message("Usage: /btw <prompt>".to_string());
            return;
        }

        let permissions = self.btw_permissions().await;
        let agent_nickname = btw_agent_nickname(trimmed_prompt);
        let subagent_spawn = self.btw_subagent_spawn(&agent_nickname);
        let thread_id = match self
            .start_btw_thread(app_server, &permissions, subagent_spawn.as_ref())
            .await
        {
            Ok(thread_id) => thread_id,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to start `/btw`: {err}"));
                return;
            }
        };
        self.upsert_agent_picker_thread(
            thread_id,
            Some(agent_nickname),
            Some("btw".to_string()),
            /*is_closed*/ false,
        );
        self.prime_btw_thread_channel(app_server, thread_id).await;
        self.btw_session = Some(BtwSessionState { thread_id });

        let turn_result = app_server
            .turn_start(
                thread_id,
                btw_turn_input(trimmed_prompt).into_iter().collect(),
                self.btw_turn_cwd_path(app_server),
                permissions.approval_policy,
                permissions.approvals_reviewer,
                permissions.permission_profile.clone(),
                permissions.active_permission_profile.clone(),
                self.chat_widget.current_model().to_string(),
                self.chat_widget.current_reasoning_effort(),
                /*summary*/ None,
                self.chat_widget
                    .current_service_tier()
                    .map(|service_tier| Some(service_tier.request_value().to_string())),
                /*collaboration_mode*/ None,
                self.config.personality,
                /*output_schema*/ None,
            )
            .await;
        if let Err(err) = turn_result {
            self.btw_session = None;
            if let Err(close_err) = app_server.thread_unsubscribe(thread_id).await {
                tracing::warn!(
                    thread_id = %thread_id,
                    error = %close_err,
                    "failed to clean up `/btw` thread after submission failure"
                );
            }
            self.chat_widget
                .add_error_message(format!("Failed to submit `/btw`: {err}"));
            return;
        }

        self.app_event_tx
            .send(AppEvent::SelectAgentThread(thread_id));
    }

    async fn start_btw_thread(
        &self,
        app_server: &AppServerSession,
        permissions: &BtwPermissions,
        subagent_spawn: Option<&SubAgentSpawnParams>,
    ) -> Result<ThreadId, String> {
        let request_handle = app_server.request_handle();
        if let Some(thread_id) = self.btw_fork_source_thread_id(&request_handle).await? {
            let response: ThreadForkResponse = request_handle
                .request_typed(ClientRequest::ThreadFork {
                    request_id: request_id(),
                    params: btw_thread_fork_params(
                        self,
                        thread_id,
                        app_server,
                        permissions,
                        subagent_spawn,
                    ),
                })
                .await
                .map_err(|err| format!("failed to fork `/btw` thread: {err}"))?;
            ThreadId::from_string(&response.thread.id)
                .map_err(|err| format!("invalid `/btw` thread id: {err}"))
        } else {
            let response: ThreadStartResponse = request_handle
                .request_typed(ClientRequest::ThreadStart {
                    request_id: request_id(),
                    params: btw_thread_start_params(self, app_server, permissions, subagent_spawn),
                })
                .await
                .map_err(|err| format!("failed to start `/btw` thread: {err}"))?;
            ThreadId::from_string(&response.thread.id)
                .map_err(|err| format!("invalid `/btw` thread id: {err}"))
        }
    }

    pub(super) async fn btw_permissions(&self) -> BtwPermissions {
        if let Some(thread_id) = self.current_displayed_thread_id()
            && let Some(channel) = self.thread_event_channels.get(&thread_id)
        {
            let store = channel.store.lock().await;
            if let Some(session) = store.session.as_ref() {
                let permission_profile = session.permission_profile.clone();
                return BtwPermissions {
                    approval_policy: session.approval_policy,
                    approvals_reviewer: session.approvals_reviewer,
                    permission_profile: permission_profile.clone(),
                    active_permission_profile: session.active_permission_profile.clone(),
                    sandbox_policy: permission_profile
                        .to_legacy_sandbox_policy(session.cwd.as_path())
                        .unwrap_or(SandboxPolicy::DangerFullAccess),
                };
            }
        }

        if let Some(session) = self.primary_session_configured.as_ref() {
            let permission_profile = session.permission_profile.clone();
            return BtwPermissions {
                approval_policy: session.approval_policy,
                approvals_reviewer: session.approvals_reviewer,
                permission_profile: permission_profile.clone(),
                active_permission_profile: session.active_permission_profile.clone(),
                sandbox_policy: permission_profile
                    .to_legacy_sandbox_policy(session.cwd.as_path())
                    .unwrap_or(SandboxPolicy::DangerFullAccess),
            };
        }

        let permission_profile = self.config.permissions.permission_profile();
        BtwPermissions {
            approval_policy: self.config.permissions.approval_policy.value().into(),
            approvals_reviewer: self.config.approvals_reviewer,
            permission_profile: permission_profile.clone(),
            active_permission_profile: self.config.permissions.active_permission_profile(),
            sandbox_policy: permission_profile
                .to_legacy_sandbox_policy(self.config.cwd.as_path())
                .unwrap_or(SandboxPolicy::DangerFullAccess),
        }
    }

    fn btw_subagent_spawn(&self, agent_nickname: &str) -> Option<SubAgentSpawnParams> {
        let parent_thread_id = self.current_displayed_thread_id()?;
        Some(SubAgentSpawnParams {
            parent_thread_id: parent_thread_id.to_string(),
            agent_nickname: Some(agent_nickname.to_string()),
            agent_role: Some("btw".to_string()),
        })
    }

    async fn btw_fork_source_thread_id(
        &self,
        request_handle: &AppServerRequestHandle,
    ) -> Result<Option<ThreadId>, String> {
        let Some(thread_id) = self.current_displayed_thread_id() else {
            return Ok(None);
        };

        let response: ThreadReadResponse = match request_handle
            .request_typed(ClientRequest::ThreadRead {
                request_id: request_id(),
                params: ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns: false,
                },
            })
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    error = %err,
                    "failed to inspect `/btw` source thread; starting a fresh thread"
                );
                return Ok(None);
            }
        };

        Ok(response
            .thread
            .path
            .as_ref()
            .is_some_and(|path| path.exists())
            .then_some(thread_id))
    }

    async fn prime_btw_thread_channel(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
    ) {
        self.ensure_thread_channel(thread_id);
        match app_server
            .resume_thread(self.config.clone(), thread_id)
            .await
        {
            Ok(started) => {
                let channel = self.ensure_thread_channel(thread_id);
                let mut store = channel.store.lock().await;
                store.set_session(started.session, started.turns);
            }
            Err(err) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    error = %err,
                    "failed to prime `/btw` thread channel before switching"
                );
            }
        }
    }

    fn btw_thread_cwd(&self, app_server: &AppServerSession) -> Option<String> {
        if app_server.is_remote() {
            app_server
                .remote_cwd_override()
                .map(|cwd| cwd.to_string_lossy().to_string())
        } else {
            Some(self.config.cwd.to_string_lossy().to_string())
        }
    }

    fn btw_turn_cwd_path(&self, app_server: &AppServerSession) -> PathBuf {
        if app_server.is_remote() {
            app_server
                .remote_cwd_override()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.config.cwd.to_path_buf())
        } else {
            self.config.cwd.to_path_buf()
        }
    }
}

fn request_id() -> RequestId {
    RequestId::String(format!("btw-{}", Uuid::new_v4()))
}

fn btw_turn_input(prompt: &str) -> Vec<UserInput> {
    vec![UserInput::Text {
        text: prompt.to_string(),
        text_elements: Vec::new(),
    }]
}

pub(super) fn btw_thread_start_params(
    app: &App,
    app_server: &AppServerSession,
    permissions: &BtwPermissions,
    subagent_spawn: Option<&SubAgentSpawnParams>,
) -> ThreadStartParams {
    ThreadStartParams {
        model: Some(app.chat_widget.current_model().to_string()),
        model_provider: (!app_server.is_remote()).then_some(app.config.model_provider_id.clone()),
        cwd: app.btw_thread_cwd(app_server),
        approval_policy: Some(permissions.approval_policy),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(
            permissions.approvals_reviewer,
        )),
        sandbox: sandbox_mode_from_policy(permissions.sandbox_policy.clone()),
        config: config_overrides(app.active_profile.as_deref()),
        developer_instructions: Some(merge_developer_instructions(
            app.config.developer_instructions.as_deref(),
        )),
        personality: app.config.personality,
        subagent_spawn: subagent_spawn.cloned(),
        ..ThreadStartParams::default()
    }
}

pub(super) fn btw_thread_fork_params(
    app: &App,
    thread_id: ThreadId,
    app_server: &AppServerSession,
    permissions: &BtwPermissions,
    subagent_spawn: Option<&SubAgentSpawnParams>,
) -> ThreadForkParams {
    ThreadForkParams {
        thread_id: thread_id.to_string(),
        model: Some(app.chat_widget.current_model().to_string()),
        model_provider: (!app_server.is_remote()).then_some(app.config.model_provider_id.clone()),
        cwd: app.btw_thread_cwd(app_server),
        approval_policy: Some(permissions.approval_policy),
        approvals_reviewer: Some(AppServerApprovalsReviewer::from(
            permissions.approvals_reviewer,
        )),
        sandbox: sandbox_mode_from_policy(permissions.sandbox_policy.clone()),
        config: config_overrides(app.active_profile.as_deref()),
        developer_instructions: Some(merge_developer_instructions(
            app.config.developer_instructions.as_deref(),
        )),
        subagent_spawn: subagent_spawn.cloned(),
        ..ThreadForkParams::default()
    }
}

fn config_overrides(active_profile: Option<&str>) -> Option<HashMap<String, serde_json::Value>> {
    active_profile.map(|profile| {
        HashMap::from([(
            "profile".to_string(),
            serde_json::Value::String(profile.to_string()),
        )])
    })
}

fn sandbox_mode_from_policy(policy: SandboxPolicy) -> Option<SandboxMode> {
    match policy {
        SandboxPolicy::DangerFullAccess => Some(SandboxMode::DangerFullAccess),
        SandboxPolicy::ReadOnly { .. } => Some(SandboxMode::ReadOnly),
        SandboxPolicy::WorkspaceWrite { .. } => Some(SandboxMode::WorkspaceWrite),
        SandboxPolicy::ExternalSandbox { .. } => None,
    }
}

fn btw_agent_nickname(prompt: &str) -> String {
    let trimmed = prompt.trim();
    let char_count = trimmed.chars().count();
    if char_count <= 32 {
        return trimmed.to_string();
    }

    let truncated: String = trimmed.chars().take(31).collect();
    format!("{truncated}…")
}

fn merge_developer_instructions(existing: Option<&str>) -> String {
    match existing {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{BTW_DEVELOPER_INSTRUCTIONS}")
        }
        _ => BTW_DEVELOPER_INSTRUCTIONS.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::merge_developer_instructions;

    #[test]
    fn merge_developer_instructions_appends_btw_guardrail() {
        assert_eq!(
            merge_developer_instructions(Some("Stay focused.")),
            format!("Stay focused.\n\n{}", super::BTW_DEVELOPER_INSTRUCTIONS)
        );
    }
}
