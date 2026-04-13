#![expect(clippy::expect_used)]

use anyhow::Result;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_mock_responses_config_toml;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadDreamStartParams;
use codex_app_server_protocol::ThreadDreamStartResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput as V2UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thread_dream_start_updates_repo_artifacts_and_returns_paths() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let turn_sse = responses::sse(vec![
        responses::ev_response_created("resp-turn"),
        responses::ev_assistant_message("msg-turn", "done"),
        responses::ev_completed("resp-turn"),
    ]);
    let repo = TempDir::new()?;
    let repo_root = std::fs::canonicalize(repo.path())?;
    let skill_dir = repo.path().join(".codex").join("skills").join("demo");
    std::fs::create_dir_all(repo.path().join(".git"))?;
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        repo.path().join("AGENTS.md"),
        "Prefer concise engineering notes.\n",
    )?;
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        "---\nname: demo\ndescription: demo skill\n---\n\nExisting skill body.\n",
    )?;
    let skill_path = std::fs::canonicalize(skill_path)?;
    let dream_response = serde_json::json!({
        "threadTitle": "Dream Retrospective",
        "threadSummaryMd": "## Summary\n\n- Captured durable repo guidance.\n",
        "memoryBlockMd": "- Feishu bridge and /dream are important workspace flows.\n",
        "nextSessionHintMd": "Start by reading AGENTS.md and the repo memory block.",
        "agentsBlockMd": "- Keep /dream focused on current-thread retrospectives.\n",
        "skills": [
            {
                "path": skill_path.display().to_string(),
                "blockMd": "- Demo skill should stay repo-local and concise.\n"
            }
        ],
        "newSkills": []
    });
    let dream_sse = responses::sse(vec![
        responses::ev_response_created("resp-dream"),
        responses::ev_assistant_message("msg-dream", &dream_response.to_string()),
        responses::ev_completed("resp-dream"),
    ]);
    responses::mount_sse_sequence(&server, vec![turn_sse, dream_sse]).await;

    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &server.uri(),
        &BTreeMap::default(),
        /*auto_compact_limit*/ 1_000,
        /*requires_openai_auth*/ None,
        "mock_provider",
        "Summarize the conversation.",
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp, &repo_root).await?;
    send_turn_and_wait(
        &mut mcp,
        &thread_id,
        &format!(
            "<skill>\n<name>demo</name>\n<path>{}</path>\nKeep dream notes for this repo.\n</skill>",
            skill_path.display()
        ),
    )
    .await?;

    let dream_id = mcp
        .send_thread_dream_start_request(ThreadDreamStartParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let dream_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(dream_id)),
    )
    .await??;
    let dream: ThreadDreamStartResponse = to_response(dream_resp)?;

    let expected_memory_root = repo_root.join(".codex").join("memory");
    assert_eq!(
        dream.memory_root,
        expected_memory_root.display().to_string()
    );
    assert_eq!(
        dream.updated_agents_path,
        repo_root.join("AGENTS.md").display().to_string()
    );
    assert_eq!(
        dream.updated_skill_paths,
        vec![skill_path.display().to_string()]
    );
    assert_eq!(
        dream.next_session_hint,
        "Start by reading AGENTS.md and the repo memory block."
    );

    let memory_md = tokio::fs::read_to_string(expected_memory_root.join("MEMORY.md")).await?;
    assert!(memory_md.contains("Feishu bridge"));
    let next_session =
        tokio::fs::read_to_string(expected_memory_root.join("next_session.md")).await?;
    assert!(next_session.contains("AGENTS.md"));
    let retrospective = tokio::fs::read_to_string(&dream.retrospective_path).await?;
    assert!(retrospective.contains("Dream Retrospective"));
    let agents_md = tokio::fs::read_to_string(repo_root.join("AGENTS.md")).await?;
    assert!(agents_md.contains("current-thread retrospectives"));
    let skill_md = tokio::fs::read_to_string(&skill_path).await?;
    assert!(skill_md.contains("Existing skill body."));
    assert!(skill_md.contains("repo-local and concise"));
    let index_text = tokio::fs::read_to_string(expected_memory_root.join("index.json")).await?;
    assert!(index_text.contains("\"kind\": \"skill\""));

    Ok(())
}

async fn start_thread(mcp: &mut McpProcess, cwd: &Path) -> Result<String> {
    let thread_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(cwd.display().to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;
    Ok(thread.id)
}

async fn send_turn_and_wait(mcp: &mut McpProcess, thread_id: &str, text: &str) -> Result<()> {
    let turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.to_string(),
            input: vec![V2UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    let turn: TurnStartResponse = to_response(turn_resp)?;
    let completed_notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let completed: TurnCompletedNotification = serde_json::from_value(
        completed_notification
            .params
            .expect("turn/completed params"),
    )?;
    assert_eq!(completed.thread_id, thread_id);
    assert_eq!(completed.turn.id, turn.turn.id);
    Ok(())
}
