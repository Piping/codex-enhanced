use crate::codex::Session;
use crate::config::Config;
use crate::memories::phase1;
use crate::memories::phase2;
use codex_features::Feature;
use codex_protocol::protocol::SessionSource;
use std::sync::Arc;
use tracing::warn;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemoriesPipelineTrigger {
    Startup,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoriesStartupSkipReason {
    EphemeralSession,
    FeatureDisabled,
    SubagentSession,
    StateDbUnavailable,
}

impl std::fmt::Display for MemoriesStartupSkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EphemeralSession => {
                f.write_str("memory startup is unavailable for ephemeral sessions")
            }
            Self::FeatureDisabled => f.write_str("memory startup is disabled by configuration"),
            Self::SubagentSession => {
                f.write_str("memory startup is unavailable for subagent sessions")
            }
            Self::StateDbUnavailable => {
                f.write_str("state db unavailable for memories startup pipeline")
            }
        }
    }
}

fn memories_startup_skip_reason(
    session: &Session,
    config: &Config,
    source: &SessionSource,
    trigger: MemoriesPipelineTrigger,
) -> Option<MemoriesStartupSkipReason> {
    if config.ephemeral {
        return Some(MemoriesStartupSkipReason::EphemeralSession);
    }
    if matches!(trigger, MemoriesPipelineTrigger::Startup)
        && !config.features.enabled(Feature::MemoryTool)
    {
        return Some(MemoriesStartupSkipReason::FeatureDisabled);
    }
    if matches!(source, SessionSource::SubAgent(_)) {
        return Some(MemoriesStartupSkipReason::SubagentSession);
    }
    if session.services.state_db.is_none() {
        return Some(MemoriesStartupSkipReason::StateDbUnavailable);
    }
    None
}

pub(crate) async fn run_memories_startup_pipeline(
    session: &Arc<Session>,
    config: Arc<Config>,
    source: &SessionSource,
) -> Result<(), MemoriesStartupSkipReason> {
    if let Some(reason) =
        memories_startup_skip_reason(session, &config, source, MemoriesPipelineTrigger::Startup)
    {
        return Err(reason);
    }

    run_memories_pipeline_inner(session, config).await;
    Ok(())
}

pub(crate) async fn run_memories_manual_pipeline(
    session: &Arc<Session>,
    config: Arc<Config>,
    source: &SessionSource,
) -> Result<(), MemoriesStartupSkipReason> {
    if let Some(reason) =
        memories_startup_skip_reason(session, &config, source, MemoriesPipelineTrigger::Manual)
    {
        return Err(reason);
    }

    run_memories_pipeline_inner(session, config).await;
    Ok(())
}

async fn run_memories_pipeline_inner(session: &Arc<Session>, config: Arc<Config>) {
    // Clean memories to preserve DB size.
    phase1::prune(session, &config).await;
    // Run phase 1.
    phase1::run(session, &config).await;
    // Run phase 2.
    phase2::run(session, config).await;
}

/// Starts the asynchronous startup memory pipeline for an eligible root session.
///
/// The pipeline is skipped for ephemeral sessions, disabled feature flags, and
/// subagent sessions.
pub(crate) fn start_memories_startup_task(
    session: &Arc<Session>,
    config: Arc<Config>,
    source: &SessionSource,
) {
    if let Some(reason) =
        memories_startup_skip_reason(session, &config, source, MemoriesPipelineTrigger::Startup)
    {
        if matches!(reason, MemoriesStartupSkipReason::StateDbUnavailable) {
            warn!("{reason}; skipping");
        }
        return;
    }

    let weak_session = Arc::downgrade(session);
    let source = source.clone();
    tokio::spawn(async move {
        let Some(session) = weak_session.upgrade() else {
            return;
        };

        if let Err(err) = run_memories_startup_pipeline(&session, config, &source).await
            && matches!(err, MemoriesStartupSkipReason::StateDbUnavailable)
        {
            warn!("{err}; skipping");
        }
    });
}
