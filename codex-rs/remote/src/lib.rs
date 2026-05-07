use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::Path as StdPath;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use codex_app_server_client::AppServerClient;
use codex_app_server_client::AppServerEvent;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::CommandExecutionRequestApprovalParams;
use codex_app_server_protocol::CommandExecutionRequestApprovalResponse;
use codex_app_server_protocol::FileChangeApprovalDecision;
use codex_app_server_protocol::FileChangeRequestApprovalParams;
use codex_app_server_protocol::FileChangeRequestApprovalResponse;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::Thread;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadStatus;
use codex_app_server_protocol::TurnInterruptParams;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput;
use codex_core::config::Config;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_rollout::RolloutConfig;
use codex_thread_store::ListThreadsParams;
use codex_thread_store::LocalThreadStore;
use codex_thread_store::ReadThreadParams;
use codex_thread_store::StoredThread;
use codex_thread_store::ThreadSortKey;
use codex_thread_store::ThreadStore;
use codex_thread_store::ThreadStoreError;
use gethostname::gethostname;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio::time::sleep;
use tokio::time::timeout;
use tracing::info;
use tracing::warn;

#[derive(Clone)]
pub struct AppState {
    config: Config,
    store: LocalThreadStore,
    server_name: String,
    runtime: Arc<RemoteRuntime>,
}

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub base_url: String,
    pub workspace_id: String,
    pub shared_secret: String,
    pub sync_interval: Duration,
}

impl AppState {
    pub async fn load() -> anyhow::Result<Self> {
        let config = Config::load_with_cli_overrides(Vec::new())
            .await
            .context("failed to load Codex config")?;
        let store = LocalThreadStore::new(RolloutConfig::from_view(&config));
        let runtime = RemoteRuntime::start(config.clone()).await?;
        let server_name = gethostname().to_string_lossy().trim().to_string();

        Ok(Self {
            config,
            store,
            server_name,
            runtime,
        })
    }

    pub fn start_relay_sync(&self, config: RelayConfig) {
        let state = self.clone();
        tokio::spawn(async move {
            if let Err(err) = run_relay_sync(state, config).await {
                warn!("relay sync task exited: {err:?}");
            }
        });
    }

    fn host_response(&self) -> HostResponse {
        HostResponse {
            server_name: self.server_name.clone(),
            codex_home: self.config.codex_home.display().to_string(),
            cwd: self.config.cwd.display().to_string(),
            model_provider_id: self.config.model_provider_id.clone(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    async fn list_sessions_snapshot(&self) -> Result<SessionListResponse, ApiError> {
        let page = self
            .store
            .list_threads(ListThreadsParams {
                page_size: 100,
                cursor: None,
                sort_key: ThreadSortKey::UpdatedAt,
                allowed_sources: Vec::new(),
                model_providers: None,
                archived: false,
                search_term: None,
            })
            .await
            .map_err(ApiError::from_thread_store)?;

        Ok(SessionListResponse {
            items: page.items.into_iter().map(SessionSummary::from).collect(),
            next_cursor: page.next_cursor,
        })
    }

    async fn session_detail_snapshot(
        &self,
        id: &str,
        include_history: bool,
        activate: bool,
    ) -> Result<SessionDetailResponse, ApiError> {
        let stored_thread = self
            .store
            .read_thread(ReadThreadParams {
                thread_id: id.try_into().map_err(|_| ApiError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("invalid thread id: {id}"),
                })?,
                include_archived: true,
                include_history,
            })
            .await
            .map_err(ApiError::from_thread_store)?;

        let mut response = SessionDetailResponse::from(stored_thread);

        if activate {
            self.runtime.ensure_thread_loaded(id).await?;
        }

        if activate || self.runtime.is_thread_loaded(id).await {
            let thread = self.runtime.read_thread(id, include_history).await?;
            response.messages = build_runtime_messages(&thread);
            response.runtime_status = Some(SessionRuntimeStatus::from(&thread.status));
            response.pending_approvals = self.runtime.pending_approvals_for(id).await;
        }

        Ok(response)
    }

    async fn send_session_message_action(
        &self,
        id: &str,
        text: String,
    ) -> Result<MessageMutationResponse, ApiError> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(ApiError::bad_request("message text cannot be empty"));
        }

        self.runtime.ensure_thread_loaded(id).await?;
        let thread = self.runtime.read_thread(id, true).await?;
        if matches!(thread.status, ThreadStatus::Active { .. }) {
            return Err(ApiError::conflict(
                "thread is currently active; interrupt it before sending another message",
            ));
        }

        let response: TurnStartResponse = self
            .runtime
            .request_typed(ClientRequest::TurnStart {
                request_id: self.runtime.next_request_id(),
                params: TurnStartParams {
                    thread_id: id.to_string(),
                    input: vec![UserInput::Text {
                        text,
                        text_elements: Vec::new(),
                    }],
                    ..Default::default()
                },
            })
            .await?;

        Ok(MessageMutationResponse {
            ok: true,
            turn_id: Some(response.turn.id),
        })
    }

    async fn interrupt_session_action(
        &self,
        id: &str,
    ) -> Result<MessageMutationResponse, ApiError> {
        self.runtime.ensure_thread_loaded(id).await?;
        let thread = self.runtime.read_thread(id, true).await?;
        let turn_id = thread
            .turns
            .iter()
            .find(|turn| {
                matches!(
                    turn.status,
                    codex_app_server_protocol::TurnStatus::InProgress
                )
            })
            .map(|turn| turn.id.clone())
            .unwrap_or_default();

        let _: TurnInterruptResponse = self
            .runtime
            .request_typed(ClientRequest::TurnInterrupt {
                request_id: self.runtime.next_request_id(),
                params: TurnInterruptParams {
                    thread_id: id.to_string(),
                    turn_id,
                },
            })
            .await?;

        Ok(MessageMutationResponse {
            ok: true,
            turn_id: None,
        })
    }

    async fn resolve_approval_action(
        &self,
        id: &str,
        request_id: &str,
        decision: &str,
    ) -> Result<MessageMutationResponse, ApiError> {
        let approval = self
            .runtime
            .find_pending_approval(id, request_id)
            .await
            .ok_or_else(|| ApiError {
                status: StatusCode::NOT_FOUND,
                message: format!("pending approval {request_id} not found"),
            })?;

        let decision = decision.trim().to_lowercase();
        match approval {
            PendingApprovalState::Command { .. } => {
                let api_response = CommandExecutionRequestApprovalResponse {
                    decision: match decision.as_str() {
                        "approve" => CommandExecutionApprovalDecision::Accept,
                        "deny" => CommandExecutionApprovalDecision::Decline,
                        _ => {
                            return Err(ApiError::bad_request(
                                "decision must be either `approve` or `deny`",
                            ));
                        }
                    },
                };
                self.runtime
                    .resolve_server_request(
                        parse_request_id_key(request_id)?,
                        serde_json::to_value(api_response)
                            .map_err(|err| ApiError::internal(err.to_string()))?,
                    )
                    .await?;
            }
            PendingApprovalState::FileChange { .. } => {
                let api_response = FileChangeRequestApprovalResponse {
                    decision: match decision.as_str() {
                        "approve" => FileChangeApprovalDecision::Accept,
                        "deny" => FileChangeApprovalDecision::Decline,
                        _ => {
                            return Err(ApiError::bad_request(
                                "decision must be either `approve` or `deny`",
                            ));
                        }
                    },
                };
                self.runtime
                    .resolve_server_request(
                        parse_request_id_key(request_id)?,
                        serde_json::to_value(api_response)
                            .map_err(|err| ApiError::internal(err.to_string()))?,
                    )
                    .await?;
            }
        }

        self.runtime.remove_pending_approval(id, request_id).await;

        Ok(MessageMutationResponse {
            ok: true,
            turn_id: None,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionListQuery {
    pub page_size: Option<usize>,
    pub cursor: Option<String>,
    pub archived: Option<bool>,
    pub search: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDetailQuery {
    pub include_archived: Option<bool>,
    pub include_history: Option<bool>,
    pub activate: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendMessageRequest {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApprovalDecisionRequest {
    pub decision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResponse {
    pub server_name: String,
    pub codex_home: String,
    pub cwd: String,
    pub model_provider_id: String,
    pub version: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResponse {
    pub items: Vec<SessionSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub model_provider: String,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub cwd: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetailResponse {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub first_user_message: Option<String>,
    pub model_provider: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub cwd: String,
    pub cli_version: String,
    pub source: String,
    pub approval_mode: String,
    pub sandbox_policy: String,
    pub forked_from_id: Option<String>,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub git_branch: Option<String>,
    pub git_commit: Option<String>,
    pub messages: Vec<SessionMessage>,
    pub runtime_status: Option<SessionRuntimeStatus>,
    pub pending_approvals: Vec<PendingApproval>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeStatus {
    pub kind: String,
    pub active_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub request_id: String,
    pub kind: String,
    pub turn_id: String,
    pub item_id: String,
    pub reason: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub grant_root: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMutationResponse {
    pub ok: bool,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    fn from_thread_store(err: ThreadStoreError) -> Self {
        match err {
            ThreadStoreError::ThreadNotFound { thread_id } => Self {
                status: StatusCode::NOT_FOUND,
                message: format!("thread {thread_id} not found"),
            },
            ThreadStoreError::InvalidRequest { message } => Self {
                status: StatusCode::BAD_REQUEST,
                message,
            },
            ThreadStoreError::Conflict { message } => Self {
                status: StatusCode::CONFLICT,
                message,
            },
            ThreadStoreError::Internal { message } => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/readyz", get(readyz))
        .route("/api/v1/host", get(get_host))
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{id}", get(get_session))
        .route("/api/v1/sessions/{id}/messages", post(post_session_message))
        .route("/api/v1/sessions/{id}/interrupt", post(interrupt_session))
        .route(
            "/api/v1/sessions/{id}/approvals/{request_id}",
            post(resolve_session_approval),
        )
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    info!("codex-remote listening on http://{addr}");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("codex-remote server exited unexpectedly")
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn readyz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn get_host(State(state): State<AppState>) -> Json<HostResponse> {
    Json(state.host_response())
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<SessionListResponse>, ApiError> {
    if query.page_size.is_none()
        && query.cursor.is_none()
        && query.archived.is_none()
        && query.search.is_none()
        && query.sort.is_none()
    {
        return state.list_sessions_snapshot().await.map(Json);
    }

    let page = state
        .store
        .list_threads(ListThreadsParams {
            page_size: query.page_size.unwrap_or(50).clamp(1, 100),
            cursor: query.cursor,
            sort_key: parse_sort_key(query.sort.as_deref()),
            allowed_sources: Vec::new(),
            model_providers: None,
            archived: query.archived.unwrap_or(false),
            search_term: normalize_query_text(query.search),
        })
        .await
        .map_err(ApiError::from_thread_store)?;

    Ok(Json(SessionListResponse {
        items: page.items.into_iter().map(SessionSummary::from).collect(),
        next_cursor: page.next_cursor,
    }))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SessionDetailQuery>,
) -> Result<Json<SessionDetailResponse>, ApiError> {
    state
        .session_detail_snapshot(
            &id,
            query.include_history.unwrap_or(true),
            query.activate.unwrap_or(false),
        )
        .await
        .map(Json)
}

async fn post_session_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendMessageRequest>,
) -> Result<Json<MessageMutationResponse>, ApiError> {
    state
        .send_session_message_action(&id, body.text)
        .await
        .map(Json)
}

async fn interrupt_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MessageMutationResponse>, ApiError> {
    state.interrupt_session_action(&id).await.map(Json)
}

async fn resolve_session_approval(
    State(state): State<AppState>,
    Path((id, request_id)): Path<(String, String)>,
    Json(body): Json<ApprovalDecisionRequest>,
) -> Result<Json<MessageMutationResponse>, ApiError> {
    state
        .resolve_approval_action(&id, &request_id, &body.decision)
        .await
        .map(Json)
}

impl ApiError {
    fn from_app_server_request(err: codex_app_server_client::TypedRequestError) -> Self {
        let message = err.to_string();
        if message.contains("not found") {
            return Self {
                status: StatusCode::NOT_FOUND,
                message,
            };
        }
        if message.contains("failed:") {
            return Self::bad_request(message);
        }
        Self::internal(message)
    }
}

fn parse_sort_key(value: Option<&str>) -> ThreadSortKey {
    match value {
        Some("created") | Some("created_at") | Some("createdAt") => ThreadSortKey::CreatedAt,
        _ => ThreadSortKey::UpdatedAt,
    }
}

fn normalize_query_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

impl From<StoredThread> for SessionSummary {
    fn from(value: StoredThread) -> Self {
        Self {
            id: value.thread_id.to_string(),
            name: value.name,
            preview: value.preview,
            model_provider: value.model_provider,
            model: value.model,
            created_at: value.created_at.to_rfc3339(),
            updated_at: value.updated_at.to_rfc3339(),
            archived_at: value.archived_at.map(|item| item.to_rfc3339()),
            cwd: path_to_string(value.cwd),
            source: format!("{:?}", value.source),
        }
    }
}

impl From<StoredThread> for SessionDetailResponse {
    fn from(value: StoredThread) -> Self {
        Self {
            id: value.thread_id.to_string(),
            name: value.name,
            preview: value.preview,
            first_user_message: value.first_user_message,
            model_provider: value.model_provider,
            model: value.model,
            reasoning_effort: value.reasoning_effort.map(|item| format!("{item:?}")),
            created_at: value.created_at.to_rfc3339(),
            updated_at: value.updated_at.to_rfc3339(),
            archived_at: value.archived_at.map(|item| item.to_rfc3339()),
            cwd: path_to_string(value.cwd),
            cli_version: value.cli_version,
            source: format!("{:?}", value.source),
            approval_mode: format!("{:?}", value.approval_mode),
            sandbox_policy: format!("{:?}", value.sandbox_policy),
            forked_from_id: value.forked_from_id.map(|item| item.to_string()),
            agent_nickname: value.agent_nickname,
            agent_role: value.agent_role,
            git_branch: value.git_info.as_ref().and_then(|item| item.branch.clone()),
            git_commit: value
                .git_info
                .as_ref()
                .and_then(|item| item.commit_hash.as_ref().map(|sha| sha.0.clone())),
            messages: build_stored_session_messages(value.history),
            runtime_status: None,
            pending_approvals: Vec::new(),
        }
    }
}

impl From<&ThreadStatus> for SessionRuntimeStatus {
    fn from(value: &ThreadStatus) -> Self {
        match value {
            ThreadStatus::NotLoaded => Self {
                kind: "notLoaded".to_string(),
                active_flags: Vec::new(),
            },
            ThreadStatus::Idle => Self {
                kind: "idle".to_string(),
                active_flags: Vec::new(),
            },
            ThreadStatus::SystemError => Self {
                kind: "systemError".to_string(),
                active_flags: Vec::new(),
            },
            ThreadStatus::Active { active_flags } => Self {
                kind: "active".to_string(),
                active_flags: active_flags
                    .iter()
                    .map(|flag| match flag {
                        codex_app_server_protocol::ThreadActiveFlag::WaitingOnApproval => {
                            "waitingOnApproval".to_string()
                        }
                        codex_app_server_protocol::ThreadActiveFlag::WaitingOnUserInput => {
                            "waitingOnUserInput".to_string()
                        }
                    })
                    .collect(),
            },
        }
    }
}

fn path_to_string(path: PathBuf) -> String {
    path.display().to_string()
}

fn build_stored_session_messages(
    history: Option<codex_thread_store::StoredThreadHistory>,
) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let Some(history) = history else {
        return messages;
    };

    for item in history.items {
        match item {
            RolloutItem::EventMsg(EventMsg::UserMessage(event)) => {
                push_message(&mut messages, "user", event.message);
            }
            RolloutItem::EventMsg(EventMsg::AgentMessage(event)) => {
                push_message(&mut messages, "assistant", event.message);
            }
            RolloutItem::ResponseItem(ResponseItem::Message { role, content, .. }) => {
                if role != "user" && role != "assistant" {
                    continue;
                }
                let text = content_to_text(&content);
                push_message(&mut messages, role.as_str(), text);
            }
            _ => {}
        }
    }

    messages
}

fn build_runtime_messages(thread: &Thread) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    for turn in &thread.turns {
        for item in &turn.items {
            match item {
                codex_app_server_protocol::ThreadItem::UserMessage { content, .. } => {
                    push_message(&mut messages, "user", user_inputs_to_text(content));
                }
                codex_app_server_protocol::ThreadItem::AgentMessage { text, .. } => {
                    push_message(&mut messages, "assistant", text.clone());
                }
                _ => {}
            }
        }
    }
    messages
}

fn user_inputs_to_text(inputs: &[UserInput]) -> String {
    inputs
        .iter()
        .filter_map(|input| match input {
            UserInput::Text { text, .. } => Some(text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn content_to_text(content: &[ContentItem]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => Some(text.trim()),
            ContentItem::InputImage { .. } => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn push_message(messages: &mut Vec<SessionMessage>, role: &str, text: String) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    if let Some(last) = messages.last()
        && last.role == role
        && last.text == trimmed
    {
        return;
    }

    messages.push(SessionMessage {
        role: role.to_string(),
        text: trimmed.to_string(),
    });
}

struct RemoteRuntime {
    client: Arc<Mutex<AppServerClient>>,
    _app_server: BackgroundAppServer,
    loaded_threads: Arc<RwLock<HashSet<String>>>,
    pending_approvals: Arc<RwLock<HashMap<String, Vec<PendingApprovalState>>>>,
    next_request_id: AtomicI64,
}

impl RemoteRuntime {
    async fn start(mut config: Config) -> anyhow::Result<Arc<Self>> {
        let codex_bin = resolve_codex_self_exe(&config)
            .context("failed to locate `codex` executable for codex-remote app-server bridge")?;
        config.codex_self_exe = Some(codex_bin.clone());

        let app_server = BackgroundAppServer::spawn(&codex_bin)
            .with_context(|| format!("failed to start `{}` app-server", codex_bin.display()))?;
        let websocket_url = app_server.websocket_url.clone();
        let client = connect_remote_app_server(&websocket_url)
            .await
            .with_context(|| format!("failed to connect to app-server at `{websocket_url}`"))?;

        let runtime = Arc::new(Self {
            client: Arc::new(Mutex::new(AppServerClient::Remote(client))),
            _app_server: app_server,
            loaded_threads: Arc::new(RwLock::new(HashSet::new())),
            pending_approvals: Arc::new(RwLock::new(HashMap::new())),
            next_request_id: AtomicI64::new(1),
        });

        Ok(runtime)
    }

    fn next_request_id(&self) -> RequestId {
        RequestId::Integer(self.next_request_id.fetch_add(1, Ordering::Relaxed))
    }

    async fn handle_event(&self, event: AppServerEvent) {
        match event {
            AppServerEvent::ServerRequest(request) => match request {
                ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
                    let thread_id = params.thread_id.clone();
                    self.insert_pending_approval(
                        &thread_id,
                        PendingApprovalState::Command {
                            request_id: String::new(),
                            params,
                        },
                        &request_id,
                    )
                    .await;
                }
                ServerRequest::FileChangeRequestApproval { request_id, params } => {
                    let thread_id = params.thread_id.clone();
                    self.insert_pending_approval(
                        &thread_id,
                        PendingApprovalState::FileChange {
                            request_id: String::new(),
                            params,
                        },
                        &request_id,
                    )
                    .await;
                }
                other => {
                    let request_id = other.id().clone();
                    let message = format!("unsupported server request: {other:?}");
                    if let Err(err) = self
                        .reject_server_request(
                            request_id,
                            JSONRPCErrorError {
                                code: -32000,
                                message,
                                data: None,
                            },
                        )
                        .await
                    {
                        warn!("failed to reject unsupported server request: {err:?}");
                    }
                }
            },
            AppServerEvent::ServerNotification(ServerNotification::ServerRequestResolved(
                payload,
            )) => {
                self.remove_pending_approval(
                    &payload.thread_id,
                    &request_id_key(&payload.request_id),
                )
                .await;
            }
            AppServerEvent::Disconnected { message } => {
                warn!("embedded app server disconnected: {message}");
            }
            AppServerEvent::Lagged { skipped } => {
                warn!("embedded app server event stream lagged; skipped {skipped} events");
            }
            _ => {}
        }
    }

    async fn ensure_thread_loaded(&self, thread_id: &str) -> Result<(), ApiError> {
        self.drain_events().await;
        if self.is_thread_loaded(thread_id).await {
            return Ok(());
        }

        let _: ThreadResumeResponse = self
            .request_typed(ClientRequest::ThreadResume {
                request_id: self.next_request_id(),
                params: ThreadResumeParams {
                    thread_id: thread_id.to_string(),
                    persist_extended_history: true,
                    ..Default::default()
                },
            })
            .await?;

        self.loaded_threads
            .write()
            .await
            .insert(thread_id.to_string());
        Ok(())
    }

    async fn read_thread(&self, thread_id: &str, include_turns: bool) -> Result<Thread, ApiError> {
        self.drain_events().await;
        let response: ThreadReadResponse = self
            .request_typed(ClientRequest::ThreadRead {
                request_id: self.next_request_id(),
                params: ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns,
                },
            })
            .await?;
        Ok(response.thread)
    }

    async fn is_thread_loaded(&self, thread_id: &str) -> bool {
        self.loaded_threads.read().await.contains(thread_id)
    }

    async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, ApiError>
    where
        T: serde::de::DeserializeOwned,
    {
        self.drain_events().await;
        let client = self.client.lock().await;
        client
            .request_typed(request)
            .await
            .map_err(ApiError::from_app_server_request)
    }

    async fn resolve_server_request(
        &self,
        request_id: RequestId,
        result: serde_json::Value,
    ) -> Result<(), ApiError> {
        self.drain_events().await;
        let client = self.client.lock().await;
        client
            .resolve_server_request(request_id, result)
            .await
            .map_err(|err| ApiError::internal(err.to_string()))
    }

    async fn reject_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> Result<(), ApiError> {
        let client = self.client.lock().await;
        client
            .reject_server_request(request_id, error)
            .await
            .map_err(|err| ApiError::internal(err.to_string()))
    }

    async fn drain_events(&self) {
        loop {
            let event = {
                let mut client = self.client.lock().await;
                timeout(Duration::from_millis(5), client.next_event()).await
            };
            match event {
                Ok(Some(event)) => self.handle_event(event.into()).await,
                Ok(None) => break,
                Err(_) => break,
            }
        }
    }

    async fn insert_pending_approval(
        &self,
        thread_id: &str,
        approval: PendingApprovalState,
        request_id: &RequestId,
    ) {
        let mut approvals = self.pending_approvals.write().await;
        let entries = approvals.entry(thread_id.to_string()).or_default();
        let key = request_id_key(request_id);
        if entries.iter().any(|entry| entry.request_id() == key) {
            return;
        }
        entries.push(approval.with_request_id(key));
    }

    async fn pending_approvals_for(&self, thread_id: &str) -> Vec<PendingApproval> {
        self.pending_approvals
            .read()
            .await
            .get(thread_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.to_api())
            .collect()
    }

    async fn find_pending_approval(
        &self,
        thread_id: &str,
        request_id: &str,
    ) -> Option<PendingApprovalState> {
        self.pending_approvals
            .read()
            .await
            .get(thread_id)
            .and_then(|items| items.iter().find(|item| item.request_id() == request_id))
            .cloned()
    }

    async fn remove_pending_approval(&self, thread_id: &str, request_id: &str) {
        let mut approvals = self.pending_approvals.write().await;
        let Some(items) = approvals.get_mut(thread_id) else {
            return;
        };
        items.retain(|item| item.request_id() != request_id);
        if items.is_empty() {
            approvals.remove(thread_id);
        }
    }
}

struct BackgroundAppServer {
    process: Child,
    websocket_url: String,
}

impl BackgroundAppServer {
    fn spawn(codex_bin: &StdPath) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .context("failed to reserve a local port for websocket app-server")?;
        let addr = listener.local_addr()?;
        drop(listener);

        let websocket_url = format!("ws://{addr}");
        let mut command = Command::new(codex_bin);
        if let Some(parent) = codex_bin.parent() {
            let mut path = std::ffi::OsString::from(parent.as_os_str());
            if let Some(existing_path) = std::env::var_os("PATH") {
                path.push(":");
                path.push(existing_path);
            }
            command.env("PATH", path);
        }

        let process = command
            .arg("app-server")
            .arg("--listen")
            .arg(&websocket_url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;

        Ok(Self {
            process,
            websocket_url,
        })
    }
}

impl Drop for BackgroundAppServer {
    fn drop(&mut self) {
        if let Ok(Some(status)) = self.process.try_wait() {
            info!("background app-server exited with status {status}");
            return;
        }

        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

async fn connect_remote_app_server(websocket_url: &str) -> anyhow::Result<RemoteAppServerClient> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let last_error = loop {
        match RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
            websocket_url: websocket_url.to_string(),
            auth_token: None,
            client_name: "codex-remote".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            experimental_api: true,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await
        {
            Ok(client) => return Ok(client),
            Err(err) => {
                if Instant::now() >= deadline {
                    break err.to_string();
                }
                sleep(Duration::from_millis(100)).await;
            }
        }
    };

    Err(anyhow::anyhow!(
        "timed out waiting for websocket app-server `{websocket_url}` to become ready: {}",
        last_error
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelaySyncRequest {
    acked_command_seq: i64,
    host: HostResponse,
    sessions: Vec<SessionSummary>,
    session_details: HashMap<String, SessionDetailResponse>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelaySyncResponse {
    commands: Vec<RelayCommand>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayCommand {
    seq: i64,
    kind: String,
    session_id: String,
    request_id: Option<String>,
    text: Option<String>,
    decision: Option<String>,
}

async fn run_relay_sync(state: AppState, config: RelayConfig) -> anyhow::Result<()> {
    let client = HttpClient::builder()
        .http1_only()
        .user_agent(format!("codex-remote/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build relay http client")?;

    let mut acked_command_seq = 0_i64;
    let mut tracked_sessions = HashSet::<String>::new();
    let relay_sync_url = format!("{}/internal/sync", config.base_url.trim_end_matches('/'));

    info!(
        "relay sync enabled: base_url={} workspace_id={}",
        config.base_url, config.workspace_id
    );

    loop {
        let payload = build_relay_sync_request(&state, acked_command_seq, &tracked_sessions).await;
        match client
            .post(&relay_sync_url)
            .header("authorization", format!("Bearer {}", config.shared_secret))
            .header("x-codex-workspace-id", &config.workspace_id)
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    warn!("relay sync failed with status {status}: {body}");
                } else {
                    let sync_response: RelaySyncResponse = response
                        .json()
                        .await
                        .context("failed to decode relay sync response")?;
                    for command in sync_response.commands {
                        if command.seq <= acked_command_seq {
                            continue;
                        }
                        tracked_sessions.insert(command.session_id.clone());
                        if let Err(err) =
                            apply_relay_command(&state, &command, &mut tracked_sessions).await
                        {
                            warn!("failed to apply relay command {:?}: {err:?}", command);
                        }
                        acked_command_seq = command.seq;
                    }
                }
            }
            Err(err) => {
                warn!("relay sync request failed: {err:?}");
            }
        }

        sleep(config.sync_interval).await;
    }
}

async fn build_relay_sync_request(
    state: &AppState,
    acked_command_seq: i64,
    tracked_sessions: &HashSet<String>,
) -> RelaySyncRequest {
    let sessions = match state.list_sessions_snapshot().await {
        Ok(value) => value.items,
        Err(err) => {
            warn!("failed to snapshot sessions for relay sync: {err:?}");
            Vec::new()
        }
    };
    let mut session_details = HashMap::new();
    for session_id in tracked_sessions {
        match state.session_detail_snapshot(session_id, true, true).await {
            Ok(detail) => {
                session_details.insert(session_id.clone(), detail);
            }
            Err(err) => {
                warn!("failed to snapshot session {session_id} for relay sync: {err:?}");
            }
        }
    }

    RelaySyncRequest {
        acked_command_seq,
        host: state.host_response(),
        sessions,
        session_details,
    }
}

async fn apply_relay_command(
    state: &AppState,
    command: &RelayCommand,
    tracked_sessions: &mut HashSet<String>,
) -> Result<(), ApiError> {
    match command.kind.as_str() {
        "activateSession" => {
            state
                .session_detail_snapshot(&command.session_id, true, true)
                .await?;
            tracked_sessions.insert(command.session_id.clone());
        }
        "sendMessage" => {
            let text = command
                .text
                .clone()
                .ok_or_else(|| ApiError::bad_request("relay command is missing text"))?;
            let _ = state
                .send_session_message_action(&command.session_id, text)
                .await?;
        }
        "interruptSession" => {
            let _ = state.interrupt_session_action(&command.session_id).await?;
        }
        "resolveApproval" => {
            let request_id = command.request_id.clone().ok_or_else(|| {
                ApiError::bad_request("relay approval command is missing request id")
            })?;
            let decision = command.decision.clone().ok_or_else(|| {
                ApiError::bad_request("relay approval command is missing decision")
            })?;
            let _ = state
                .resolve_approval_action(&command.session_id, &request_id, &decision)
                .await?;
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "unsupported relay command kind: {other}"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum PendingApprovalState {
    Command {
        request_id: String,
        params: CommandExecutionRequestApprovalParams,
    },
    FileChange {
        request_id: String,
        params: FileChangeRequestApprovalParams,
    },
}

impl PendingApprovalState {
    fn with_request_id(self, request_id: String) -> Self {
        match self {
            Self::Command { params, .. } => Self::Command { request_id, params },
            Self::FileChange { params, .. } => Self::FileChange { request_id, params },
        }
    }

    fn request_id(&self) -> &str {
        match self {
            Self::Command { request_id, .. } => request_id,
            Self::FileChange { request_id, .. } => request_id,
        }
    }

    fn to_api(self) -> PendingApproval {
        match self {
            Self::Command { request_id, params } => PendingApproval {
                request_id,
                kind: "commandExecution".to_string(),
                turn_id: params.turn_id,
                item_id: params.item_id,
                reason: params.reason,
                command: params.command,
                cwd: params.cwd.map(|path| path.display().to_string()),
                grant_root: None,
            },
            Self::FileChange { request_id, params } => PendingApproval {
                request_id,
                kind: "fileChange".to_string(),
                turn_id: params.turn_id,
                item_id: params.item_id,
                reason: params.reason,
                command: None,
                cwd: None,
                grant_root: params.grant_root.map(|path| path.display().to_string()),
            },
        }
    }
}

fn request_id_key(request_id: &RequestId) -> String {
    match request_id {
        RequestId::Integer(value) => format!("i:{value}"),
        RequestId::String(value) => format!("s:{value}"),
    }
}

fn parse_request_id_key(value: &str) -> Result<RequestId, ApiError> {
    if let Some(id) = value.strip_prefix("i:") {
        let parsed = id
            .parse::<i64>()
            .map_err(|_| ApiError::bad_request(format!("invalid request id: {value}")))?;
        return Ok(RequestId::Integer(parsed));
    }
    if let Some(id) = value.strip_prefix("s:") {
        return Ok(RequestId::String(id.to_string()));
    }
    Err(ApiError::bad_request(format!(
        "invalid request id: {value}"
    )))
}

fn resolve_codex_self_exe(config: &Config) -> Option<PathBuf> {
    if let Some(existing) = config.codex_self_exe.clone()
        && existing
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "codex")
    {
        return Some(existing);
    }

    let current_exe = std::env::current_exe().ok()?;
    let sibling = current_exe.parent()?.join("codex");
    sibling.exists().then_some(sibling)
}
