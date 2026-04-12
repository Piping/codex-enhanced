use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use codex_rollout::policy::should_persist_response_item_for_memories;

pub fn serialize_filtered_rollout_response_items(items: &[RolloutItem]) -> Result<String> {
    let filtered = items
        .iter()
        .filter_map(|item| {
            if let RolloutItem::ResponseItem(item) = item {
                sanitize_response_item_for_retrospective(item)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&filtered).map_err(|err| {
        CodexErr::InvalidRequest(format!("failed to serialize rollout memory: {err}"))
    })
}

fn sanitize_response_item_for_retrospective(item: &ResponseItem) -> Option<ResponseItem> {
    let ResponseItem::Message {
        id,
        role,
        content,
        end_turn,
        phase,
    } = item
    else {
        return should_persist_response_item_for_memories(item).then(|| item.clone());
    };

    if role == "developer" {
        return None;
    }

    if content.is_empty() {
        return None;
    }

    Some(ResponseItem::Message {
        id: id.clone(),
        role: role.clone(),
        content: content.clone(),
        end_turn: *end_turn,
        phase: phase.clone(),
    })
}

#[cfg(test)]
mod tests {
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::RolloutItem;
    use pretty_assertions::assert_eq;

    use super::serialize_filtered_rollout_response_items;

    #[test]
    fn serializes_memory_rollout_with_agents_skills_and_environment_kept() {
        let mixed_contextual_message = ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![
                ContentItem::InputText {
                    text:
                        "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nbody\n</INSTRUCTIONS>"
                            .to_string(),
                },
                ContentItem::InputText {
                    text: "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>"
                        .to_string(),
                },
            ],
            end_turn: None,
            phase: None,
        };
        let skill_message = ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text:
                    "<skill>\n<name>demo</name>\n<path>skills/demo/SKILL.md</path>\nbody\n</skill>"
                        .to_string(),
            }],
            end_turn: None,
            phase: None,
        };
        let subagent_message = ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "<subagent_notification>{\"agent_id\":\"a\",\"status\":\"completed\"}</subagent_notification>"
                    .to_string(),
            }],
            end_turn: None,
            phase: None,
        };

        let serialized = serialize_filtered_rollout_response_items(&[
            RolloutItem::ResponseItem(mixed_contextual_message),
            RolloutItem::ResponseItem(skill_message),
            RolloutItem::ResponseItem(subagent_message.clone()),
        ])
        .expect("serialize");
        let parsed: Vec<ResponseItem> = serde_json::from_str(&serialized).expect("parse");

        assert_eq!(
            parsed,
            vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![
                        ContentItem::InputText {
                            text: "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nbody\n</INSTRUCTIONS>"
                                .to_string(),
                        },
                        ContentItem::InputText {
                            text: "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>"
                                .to_string(),
                        },
                    ],
                    end_turn: None,
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text:
                            "<skill>\n<name>demo</name>\n<path>skills/demo/SKILL.md</path>\nbody\n</skill>"
                                .to_string(),
                    }],
                    end_turn: None,
                    phase: None,
                },
                subagent_message,
            ]
        );
    }
}
