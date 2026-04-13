use super::*;
use crate::app_event::AppEvent;
use crate::app_event::ClawbotControlsDestination;
use crate::app_event::ClawbotForwardingChannel;
use crate::app_event::ClawbotSessionBindSource;
use codex_clawbot::ClawbotRuntime;
use codex_clawbot::ClawbotStore;
use codex_clawbot::ClawbotTurnMode;
use codex_clawbot::PendingClawbotTurn;
use codex_clawbot::ProviderEvent as ClawbotProviderEvent;
use codex_clawbot::ProviderKind as ClawbotProviderKind;
use codex_clawbot::ProviderMessageRef;
use codex_clawbot::ProviderOutboundReaction;
use codex_clawbot::ProviderOutboundTextMessage;
use codex_clawbot::ProviderSession;
use codex_clawbot::ProviderSessionRef;
use codex_clawbot::SessionStatus as ClawbotSessionStatus;
use pretty_assertions::assert_eq;

async fn bind_test_clawbot_session(
    app: &mut App,
    app_server: &mut AppServerSession,
    session_id: &str,
) -> Result<(ThreadId, ProviderSessionRef)> {
    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start thread");
    let thread_id = started.session.thread_id;
    let session = ProviderSessionRef::new(ClawbotProviderKind::Feishu, session_id);
    let mut runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .persist_session(ProviderSession {
            provider: ClawbotProviderKind::Feishu,
            session_id: session_id.to_string(),
            display_name: Some("Alice".to_string()),
            unread_count: 0,
            last_message_at: None,
            status: ClawbotSessionStatus::Discovered,
            bound_thread_id: None,
        })
        .expect("persist session");
    if app.primary_thread_id.is_none() {
        app.primary_thread_id = Some(thread_id);
    }
    runtime
        .connect_session_to_thread(
            &session,
            thread_id.to_string(),
            app.clawbot_owner_primary_thread_id(),
        )
        .expect("connect session");
    app.sync_clawbot_workspace(app_server).await;
    Ok((thread_id, session))
}

#[tokio::test]
async fn clawbot_inbound_message_resumes_bound_thread_and_starts_turn() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_resume").await?;

    app.handle_clawbot_provider_event(
        &mut app_server,
        ClawbotProviderEvent::InboundMessage(codex_clawbot::ProviderInboundMessage {
            session: session.clone(),
            message_id: "msg_1".to_string(),
            text: "hello from feishu".to_string(),
            received_at: 1,
        }),
    )
    .await
    .expect("handle clawbot inbound message");

    assert!(app.thread_event_channels.contains_key(&thread_id));
    assert_eq!(
        app.clawbot.outbound_reactions,
        vec![ProviderOutboundReaction {
            target: ProviderMessageRef::new(ClawbotProviderKind::Feishu, "chat_resume", "msg_1"),
            emoji_type: "TONGUE".to_string(),
        }]
    );
    assert_eq!(
        app.clawbot
            .pending_turns
            .get(&thread_id)
            .map(std::collections::VecDeque::len),
        Some(1)
    );
    assert!(app.active_turn_id_for_thread(thread_id).await.is_some());

    Ok(())
}

#[tokio::test]
async fn clawbot_inbound_message_to_inactive_thread_shows_jump_hint() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let current_started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("current thread");
    app.active_thread_id = Some(current_started.session.thread_id);
    app.primary_thread_id = Some(current_started.session.thread_id);

    let (bound_thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_inactive_hint").await?;
    app.upsert_agent_picker_thread(
        bound_thread_id,
        Some("Inbox Agent".to_string()),
        /*agent_role*/ None,
        /*is_closed*/ false,
    );
    app.active_thread_id = Some(current_started.session.thread_id);

    app.handle_clawbot_provider_event(
        &mut app_server,
        ClawbotProviderEvent::InboundMessage(codex_clawbot::ProviderInboundMessage {
            session,
            message_id: "msg_1".to_string(),
            text: "hello from inactive".to_string(),
            received_at: 1,
        }),
    )
    .await
    .expect("handle clawbot inbound message");

    let cell = match app_event_rx.try_recv() {
        Ok(AppEvent::InsertHistoryCell(cell)) => cell,
        other => panic!("expected InsertHistoryCell event, got {other:?}"),
    };
    let rendered = cell
        .display_lines(/*width*/ 120)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("imported into agent thread Inbox Agent."));
    assert!(rendered.contains("Open /clawbot to inspect bindings and jump."));

    Ok(())
}

#[tokio::test]
async fn noninteractive_clawbot_request_user_input_builds_auto_response() {
    let mut app = make_test_app().await;
    let thread_id = ThreadId::new();
    app.clawbot.pending_turns.insert(
        thread_id,
        VecDeque::from([PendingClawbotTurn {
            thread_id: thread_id.to_string(),
            turn_id: "turn-1".to_string(),
            owner_primary_thread_id: Some(thread_id.to_string()),
            session: ProviderSessionRef::new(ClawbotProviderKind::Feishu, "chat_auto"),
            message_id: "msg-1".to_string(),
            auto_ack_reaction_id: None,
            turn_mode: ClawbotTurnMode::NonInteractive,
        }]),
    );
    let request = ServerRequest::ToolRequestUserInput {
        request_id: AppServerRequestId::Integer(1),
        params: ToolRequestUserInputParams {
            thread_id: thread_id.to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "call-1".to_string(),
            questions: Vec::new(),
        },
    };

    let op = app
        .clawbot_auto_response_op_for_server_request(thread_id, &request)
        .expect("auto response op");

    match op.view() {
        crate::app_command::AppCommandView::UserInputAnswer { id, response } => {
            assert_eq!(id, "turn-1");
            assert_eq!(
                response,
                &codex_protocol::request_user_input::RequestUserInputResponse {
                    answers: HashMap::new(),
                }
            );
        }
        _ => panic!("expected UserInputAnswer"),
    }
}

#[tokio::test]
async fn noninteractive_clawbot_permissions_request_builds_auto_response() {
    let mut app = make_test_app().await;
    let thread_id = ThreadId::new();
    app.clawbot.pending_turns.insert(
        thread_id,
        VecDeque::from([PendingClawbotTurn {
            thread_id: thread_id.to_string(),
            turn_id: "turn-1".to_string(),
            owner_primary_thread_id: Some(thread_id.to_string()),
            session: ProviderSessionRef::new(ClawbotProviderKind::Feishu, "chat_auto"),
            message_id: "msg-1".to_string(),
            auto_ack_reaction_id: None,
            turn_mode: ClawbotTurnMode::NonInteractive,
        }]),
    );
    let request = ServerRequest::PermissionsRequestApproval {
        request_id: AppServerRequestId::Integer(7),
        params: PermissionsRequestApprovalParams {
            thread_id: thread_id.to_string(),
            turn_id: "turn-1".to_string(),
            item_id: "call-approval".to_string(),
            reason: Some("Need access".to_string()),
            permissions: codex_app_server_protocol::RequestPermissionProfile {
                network: None,
                file_system: None,
            },
        },
    };

    let op = app
        .clawbot_auto_response_op_for_server_request(thread_id, &request)
        .expect("auto response op");

    match op.view() {
        crate::app_command::AppCommandView::RequestPermissionsResponse { id, response } => {
            assert_eq!(id, "call-approval");
            assert_eq!(
                response,
                &codex_protocol::request_permissions::RequestPermissionsResponse {
                    permissions: Default::default(),
                    scope: codex_protocol::request_permissions::PermissionGrantScope::Turn,
                }
            );
        }
        _ => panic!("expected RequestPermissionsResponse"),
    }
}

#[tokio::test]
async fn clawbot_turn_completed_forwards_reply_and_drains_next_message() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_reply").await?;

    app.handle_clawbot_provider_event(
        &mut app_server,
        ClawbotProviderEvent::InboundMessage(codex_clawbot::ProviderInboundMessage {
            session: session.clone(),
            message_id: "msg_1".to_string(),
            text: "first".to_string(),
            received_at: 1,
        }),
    )
    .await
    .expect("handle first clawbot inbound");
    app.handle_clawbot_provider_event(
        &mut app_server,
        ClawbotProviderEvent::InboundMessage(codex_clawbot::ProviderInboundMessage {
            session: session.clone(),
            message_id: "msg_2".to_string(),
            text: "second".to_string(),
            received_at: 2,
        }),
    )
    .await
    .expect("handle second clawbot inbound");

    let first_turn_id = app
        .clawbot
        .pending_turns
        .get(&thread_id)
        .and_then(|queue| queue.front())
        .map(|pending| pending.turn_id.clone())
        .expect("first pending turn");
    let queued_runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(queued_runtime.snapshot().unread_message_count, 2);

    app.enqueue_thread_notification(
        thread_id,
        turn_completed_notification_with_agent_message(
            thread_id,
            &first_turn_id,
            TurnStatus::Completed,
            "forwarded reply",
        ),
    )
    .await?;
    app.handle_clawbot_turn_completed(
        &mut app_server,
        thread_id,
        test_turn(
            &first_turn_id,
            TurnStatus::Completed,
            vec![ThreadItem::AgentMessage {
                id: "agent-1".to_string(),
                text: "forwarded reply".to_string(),
                phase: None,
                memory_citation: None,
            }],
        ),
    )
    .await
    .expect("handle clawbot turn completion");

    assert_eq!(
        app.clawbot.outbound_messages,
        vec![ProviderOutboundTextMessage {
            session: session.clone(),
            text: "forwarded reply".to_string(),
        }]
    );
    assert_eq!(
        app.clawbot.removed_outbound_reactions,
        vec![ProviderOutboundReaction {
            target: ProviderMessageRef::new(ClawbotProviderKind::Feishu, "chat_reply", "msg_1"),
            emoji_type: "TONGUE".to_string(),
        }]
    );
    assert_eq!(
        app.clawbot
            .pending_turns
            .get(&thread_id)
            .map(std::collections::VecDeque::len),
        Some(1)
    );
    let drained_runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(drained_runtime.snapshot().unread_message_count, 1);

    Ok(())
}

#[tokio::test]
async fn clawbot_restart_recovers_pending_turn_and_forwards_reply() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_restart").await?;

    app.handle_clawbot_provider_event(
        &mut app_server,
        ClawbotProviderEvent::InboundMessage(codex_clawbot::ProviderInboundMessage {
            session: session.clone(),
            message_id: "msg_1".to_string(),
            text: "hello after restart".to_string(),
            received_at: 1,
        }),
    )
    .await
    .expect("handle clawbot inbound");

    let store = ClawbotStore::new(app.config.cwd.to_path_buf());
    assert_eq!(store.load_pending_turns().expect("pending turns").len(), 1);

    let mut restarted_app = make_test_app().await;
    restarted_app.config.cwd = tempdir.path().to_path_buf().abs();
    restarted_app.primary_thread_id = Some(thread_id);
    restarted_app.sync_clawbot_workspace(&mut app_server).await;

    assert_eq!(
        restarted_app
            .clawbot
            .pending_turns
            .get(&thread_id)
            .map(std::collections::VecDeque::len),
        Some(1)
    );
    let restored_turn_id = restarted_app
        .clawbot
        .pending_turns
        .get(&thread_id)
        .and_then(|queue| queue.front())
        .map(|pending| pending.turn_id.clone())
        .expect("restored pending turn");

    restarted_app
        .handle_clawbot_turn_completed(
            &mut app_server,
            thread_id,
            test_turn(
                &restored_turn_id,
                TurnStatus::Completed,
                vec![ThreadItem::AgentMessage {
                    id: "agent-1".to_string(),
                    text: "restart reply".to_string(),
                    phase: None,
                    memory_citation: None,
                }],
            ),
        )
        .await
        .expect("complete restored clawbot turn");

    assert_eq!(
        restarted_app.clawbot.outbound_messages,
        vec![ProviderOutboundTextMessage {
            session,
            text: "restart reply".to_string(),
        }]
    );
    assert_eq!(
        restarted_app.clawbot.removed_outbound_reactions,
        vec![ProviderOutboundReaction {
            target: ProviderMessageRef::new(ClawbotProviderKind::Feishu, "chat_restart", "msg_1"),
            emoji_type: "TONGUE".to_string(),
        }]
    );

    let runtime = ClawbotRuntime::load(restarted_app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(runtime.snapshot().unread_message_count, 0);
    assert_eq!(
        store.load_pending_turns().expect("pending turns"),
        Vec::new()
    );

    Ok(())
}

#[tokio::test]
async fn clawbot_sync_ignores_binding_owned_by_another_app_instance() -> Result<()> {
    let mut owner_app = make_test_app().await;
    let mut app_server =
        crate::start_embedded_app_server_for_picker(owner_app.chat_widget.config_ref())
            .await
            .expect("embedded app server");
    let tempdir = tempdir()?;
    owner_app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, session) =
        bind_test_clawbot_session(&mut owner_app, &mut app_server, "chat_owner").await?;

    let mut other_app = make_test_app().await;
    other_app.config.cwd = tempdir.path().to_path_buf().abs();
    other_app.primary_thread_id = Some(ThreadId::new());

    let mut runtime = ClawbotRuntime::load(owner_app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .apply_provider_event(ClawbotProviderEvent::InboundMessage(
            codex_clawbot::ProviderInboundMessage {
                session: session.clone(),
                message_id: "msg_1".to_string(),
                text: "owner only".to_string(),
                received_at: 1,
            },
        ))
        .expect("queue unread");

    other_app.sync_clawbot_workspace(&mut app_server).await;
    assert_eq!(other_app.clawbot.pending_turns.get(&thread_id), None);
    assert_eq!(other_app.clawbot.outbound_reactions, Vec::new());

    owner_app.sync_clawbot_workspace(&mut app_server).await;
    assert_eq!(
        owner_app
            .clawbot
            .pending_turns
            .get(&thread_id)
            .map(std::collections::VecDeque::len),
        Some(1)
    );
    assert_eq!(
        owner_app.clawbot.outbound_reactions,
        vec![ProviderOutboundReaction {
            target: ProviderMessageRef::new(ClawbotProviderKind::Feishu, "chat_owner", "msg_1"),
            emoji_type: "TONGUE".to_string(),
        }]
    );

    Ok(())
}

#[test]
fn clawbot_store_persists_auto_ack_reaction_id() -> Result<()> {
    let tempdir = tempdir()?;
    let store = ClawbotStore::new(tempdir.path().to_path_buf());
    let pending_turn = PendingClawbotTurn {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        owner_primary_thread_id: Some("owner-thread-1".to_string()),
        session: ProviderSessionRef::new(ClawbotProviderKind::Feishu, "chat_store"),
        message_id: "msg-1".to_string(),
        auto_ack_reaction_id: Some("reaction-1".to_string()),
        turn_mode: ClawbotTurnMode::NonInteractive,
    };

    store
        .upsert_pending_turn(pending_turn.clone())
        .expect("persist pending turn");

    assert_eq!(
        store.load_pending_turns().expect("pending turns"),
        vec![pending_turn]
    );

    Ok(())
}

#[tokio::test]
async fn clawbot_bound_thread_completion_forwards_final_reply_without_pending_turn() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_manual").await?;

    app.handle_clawbot_turn_completed(
        &mut app_server,
        thread_id,
        test_turn(
            "turn-manual",
            TurnStatus::Completed,
            vec![
                ThreadItem::AgentMessage {
                    id: "agent-1".to_string(),
                    text: "draft reply".to_string(),
                    phase: None,
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "agent-2".to_string(),
                    text: "final reply".to_string(),
                    phase: None,
                    memory_citation: None,
                },
            ],
        ),
    )
    .await
    .expect("forward bound thread reply");

    assert_eq!(
        app.clawbot.outbound_messages,
        vec![ProviderOutboundTextMessage {
            session,
            text: "final reply".to_string(),
        }]
    );

    Ok(())
}

#[tokio::test]
async fn clawbot_sync_clears_stale_pending_turn_and_redelivers_unread() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_stale").await?;

    let mut runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .apply_provider_event(ClawbotProviderEvent::InboundMessage(
            codex_clawbot::ProviderInboundMessage {
                session: session.clone(),
                message_id: "msg_1".to_string(),
                text: "deliver me".to_string(),
                received_at: 1,
            },
        ))
        .expect("queue unread");

    let store = ClawbotStore::new(app.config.cwd.to_path_buf());
    store
        .upsert_pending_turn(PendingClawbotTurn {
            thread_id: thread_id.to_string(),
            turn_id: "stale-turn".to_string(),
            owner_primary_thread_id: app.clawbot_owner_primary_thread_id(),
            session: session.clone(),
            message_id: "msg_1".to_string(),
            auto_ack_reaction_id: None,
            turn_mode: ClawbotTurnMode::NonInteractive,
        })
        .expect("persist stale pending turn");
    app.clawbot.pending_turns.clear();

    app.sync_clawbot_workspace(&mut app_server).await;

    assert_eq!(
        app.clawbot
            .pending_turns
            .get(&thread_id)
            .map(std::collections::VecDeque::len),
        Some(1)
    );
    assert_eq!(app.clawbot.outbound_reactions.len(), 1);
    assert_eq!(
        store
            .load_pending_turns()
            .expect("pending turns")
            .into_iter()
            .map(|pending| pending.turn_id)
            .collect::<Vec<_>>(),
        app.clawbot
            .pending_turns
            .get(&thread_id)
            .expect("pending queue")
            .iter()
            .map(|pending| pending.turn_id.clone())
            .collect::<Vec<_>>()
    );
    assert_ne!(
        app.clawbot
            .pending_turns
            .get(&thread_id)
            .and_then(|queue| queue.front())
            .map(|pending| pending.turn_id.as_str()),
        Some("stale-turn")
    );

    Ok(())
}

#[tokio::test]
async fn clawbot_manual_bind_replays_cached_unread_messages() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start thread");
    let thread_id = started.session.thread_id;
    app.active_thread_id = Some(thread_id);

    let session = ProviderSessionRef::new(ClawbotProviderKind::Feishu, "chat_bind");
    let mut runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .apply_provider_event(ClawbotProviderEvent::InboundMessage(
            codex_clawbot::ProviderInboundMessage {
                session: session.clone(),
                message_id: "msg_1".to_string(),
                text: "queued before bind".to_string(),
                received_at: 1,
            },
        ))
        .expect("queue unread");

    app.bind_clawbot_session_to_current_thread(
        &mut app_server,
        "chat_bind".to_string(),
        ClawbotSessionBindSource::ManualSessionId,
    )
    .await
    .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;

    let runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(
        runtime
            .bound_session_for_thread(&thread_id.to_string())
            .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?,
        Some(session)
    );
    assert_eq!(
        app.clawbot
            .pending_turns
            .get(&thread_id)
            .map(std::collections::VecDeque::len),
        Some(1)
    );
    Ok(())
}

#[tokio::test]
async fn clawbot_manual_bind_allows_undiscovered_chat_id_with_configured_feishu() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start thread");
    let thread_id = started.session.thread_id;
    app.active_thread_id = Some(thread_id);

    let mut runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .update_feishu_config(Some(codex_clawbot::FeishuConfig {
            app_id: "cli_app_123".to_string(),
            app_secret: "secret_value_4567".to_string(),
            verification_token: None,
            encrypt_key: None,
            bot_open_id: Some("ou_bot_open_id".to_string()),
            bot_user_id: None,
        }))
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;

    app.bind_clawbot_session_to_current_thread(
        &mut app_server,
        "chat_manual_only".to_string(),
        ClawbotSessionBindSource::ManualSessionId,
    )
    .await
    .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;

    let runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(
        runtime
            .bound_session_for_thread(&thread_id.to_string())
            .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?,
        Some(ProviderSessionRef::new(
            ClawbotProviderKind::Feishu,
            "chat_manual_only"
        ))
    );
    Ok(())
}

#[tokio::test]
async fn clawbot_current_thread_controls_update_binding_state() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, _session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_controls").await?;
    app.active_thread_id = Some(thread_id);

    app.clawbot_set_current_thread_forwarding(
        ClawbotForwardingChannel::Inbound,
        /*enabled*/ false,
    )
    .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    app.clawbot_set_current_thread_forwarding(
        ClawbotForwardingChannel::Outbound,
        /*enabled*/ false,
    )
    .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;

    let runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    let binding = runtime
        .load_binding_for_thread(&thread_id.to_string())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?
        .expect("binding");
    assert!(!binding.inbound_forwarding_enabled);
    assert!(!binding.outbound_forwarding_enabled);

    app.clawbot_disconnect_current_thread()
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    let runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(
        runtime
            .load_binding_for_thread(&thread_id.to_string())
            .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn clawbot_management_popup_snapshot() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let (thread_id, _session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_snapshot").await?;
    app.active_thread_id = Some(thread_id);
    app.primary_thread_id = Some(thread_id);

    let mut runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .update_feishu_config(Some(codex_clawbot::FeishuConfig {
            app_id: "cli_app_123".to_string(),
            app_secret: "secret_value_4567".to_string(),
            verification_token: Some("verify_token".to_string()),
            encrypt_key: None,
            bot_open_id: Some("ou_bot_open_id".to_string()),
            bot_user_id: None,
        }))
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .persist_session(ProviderSession {
            provider: ClawbotProviderKind::Feishu,
            session_id: "chat_discovered".to_string(),
            display_name: Some("Bob".to_string()),
            unread_count: 0,
            last_message_at: None,
            status: ClawbotSessionStatus::Discovered,
            bound_thread_id: None,
        })
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .apply_provider_event(ClawbotProviderEvent::InboundMessage(
            codex_clawbot::ProviderInboundMessage {
                session: ProviderSessionRef::new(ClawbotProviderKind::Feishu, "chat_discovered"),
                message_id: "msg_discovered".to_string(),
                text: "hello".to_string(),
                received_at: 10,
            },
        ))
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    let second_started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("second thread");
    app.upsert_agent_picker_thread(
        second_started.session.thread_id,
        Some("Inbox Agent".to_string()),
        /*agent_role*/ None,
        /*is_closed*/ false,
    );
    runtime
        .persist_session(ProviderSession {
            provider: ClawbotProviderKind::Feishu,
            session_id: "chat_ops".to_string(),
            display_name: Some("Ops".to_string()),
            unread_count: 0,
            last_message_at: None,
            status: ClawbotSessionStatus::Discovered,
            bound_thread_id: None,
        })
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    runtime
        .connect_session_to_thread(
            &ProviderSessionRef::new(ClawbotProviderKind::Feishu, "chat_ops"),
            second_started.session.thread_id.to_string(),
            app.clawbot_owner_primary_thread_id(),
        )
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;

    app.open_clawbot_management_popup();

    let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
    assert_snapshot!("clawbot_management_popup", popup);

    app.open_clawbot_management_view(ClawbotControlsDestination::Channels);
    let channels_popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
    assert_snapshot!("clawbot_channels_popup", channels_popup);

    app.open_clawbot_management_view(ClawbotControlsDestination::UnboundSessions);
    let unbound_sessions_popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
    assert_snapshot!("clawbot_unbound_sessions_popup", unbound_sessions_popup);
    Ok(())
}

#[tokio::test]
async fn clawbot_rebinds_discovered_session_from_management_actions() -> Result<()> {
    let mut app = make_test_app().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let tempdir = tempdir()?;
    app.config.cwd = tempdir.path().to_path_buf().abs();

    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start thread");
    let target_thread_id = started.session.thread_id;
    app.active_thread_id = Some(target_thread_id);

    let (source_thread_id, session) =
        bind_test_clawbot_session(&mut app, &mut app_server, "chat_rebind").await?;

    app.bind_clawbot_session_to_current_thread(
        &mut app_server,
        "chat_rebind".to_string(),
        ClawbotSessionBindSource::DiscoveredSession,
    )
    .await
    .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;

    let runtime = ClawbotRuntime::load(app.config.cwd.to_path_buf())
        .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?;
    assert_eq!(
        runtime
            .bound_session_for_thread(&target_thread_id.to_string())
            .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?,
        Some(session.clone())
    );
    assert_eq!(
        runtime
            .bound_session_for_thread(&source_thread_id.to_string())
            .map_err(|err| color_eyre::eyre::eyre!(err.to_string()))?,
        None
    );
    Ok(())
}
