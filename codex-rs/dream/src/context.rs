use crate::types::DreamContext;
use crate::types::DreamPipelineRequest;
use crate::types::DreamSkillCandidate;
use codex_git_utils::resolve_root_git_project_for_trust;
use codex_instructions::AGENTS_MD_FRAGMENT;
use codex_instructions::SKILL_FRAGMENT;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use walkdir::WalkDir;

pub(crate) async fn load_dream_context(
    request: &DreamPipelineRequest<'_>,
) -> anyhow::Result<DreamContext> {
    let repo_root = resolve_root_git_project_for_trust(request.cwd)
        .unwrap_or_else(|| request.cwd.to_path_buf());
    let memory_root = repo_root.join(".codex").join("memory");
    let repo_skill_root = repo_root.join(".codex").join("skills");
    let agents_path = repo_root.join("AGENTS.md");
    let visible_agents_fragments = visible_agents_fragments(request.rollout_items);
    let visible_skills =
        explicit_repo_local_skill_candidates(request.rollout_items, repo_root.as_path()).await?;
    let skill_candidates =
        repo_local_skill_candidates(request.rollout_items, repo_root.as_path()).await?;
    let visible_skill_fragments = visible_skills
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
        thread_id: request.thread_id,
        rollout_path: request.rollout_path.to_path_buf(),
        repo_root: repo_root.clone(),
        memory_root: memory_root.clone(),
        repo_skill_root,
        existing_memory: read_text_if_exists(&memory_root.join("MEMORY.md")).await?,
        existing_agents: read_text_if_exists(&agents_path).await?,
        agents_path,
        skill_candidates,
        visible_agents_fragments,
        visible_skill_fragments,
        rollout_items_json: request.rollout_items_json.clone(),
    })
}

fn visible_agents_fragments(items: &[RolloutItem]) -> Vec<String> {
    items
        .iter()
        .flat_map(user_texts_from_rollout_item)
        .filter(|text| AGENTS_MD_FRAGMENT.matches_text(text))
        .map(ToOwned::to_owned)
        .collect()
}

async fn repo_local_skill_candidates(
    items: &[RolloutItem],
    repo_root: &Path,
) -> anyhow::Result<Vec<DreamSkillCandidate>> {
    let repo_root = canonicalize_best_effort(repo_root).await?;
    let repo_skill_root = repo_root.join(".codex").join("skills");
    let mut by_path = explicit_repo_local_skill_candidates(items, &repo_root)
        .await?
        .into_iter()
        .map(|skill| (skill.path.clone(), skill))
        .collect::<BTreeMap<_, _>>();

    for skill in discover_repo_local_skills(&repo_skill_root).await? {
        by_path.entry(skill.path.clone()).or_insert(skill);
    }

    Ok(by_path.into_values().collect())
}

async fn explicit_repo_local_skill_candidates(
    items: &[RolloutItem],
    repo_root: &Path,
) -> anyhow::Result<Vec<DreamSkillCandidate>> {
    let repo_root = canonicalize_best_effort(repo_root).await?;
    let mut by_path = BTreeMap::<PathBuf, DreamSkillCandidate>::new();

    for text in items.iter().flat_map(user_texts_from_rollout_item) {
        if !SKILL_FRAGMENT.matches_text(text) {
            continue;
        }
        let Some(name) = extract_tag(text, "name") else {
            continue;
        };
        let Some(path_text) = extract_tag(text, "path") else {
            continue;
        };
        let skill_path = resolve_repo_relative_path(&repo_root, &path_text);
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

async fn discover_repo_local_skills(
    repo_skill_root: &Path,
) -> anyhow::Result<Vec<DreamSkillCandidate>> {
    let repo_skill_root = canonicalize_best_effort(repo_skill_root).await?;
    if !repo_skill_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut by_path = BTreeMap::<PathBuf, DreamSkillCandidate>::new();
    for entry in WalkDir::new(&repo_skill_root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file()
            || path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md")
        {
            continue;
        }
        let canonical_skill_path = canonicalize_best_effort(path).await?;
        let Ok(contents) = tokio::fs::read_to_string(&canonical_skill_path).await else {
            continue;
        };
        by_path.insert(
            canonical_skill_path.clone(),
            DreamSkillCandidate {
                name: skill_name_from_contents(&contents, &canonical_skill_path),
                path: canonical_skill_path,
                contents,
            },
        );
    }

    Ok(by_path.into_values().collect())
}

fn user_texts_from_rollout_item(item: &RolloutItem) -> impl Iterator<Item = &str> {
    let content = match item {
        RolloutItem::ResponseItem(ResponseItem::Message { role, content, .. })
            if role == "user" =>
        {
            Some(content.as_slice())
        }
        _ => None,
    };

    content
        .into_iter()
        .flat_map(|content| content.iter())
        .filter_map(|content| match content {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = text[start..].find(&end_tag)? + start;
    Some(text[start..end].trim().to_string())
}

fn resolve_repo_relative_path(repo_root: &Path, path_text: &str) -> PathBuf {
    let path = PathBuf::from(path_text);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn skill_name_from_contents(contents: &str, skill_path: &Path) -> String {
    #[derive(Deserialize)]
    struct SkillFrontmatter {
        name: Option<String>,
    }

    extract_frontmatter(contents)
        .and_then(|frontmatter| serde_yaml::from_str::<SkillFrontmatter>(&frontmatter).ok())
        .and_then(|frontmatter| frontmatter.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| {
            skill_path
                .parent()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("repo-skill")
                .to_string()
        })
}

fn extract_frontmatter(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()? != "---" {
        return None;
    }

    let mut frontmatter = Vec::new();
    for line in lines {
        if line == "---" {
            return (!frontmatter.is_empty()).then(|| frontmatter.join("\n"));
        }
        frontmatter.push(line);
    }
    None
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
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::RolloutItem;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::extract_tag;
    use super::repo_local_skill_candidates;

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

    #[tokio::test]
    async fn repo_local_skill_candidates_discovers_repo_local_skills_without_thread_fragments() {
        let repo = TempDir::new().expect("temp repo");
        let skill_dir = repo
            .path()
            .join(".codex")
            .join("skills")
            .join("deep-review");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: deep-review\ndescription: review failures\n---\n\n# Body\n",
        )
        .expect("write skill");
        let skill_path = std::fs::canonicalize(&skill_path).expect("canonical skill path");

        let candidates = repo_local_skill_candidates(&[], repo.path())
            .await
            .expect("skill candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "deep-review");
        assert_eq!(candidates[0].path, skill_path);
    }
}
