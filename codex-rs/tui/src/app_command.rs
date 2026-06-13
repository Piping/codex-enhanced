use std::path::PathBuf;

use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::FileChangeApprovalDecision;
use codex_app_server_protocol::McpServerElicitationAction;
use codex_app_server_protocol::RequestId as AppServerRequestId;
use codex_app_server_protocol::ReviewTarget;
use codex_app_server_protocol::ThreadRealtimeAudioChunk;
use codex_app_server_protocol::ThreadRealtimeStartTransport;
use codex_app_server_protocol::ToolRequestUserInputResponse;
use codex_app_server_protocol::UserInput;
use codex_config::types::ApprovalsReviewer;
use codex_protocol::approvals::ElicitationAction;
use codex_protocol::approvals::GuardianAssessmentEvent;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::mcp::RequestId as CoreRequestId;
use codex_protocol::models::PermissionProfile;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::ConversationAudioParams;
use codex_protocol::protocol::ConversationStartParams;
use codex_protocol::protocol::ConversationStartTransport;
use codex_protocol::protocol::ConversationTextParams;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::RealtimeOutputModality;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::ReviewRequest;
use codex_protocol::protocol::ReviewTarget as CoreReviewTarget;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputAnswer as CoreRequestUserInputAnswer;
use codex_protocol::request_user_input::RequestUserInputResponse as CoreRequestUserInputResponse;
use serde::Serialize;
use serde_json::Value;

use crate::permission_compat::legacy_compatible_permission_profile;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) enum AppCommand {
    Interrupt,
    CleanBackgroundTerminals,
    RealtimeConversationStart(ConversationStartParams),
    RealtimeConversationAudio(ConversationAudioParams),
    RealtimeConversationText(ConversationTextParams),
    RealtimeConversationClose,
    RunUserShellCommand {
        command: String,
    },
    UserTurn {
        items: Vec<UserInput>,
        cwd: PathBuf,
        approval_policy: AskForApproval,
        approvals_reviewer: Option<ApprovalsReviewer>,
        permission_profile: PermissionProfile,
        sandbox_policy: SandboxPolicy,
        model: String,
        effort: Option<ReasoningEffortConfig>,
        summary: Option<ReasoningSummaryConfig>,
        service_tier: Option<Option<String>>,
        final_output_json_schema: Option<Value>,
        collaboration_mode: Option<CollaborationMode>,
        personality: Option<Personality>,
    },
    OverrideTurnContext {
        cwd: Option<PathBuf>,
        approval_policy: Option<AskForApproval>,
        approvals_reviewer: Option<ApprovalsReviewer>,
        permission_profile: Option<PermissionProfile>,
        sandbox_policy: Option<SandboxPolicy>,
        windows_sandbox_level: Option<WindowsSandboxLevel>,
        model: Option<String>,
        effort: Option<Option<ReasoningEffortConfig>>,
        summary: Option<ReasoningSummaryConfig>,
        service_tier: Option<Option<String>>,
        collaboration_mode: Option<CollaborationMode>,
        personality: Option<Personality>,
    },
    ExecApproval {
        id: String,
        turn_id: Option<String>,
        decision: CommandExecutionApprovalDecision,
    },
    PatchApproval {
        id: String,
        decision: FileChangeApprovalDecision,
    },
    ResolveElicitation {
        server_name: String,
        request_id: AppServerRequestId,
        decision: McpServerElicitationAction,
        content: Option<Value>,
        meta: Option<Value>,
    },
    UserInputAnswer {
        id: String,
        response: ToolRequestUserInputResponse,
    },
    RequestPermissionsResponse {
        id: String,
        response: RequestPermissionsResponse,
    },
    ReloadUserConfig,
    ListSkills {
        cwds: Vec<PathBuf>,
        force_reload: bool,
    },
    Compact,
    SetThreadName {
        name: String,
    },
    Shutdown,
    ThreadRollback {
        num_turns: u32,
    },
    Review {
        target: ReviewTarget,
    },
    ApproveGuardianDeniedAction {
        event: GuardianAssessmentEvent,
    },
    Other(Op),
}

#[allow(clippy::large_enum_variant)]
#[allow(dead_code)]
pub(crate) enum AppCommandView<'a> {
    Interrupt,
    CleanBackgroundTerminals,
    RealtimeConversationStart(&'a ConversationStartParams),
    RealtimeConversationAudio(&'a ConversationAudioParams),
    RealtimeConversationText(&'a ConversationTextParams),
    RealtimeConversationClose,
    RunUserShellCommand {
        command: &'a str,
    },
    UserTurn {
        items: &'a [UserInput],
        cwd: &'a PathBuf,
        approval_policy: AskForApproval,
        approvals_reviewer: &'a Option<ApprovalsReviewer>,
        sandbox_policy: &'a SandboxPolicy,
        model: &'a str,
        effort: Option<ReasoningEffortConfig>,
        summary: &'a Option<ReasoningSummaryConfig>,
        service_tier: &'a Option<Option<String>>,
        final_output_json_schema: &'a Option<Value>,
        collaboration_mode: &'a Option<CollaborationMode>,
        personality: &'a Option<Personality>,
    },
    OverrideTurnContext {
        cwd: &'a Option<PathBuf>,
        approval_policy: &'a Option<AskForApproval>,
        approvals_reviewer: &'a Option<ApprovalsReviewer>,
        permission_profile: &'a Option<PermissionProfile>,
        sandbox_policy: &'a Option<SandboxPolicy>,
        windows_sandbox_level: &'a Option<WindowsSandboxLevel>,
        model: &'a Option<String>,
        effort: &'a Option<Option<ReasoningEffortConfig>>,
        summary: &'a Option<ReasoningSummaryConfig>,
        service_tier: &'a Option<Option<String>>,
        collaboration_mode: &'a Option<CollaborationMode>,
        personality: &'a Option<Personality>,
    },
    ExecApproval {
        id: &'a str,
        turn_id: &'a Option<String>,
        decision: &'a CommandExecutionApprovalDecision,
    },
    PatchApproval {
        id: &'a str,
        decision: &'a FileChangeApprovalDecision,
    },
    ResolveElicitation {
        server_name: &'a str,
        request_id: &'a AppServerRequestId,
        decision: &'a McpServerElicitationAction,
        content: &'a Option<Value>,
        meta: &'a Option<Value>,
    },
    UserInputAnswer {
        id: &'a str,
        response: &'a ToolRequestUserInputResponse,
    },
    RequestPermissionsResponse {
        id: &'a str,
        response: &'a RequestPermissionsResponse,
    },
    ReloadUserConfig,
    ListSkills {
        cwds: &'a [PathBuf],
        force_reload: bool,
    },
    Compact,
    SetThreadName {
        name: &'a str,
    },
    Shutdown,
    ThreadRollback {
        num_turns: u32,
    },
    Review {
        target: &'a ReviewTarget,
    },
    ApproveGuardianDeniedAction {
        event: &'a GuardianAssessmentEvent,
    },
    Other(&'a Op),
}

impl AppCommand {
    pub(crate) fn interrupt() -> Self {
        Self::Interrupt
    }

    pub(crate) fn clean_background_terminals() -> Self {
        Self::CleanBackgroundTerminals
    }

    pub(crate) fn realtime_conversation_start(
        transport: Option<ThreadRealtimeStartTransport>,
        voice: Option<Value>,
    ) -> Self {
        Self::RealtimeConversationStart(ConversationStartParams {
            output_modality: RealtimeOutputModality::Audio,
            prompt: None,
            realtime_session_id: None,
            transport: transport.map(|transport| match transport {
                ThreadRealtimeStartTransport::Websocket => ConversationStartTransport::Websocket,
                ThreadRealtimeStartTransport::Webrtc { sdp } => {
                    ConversationStartTransport::Webrtc { sdp }
                }
            }),
            voice: voice.and_then(|voice| serde_json::from_value(voice).ok()),
        })
    }

    #[cfg_attr(not(feature = "voice"), allow(dead_code))]
    pub(crate) fn realtime_conversation_audio(frame: ThreadRealtimeAudioChunk) -> Self {
        Self::RealtimeConversationAudio(ConversationAudioParams {
            frame: frame.into(),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn realtime_conversation_text(text: String) -> Self {
        Self::RealtimeConversationText(ConversationTextParams { text })
    }

    pub(crate) fn realtime_conversation_close() -> Self {
        Self::RealtimeConversationClose
    }

    pub(crate) fn run_user_shell_command(command: String) -> Self {
        Self::RunUserShellCommand { command }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn user_turn(
        items: Vec<UserInput>,
        cwd: PathBuf,
        approval_policy: AskForApproval,
        permission_profile: PermissionProfile,
        model: String,
        effort: Option<ReasoningEffortConfig>,
        summary: Option<ReasoningSummaryConfig>,
        service_tier: Option<Option<String>>,
        final_output_json_schema: Option<Value>,
        collaboration_mode: Option<CollaborationMode>,
        personality: Option<Personality>,
    ) -> Self {
        let permission_profile =
            legacy_compatible_permission_profile(&permission_profile, cwd.as_path());
        let sandbox_policy = permission_profile
            .to_legacy_sandbox_policy(cwd.as_path())
            .unwrap_or_else(|err| {
                panic!("compatible permission profile must project to legacy sandbox policy: {err}")
            });

        Self::UserTurn {
            items,
            cwd,
            approval_policy,
            approvals_reviewer: None,
            permission_profile,
            sandbox_policy,
            model,
            effort,
            summary,
            service_tier,
            final_output_json_schema,
            collaboration_mode,
            personality,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn override_turn_context(
        cwd: Option<PathBuf>,
        approval_policy: Option<AskForApproval>,
        approvals_reviewer: Option<ApprovalsReviewer>,
        permission_profile: Option<PermissionProfile>,
        windows_sandbox_level: Option<WindowsSandboxLevel>,
        model: Option<String>,
        effort: Option<Option<ReasoningEffortConfig>>,
        summary: Option<ReasoningSummaryConfig>,
        service_tier: Option<Option<String>>,
        collaboration_mode: Option<CollaborationMode>,
        personality: Option<Personality>,
    ) -> Self {
        let sandbox_policy = match (cwd.as_ref(), permission_profile.as_ref()) {
            (Some(cwd), Some(permission_profile)) => permission_profile
                .to_legacy_sandbox_policy(cwd.as_path())
                .ok(),
            _ => None,
        };

        Self::OverrideTurnContext {
            cwd,
            approval_policy,
            approvals_reviewer,
            permission_profile,
            sandbox_policy,
            windows_sandbox_level,
            model,
            effort,
            summary,
            service_tier,
            collaboration_mode,
            personality,
        }
    }

    pub(crate) fn exec_approval(
        id: String,
        turn_id: Option<String>,
        decision: CommandExecutionApprovalDecision,
    ) -> Self {
        Self::ExecApproval {
            id,
            turn_id,
            decision,
        }
    }

    pub(crate) fn patch_approval(id: String, decision: FileChangeApprovalDecision) -> Self {
        Self::PatchApproval { id, decision }
    }

    pub(crate) fn resolve_elicitation(
        server_name: String,
        request_id: AppServerRequestId,
        decision: McpServerElicitationAction,
        content: Option<Value>,
        meta: Option<Value>,
    ) -> Self {
        Self::ResolveElicitation {
            server_name,
            request_id,
            decision,
            content,
            meta,
        }
    }

    pub(crate) fn user_input_answer(id: String, response: ToolRequestUserInputResponse) -> Self {
        Self::UserInputAnswer { id, response }
    }

    pub(crate) fn request_permissions_response(
        id: String,
        response: RequestPermissionsResponse,
    ) -> Self {
        Self::RequestPermissionsResponse { id, response }
    }

    pub(crate) fn reload_user_config() -> Self {
        Self::ReloadUserConfig
    }

    pub(crate) fn list_skills(cwds: Vec<PathBuf>, force_reload: bool) -> Self {
        Self::ListSkills { cwds, force_reload }
    }

    pub(crate) fn compact() -> Self {
        Self::Compact
    }

    pub(crate) fn set_thread_name(name: String) -> Self {
        Self::SetThreadName { name }
    }

    #[allow(dead_code)]
    pub(crate) fn shutdown() -> Self {
        Self::Shutdown
    }

    pub(crate) fn thread_rollback(num_turns: u32) -> Self {
        Self::ThreadRollback { num_turns }
    }

    pub(crate) fn review(target: ReviewTarget) -> Self {
        Self::Review { target }
    }

    pub(crate) fn approve_guardian_denied_action(event: GuardianAssessmentEvent) -> Self {
        Self::ApproveGuardianDeniedAction { event }
    }

    pub(crate) fn from_core(op: Op) -> Self {
        match op {
            Op::Interrupt => Self::Interrupt,
            Op::CleanBackgroundTerminals => Self::CleanBackgroundTerminals,
            Op::RealtimeConversationStart(params) => Self::RealtimeConversationStart(params),
            Op::RealtimeConversationAudio(params) => Self::RealtimeConversationAudio(params),
            Op::RealtimeConversationText(params) => Self::RealtimeConversationText(params),
            Op::RealtimeConversationClose => Self::RealtimeConversationClose,
            Op::RunUserShellCommand { command } => Self::RunUserShellCommand { command },
            Op::UserTurn {
                items,
                cwd,
                approval_policy,
                approvals_reviewer,
                sandbox_policy,
                permission_profile,
                model,
                effort,
                summary,
                service_tier,
                final_output_json_schema,
                collaboration_mode,
                personality,
                ..
            } => {
                let permission_profile = permission_profile.unwrap_or_else(|| {
                    PermissionProfile::from_legacy_sandbox_policy_for_cwd(&sandbox_policy, &cwd)
                });
                Self::UserTurn {
                    items: items.into_iter().map(Into::into).collect(),
                    cwd,
                    approval_policy: approval_policy.into(),
                    approvals_reviewer,
                    permission_profile,
                    sandbox_policy,
                    model,
                    effort,
                    summary,
                    service_tier,
                    final_output_json_schema,
                    collaboration_mode,
                    personality,
                }
            }
            Op::OverrideTurnContext {
                cwd,
                approval_policy,
                approvals_reviewer,
                sandbox_policy,
                permission_profile,
                windows_sandbox_level,
                model,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            } => Self::OverrideTurnContext {
                cwd,
                approval_policy: approval_policy.map(Into::into),
                approvals_reviewer,
                permission_profile,
                sandbox_policy,
                windows_sandbox_level,
                model,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            },
            Op::ExecApproval {
                id,
                turn_id,
                decision,
            } => Self::ExecApproval {
                id,
                turn_id,
                decision: decision.into(),
            },
            Op::PatchApproval { id, decision } => Self::PatchApproval {
                id,
                decision: core_review_decision_to_file_change(decision),
            },
            Op::ResolveElicitation {
                server_name,
                request_id,
                decision,
                content,
                meta,
            } => Self::ResolveElicitation {
                server_name,
                request_id: core_request_id_to_app_server_request_id(request_id),
                decision: core_elicitation_action_to_app_server_elicitation_action(decision),
                content,
                meta,
            },
            Op::UserInputAnswer { id, response } => Self::UserInputAnswer {
                id,
                response: core_request_user_input_response_to_tool_request_user_input_response(
                    response,
                ),
            },
            Op::RequestPermissionsResponse { id, response } => {
                Self::RequestPermissionsResponse { id, response }
            }
            Op::ReloadUserConfig => Self::ReloadUserConfig,
            Op::Compact => Self::Compact,
            Op::Shutdown => Self::Shutdown,
            Op::ThreadRollback { num_turns } => Self::ThreadRollback { num_turns },
            Op::Review { review_request } => Self::Review {
                target: core_review_target_to_app_server(review_request.target),
            },
            Op::ApproveGuardianDeniedAction { event } => {
                Self::ApproveGuardianDeniedAction { event }
            }
            other => Self::Other(other),
        }
    }

    pub(crate) fn is_review(&self) -> bool {
        matches!(self, Self::Review { .. })
    }

    #[allow(dead_code)]
    pub(crate) fn view(&self) -> AppCommandView<'_> {
        match self {
            Self::Interrupt => AppCommandView::Interrupt,
            Self::CleanBackgroundTerminals => AppCommandView::CleanBackgroundTerminals,
            Self::RealtimeConversationStart(params) => {
                AppCommandView::RealtimeConversationStart(params)
            }
            Self::RealtimeConversationAudio(params) => {
                AppCommandView::RealtimeConversationAudio(params)
            }
            Self::RealtimeConversationText(params) => {
                AppCommandView::RealtimeConversationText(params)
            }
            Self::RealtimeConversationClose => AppCommandView::RealtimeConversationClose,
            Self::RunUserShellCommand { command } => {
                AppCommandView::RunUserShellCommand { command }
            }
            Self::UserTurn {
                items,
                cwd,
                approval_policy,
                approvals_reviewer,
                sandbox_policy,
                model,
                effort,
                summary,
                service_tier,
                final_output_json_schema,
                collaboration_mode,
                personality,
                ..
            } => AppCommandView::UserTurn {
                items,
                cwd,
                approval_policy: *approval_policy,
                approvals_reviewer,
                sandbox_policy,
                model,
                effort: *effort,
                summary,
                service_tier,
                final_output_json_schema,
                collaboration_mode,
                personality,
            },
            Self::OverrideTurnContext {
                cwd,
                approval_policy,
                approvals_reviewer,
                permission_profile,
                sandbox_policy,
                windows_sandbox_level,
                model,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            } => AppCommandView::OverrideTurnContext {
                cwd,
                approval_policy,
                approvals_reviewer,
                permission_profile,
                sandbox_policy,
                windows_sandbox_level,
                model,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            },
            Self::ExecApproval {
                id,
                turn_id,
                decision,
            } => AppCommandView::ExecApproval {
                id,
                turn_id,
                decision,
            },
            Self::PatchApproval { id, decision } => AppCommandView::PatchApproval { id, decision },
            Self::ResolveElicitation {
                server_name,
                request_id,
                decision,
                content,
                meta,
            } => AppCommandView::ResolveElicitation {
                server_name,
                request_id,
                decision,
                content,
                meta,
            },
            Self::UserInputAnswer { id, response } => {
                AppCommandView::UserInputAnswer { id, response }
            }
            Self::RequestPermissionsResponse { id, response } => {
                AppCommandView::RequestPermissionsResponse { id, response }
            }
            Self::ReloadUserConfig => AppCommandView::ReloadUserConfig,
            Self::ListSkills { cwds, force_reload } => AppCommandView::ListSkills {
                cwds,
                force_reload: *force_reload,
            },
            Self::Compact => AppCommandView::Compact,
            Self::SetThreadName { name } => AppCommandView::SetThreadName { name },
            Self::Shutdown => AppCommandView::Shutdown,
            Self::ThreadRollback { num_turns } => AppCommandView::ThreadRollback {
                num_turns: *num_turns,
            },
            Self::Review { target } => AppCommandView::Review { target },
            Self::ApproveGuardianDeniedAction { event } => {
                AppCommandView::ApproveGuardianDeniedAction { event }
            }
            Self::Other(op) => AppCommandView::Other(op),
        }
    }

    pub(crate) fn into_core(self) -> Op {
        match self {
            Self::Interrupt => Op::Interrupt,
            Self::CleanBackgroundTerminals => Op::CleanBackgroundTerminals,
            Self::RealtimeConversationStart(params) => Op::RealtimeConversationStart(params),
            Self::RealtimeConversationAudio(params) => Op::RealtimeConversationAudio(params),
            Self::RealtimeConversationText(params) => Op::RealtimeConversationText(params),
            Self::RealtimeConversationClose => Op::RealtimeConversationClose,
            Self::RunUserShellCommand { command } => Op::RunUserShellCommand { command },
            Self::UserTurn {
                items,
                cwd,
                approval_policy,
                approvals_reviewer,
                permission_profile,
                sandbox_policy,
                model,
                effort,
                summary,
                service_tier,
                final_output_json_schema,
                collaboration_mode,
                personality,
            } => Op::UserTurn {
                items: items.into_iter().map(UserInput::into_core).collect(),
                cwd,
                approval_policy: approval_policy.to_core(),
                approvals_reviewer,
                sandbox_policy,
                permission_profile: Some(permission_profile),
                model,
                effort,
                summary,
                service_tier,
                final_output_json_schema,
                collaboration_mode,
                personality,
                environments: None,
            },
            Self::OverrideTurnContext {
                cwd,
                approval_policy,
                approvals_reviewer,
                permission_profile,
                sandbox_policy,
                windows_sandbox_level,
                model,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            } => Op::OverrideTurnContext {
                cwd,
                approval_policy: approval_policy.map(AskForApproval::to_core),
                approvals_reviewer,
                sandbox_policy,
                permission_profile,
                windows_sandbox_level,
                model,
                effort,
                summary,
                service_tier,
                collaboration_mode,
                personality,
            },
            Self::ExecApproval {
                id,
                turn_id,
                decision,
            } => Op::ExecApproval {
                id,
                turn_id,
                decision: command_execution_decision_to_core_review_decision(decision),
            },
            Self::PatchApproval { id, decision } => Op::PatchApproval {
                id,
                decision: file_change_approval_decision_to_core_review_decision(decision),
            },
            Self::ResolveElicitation {
                server_name,
                request_id,
                decision,
                content,
                meta,
            } => Op::ResolveElicitation {
                server_name,
                request_id: core_app_server_request_id_to_core_request_id(request_id),
                decision: elicitation_action_to_core_elicitation_action(decision),
                content,
                meta,
            },
            Self::UserInputAnswer { id, response } => Op::UserInputAnswer {
                id,
                response: tool_request_user_input_response_to_core_request_user_input_response(
                    response,
                ),
            },
            Self::RequestPermissionsResponse { id, response } => {
                Op::RequestPermissionsResponse { id, response }
            }
            Self::ReloadUserConfig => Op::ReloadUserConfig,
            Self::Compact => Op::Compact,
            Self::Shutdown => Op::Shutdown,
            Self::ThreadRollback { num_turns } => Op::ThreadRollback { num_turns },
            Self::Review { target } => Op::Review {
                review_request: ReviewRequest {
                    target: app_server_review_target_to_core(target),
                    user_facing_hint: None,
                },
            },
            Self::ApproveGuardianDeniedAction { event } => {
                Op::ApproveGuardianDeniedAction { event }
            }
            Self::ListSkills { .. } | Self::SetThreadName { .. } => {
                unreachable!("app-server-only command cannot convert into a core op")
            }
            Self::Other(op) => op,
        }
    }
}

fn core_request_id_to_app_server_request_id(id: CoreRequestId) -> AppServerRequestId {
    match id {
        CoreRequestId::String(value) => AppServerRequestId::String(value),
        CoreRequestId::Integer(value) => AppServerRequestId::Integer(value),
    }
}

fn core_app_server_request_id_to_core_request_id(id: AppServerRequestId) -> CoreRequestId {
    match id {
        AppServerRequestId::String(value) => CoreRequestId::String(value),
        AppServerRequestId::Integer(value) => CoreRequestId::Integer(value),
    }
}

fn elicitation_action_to_core_elicitation_action(
    decision: McpServerElicitationAction,
) -> ElicitationAction {
    match decision {
        McpServerElicitationAction::Accept => ElicitationAction::Accept,
        McpServerElicitationAction::Decline => ElicitationAction::Decline,
        McpServerElicitationAction::Cancel => ElicitationAction::Cancel,
    }
}

fn core_elicitation_action_to_app_server_elicitation_action(
    decision: ElicitationAction,
) -> McpServerElicitationAction {
    match decision {
        ElicitationAction::Accept => McpServerElicitationAction::Accept,
        ElicitationAction::Decline => McpServerElicitationAction::Decline,
        ElicitationAction::Cancel => McpServerElicitationAction::Cancel,
    }
}

fn command_execution_decision_to_core_review_decision(
    decision: CommandExecutionApprovalDecision,
) -> ReviewDecision {
    match decision {
        CommandExecutionApprovalDecision::Accept => ReviewDecision::Approved,
        CommandExecutionApprovalDecision::AcceptForSession => ReviewDecision::ApprovedForSession,
        CommandExecutionApprovalDecision::AcceptWithExecpolicyAmendment {
            execpolicy_amendment,
        } => ReviewDecision::ApprovedExecpolicyAmendment {
            proposed_execpolicy_amendment: execpolicy_amendment.into_core(),
        },
        CommandExecutionApprovalDecision::ApplyNetworkPolicyAmendment {
            network_policy_amendment,
        } => ReviewDecision::NetworkPolicyAmendment {
            network_policy_amendment: network_policy_amendment.into_core(),
        },
        CommandExecutionApprovalDecision::Decline => ReviewDecision::Denied,
        CommandExecutionApprovalDecision::Cancel => ReviewDecision::Abort,
    }
}

fn file_change_approval_decision_to_core_review_decision(
    decision: FileChangeApprovalDecision,
) -> ReviewDecision {
    match decision {
        FileChangeApprovalDecision::Accept | FileChangeApprovalDecision::AcceptForSession => {
            ReviewDecision::Approved
        }
        FileChangeApprovalDecision::Decline => ReviewDecision::Denied,
        FileChangeApprovalDecision::Cancel => ReviewDecision::Abort,
    }
}

fn app_server_review_target_to_core(target: ReviewTarget) -> CoreReviewTarget {
    match target {
        ReviewTarget::UncommittedChanges => CoreReviewTarget::UncommittedChanges,
        ReviewTarget::BaseBranch { branch } => CoreReviewTarget::BaseBranch { branch },
        ReviewTarget::Commit { sha, title } => CoreReviewTarget::Commit { sha, title },
        ReviewTarget::Custom { instructions } => CoreReviewTarget::Custom { instructions },
    }
}

fn core_review_target_to_app_server(target: CoreReviewTarget) -> ReviewTarget {
    match target {
        CoreReviewTarget::UncommittedChanges => ReviewTarget::UncommittedChanges,
        CoreReviewTarget::BaseBranch { branch } => ReviewTarget::BaseBranch { branch },
        CoreReviewTarget::Commit { sha, title } => ReviewTarget::Commit { sha, title },
        CoreReviewTarget::Custom { instructions } => ReviewTarget::Custom { instructions },
    }
}

fn tool_request_user_input_response_to_core_request_user_input_response(
    response: ToolRequestUserInputResponse,
) -> CoreRequestUserInputResponse {
    CoreRequestUserInputResponse {
        answers: response
            .answers
            .into_iter()
            .map(|(id, answer)| {
                (
                    id,
                    CoreRequestUserInputAnswer {
                        answers: answer.answers,
                    },
                )
            })
            .collect(),
    }
}

fn core_request_user_input_response_to_tool_request_user_input_response(
    response: CoreRequestUserInputResponse,
) -> ToolRequestUserInputResponse {
    ToolRequestUserInputResponse {
        answers: response
            .answers
            .into_iter()
            .map(|(id, answer)| {
                (
                    id,
                    codex_app_server_protocol::ToolRequestUserInputAnswer {
                        answers: answer.answers,
                    },
                )
            })
            .collect(),
    }
}

fn core_review_decision_to_file_change(decision: ReviewDecision) -> FileChangeApprovalDecision {
    match decision {
        ReviewDecision::Approved | ReviewDecision::ApprovedForSession => {
            FileChangeApprovalDecision::Accept
        }
        ReviewDecision::Denied => FileChangeApprovalDecision::Decline,
        ReviewDecision::TimedOut | ReviewDecision::Abort => FileChangeApprovalDecision::Cancel,
        ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::NetworkPolicyAmendment { .. } => FileChangeApprovalDecision::Accept,
    }
}

impl From<&AppCommand> for AppCommand {
    fn from(value: &AppCommand) -> Self {
        value.clone()
    }
}

impl From<Op> for AppCommand {
    fn from(value: Op) -> Self {
        Self::from_core(value)
    }
}

impl From<&Op> for AppCommand {
    fn from(value: &Op) -> Self {
        Self::from_core(value.clone())
    }
}

impl From<AppCommand> for Op {
    fn from(value: AppCommand) -> Self {
        value.into_core()
    }
}
