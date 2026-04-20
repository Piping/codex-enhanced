use super::*;
use codex_app_server_protocol::ApprovalsReviewer as AppServerApprovalsReviewer;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::SubAgentSpawnParams;
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
    assert_eq!(
        app.btw_session,
        Some(BtwSessionState {
            thread_id: btw_thread_id,
        })
    );
    assert!(app.thread_event_channels.contains_key(&btw_thread_id));
    Ok(())
}

#[tokio::test]
async fn btw_thread_start_params_inherit_visible_thread_permissions() -> Result<()> {
    let mut app = make_test_app().await;
    let thread_id = ThreadId::new();
    let session = ThreadSessionState {
        approval_policy: AskForApproval::Never,
        approvals_reviewer: ApprovalsReviewer::User,
        sandbox_policy: SandboxPolicy::DangerFullAccess,
        ..test_thread_session(thread_id, test_path_buf("/tmp/project"))
    };
    app.primary_thread_id = Some(thread_id);
    app.active_thread_id = Some(thread_id);
    app.chat_widget.handle_thread_session(session.clone());
    app.thread_event_channels.insert(
        thread_id,
        ThreadEventChannel::new_with_session(/*capacity*/ 1, session.clone(), Vec::new()),
    );

    let app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let permissions = app.btw_permissions().await;
    let subagent_spawn = Some(SubAgentSpawnParams {
        parent_thread_id: thread_id.to_string(),
        agent_nickname: Some("Scout".to_string()),
        agent_role: Some("btw".to_string()),
    });
    let params = crate::app::btw::btw_thread_start_params(
        &app,
        &app_server,
        &permissions,
        subagent_spawn.as_ref(),
    );

    assert_eq!(permissions.approval_policy, session.approval_policy);
    assert_eq!(permissions.approvals_reviewer, session.approvals_reviewer);
    assert_eq!(permissions.sandbox_policy, session.sandbox_policy);
    assert_eq!(params.approval_policy, Some(session.approval_policy.into()));
    assert_eq!(
        params.approvals_reviewer,
        Some(AppServerApprovalsReviewer::from(session.approvals_reviewer))
    );
    assert_eq!(params.sandbox, Some(SandboxMode::DangerFullAccess));
    assert_eq!(params.ephemeral, None);
    assert_eq!(params.subagent_spawn, subagent_spawn);
    Ok(())
}

#[tokio::test]
async fn btw_thread_fork_params_create_persistent_subagent_thread() -> Result<()> {
    let mut app = make_test_app().await;
    let thread_id = ThreadId::new();
    app.primary_thread_id = Some(thread_id);
    app.active_thread_id = Some(thread_id);

    let app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let permissions = app.btw_permissions().await;
    let subagent_spawn = Some(SubAgentSpawnParams {
        parent_thread_id: thread_id.to_string(),
        agent_nickname: Some("Scout".to_string()),
        agent_role: Some("btw".to_string()),
    });
    let params = crate::app::btw::btw_thread_fork_params(
        &app,
        thread_id,
        &app_server,
        &permissions,
        subagent_spawn.as_ref(),
    );

    assert!(!params.ephemeral);
    assert_eq!(params.subagent_spawn, subagent_spawn);
    Ok(())
}

#[tokio::test]
async fn btw_permissions_fall_back_to_config_when_thread_session_is_missing() -> Result<()> {
    let mut app = make_test_app().await;
    app.config.approvals_reviewer = ApprovalsReviewer::GuardianSubagent;
    app.config
        .permissions
        .approval_policy
        .set(AskForApproval::OnRequest)?;
    app.config
        .permissions
        .sandbox_policy
        .set(SandboxPolicy::new_workspace_write_policy())?;

    let app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let permissions = app.btw_permissions().await;
    let params = crate::app::btw::btw_thread_start_params(
        &app,
        &app_server,
        &permissions,
        /*subagent_spawn*/ None,
    );

    assert_eq!(permissions.approval_policy, AskForApproval::OnRequest);
    assert_eq!(
        permissions.approvals_reviewer,
        ApprovalsReviewer::GuardianSubagent
    );
    assert_eq!(
        permissions.sandbox_policy,
        SandboxPolicy::new_workspace_write_policy()
    );
    assert_eq!(
        params.approval_policy,
        Some(AskForApproval::OnRequest.into())
    );
    assert_eq!(
        params.approvals_reviewer,
        Some(AppServerApprovalsReviewer::GuardianSubagent)
    );
    assert_eq!(params.sandbox, Some(SandboxMode::WorkspaceWrite));
    assert_eq!(params.subagent_spawn, None);
    Ok(())
}

#[tokio::test]
async fn backfill_restores_unloaded_btw_thread_after_restart() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start primary thread");
    let primary_session = started.session.clone();
    let primary_turns = started.turns.clone();
    app.enqueue_primary_thread_session(started.session, started.turns)
        .await?;

    let agent_nickname = "compare the two approaches".to_string();
    app.start_btw_discussion(&mut app_server, agent_nickname.clone())
        .await;
    let btw_thread_id = std::iter::from_fn(|| app_event_rx.try_recv().ok())
        .find_map(|event| match event {
            AppEvent::SelectAgentThread(thread_id) => Some(thread_id),
            _ => None,
        })
        .expect("expected SelectAgentThread event");

    app_server.thread_unsubscribe(btw_thread_id).await?;

    let mut restarted_app = make_test_app().await;
    restarted_app
        .enqueue_primary_thread_session(primary_session, primary_turns)
        .await?;

    assert!(
        restarted_app
            .backfill_loaded_subagent_threads(&mut app_server)
            .await
    );
    assert_eq!(
        restarted_app.agent_navigation.get(&btw_thread_id),
        Some(&AgentPickerThreadEntry {
            agent_nickname: Some(agent_nickname),
            agent_role: Some("btw".to_string()),
            is_closed: false,
        })
    );

    Ok(())
}

#[tokio::test]
async fn startup_resume_restores_unloaded_btw_thread_into_agent_slots() -> Result<()> {
    let (mut app, mut app_event_rx, _op_rx) = make_test_app_with_channels().await;
    let mut app_server = crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref())
        .await
        .expect("embedded app server");
    let started = app_server
        .start_thread(app.chat_widget.config_ref())
        .await
        .expect("start primary thread");
    let primary_thread_id = started.session.thread_id;
    app.enqueue_primary_thread_session(started.session, started.turns)
        .await?;

    let agent_nickname = "compare the two approaches".to_string();
    app.start_btw_discussion(&mut app_server, agent_nickname.clone())
        .await;
    let btw_thread_id = std::iter::from_fn(|| app_event_rx.try_recv().ok())
        .find_map(|event| match event {
            AppEvent::SelectAgentThread(thread_id) => Some(thread_id),
            _ => None,
        })
        .expect("expected SelectAgentThread event");

    app_server.thread_unsubscribe(btw_thread_id).await?;

    let mut restarted_app = make_test_app().await;
    let resumed = app_server
        .resume_thread(
            restarted_app.chat_widget.config_ref().clone(),
            primary_thread_id,
        )
        .await?;
    restarted_app
        .restore_started_thread_state(&mut app_server, resumed)
        .await?;

    assert_eq!(
        restarted_app.agent_navigation.get(&btw_thread_id),
        Some(&AgentPickerThreadEntry {
            agent_nickname: Some(agent_nickname),
            agent_role: Some("btw".to_string()),
            is_closed: false,
        })
    );
    assert_eq!(
        restarted_app
            .agent_navigation
            .thread_id_for_slot(restarted_app.primary_thread_id, 2),
        Some(btw_thread_id)
    );

    Ok(())
}
