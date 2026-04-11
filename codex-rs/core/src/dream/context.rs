use super::types::DreamContext;
use super::types::DreamSkillCandidate;
use crate::RolloutRecorder;
use crate::config::Config;
use codex_git_utils::resolve_root_git_project_for_trust;
use codex_instructions::AGENTS_MD_FRAGMENT;
use codex_instructions::SKILL_FRAGMENT;
use codex_protocol::ThreadId;
use codex_protocol::protocol::RolloutItem;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

pub(super) async fn load_dream_context(
    config: &Config,
    thread_id: ThreadId,
    rollout_path: &Path,
) -> anyhow::Result<DreamContext> {
    let repo_root = resolve_root_git_project_for_trust(config.cwd.as_path())
        .unwrap_or_else(|| config.cwd.to_path_buf());
    let memory_root = repo_root.join(".codex").join("memory");
    let agents_path = repo_root.join("AGENTS.md");

    let (rollout_items, _, _) = RolloutRecorder::load_rollout_items(rollout_path).await?;
    let rollout_items_json =
        crate::memories::serialize_filtered_rollout_response_items(&rollout_items)?;
    let visible_agents_fragments = visible_agents_fragments(&rollout_items);
    let skill_candidates = repo_local_skill_candidates(&rollout_items, repo_root.as_path()).await?;
    let visible_skill_fragments = skill_candidates
        .iter()
        .map(|skill| {
            format!(
                "name: {}\npath: {}\n\n{}",
                skill.name,
                skill.path.display(),
                skill.contents
            )
        })
        .collect::<Vec<_>>();

    Ok(DreamContext {
        thread_id,
        rollout_path: rollout_path.to_path_buf(),
        repo_root: repo_root.clone(),
        memory_root: memory_root.clone(),
        existing_memory: read_text_if_exists(&memory_root.join("MEMORY.md")).await?,
        existing_agents: read_text_if_exists(&agents_path).await?,
        agents_path,
        skill_candidates,
        visible_agents_fragments,
        visible_skill_fragments,
        rollout_items_json,
    })
}

fn visible_agents_fragments(items: &[RolloutItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(user_text_from_rollout_item)
        .filter(|text| AGENTS_MD_FRAGMENT.matches_text(text))
        .map(ToOwned::to_owned)
        .collect()
}

async fn repo_local_skill_candidates(
    items: &[RolloutItem],
    repo_root: &Path,
) -> anyhow::Result<Vec<DreamSkillCandidate>> {
    let repo_root = canonicalize_best_effort(repo_root).await?;
    let mut by_path = BTreeMap::<PathBuf, DreamSkillCandidate>::new();

    for text in items.iter().filter_map(user_text_from_rollout_item) {
        if !SKILL_FRAGMENT.matches_text(text) {
            continue;
        }
        let Some(name) = extract_tag(text, "name") else {
            continue;
        };
        let Some(path_text) = extract_tag(text, "path") else {
            continue;
        };
        let skill_path = PathBuf::from(path_text);
        let canonical_skill_path = match canonicalize_best_effort(&skill_path).await {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !canonical_skill_path.starts_with(&repo_root) || !canonical_skill_path.is_file() {
            continue;
        }
        let contents = match tokio::fs::read_to_string(&canonical_skill_path).await {
            Ok(contents) => contents,
            Err(_) => continue,
        };
        by_path
            .entry(canonical_skill_path.clone())
            .or_insert(DreamSkillCandidate {
                name,
                path: canonical_skill_path,
                contents,
            });
    }

    Ok(by_path.into_values().collect())
}

fn user_text_from_rollout_item(item: &RolloutItem) -> Option<&str> {
    let RolloutItem::ResponseItem(codex_protocol::models::ResponseItem::Message {
        role,
        content,
        ..
    }) = item
    else {
        return None;
    };
    if role != "user" {
        return None;
    }
    let [codex_protocol::models::ContentItem::InputText { text }] = content.as_slice() else {
        return None;
    };
    Some(text.as_str())
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = text[start..].find(&end_tag)? + start;
    Some(text[start..end].trim().to_string())
}

async fn read_text_if_exists(path: &Path) -> anyhow::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

async fn canonicalize_best_effort(path: &Path) -> anyhow::Result<PathBuf> {
    match tokio::fs::canonicalize(path).await {
        Ok(path) => Ok(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::extract_tag;
    use super::repo_local_skill_candidates;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::RolloutItem;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn extract_tag_returns_trimmed_value() {
        let text = "<skill>\n<name> demo </name>\n<path>/tmp/demo</path>\nbody\n</skill>";
        assert_eq!(extract_tag(text, "name"), Some("demo".to_string()));
        assert_eq!(extract_tag(text, "path"), Some("/tmp/demo".to_string()));
        assert_eq!(extract_tag(text, "missing"), None);
    }

    #[tokio::test]
    async fn repo_local_skill_candidates_keeps_repo_local_visible_skills() {
        let repo = TempDir::new().expect("temp repo");
        let skill_dir = repo.path().join(".codex").join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, "skill body").expect("write skill");
        let skill_path = std::fs::canonicalize(&skill_path).expect("canonical skill path");

        let items = vec![RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: format!(
                    "<skill>\n<name>demo</name>\n<path>{}</path>\nbody\n</skill>",
                    skill_path.display()
                ),
            }],
            end_turn: None,
            phase: None,
        })];

        let candidates = repo_local_skill_candidates(&items, repo.path())
            .await
            .expect("skill candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "demo");
        assert_eq!(candidates[0].path, skill_path);
        assert_eq!(candidates[0].contents, "skill body");
    }
}
