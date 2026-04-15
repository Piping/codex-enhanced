use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use tokio::select;
use tokio::time::Instant;
use tokio::time::sleep;
use tokio::time::timeout;

#[tokio::test]
async fn after_turn_workflow_followup_starts_second_turn_in_tui_pty() -> Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    let codex = if let Ok(path) = codex_utils_cargo_bin::cargo_bin("codex") {
        path
    } else {
        let fallback = codex_utils_cargo_bin::repo_root()?.join("codex-rs/target/debug/codex");
        if fallback.is_file() {
            fallback
        } else {
            eprintln!("skipping PTY workflow test because codex binary is unavailable");
            return Ok(());
        }
    };

    let fixture =
        codex_utils_cargo_bin::find_resource!("../core/tests/cli_responses_fixture.sse")?;
    let tmp = tempfile::tempdir()?;
    let repo = tmp.path().join("repo");
    let codex_home = tmp.path().join("codex-home");
    std::fs::create_dir_all(repo.join(".codex/workflows"))?;
    std::fs::create_dir_all(&codex_home)?;

    std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(&repo)
        .status()
        .context("failed to init temp git repo")?;

    write_after_turn_prompt_workflow(&repo)?;
    write_test_config(&codex_home, &repo)?;

    let seed_output = std::process::Command::new(&codex)
        .arg("-p")
        .arg("newapi")
        .arg("exec")
        .arg("--skip-git-repo-check")
        .arg("-C")
        .arg(&repo)
        .arg("seed session for workflow follow-up")
        .env("CODEX_HOME", &codex_home)
        .env("OPENAI_API_KEY", "dummy")
        .env("CODEX_RS_SSE_FIXTURE", &fixture)
        .output()
        .context("failed to seed resume session")?;
    anyhow::ensure!(
        seed_output.status.success(),
        "codex exec failed: {}",
        String::from_utf8_lossy(&seed_output.stderr)
    );

    let mut env = HashMap::new();
    env.insert("CODEX_HOME".to_string(), codex_home.display().to_string());
    env.insert("OPENAI_API_KEY".to_string(), "dummy".to_string());
    env.insert(
        "CODEX_RS_SSE_FIXTURE".to_string(),
        fixture.display().to_string(),
    );

    let args = vec![
        "-p".to_string(),
        "newapi".to_string(),
        "resume".to_string(),
        "--last".to_string(),
        "--no-alt-screen".to_string(),
        "-C".to_string(),
        repo.display().to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
        "-c".to_string(),
        "disable_paste_burst=true".to_string(),
    ];

    let spawned = codex_utils_pty::spawn_pty_process(
        codex.to_string_lossy().as_ref(),
        &args,
        &repo,
        &env,
        &None,
        codex_utils_pty::TerminalSize::default(),
    )
    .await?;

    let codex_utils_pty::SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = spawned;
    let mut output_rx = codex_utils_pty::combine_output_receivers(stdout_rx, stderr_rx);
    let mut exit_rx = exit_rx;
    let writer_tx = session.writer_sender();

    let mut output = Vec::new();
    let startup_deadline = Instant::now() + Duration::from_secs(2);
    let mut answered_cursor_query = false;
    let mut startup_ready = false;
    let mut prompt_sent = false;
    let mut interrupt_sent = false;

    let exit_code_result = timeout(Duration::from_secs(30), async {
        loop {
            select! {
                result = output_rx.recv() => match result {
                    Ok(chunk) => {
                        let has_cursor_query = chunk.windows(4).any(|window| window == b"\x1b[6n");
                        if has_cursor_query {
                            let _ = writer_tx.send(b"\x1b[1;1R".to_vec()).await;
                            answered_cursor_query = true;
                        }
                        output.extend_from_slice(&chunk);

                        if !startup_ready
                            && ((!has_cursor_query && answered_cursor_query)
                                || Instant::now() >= startup_deadline)
                        {
                            startup_ready = true;
                        }

                        if startup_ready && !prompt_sent {
                            prompt_sent = true;
                            let _ = writer_tx.send(b"trigger workflow follow-up".to_vec()).await;
                            sleep(Duration::from_millis(100)).await;
                            let _ = writer_tx.send(b"\r".to_vec()).await;
                        }

                        let output_text = String::from_utf8_lossy(&output);
                        let fixture_hello_count = output_text.matches("fixture hello").count();
                        if !interrupt_sent
                            && fixture_hello_count >= 2
                            && output_text.contains("Workflow trigger completed")
                            && output_text.contains("Workflow reply")
                        {
                            interrupt_sent = true;
                            for _ in 0..4 {
                                let _ = writer_tx.send(vec![3]).await;
                                sleep(Duration::from_millis(200)).await;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break exit_rx.await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                },
                result = &mut exit_rx => break result,
            }
        }
    })
    .await;

    let exit_code = match exit_code_result {
        Ok(Ok(code)) => code,
        Ok(Err(err)) => return Err(err.into()),
        Err(_) => {
            session.terminate();
            let output_text = String::from_utf8_lossy(&output);
            anyhow::bail!(
                "timed out waiting for TUI workflow follow-up session to exit; \
answered_cursor_query={answered_cursor_query}; startup_ready={startup_ready}; \
prompt_sent={prompt_sent}; interrupt_sent={interrupt_sent}; output: {output_text}"
            );
        }
    };

    let output_text = String::from_utf8_lossy(&output);
    let interrupt_only_output = {
        let trimmed = output_text.trim();
        !trimmed.is_empty()
            && trimmed
                .chars()
                .all(|character| character == '^' || character == 'C' || character.is_whitespace())
    };
    anyhow::ensure!(
        exit_code == 0 || exit_code == 130 || (exit_code == 1 && interrupt_only_output),
        "unexpected exit code from codex resume: {exit_code}; output: {output_text}",
    );

    anyhow::ensure!(
        output_text.contains("Workflow trigger completed"),
        "expected workflow trigger completion in output: {output_text}",
    );
    anyhow::ensure!(
        output_text.contains("Workflow reply"),
        "expected workflow reply marker in output: {output_text}",
    );
    anyhow::ensure!(
        output_text.matches("fixture hello").count() >= 2,
        "expected at least two assistant replies in output: {output_text}",
    );

    Ok(())
}

fn write_after_turn_prompt_workflow(repo: &Path) -> Result<()> {
    std::fs::write(
        repo.join(".codex/workflows/after_turn.yaml"),
        r#"name: director

triggers:
  - type: after_turn
    id: followup
    jobs: [followup]

jobs:
  followup:
    response: user
    steps:
      - prompt: |
          generate a follow-up reply
"#,
    )?;
    Ok(())
}

fn write_test_config(codex_home: &Path, repo: &Path) -> Result<()> {
    let canonical_repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut trusted_paths = vec![repo.display().to_string()];
    if canonical_repo != repo {
        trusted_paths.push(canonical_repo.display().to_string());
    }
    let trusted_projects = trusted_paths
        .into_iter()
        .map(|path| format!("[projects.\"{path}\"]\ntrust_level = \"trusted\"\n"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"[notice.model_migrations]
"gpt-5.1" = "gpt-5.3-codex"

[profiles.newapi]
model = "gpt-5.4"
model_provider = "newapi"

[model_providers.newapi]
name = "newapi"
base_url = "http://127.0.0.1:3000/v1"
experimental_bearer_token = "test-token"
requires_openai_auth = false

{trusted_projects}
"#,
            trusted_projects = trusted_projects,
        ),
    )?;
    Ok(())
}
