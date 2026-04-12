use std::sync::LazyLock;

use codex_secrets::redact_secrets;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_text;
use codex_utils_template::Template;
use serde_json::Value;
use serde_json::json;

use crate::types::DreamContext;
use crate::types::DreamModelOutput;
use crate::types::DreamPromptRequest;

static DREAM_SYSTEM_TEMPLATE: LazyLock<Template> = LazyLock::new(|| {
    parse_embedded_template(
        include_str!("../templates/dream/system.md"),
        "dream/system.md",
    )
});
static DREAM_INPUT_TEMPLATE: LazyLock<Template> = LazyLock::new(|| {
    parse_embedded_template(
        include_str!("../templates/dream/input.md"),
        "dream/input.md",
    )
});

const ROLLOUT_TOKEN_LIMIT: usize = 80_000;
const EXISTING_MEMORY_TOKEN_LIMIT: usize = 6_000;
const EXISTING_AGENTS_TOKEN_LIMIT: usize = 6_000;
const VISIBLE_FRAGMENT_TOKEN_LIMIT: usize = 4_000;
const SKILL_CONTENT_TOKEN_LIMIT: usize = 4_000;

pub(crate) fn build_dream_prompt_request(
    context: &DreamContext,
) -> anyhow::Result<DreamPromptRequest> {
    Ok(DreamPromptRequest {
        system_prompt: DREAM_SYSTEM_TEMPLATE.render(std::iter::empty::<(&str, &str)>())?,
        input_message: build_dream_input_message(context)?,
        output_schema: output_schema(),
    })
}

pub(crate) fn parse_dream_model_output(result: &str) -> anyhow::Result<DreamModelOutput> {
    let mut output: DreamModelOutput = serde_json::from_str(result)?;
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
    Ok(output)
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
    use pretty_assertions::assert_eq;

    use super::output_schema;

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
