use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn start_btw_discussion_registers_thread_and_requests_switch() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start primary thread");
    let primary_thread_id = started.session.thread_id;
    app.primary_thread_id = Some(primary_thread_id);
    app.active_thread_id = Some(primary_thread_id);

    app.start_btw_discussion(&mut app_server, "compare the two approaches".to_string())
        .await;

    let btw_thread_id = std::iter::from_fn(|| app_event_rx.try_recv().ok())
        .find_map(|event| match event {
            AppEvent::SelectAgentThread(thread_id) => Some(thread_id),
            _ => None,
        })
        .expect("expected SelectAgentThread event");
    assert_ne!(btw_thread_id, primary_thread_id);
    assert_eq!(
        app.agent_navigation.get(&btw_thread_id),
        Some(&AgentPickerThreadEntry {
            agent_nickname: Some("compare the two approaches".to_string()),
            agent_role: Some("btw".to_string()),
            is_closed: false,
        })
    );
    assert!(app.thread_event_channels.contains_key(&btw_thread_id));
    Ok(())
}

#[tokio::test]
async fn start_btw_discussion_falls_back_to_fresh_thread_when_current_thread_cannot_fork()
-> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let primary_thread_id = ThreadId::new();
    app.primary_thread_id = Some(primary_thread_id);
    app.active_thread_id = Some(primary_thread_id);
    app.chat_widget.handle_codex_event(Event {
        id: String::new(),
        msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
            session_id: primary_thread_id,
            forked_from_id: None,
            thread_name: None,
            model: "gpt-test".to_string(),
            model_provider_id: "test-provider".to_string(),
            service_tier: None,
            approval_policy: AskForApproval::Never,
            approvals_reviewer: ApprovalsReviewer::User,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            cwd: test_path_buf("/tmp/project").abs(),
            reasoning_effort: None,
            history_log_id: 0,
            history_entry_count: 0,
            initial_messages: None,
            network_proxy: None,
            rollout_path: None,
        }),
    });

    app.start_btw_discussion(&mut app_server, "compare the two approaches".to_string())
        .await;

    let btw_thread_id = std::iter::from_fn(|| app_event_rx.try_recv().ok())
        .find_map(|event| match event {
            AppEvent::SelectAgentThread(thread_id) => Some(thread_id),
            _ => None,
        })
        .expect("expected SelectAgentThread event");
    assert_ne!(btw_thread_id, primary_thread_id);
    assert_eq!(
        app.agent_navigation.get(&btw_thread_id),
        Some(&AgentPickerThreadEntry {
            agent_nickname: Some("compare the two approaches".to_string()),
            agent_role: Some("btw".to_string()),
            is_closed: false,
        })
    );
    assert!(app.thread_event_channels.contains_key(&btw_thread_id));
    Ok(())
}

#[tokio::test]
async fn btw_completion_notification_emits_completion_event_and_is_swallowed() {
    let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let thread_id = ThreadId::new();
    app.btw_session = Some(BtwSessionState {
        thread_id,
        final_message: None,
        last_status: None,
    });

    let swallowed = app.handle_btw_notification(
        thread_id,
        &turn_completed_notification_with_agent_message(
            thread_id,
            "turn-btw",
            TurnStatus::Completed,
            "Temporary answer",
        ),
    );

    assert!(swallowed);
    match app_event_rx.try_recv() {
        Ok(AppEvent::BtwCompleted {
            thread_id: actual_thread_id,
            result: Ok(message),
        }) => {
            assert_eq!(actual_thread_id, thread_id);
            assert_eq!(message, "Temporary answer");
        }
        other => panic!("expected BtwCompleted event, got {other:?}"),
    }
}

#[tokio::test]
async fn btw_loading_popup_surfaces_hidden_hook_status() {
    let mut app = make_test_app().await;
    let thread_id = ThreadId::new();
    app.btw_session = Some(BtwSessionState {
        thread_id,
        final_message: None,
        last_status: None,
    });

    let swallowed =
        app.handle_btw_notification(thread_id, &hook_started_notification(thread_id, "turn-btw"));

    assert!(swallowed);
    let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
    assert!(
        popup.contains("Current hidden status:")
            && popup.contains("Running UserPromptSubmit hook: checking")
            && popup.contains("go-workflow input policy"),
        "expected hidden hook status in /btw popup: {popup}"
    );
}

#[tokio::test]
async fn btw_request_user_input_opens_failure_popup_instead_of_hanging() {
    let mut app = make_test_app().await;
    let thread_id = ThreadId::new();
    app.btw_session = Some(BtwSessionState {
        thread_id,
        final_message: None,
        last_status: None,
    });

    let reason = app.reject_btw_request(
        thread_id,
        &request_user_input_request(thread_id, "turn-btw", "call-1"),
    );

    assert_eq!(
        reason,
        Some(
            "the hidden temporary discussion asked for interactive user input. Run the prompt in the main thread instead.".to_string()
        )
    );
    let popup = render_bottom_popup(&app.chat_widget, /*width*/ 100);
    assert!(
        popup.contains("asked for interactive user input"),
        "expected /btw failure popup for hidden request_user_input: {popup}"
    );
}
