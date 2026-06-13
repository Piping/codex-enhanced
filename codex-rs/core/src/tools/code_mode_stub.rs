use std::sync::Arc;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::ToolRouter;
use crate::tools::context::SharedTurnDiffTracker;

pub(crate) struct CodeModeService;

impl CodeModeService {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) async fn start_turn_worker(
        &self,
        _session: &Arc<Session>,
        _turn: &Arc<TurnContext>,
        _router: Arc<ToolRouter>,
        _tracker: SharedTurnDiffTracker,
    ) -> Option<CodeModeTurnWorker> {
        None
    }
}

pub(crate) struct CodeModeTurnWorker;
