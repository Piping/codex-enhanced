pub use codex_dream::DreamIndex;
pub use codex_dream::DreamIndexDocument;
pub use codex_dream::DreamPipelineResult;
pub use codex_dream::DreamSearchResult;
pub use codex_dream::search_index;

use std::sync::Arc;

use codex_dream::BoxFuture as DreamBoxFuture;
use codex_dream::DreamPipelineRequest;
use codex_dream::DreamPromptRequest;
use codex_dream::DreamPromptSampler;
use codex_dream::run_dream_pipeline as run_shared_dream_pipeline;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_retrospective::serialize_filtered_rollout_response_items;
use futures::StreamExt;

use crate::Prompt;
use crate::ResponseEvent;
use crate::RolloutRecorder;
use crate::codex::Session;
use crate::config::Config;
use crate::content_items_to_text;

struct CoreDreamPromptSampler<'a> {
    session: &'a Session,
    config: &'a Config,
    model_name: &'a str,
}

impl DreamPromptSampler for CoreDreamPromptSampler<'_> {
    fn sample_dream<'a>(
        &'a self,
        request: DreamPromptRequest,
    ) -> DreamBoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let model_info = self
                .session
                .services
                .models_manager
                .get_model_info(self.model_name, &self.config.to_models_manager_config())
                .await;
            let turn_context = self.session.new_default_turn().await;
            let turn_metadata_header = turn_context.turn_metadata_state.current_header_value();
            let prompt = Prompt {
                input: vec![ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: request.input_message,
                    }],
                    end_turn: None,
                    phase: None,
                }],
                tools: Vec::new(),
                parallel_tool_calls: false,
                base_instructions: BaseInstructions {
                    text: request.system_prompt,
                },
                personality: None,
                output_schema: Some(request.output_schema),
            };

            let mut client_session = self.session.services.model_client.new_session();
            let mut stream = client_session
                .stream(
                    &prompt,
                    &model_info,
                    &turn_context.session_telemetry,
                    Some(ReasoningEffort::Medium),
                    turn_context.reasoning_summary,
                    turn_context.config.service_tier,
                    turn_metadata_header.as_deref(),
                )
                .await?;

            let mut result = String::new();
            while let Some(message) = stream.next().await.transpose()? {
                match message {
                    ResponseEvent::OutputTextDelta(delta) => result.push_str(&delta),
                    ResponseEvent::OutputItemDone(item) => {
                        if result.is_empty()
                            && let ResponseItem::Message { content, .. } = item
                            && let Some(text) = content_items_to_text(&content)
                        {
                            result.push_str(&text);
                        }
                    }
                    ResponseEvent::Completed { .. } => break,
                    _ => {}
                }
            }

            Ok(result)
        })
    }
}

pub async fn run_dream_pipeline(
    session: &Arc<Session>,
    config: Arc<Config>,
    thread_id: codex_protocol::ThreadId,
    rollout_path: &std::path::Path,
    model_name: &str,
) -> anyhow::Result<DreamPipelineResult> {
    let (rollout_items, _, _) = RolloutRecorder::load_rollout_items(rollout_path).await?;
    let rollout_items_json = serialize_filtered_rollout_response_items(&rollout_items)?;
    let sampler = CoreDreamPromptSampler {
        session: session.as_ref(),
        config: config.as_ref(),
        model_name,
    };
    run_shared_dream_pipeline(
        &sampler,
        DreamPipelineRequest {
            cwd: config.cwd.as_path(),
            thread_id,
            rollout_path,
            rollout_items: &rollout_items,
            rollout_items_json,
        },
    )
    .await
}
