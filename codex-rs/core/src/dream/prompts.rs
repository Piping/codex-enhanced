use super::types::DreamContext;
use super::types::DreamModelOutput;
use crate::Prompt;
use crate::ResponseEvent;
use crate::codex::Session;
use crate::config::Config;
use crate::content_items_to_text;
use codex_otel::SessionTelemetry;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::TokenUsage;
use codex_secrets::redact_secrets;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_text;
use codex_utils_template::Template;
use futures::StreamExt;
use serde_json::Value;
use serde_json::json;
use std::sync::LazyLock;
use tracing::warn;

static DREAM_SYSTEM_TEMPLATE: LazyLock<Template> = LazyLock::new(|| {
    parse_embedded_template(
        include_str!("../../templates/dream/system.md"),
        "dream/system.md",
    )
});
static DREAM_INPUT_TEMPLATE: LazyLock<Template> = LazyLock::new(|| {
    parse_embedded_template(
        include_str!("../../templates/dream/input.md"),
        "dream/input.md",
    )
});

const ROLLOUT_TOKEN_LIMIT: usize = 80_000;
const EXISTING_MEMORY_TOKEN_LIMIT: usize = 6_000;
const EXISTING_AGENTS_TOKEN_LIMIT: usize = 6_000;
const VISIBLE_FRAGMENT_TOKEN_LIMIT: usize = 4_000;
const SKILL_CONTENT_TOKEN_LIMIT: usize = 4_000;

#[derive(Clone)]
pub(super) struct DreamRequestContext {
    pub(super) model_info: ModelInfo,
    pub(super) session_telemetry: SessionTelemetry,
    pub(super) reasoning_effort: Option<ReasoningEffort>,
    pub(super) reasoning_summary: ReasoningSummaryConfig,
    pub(super) service_tier: Option<ServiceTier>,
    pub(super) turn_metadata_header: Option<String>,
}

pub(super) async fn build_request_context(
    session: &Session,
    config: &Config,
    model_name: &str,
) -> DreamRequestContext {
    let model_info = session
        .services
        .models_manager
        .get_model_info(model_name, &config.to_models_manager_config())
        .await;
    let turn_context = session.new_default_turn().await;
    DreamRequestContext {
        model_info,
        session_telemetry: turn_context.session_telemetry.clone(),
        reasoning_effort: Some(ReasoningEffort::Medium),
        reasoning_summary: turn_context.reasoning_summary,
        service_tier: turn_context.config.service_tier,
        turn_metadata_header: turn_context.turn_metadata_state.current_header_value(),
    }
}

pub(super) async fn sample(
    session: &Session,
    context: &DreamContext,
    request_context: &DreamRequestContext,
) -> anyhow::Result<(DreamModelOutput, Option<TokenUsage>)> {
    let prompt = Prompt {
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: build_dream_input_message(context)?,
            }],
            end_turn: None,
            phase: None,
        }],
        tools: Vec::new(),
        parallel_tool_calls: false,
        base_instructions: BaseInstructions {
            text: DREAM_SYSTEM_TEMPLATE
                .render(std::iter::empty::<(&str, &str)>())
                .unwrap_or_else(|err| {
                    warn!("failed to render dream system prompt template: {err}");
                    "Produce a current-thread retrospective as structured JSON.".to_string()
                }),
        },
        personality: None,
        output_schema: Some(output_schema()),
    };

    let mut client_session = session.services.model_client.new_session();
    let mut stream = client_session
        .stream(
            &prompt,
            &request_context.model_info,
            &request_context.session_telemetry,
            request_context.reasoning_effort,
            request_context.reasoning_summary,
            request_context.service_tier,
            request_context.turn_metadata_header.as_deref(),
        )
        .await?;

    let mut result = String::new();
    let mut token_usage = None;
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
            ResponseEvent::Completed {
                token_usage: usage, ..
            } => {
                token_usage = usage;
                break;
            }
            _ => {}
        }
    }

    let mut output: DreamModelOutput = serde_json::from_str(&result)?;
    output.thread_title = redact_secrets(output.thread_title);
    output.thread_summary_md = redact_secrets(output.thread_summary_md);
    output.memory_block_md = redact_secrets(output.memory_block_md);
    output.next_session_hint_md = redact_secrets(output.next_session_hint_md);
    output.agents_block_md = redact_secrets(output.agents_block_md);
    output.skills = output
        .skills
        .into_iter()
        .map(|mut skill| {
            skill.block_md = redact_secrets(skill.block_md);
            skill
        })
        .collect();

    Ok((output, token_usage))
}

fn build_dream_input_message(context: &DreamContext) -> anyhow::Result<String> {
    let repo_root = context.repo_root.display().to_string();
    let memory_root = context.memory_root.display().to_string();
    let agents_path = context.agents_path.display().to_string();
    let thread_id = context.thread_id.to_string();
    let rollout_path = context.rollout_path.display().to_string();
    let existing_memory = truncate_segment(
        context.existing_memory.as_deref().unwrap_or("None."),
        EXISTING_MEMORY_TOKEN_LIMIT,
    );
    let existing_agents = truncate_segment(
        context.existing_agents.as_deref().unwrap_or("None."),
        EXISTING_AGENTS_TOKEN_LIMIT,
    );
    let visible_agents_fragments = truncate_joined_segments(
        &context.visible_agents_fragments,
        VISIBLE_FRAGMENT_TOKEN_LIMIT,
    );
    let visible_skill_fragments = truncate_joined_segments(
        &context.visible_skill_fragments,
        VISIBLE_FRAGMENT_TOKEN_LIMIT,
    );
    let skill_candidates = if context.skill_candidates.is_empty() {
        "None.".to_string()
    } else {
        context
            .skill_candidates
            .iter()
            .map(|skill| {
                format!(
                    "name: {}\npath: {}\n\n{}",
                    skill.name,
                    skill.path.display(),
                    truncate_segment(&skill.contents, SKILL_CONTENT_TOKEN_LIMIT)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    };
    let rollout_items_json = truncate_segment(&context.rollout_items_json, ROLLOUT_TOKEN_LIMIT);

    Ok(DREAM_INPUT_TEMPLATE.render([
        ("thread_id", thread_id.as_str()),
        ("rollout_path", rollout_path.as_str()),
        ("repo_root", repo_root.as_str()),
        ("memory_root", memory_root.as_str()),
        ("agents_path", agents_path.as_str()),
        ("existing_memory", existing_memory.as_str()),
        ("existing_agents", existing_agents.as_str()),
        (
            "visible_agents_fragments",
            visible_agents_fragments.as_str(),
        ),
        ("visible_skill_fragments", visible_skill_fragments.as_str()),
        ("skill_candidates", skill_candidates.as_str()),
        ("rollout_items_json", rollout_items_json.as_str()),
    ])?)
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "threadTitle": { "type": "string" },
            "threadSummaryMd": { "type": "string" },
            "memoryBlockMd": { "type": "string" },
            "nextSessionHintMd": { "type": "string" },
            "agentsBlockMd": { "type": "string" },
            "skills": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "blockMd": { "type": "string" }
                    },
                    "required": ["path", "blockMd"],
                    "additionalProperties": false
                }
            }
        },
        "required": [
            "threadTitle",
            "threadSummaryMd",
            "memoryBlockMd",
            "nextSessionHintMd",
            "agentsBlockMd",
            "skills"
        ],
        "additionalProperties": false
    })
}

fn truncate_segment(text: &str, token_limit: usize) -> String {
    truncate_text(text, TruncationPolicy::Tokens(token_limit))
}

fn truncate_joined_segments(segments: &[String], token_limit: usize) -> String {
    if segments.is_empty() {
        return "None.".to_string();
    }
    truncate_segment(&segments.join("\n\n---\n\n"), token_limit)
}

fn parse_embedded_template(source: &'static str, template_name: &str) -> Template {
    match Template::parse(source) {
        Ok(template) => template,
        Err(err) => panic!("embedded template {template_name} is invalid: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::output_schema;
    use pretty_assertions::assert_eq;

    #[test]
    fn dream_output_schema_requires_expected_fields() {
        let schema = output_schema();
        let required = schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            required,
            vec![
                "threadTitle",
                "threadSummaryMd",
                "memoryBlockMd",
                "nextSessionHintMd",
                "agentsBlockMd",
                "skills",
            ]
        );
    }
}
