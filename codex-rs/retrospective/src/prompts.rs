use std::path::Path;
use std::sync::LazyLock;

use codex_protocol::openai_models::ModelInfo;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_text;
use codex_utils_template::Template;

static ROLLOUT_RETROSPECTIVE_INPUT_TEMPLATE: LazyLock<Template> = LazyLock::new(|| {
    parse_embedded_template(
        include_str!("../templates/retrospective/rollout_input.md"),
        "retrospective/rollout_input.md",
    )
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrospectiveInputBudget {
    pub fallback_rollout_token_limit: usize,
    pub context_window_percent: i64,
}

pub fn build_rollout_retrospective_input_message(
    model_info: &ModelInfo,
    budget: RetrospectiveInputBudget,
    rollout_path: &Path,
    rollout_cwd: &Path,
    rollout_contents: &str,
) -> anyhow::Result<String> {
    let rollout_token_limit = model_info
        .context_window
        .and_then(|limit| (limit > 0).then_some(limit))
        .map(|limit| limit.saturating_mul(model_info.effective_context_window_percent) / 100)
        .map(|limit| (limit.saturating_mul(budget.context_window_percent) / 100).max(1))
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(budget.fallback_rollout_token_limit);
    let truncated_rollout_contents = truncate_text(
        rollout_contents,
        TruncationPolicy::Tokens(rollout_token_limit),
    );

    let rollout_path = rollout_path.display().to_string();
    let rollout_cwd = rollout_cwd.display().to_string();
    Ok(ROLLOUT_RETROSPECTIVE_INPUT_TEMPLATE.render([
        ("rollout_path", rollout_path.as_str()),
        ("rollout_cwd", rollout_cwd.as_str()),
        ("rollout_contents", truncated_rollout_contents.as_str()),
    ])?)
}

fn parse_embedded_template(source: &'static str, template_name: &str) -> Template {
    match Template::parse(source) {
        Ok(template) => template,
        Err(err) => panic!("embedded template {template_name} is invalid: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use codex_models_manager::model_info::model_info_from_slug;
    use codex_utils_output_truncation::TruncationPolicy;
    use codex_utils_output_truncation::truncate_text;

    use super::RetrospectiveInputBudget;
    use super::build_rollout_retrospective_input_message;

    #[test]
    fn build_rollout_retrospective_input_message_truncates_rollout_using_model_context_window() {
        let input = format!("{}{}{}", "a".repeat(700_000), "middle", "z".repeat(700_000));
        let mut model_info = model_info_from_slug("gpt-5.2-codex");
        model_info.context_window = Some(123_000);
        let expected_rollout_token_limit = usize::try_from(
            ((123_000_i64 * model_info.effective_context_window_percent) / 100) * 70 / 100,
        )
        .unwrap();
        let expected_truncated = truncate_text(
            &input,
            TruncationPolicy::Tokens(expected_rollout_token_limit),
        );

        let message = build_rollout_retrospective_input_message(
            &model_info,
            RetrospectiveInputBudget {
                fallback_rollout_token_limit: 150_000,
                context_window_percent: 70,
            },
            Path::new("/tmp/rollout.jsonl"),
            Path::new("/tmp"),
            &input,
        )
        .unwrap();

        assert!(expected_truncated.contains("tokens truncated"));
        assert!(message.contains(&expected_truncated));
        assert!(message.contains("/tmp/rollout.jsonl"));
        assert!(message.contains("/tmp"));
    }

    #[test]
    fn build_rollout_retrospective_input_message_uses_default_limit_when_context_window_missing() {
        let input = format!("{}{}{}", "a".repeat(700_000), "middle", "z".repeat(700_000));
        let mut model_info = model_info_from_slug("gpt-5.2-codex");
        model_info.context_window = None;
        let expected_truncated = truncate_text(&input, TruncationPolicy::Tokens(150_000));

        let message = build_rollout_retrospective_input_message(
            &model_info,
            RetrospectiveInputBudget {
                fallback_rollout_token_limit: 150_000,
                context_window_percent: 70,
            },
            Path::new("/tmp/rollout.jsonl"),
            Path::new("/tmp"),
            &input,
        )
        .unwrap();

        assert!(message.contains(&expected_truncated));
    }
}
