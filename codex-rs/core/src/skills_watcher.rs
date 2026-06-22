//! Skills-specific watcher built on top of the generic [`FileWatcher`].

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::warn;

use crate::SkillsManager;
use crate::config::Config;
use crate::file_watcher::FileWatcher;
use crate::file_watcher::FileWatcherSubscriber;
use crate::file_watcher::Receiver;
use crate::file_watcher::ThrottledWatchReceiver;
use crate::file_watcher::WatchPath;
use crate::file_watcher::WatchRegistration;
use crate::skills_load_input_from_config;
use codex_core_plugins::PluginsManager;
use codex_core_skills::loader::SkillRoot;
use codex_exec_server::ExecutorFileSystem;
use codex_protocol::protocol::SkillScope;
use codex_utils_absolute_path::AbsolutePathBuf;

#[cfg(not(test))]
const WATCHER_THROTTLE_INTERVAL: Duration = Duration::from_secs(10);
#[cfg(test)]
const WATCHER_THROTTLE_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsWatcherEvent {
    SkillsChanged { paths: Vec<PathBuf> },
}

pub(crate) struct SkillsWatcher {
    subscriber: FileWatcherSubscriber,
    tx: broadcast::Sender<SkillsWatcherEvent>,
}

impl SkillsWatcher {
    pub(crate) fn new(file_watcher: &Arc<FileWatcher>) -> Self {
        let (subscriber, rx) = file_watcher.add_subscriber();
        let (tx, _) = broadcast::channel(128);
        let skills_watcher = Self {
            subscriber,
            tx: tx.clone(),
        };
        Self::spawn_event_loop(rx, tx);
        skills_watcher
    }

    pub(crate) fn noop() -> Self {
        Self::new(&Arc::new(FileWatcher::noop()))
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SkillsWatcherEvent> {
        self.tx.subscribe()
    }

    pub(crate) async fn register_config(
        &self,
        config: &Config,
        skills_manager: &SkillsManager,
        plugins_manager: &PluginsManager,
        fs: Option<Arc<dyn codex_exec_server::ExecutorFileSystem>>,
    ) -> WatchRegistration {
        let plugins_input = config.plugins_config_input();
        let plugin_outcome = plugins_manager.plugins_for_config(&plugins_input).await;
        let effective_skill_roots = plugin_outcome.effective_plugin_skill_roots();
        let skills_input = skills_load_input_from_config(config, effective_skill_roots);
        let roots = skills_manager
            .skill_roots_for_config(&skills_input, fs.clone())
            .await;
        let watch_paths =
            collect_watch_paths_for_roots(roots, skills_input.scan_max_depth, fs).await;
        self.subscriber.register_paths(watch_paths)
    }

    fn spawn_event_loop(rx: Receiver, tx: broadcast::Sender<SkillsWatcherEvent>) {
        let mut rx = ThrottledWatchReceiver::new(rx, WATCHER_THROTTLE_INTERVAL);
        if let Ok(handle) = Handle::try_current() {
            handle.spawn(async move {
                while let Some(event) = rx.recv().await {
                    let _ = tx.send(SkillsWatcherEvent::SkillsChanged { paths: event.paths });
                }
            });
        } else {
            warn!("skills watcher listener skipped: no Tokio runtime available");
        }
    }
}

async fn collect_watch_paths_for_roots(
    roots: Vec<SkillRoot>,
    scan_max_depth: usize,
    repo_fs: Option<Arc<dyn ExecutorFileSystem>>,
) -> Vec<WatchPath> {
    let mut watch_paths = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let fs: Arc<dyn ExecutorFileSystem> = match root.scope {
            SkillScope::Repo => repo_fs
                .as_ref()
                .map_or_else(|| Arc::clone(&root.file_system), Arc::clone),
            _ => Arc::clone(&root.file_system),
        };
        let mut root_paths =
            collect_watch_paths_for_root(fs.as_ref(), root.path, scan_max_depth).await;
        root_paths.retain(|path| seen.insert(path.clone()));
        watch_paths.extend(root_paths);
    }
    watch_paths
}

async fn collect_watch_paths_for_root(
    fs: &dyn ExecutorFileSystem,
    root: AbsolutePathBuf,
    scan_max_depth: usize,
) -> Vec<WatchPath> {
    let mut watch_paths = vec![WatchPath {
        path: root.to_path_buf(),
        recursive: false,
    }];

    let mut frontier = vec![(root, 0usize)];
    while let Some((dir, depth)) = frontier.pop() {
        if depth >= scan_max_depth {
            continue;
        }
        let Ok(entries) = fs.read_directory(&dir, /*sandbox*/ None).await else {
            continue;
        };
        for entry in entries {
            if entry.file_name.starts_with('.') {
                continue;
            }
            let path = dir.join(&entry.file_name);
            let Ok(metadata) = fs.get_metadata(&path, /*sandbox*/ None).await else {
                continue;
            };
            if metadata.is_directory || metadata.is_symlink {
                watch_paths.push(WatchPath {
                    path: path.to_path_buf(),
                    recursive: false,
                });
                frontier.push((path, depth + 1));
            }
        }
    }

    watch_paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_absolute_path::test_support::PathExt;
    use pretty_assertions::assert_eq;
    use tokio::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn forwards_file_watcher_events() {
        let file_watcher = Arc::new(FileWatcher::noop());
        let skills_watcher = SkillsWatcher::new(&file_watcher);
        let mut rx = skills_watcher.subscribe();
        let _registration = skills_watcher
            .subscriber
            .register_path(PathBuf::from("/tmp/skill"), /*recursive*/ true);

        file_watcher
            .send_paths_for_test(vec![PathBuf::from("/tmp/skill/SKILL.md")])
            .await;

        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("skills watcher event")
            .expect("broadcast recv");
        assert_eq!(
            event,
            SkillsWatcherEvent::SkillsChanged {
                paths: vec![PathBuf::from("/tmp/skill/SKILL.md")],
            }
        );
    }

    #[tokio::test]
    async fn collect_watch_paths_for_root_limits_depth() {
        let root = tempfile::tempdir().expect("tempdir");
        let level1 = root.path().join("level1");
        let level2 = level1.join("level2");
        std::fs::create_dir_all(&level2).expect("create nested dirs");

        let watch_paths = collect_watch_paths_for_root(
            codex_exec_server::LOCAL_FS.as_ref(),
            root.path().abs(),
            1,
        )
        .await;
        let actual = watch_paths
            .into_iter()
            .map(|path| path.path)
            .collect::<BTreeSet<_>>();

        assert_eq!(actual, BTreeSet::from([root.path().to_path_buf(), level1,]));
    }
}
