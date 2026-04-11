use anyhow::Result;
use codex_core::DreamIndex;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::Mutex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dream_pipeline_writes_repo_memory_and_updates_repo_guidance() -> Result<()> {
    let server = start_mock_server().await;
    let skill_path_holder = Arc::new(Mutex::new(None));
    let skill_path_for_hook = Arc::clone(&skill_path_holder);

    let mut builder = test_codex().with_pre_build_hook(move |cwd| {
        std::fs::create_dir_all(cwd.join(".git")).expect("create git dir");
        std::fs::write(cwd.join("AGENTS.md"), "Prefer concise engineering notes.\n")
            .expect("write agents");
        let skill_dir = cwd.join(".codex").join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: demo\ndescription: demo skill\n---\n\nExisting skill body.\n",
        )
        .expect("write skill");
        *skill_path_for_hook.lock().expect("skill path lock") =
            Some(std::fs::canonicalize(&skill_path).expect("canonical skill path"));
    });

    let turn_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-turn"),
            ev_assistant_message("msg-turn", "done"),
            ev_completed("resp-turn"),
        ]),
    )
    .await;

    let test = builder.build(&server).await?;
    test.submit_turn("Use $demo and keep dream notes for this repo.")
        .await?;
    let _turn_request = turn_mock.single_request();

    let skill_path = skill_path_holder
        .lock()
        .expect("skill path lock")
        .clone()
        .expect("skill path");
    let dream_response = serde_json::json!({
        "threadTitle": "Dream Retrospective",
        "threadSummaryMd": "## Summary\n\n- Captured durable repo guidance.\n",
        "memoryBlockMd": "- Feishu bridge and /dream are important workspace flows.\n",
        "nextSessionHintMd": "Start by reading AGENTS.md and the repo memory block.",
        "agentsBlockMd": "- Keep /dream focused on current-thread retrospectives.\n",
        "skills": []
    });
    let dream_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-dream"),
            ev_assistant_message("msg-dream", &dream_response.to_string()),
            ev_completed("resp-dream"),
        ]),
    )
    .await;

    let result = test
        .codex
        .run_dream_pipeline_now()
        .await
        .map_err(anyhow::Error::msg)?;
    let dream_request = dream_mock.single_request();
    let dream_input = serde_json::to_string(&dream_request.input())?;
    assert!(
        dream_input.contains("Repo-local Skill Candidates"),
        "expected dream prompt to include skill candidates: {dream_input}"
    );

    let memory_md = tokio::fs::read_to_string(result.memory_root.join("MEMORY.md")).await?;
    assert!(memory_md.contains("Feishu bridge"));
    let next_session =
        tokio::fs::read_to_string(result.memory_root.join("next_session.md")).await?;
    assert!(next_session.contains("AGENTS.md"));
    let retrospective = tokio::fs::read_to_string(&result.retrospective_path).await?;
    assert!(retrospective.contains("Dream Retrospective"));
    let agents_md = tokio::fs::read_to_string(&result.updated_agents_path).await?;
    assert!(agents_md.contains("current-thread retrospectives"));
    let skill_md = tokio::fs::read_to_string(&skill_path).await?;
    assert!(skill_md.contains("Existing skill body."));
    assert!(result.updated_skill_paths.is_empty());

    let index_text = tokio::fs::read_to_string(result.memory_root.join("index.json")).await?;
    let index: DreamIndex = serde_json::from_str(&index_text)?;
    assert_eq!(index.documents.len(), 4);

    Ok(())
}
