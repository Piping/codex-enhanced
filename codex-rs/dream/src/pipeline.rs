use std::future::Future;
use std::pin::Pin;

use crate::context::load_dream_context;
use crate::prompts::build_dream_prompt_request;
use crate::prompts::parse_dream_model_output;
use crate::storage::write_dream_artifacts;
use crate::types::DreamPipelineRequest;
use crate::types::DreamPipelineResult;
use crate::types::DreamPromptRequest;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Bridge for executing the `/dream` model request with app-specific runtime
/// state while leaving prompt construction, output parsing, and artifact
/// persistence in the shared crate.
pub trait DreamPromptSampler: Send + Sync {
    fn sample_dream<'a>(
        &'a self,
        request: DreamPromptRequest,
    ) -> BoxFuture<'a, anyhow::Result<String>>;
}

pub async fn run_dream_pipeline<S: DreamPromptSampler>(
    sampler: &S,
    request: DreamPipelineRequest<'_>,
) -> anyhow::Result<DreamPipelineResult> {
    let context = load_dream_context(&request).await?;
    let prompt_request = build_dream_prompt_request(&context)?;
    let raw_output = sampler.sample_dream(prompt_request).await?;
    let output = parse_dream_model_output(&raw_output)?;
    write_dream_artifacts(&context, &output).await
}

#[cfg(test)]
mod tests {
    use codex_protocol::ThreadId;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::RolloutItem;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::DreamPipelineRequest;
    use super::DreamPromptRequest;
    use super::DreamPromptSampler;
    use super::run_dream_pipeline;

    struct FakeDreamPromptSampler {
        response: String,
    }

    impl DreamPromptSampler for FakeDreamPromptSampler {
        fn sample_dream<'a>(
            &'a self,
            _request: DreamPromptRequest,
        ) -> super::BoxFuture<'a, anyhow::Result<String>> {
            Box::pin(async move { Ok(self.response.clone()) })
        }
    }

    #[tokio::test]
    async fn run_dream_pipeline_writes_artifacts_from_sampler_output() {
        let repo = tempdir().expect("repo");
        std::fs::create_dir_all(repo.path().join(".git")).expect("git dir");
        std::fs::write(repo.path().join("AGENTS.md"), "Existing agents\n").expect("agents");
        let rollout_path = repo.path().join("rollout.jsonl");
        let rollout_items = vec![RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Keep repo guidance concise.".to_string(),
            }],
            end_turn: None,
            phase: None,
        })];
        let sampler = FakeDreamPromptSampler {
            response: serde_json::json!({
                "threadTitle": "Dream Retrospective",
                "threadSummaryMd": "## Summary\n\n- Durable note.\n",
                "memoryBlockMd": "- Keep repo guidance concise.\n",
                "nextSessionHintMd": "Read AGENTS.md first.",
                "agentsBlockMd": "- Prefer concise guidance.\n",
                "skills": [],
                "newSkills": []
            })
            .to_string(),
        };

        let result = run_dream_pipeline(
            &sampler,
            DreamPipelineRequest {
                cwd: repo.path(),
                thread_id: ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000")
                    .expect("thread id"),
                rollout_path: rollout_path.as_path(),
                rollout_items: &rollout_items,
                rollout_items_json: "[]".to_string(),
            },
        )
        .await
        .expect("dream pipeline");

        let memory_md = tokio::fs::read_to_string(result.memory_root.join("MEMORY.md"))
            .await
            .expect("memory");
        assert!(memory_md.contains("Keep repo guidance concise"));
        assert_eq!(
            result.next_session_hint,
            "Read AGENTS.md first.".to_string()
        );
    }

    #[tokio::test]
    async fn run_dream_pipeline_creates_new_repo_local_skills() {
        let repo = tempdir().expect("repo");
        std::fs::create_dir_all(repo.path().join(".git")).expect("git dir");
        let sampler = FakeDreamPromptSampler {
            response: serde_json::json!({
                "threadTitle": "Dream Retrospective",
                "threadSummaryMd": "## Summary\n\n- Durable note.\n",
                "memoryBlockMd": "- Keep repo guidance concise.\n",
                "nextSessionHintMd": "Read AGENTS.md first.",
                "agentsBlockMd": "- Prefer concise guidance.\n",
                "skills": [],
                "newSkills": [
                    {
                        "name": "deep-review",
                        "description": "Capture retrospective workflow",
                        "contentsMd": "# Deep Review\n\nUse for dream follow-up.\n"
                    }
                ]
            })
            .to_string(),
        };

        let result = run_dream_pipeline(
            &sampler,
            DreamPipelineRequest {
                cwd: repo.path(),
                thread_id: ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000")
                    .expect("thread id"),
                rollout_path: repo.path().join("rollout.jsonl").as_path(),
                rollout_items: &[],
                rollout_items_json: "[]".to_string(),
            },
        )
        .await
        .expect("dream pipeline");

        assert_eq!(result.updated_skill_paths.len(), 1);
        let skill_md = tokio::fs::read_to_string(&result.updated_skill_paths[0])
            .await
            .expect("skill");
        assert!(skill_md.contains("name: deep-review"));
        assert!(skill_md.contains("# Deep Review"));
    }
}
