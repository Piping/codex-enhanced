use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamPipelineResult {
    pub memory_root: PathBuf,
    pub retrospective_path: PathBuf,
    pub updated_agents_path: PathBuf,
    pub updated_skill_paths: Vec<PathBuf>,
    pub next_session_hint: String,
}

#[derive(Debug, Clone)]
pub(super) struct DreamContext {
    pub(super) thread_id: ThreadId,
    pub(super) rollout_path: PathBuf,
    pub(super) repo_root: PathBuf,
    pub(super) memory_root: PathBuf,
    pub(super) existing_memory: Option<String>,
    pub(super) existing_agents: Option<String>,
    pub(super) agents_path: PathBuf,
    pub(super) skill_candidates: Vec<DreamSkillCandidate>,
    pub(super) visible_agents_fragments: Vec<String>,
    pub(super) visible_skill_fragments: Vec<String>,
    pub(super) rollout_items_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DreamSkillCandidate {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) contents: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DreamModelOutput {
    pub(super) thread_title: String,
    pub(super) thread_summary_md: String,
    pub(super) memory_block_md: String,
    pub(super) next_session_hint_md: String,
    pub(super) agents_block_md: String,
    #[serde(default)]
    pub(super) skills: Vec<DreamSkillUpdate>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DreamSkillUpdate {
    pub(super) path: String,
    pub(super) block_md: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DreamIndex {
    pub updated_at: DateTime<Utc>,
    pub documents: Vec<DreamIndexDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DreamIndexDocument {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamSearchResult {
    pub document: DreamIndexDocument,
    pub score: f32,
}
