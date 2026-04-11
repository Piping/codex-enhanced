mod context;
mod prompts;
mod storage;
mod types;

pub use storage::search_index;
pub use types::DreamIndex;
pub use types::DreamIndexDocument;
pub use types::DreamPipelineResult;
pub use types::DreamSearchResult;

use crate::codex::Session;
use crate::config::Config;
use std::path::Path;
use std::sync::Arc;

pub async fn run_dream_pipeline(
    session: &Arc<Session>,
    config: Arc<Config>,
    thread_id: codex_protocol::ThreadId,
    rollout_path: &Path,
    model_name: &str,
) -> anyhow::Result<DreamPipelineResult> {
    let context = context::load_dream_context(&config, thread_id, rollout_path).await?;
    let request_context =
        prompts::build_request_context(session.as_ref(), &config, model_name).await;
    let (output, _token_usage) =
        prompts::sample(session.as_ref(), &context, &request_context).await?;
    storage::write_dream_artifacts(&context, &output).await
}
