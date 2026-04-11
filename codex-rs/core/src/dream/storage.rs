use super::types::DreamContext;
use super::types::DreamIndex;
use super::types::DreamIndexDocument;
use super::types::DreamModelOutput;
use super::types::DreamPipelineResult;
use super::types::DreamSearchResult;
use bm25::Document;
use bm25::Language;
use bm25::SearchEngineBuilder;
use chrono::Utc;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

const DREAM_START_MARKER: &str = "<!-- codex:dream:start -->";
const DREAM_END_MARKER: &str = "<!-- codex:dream:end -->";

pub(super) async fn write_dream_artifacts(
    context: &DreamContext,
    output: &DreamModelOutput,
) -> anyhow::Result<DreamPipelineResult> {
    tokio::fs::create_dir_all(&context.memory_root).await?;

    let memory_path = context.memory_root.join("MEMORY.md");
    write_managed_block_file(
        &memory_path,
        "Dream Memory",
        &output.memory_block_md,
        Some("# Codex Memory"),
    )
    .await?;

    let updated_agents_path = context.agents_path.clone();
    write_managed_block_file(
        &updated_agents_path,
        "Dream Guidance",
        &output.agents_block_md,
        None,
    )
    .await?;

    let thread_dir = context
        .memory_root
        .join("threads")
        .join(context.thread_id.to_string());
    tokio::fs::create_dir_all(&thread_dir).await?;
    let retrospective_path = thread_dir.join("retrospective.md");
    tokio::fs::write(
        &retrospective_path,
        build_retrospective_markdown(context, output),
    )
    .await?;

    let next_session_hint = output.next_session_hint_md.trim().to_string();
    let next_session_path = context.memory_root.join("next_session.md");
    tokio::fs::write(&next_session_path, format!("{next_session_hint}\n")).await?;

    let skill_candidates = context
        .skill_candidates
        .iter()
        .map(|skill| (skill.path.to_string_lossy().to_string(), skill.path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut updated_skill_paths = Vec::new();
    for skill in &output.skills {
        let Some(skill_path) = skill_candidates.get(&skill.path).cloned() else {
            continue;
        };
        write_managed_block_file(&skill_path, "Dream Notes", &skill.block_md, None).await?;
        updated_skill_paths.push(skill_path);
    }

    let index = build_index(
        &memory_path,
        &next_session_path,
        &retrospective_path,
        &updated_agents_path,
        &updated_skill_paths,
    )
    .await?;
    let index_path = context.memory_root.join("index.json");
    tokio::fs::write(&index_path, serde_json::to_vec_pretty(&index)?).await?;

    Ok(DreamPipelineResult {
        memory_root: context.memory_root.clone(),
        retrospective_path,
        updated_agents_path,
        updated_skill_paths,
        next_session_hint,
    })
}

pub fn search_index(index: &DreamIndex, query: &str, limit: usize) -> Vec<DreamSearchResult> {
    if query.trim().is_empty() || limit == 0 || index.documents.is_empty() {
        return Vec::new();
    }

    let documents = index
        .documents
        .iter()
        .enumerate()
        .map(|(idx, doc)| {
            Document::new(
                idx,
                format!("{} {} {} {}", doc.title, doc.kind, doc.path, doc.text),
            )
        })
        .collect::<Vec<_>>();
    let search_engine =
        SearchEngineBuilder::<usize>::with_documents(Language::English, documents).build();

    search_engine
        .search(query, limit)
        .into_iter()
        .filter_map(|result| {
            index
                .documents
                .get(result.document.id)
                .cloned()
                .map(|document| DreamSearchResult {
                    document,
                    score: result.score,
                })
        })
        .collect()
}

async fn build_index(
    memory_path: &Path,
    next_session_path: &Path,
    retrospective_path: &Path,
    agents_path: &Path,
    skill_paths: &[PathBuf],
) -> anyhow::Result<DreamIndex> {
    let mut documents = Vec::new();
    append_document(
        &mut documents,
        "memory",
        "Dream Memory",
        memory_path,
        /*managed_only*/ false,
    )
    .await?;
    append_document(
        &mut documents,
        "nextSession",
        "Next Session Hint",
        next_session_path,
        /*managed_only*/ false,
    )
    .await?;
    append_document(
        &mut documents,
        "threadRetrospective",
        "Thread Retrospective",
        retrospective_path,
        /*managed_only*/ false,
    )
    .await?;
    append_document(
        &mut documents,
        "agents",
        "Dream Guidance",
        agents_path,
        /*managed_only*/ true,
    )
    .await?;
    for path in skill_paths {
        let title = format!("Skill Dream Notes {}", path.display());
        append_document(
            &mut documents,
            "skill",
            title.as_str(),
            path,
            /*managed_only*/ true,
        )
        .await?;
    }

    Ok(DreamIndex {
        updated_at: Utc::now(),
        documents,
    })
}

async fn append_document(
    documents: &mut Vec<DreamIndexDocument>,
    kind: &str,
    title: &str,
    path: &Path,
    managed_only: bool,
) -> anyhow::Result<()> {
    let contents = tokio::fs::read_to_string(path).await?;
    let text = if managed_only {
        managed_block_contents(&contents).unwrap_or(contents)
    } else {
        contents
    };
    documents.push(DreamIndexDocument {
        id: format!("{kind}:{}", path.display()),
        title: title.to_string(),
        kind: kind.to_string(),
        path: path.display().to_string(),
        text,
    });
    Ok(())
}

async fn write_managed_block_file(
    path: &Path,
    title: &str,
    body: &str,
    default_prefix: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let existing = match tokio::fs::read_to_string(path).await {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    let updated = upsert_managed_block(existing.as_deref(), title, body, default_prefix);
    tokio::fs::write(path, updated).await?;
    Ok(())
}

fn build_retrospective_markdown(context: &DreamContext, output: &DreamModelOutput) -> String {
    format!(
        "# {}\n\nthread_id: {}\nrollout_path: {}\nrepo_root: {}\ngenerated_at: {}\n\n{}",
        output.thread_title.trim(),
        context.thread_id,
        context.rollout_path.display(),
        context.repo_root.display(),
        Utc::now().to_rfc3339(),
        output.thread_summary_md.trim(),
    )
}

fn upsert_managed_block(
    existing: Option<&str>,
    title: &str,
    body: &str,
    default_prefix: Option<&str>,
) -> String {
    let managed_block = managed_block(title, body);
    match existing {
        Some(existing)
            if existing.contains(DREAM_START_MARKER) && existing.contains(DREAM_END_MARKER) =>
        {
            if let (Some(start), Some(end_start)) = (
                existing.find(DREAM_START_MARKER),
                existing.find(DREAM_END_MARKER),
            ) {
                let end = end_start + DREAM_END_MARKER.len();
                format!(
                    "{}{}{}",
                    &existing[..start],
                    managed_block,
                    &existing[end..]
                )
            } else {
                with_default_prefix(managed_block, default_prefix)
            }
        }
        Some(existing) if existing.trim().is_empty() => {
            with_default_prefix(managed_block, default_prefix)
        }
        Some(existing) => {
            let mut updated = existing.trim_end().to_string();
            updated.push_str("\n\n");
            updated.push_str(&managed_block);
            updated.push('\n');
            updated
        }
        None => with_default_prefix(managed_block, default_prefix),
    }
}

fn with_default_prefix(managed_block: String, default_prefix: Option<&str>) -> String {
    match default_prefix {
        Some(prefix) if !prefix.trim().is_empty() => {
            format!("{}\n\n{}\n", prefix.trim_end(), managed_block)
        }
        _ => format!("{managed_block}\n"),
    }
}

fn managed_block(title: &str, body: &str) -> String {
    format!(
        "{DREAM_START_MARKER}\n## {title}\n\n{}\n{DREAM_END_MARKER}",
        body.trim()
    )
}

fn managed_block_contents(text: &str) -> Option<String> {
    let start = text.find(DREAM_START_MARKER)? + DREAM_START_MARKER.len();
    let end = text.find(DREAM_END_MARKER)?;
    Some(text[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::managed_block_contents;
    use super::search_index;
    use super::upsert_managed_block;
    use crate::dream::types::DreamIndex;
    use crate::dream::types::DreamIndexDocument;
    use chrono::Utc;
    use pretty_assertions::assert_eq;

    #[test]
    fn upsert_managed_block_appends_when_markers_are_missing() {
        let updated = upsert_managed_block(Some("# Existing"), "Dream Guidance", "body", None);
        assert!(updated.contains("# Existing"));
        assert!(updated.contains("## Dream Guidance"));
        assert!(updated.contains("body"));
    }

    #[test]
    fn upsert_managed_block_replaces_existing_marked_section() {
        let existing = "\
before
<!-- codex:dream:start -->
old
<!-- codex:dream:end -->
after";
        let updated = upsert_managed_block(Some(existing), "Dream Guidance", "new body", None);
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
        assert!(updated.contains("new body"));
        assert!(!updated.contains("old"));
    }

    #[test]
    fn managed_block_contents_extracts_inner_body() {
        let contents = managed_block_contents(
            "<!-- codex:dream:start -->\n## Dream Guidance\n\nhello\n<!-- codex:dream:end -->",
        );
        assert_eq!(contents, Some("## Dream Guidance\n\nhello".to_string()));
    }

    #[test]
    fn search_index_returns_keyword_matches() {
        let index = DreamIndex {
            updated_at: Utc::now(),
            documents: vec![
                DreamIndexDocument {
                    id: "memory:1".to_string(),
                    title: "Dream Memory".to_string(),
                    kind: "memory".to_string(),
                    path: "/tmp/MEMORY.md".to_string(),
                    text: "workspace uses feishu websocket runtime".to_string(),
                },
                DreamIndexDocument {
                    id: "memory:2".to_string(),
                    title: "Next Session Hint".to_string(),
                    kind: "nextSession".to_string(),
                    path: "/tmp/next_session.md".to_string(),
                    text: "focus on slash commands".to_string(),
                },
            ],
        };

        let results = search_index(&index, "feishu runtime", 2);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document.id, "memory:1");
    }
}
