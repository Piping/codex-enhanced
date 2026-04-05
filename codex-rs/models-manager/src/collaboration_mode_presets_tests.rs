use super::*;
use pretty_assertions::assert_eq;

#[test]
fn preset_names_use_mode_display_names() {
    assert_eq!(plan_preset().name, ModeKind::Plan.display_name());
    assert_eq!(default_preset().name, ModeKind::Default.display_name());
    assert_eq!(plan_preset().model, None);
    assert_eq!(
        plan_preset().reasoning_effort,
        Some(Some(ReasoningEffort::Medium))
    );
    assert_eq!(default_preset().model, None);
    assert_eq!(default_preset().reasoning_effort, None);
}

#[test]
fn default_mode_instructions_replace_mode_names_placeholder() {
    let default_instructions = default_preset()
        .developer_instructions
        .expect("default preset should include instructions")
        .expect("default instructions should be set");

    assert!(!default_instructions.contains("{{KNOWN_MODE_NAMES}}"));

    let known_mode_names = format_mode_names(&TUI_VISIBLE_COLLABORATION_MODES);
    let expected_snippet = format!("Known mode names are {known_mode_names}.");
    assert!(default_instructions.contains(&expected_snippet));

    let expected_availability_message =
        request_user_input_availability_message(/*default_mode_request_user_input*/ true);
    assert!(default_instructions.contains(&expected_availability_message));
    assert!(default_instructions.contains("prefer using the `question` tool"));
    assert!(default_instructions.contains("legacy `request_user_input` tool is still available"));
}

#[test]
fn asking_questions_guidance_uses_plain_text_questions_when_feature_disabled() {
    let guidance =
        asking_questions_guidance_message(/*default_mode_request_user_input*/ false);
    assert!(guidance.contains("ask the user directly with a concise plain-text question"));
    assert!(guidance.contains("prefer using the `question` tool"));
}
