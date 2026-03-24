use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use codex_core::config::Config;
use codex_core::config::edit::ConfigEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayPreferenceKey {
    RawThinking,
    StartupTooltips,
    ToolResults,
    ExecCommands,
    WaitedMessages,
    PatchDiffs,
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayPreferences {
    show_raw_thinking: Arc<AtomicBool>,
    show_startup_tooltips: Arc<AtomicBool>,
    show_tool_results: Arc<AtomicBool>,
    show_exec_commands: Arc<AtomicBool>,
    show_waited_messages: Arc<AtomicBool>,
    show_patch_diffs: Arc<AtomicBool>,
}

impl DisplayPreferences {
    pub(crate) fn from_config(config: &Config) -> Self {
        let preferences = Self::default();
        preferences.sync_from_config(config);
        preferences
    }

    pub(crate) fn is_enabled(&self, key: DisplayPreferenceKey) -> bool {
        match key {
            DisplayPreferenceKey::RawThinking => self.show_raw_thinking(),
            DisplayPreferenceKey::StartupTooltips => self.show_startup_tooltips(),
            DisplayPreferenceKey::ToolResults => self.show_tool_results(),
            DisplayPreferenceKey::ExecCommands => self.show_exec_commands(),
            DisplayPreferenceKey::WaitedMessages => self.show_waited_messages(),
            DisplayPreferenceKey::PatchDiffs => self.show_patch_diffs(),
        }
    }

    pub(crate) fn set_enabled(&self, key: DisplayPreferenceKey, enabled: bool) {
        let atomic = match key {
            DisplayPreferenceKey::RawThinking => &self.show_raw_thinking,
            DisplayPreferenceKey::StartupTooltips => &self.show_startup_tooltips,
            DisplayPreferenceKey::ToolResults => &self.show_tool_results,
            DisplayPreferenceKey::ExecCommands => &self.show_exec_commands,
            DisplayPreferenceKey::WaitedMessages => &self.show_waited_messages,
            DisplayPreferenceKey::PatchDiffs => &self.show_patch_diffs,
        };
        atomic.store(enabled, Ordering::Relaxed);
    }

    pub(crate) fn show_raw_thinking(&self) -> bool {
        self.show_raw_thinking.load(Ordering::Relaxed)
    }

    pub(crate) fn show_startup_tooltips(&self) -> bool {
        self.show_startup_tooltips.load(Ordering::Relaxed)
    }

    pub(crate) fn show_tool_results(&self) -> bool {
        self.show_tool_results.load(Ordering::Relaxed)
    }

    pub(crate) fn show_exec_commands(&self) -> bool {
        self.show_exec_commands.load(Ordering::Relaxed)
    }

    pub(crate) fn show_waited_messages(&self) -> bool {
        self.show_waited_messages.load(Ordering::Relaxed)
    }

    pub(crate) fn show_patch_diffs(&self) -> bool {
        self.show_patch_diffs.load(Ordering::Relaxed)
    }

    pub(crate) fn sync_from_config(&self, config: &Config) {
        self.show_raw_thinking
            .store(config.show_raw_agent_reasoning, Ordering::Relaxed);
        self.show_startup_tooltips
            .store(config.show_tooltips, Ordering::Relaxed);
        self.show_tool_results.store(
            config.tui_display_preferences.show_tool_results,
            Ordering::Relaxed,
        );
        self.show_exec_commands.store(
            config.tui_display_preferences.show_exec_commands,
            Ordering::Relaxed,
        );
        self.show_waited_messages.store(
            config.tui_display_preferences.show_waited_messages,
            Ordering::Relaxed,
        );
        self.show_patch_diffs.store(
            config.tui_display_preferences.show_patch_diffs,
            Ordering::Relaxed,
        );
    }
}

impl Default for DisplayPreferences {
    fn default() -> Self {
        Self {
            show_raw_thinking: Arc::new(AtomicBool::new(false)),
            show_startup_tooltips: Arc::new(AtomicBool::new(true)),
            show_tool_results: Arc::new(AtomicBool::new(true)),
            show_exec_commands: Arc::new(AtomicBool::new(true)),
            show_waited_messages: Arc::new(AtomicBool::new(true)),
            show_patch_diffs: Arc::new(AtomicBool::new(true)),
        }
    }
}

pub(crate) fn display_preference_edit(key: DisplayPreferenceKey, enabled: bool) -> ConfigEdit {
    let segments = match key {
        DisplayPreferenceKey::RawThinking => {
            return ConfigEdit::SetPath {
                segments: vec!["show_raw_agent_reasoning".to_string()],
                value: enabled.into(),
            };
        }
        DisplayPreferenceKey::StartupTooltips => {
            vec!["tui".to_string(), "show_tooltips".to_string()]
        }
        DisplayPreferenceKey::ToolResults => vec![
            "tui".to_string(),
            "display_preferences".to_string(),
            "show_tool_results".to_string(),
        ],
        DisplayPreferenceKey::ExecCommands => vec![
            "tui".to_string(),
            "display_preferences".to_string(),
            "show_exec_commands".to_string(),
        ],
        DisplayPreferenceKey::WaitedMessages => vec![
            "tui".to_string(),
            "display_preferences".to_string(),
            "show_waited_messages".to_string(),
        ],
        DisplayPreferenceKey::PatchDiffs => vec![
            "tui".to_string(),
            "display_preferences".to_string(),
            "show_patch_diffs".to_string(),
        ],
    };

    ConfigEdit::SetPath {
        segments,
        value: enabled.into(),
    }
}

pub(crate) fn set_display_preference_in_config(
    config: &mut Config,
    key: DisplayPreferenceKey,
    enabled: bool,
) {
    match key {
        DisplayPreferenceKey::RawThinking => config.show_raw_agent_reasoning = enabled,
        DisplayPreferenceKey::StartupTooltips => config.show_tooltips = enabled,
        DisplayPreferenceKey::ToolResults => {
            config.tui_display_preferences.show_tool_results = enabled;
        }
        DisplayPreferenceKey::ExecCommands => {
            config.tui_display_preferences.show_exec_commands = enabled;
        }
        DisplayPreferenceKey::WaitedMessages => {
            config.tui_display_preferences.show_waited_messages = enabled;
        }
        DisplayPreferenceKey::PatchDiffs => {
            config.tui_display_preferences.show_patch_diffs = enabled;
        }
    }
}
