use super::*;
use pretty_assertions::assert_eq;

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
