use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::protocol::RolloutItem;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
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
pub struct DreamPipelineRequest<'a> {
    pub cwd: &'a Path,
    pub thread_id: ThreadId,
    pub rollout_path: &'a Path,
    pub rollout_items: &'a [RolloutItem],
    pub rollout_items_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamPromptRequest {
    pub system_prompt: String,
    pub input_message: String,
    pub output_schema: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct DreamContext {
    pub(crate) thread_id: ThreadId,
    pub(crate) rollout_path: PathBuf,
    pub(crate) repo_root: PathBuf,
    pub(crate) memory_root: PathBuf,
    pub(crate) existing_memory: Option<String>,
    pub(crate) existing_agents: Option<String>,
    pub(crate) agents_path: PathBuf,
    pub(crate) skill_candidates: Vec<DreamSkillCandidate>,
    pub(crate) visible_agents_fragments: Vec<String>,
    pub(crate) visible_skill_fragments: Vec<String>,
    pub(crate) rollout_items_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DreamSkillCandidate {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) contents: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DreamModelOutput {
    pub(crate) thread_title: String,
    pub(crate) thread_summary_md: String,
    pub(crate) memory_block_md: String,
    pub(crate) next_session_hint_md: String,
    pub(crate) agents_block_md: String,
    #[serde(default)]
    pub(crate) skills: Vec<DreamSkillUpdate>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DreamSkillUpdate {
    pub(crate) path: String,
    pub(crate) block_md: String,
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
