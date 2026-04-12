use std::path::Path;
use std::path::PathBuf;

use notify::Event;
use notify::EventKind;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;

pub(crate) struct WorkflowFileWatchState {
    _watcher: RecommendedWatcher,
}

impl WorkflowFileWatchState {
    pub(crate) fn new(cwd: &Path, app_event_tx: AppEventSender) -> Result<Self, String> {
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                Ok(event) => {
                    if !should_forward_event(&event) {
                        return;
                    }
                    let changed_paths = event
                        .paths
                        .into_iter()
                        .filter(|path| path.is_absolute())
                        .collect::<Vec<PathBuf>>();
                    if changed_paths.is_empty() {
                        return;
                    }
                    app_event_tx.send(AppEvent::WorkflowWorkspaceFilesChanged { changed_paths });
                }
                Err(err) => {
                    tracing::warn!("workflow file watcher event failed: {err}");
                }
            })
            .map_err(|err| format!("failed to create watcher: {err}"))?;
        watcher
            .watch(cwd, RecursiveMode::Recursive)
            .map_err(|err| format!("failed to watch `{}`: {err}", cwd.display()))?;
        Ok(Self { _watcher: watcher })
    }
}

fn should_forward_event(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
    )
}

pub(crate) fn is_relevant_workspace_change(cwd: &Path, path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(relative_path) = path.strip_prefix(cwd) else {
        return false;
    };
    if relative_path.as_os_str().is_empty() {
        return true;
    }
    !relative_path.starts_with(".git") && !relative_path.starts_with(".codex")
}

#[cfg(test)]
mod tests {
    use super::is_relevant_workspace_change;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn relevant_workspace_change_accepts_regular_file_and_directory_paths() {
        let tempdir = tempdir().expect("tempdir");
        let cwd = tempdir.path();
        let file_path = cwd.join("src/main.rs");
        let directory_path = cwd.join("src");

        assert_eq!(is_relevant_workspace_change(cwd, &file_path), true);
        assert_eq!(is_relevant_workspace_change(cwd, &directory_path), true);
        assert_eq!(
            is_relevant_workspace_change(cwd, cwd.join(".git/index").as_path()),
            false,
        );
        assert_eq!(
            is_relevant_workspace_change(cwd, cwd.join(".codex/workflows/test.yaml").as_path()),
            false,
        );
    }
}
