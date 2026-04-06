use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use codex_core::config::Config;
use codex_core::config::edit::ConfigEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayPreferenceKey {
    StartupTooltips,
    RawThinking,
    ToolResults,
    PatchDiffs,
}

#[derive(Clone, Debug)]
pub(crate) struct DisplayPreferences {
    show_startup_tooltips: Arc<AtomicBool>,
    show_raw_thinking: Arc<AtomicBool>,
    show_tool_results: Arc<AtomicBool>,
    show_patch_diffs: Arc<AtomicBool>,
}

impl Default for DisplayPreferences {
    fn default() -> Self {
        Self {
            show_startup_tooltips: Arc::new(AtomicBool::new(true)),
            show_raw_thinking: Arc::new(AtomicBool::new(false)),
            show_tool_results: Arc::new(AtomicBool::new(true)),
            show_patch_diffs: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl DisplayPreferences {
    pub(crate) fn from_config(config: &Config) -> Self {
        let preferences = Self::default();
        preferences.sync_from_config(config);
        preferences
    }

    pub(crate) fn is_enabled(&self, key: DisplayPreferenceKey) -> bool {
        match key {
            DisplayPreferenceKey::StartupTooltips => self.show_startup_tooltips(),
            DisplayPreferenceKey::RawThinking => self.show_raw_thinking(),
            DisplayPreferenceKey::ToolResults => self.show_tool_results(),
            DisplayPreferenceKey::PatchDiffs => self.show_patch_diffs(),
        }
    }

    pub(crate) fn set_enabled(&self, key: DisplayPreferenceKey, enabled: bool) {
        match key {
            DisplayPreferenceKey::StartupTooltips => {
                self.show_startup_tooltips.store(enabled, Ordering::Relaxed);
            }
            DisplayPreferenceKey::RawThinking => {
                self.show_raw_thinking.store(enabled, Ordering::Relaxed);
            }
            DisplayPreferenceKey::ToolResults => {
                self.show_tool_results.store(enabled, Ordering::Relaxed);
            }
            DisplayPreferenceKey::PatchDiffs => {
                self.show_patch_diffs.store(enabled, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn show_startup_tooltips(&self) -> bool {
        self.show_startup_tooltips.load(Ordering::Relaxed)
    }

    pub(crate) fn show_raw_thinking(&self) -> bool {
        self.show_raw_thinking.load(Ordering::Relaxed)
    }

    pub(crate) fn show_tool_results(&self) -> bool {
        self.show_tool_results.load(Ordering::Relaxed)
    }

    pub(crate) fn show_patch_diffs(&self) -> bool {
        self.show_patch_diffs.load(Ordering::Relaxed)
    }

    pub(crate) fn sync_from_config(&self, config: &Config) {
        self.show_startup_tooltips
            .store(config.show_tooltips, Ordering::Relaxed);
        self.show_raw_thinking
            .store(config.show_raw_agent_reasoning, Ordering::Relaxed);
        self.show_tool_results.store(
            config.tui_display_preferences.show_tool_results,
            Ordering::Relaxed,
        );
        self.show_patch_diffs.store(
            config.tui_display_preferences.show_patch_diffs,
            Ordering::Relaxed,
        );
    }
}

pub(crate) fn display_preference_edit(key: DisplayPreferenceKey, enabled: bool) -> ConfigEdit {
    match key {
        DisplayPreferenceKey::StartupTooltips => ConfigEdit::SetPath {
            segments: vec!["tui".to_string(), "show_tooltips".to_string()],
            value: enabled.into(),
        },
        DisplayPreferenceKey::RawThinking => ConfigEdit::SetPath {
            segments: vec!["show_raw_agent_reasoning".to_string()],
            value: enabled.into(),
        },
        DisplayPreferenceKey::ToolResults => ConfigEdit::SetPath {
            segments: vec![
                "tui".to_string(),
                "display_preferences".to_string(),
                "show_tool_results".to_string(),
            ],
            value: enabled.into(),
        },
        DisplayPreferenceKey::PatchDiffs => ConfigEdit::SetPath {
            segments: vec![
                "tui".to_string(),
                "display_preferences".to_string(),
                "show_patch_diffs".to_string(),
            ],
            value: enabled.into(),
        },
    }
}

pub(crate) fn set_display_preference_in_config(
    config: &mut Config,
    key: DisplayPreferenceKey,
    enabled: bool,
) {
    match key {
        DisplayPreferenceKey::StartupTooltips => config.show_tooltips = enabled,
        DisplayPreferenceKey::RawThinking => config.show_raw_agent_reasoning = enabled,
        DisplayPreferenceKey::ToolResults => {
            config.tui_display_preferences.show_tool_results = enabled;
        }
        DisplayPreferenceKey::PatchDiffs => {
            config.tui_display_preferences.show_patch_diffs = enabled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayPreferenceKey;
    use super::DisplayPreferences;
    use super::display_preference_edit;
    use super::set_display_preference_in_config;
    use codex_core::config::ConfigBuilder;
    use codex_core::config::edit::ConfigEdit;

    #[tokio::test]
    async fn startup_tooltips_follow_config_and_setters() {
        let mut config = ConfigBuilder::default().build().await.expect("config");
        config.show_tooltips = false;

        let preferences = DisplayPreferences::from_config(&config);
        assert!(!preferences.show_startup_tooltips());

        preferences.set_enabled(DisplayPreferenceKey::StartupTooltips, /*enabled*/ true);
        assert!(preferences.show_startup_tooltips());

        set_display_preference_in_config(
            &mut config,
            DisplayPreferenceKey::StartupTooltips,
            /*enabled*/ true,
        );
        assert!(config.show_tooltips);
    }

    #[tokio::test]
    async fn transcript_visibility_preferences_follow_config_and_setters() {
        let mut config = ConfigBuilder::default().build().await.expect("config");
        config.tui_display_preferences.show_tool_results = false;
        config.tui_display_preferences.show_patch_diffs = false;

        let preferences = DisplayPreferences::from_config(&config);
        assert!(!preferences.show_tool_results());
        assert!(!preferences.show_patch_diffs());

        preferences.set_enabled(DisplayPreferenceKey::ToolResults, /*enabled*/ true);
        preferences.set_enabled(DisplayPreferenceKey::PatchDiffs, /*enabled*/ true);
        assert!(preferences.show_tool_results());
        assert!(preferences.show_patch_diffs());

        set_display_preference_in_config(
            &mut config,
            DisplayPreferenceKey::ToolResults,
            /*enabled*/ true,
        );
        set_display_preference_in_config(
            &mut config,
            DisplayPreferenceKey::PatchDiffs,
            /*enabled*/ true,
        );
        assert!(config.tui_display_preferences.show_tool_results);
        assert!(config.tui_display_preferences.show_patch_diffs);
    }

    #[test]
    fn startup_tooltips_edit_targets_tui_show_tooltips() {
        match display_preference_edit(
            DisplayPreferenceKey::StartupTooltips,
            /*enabled*/ false,
        ) {
            ConfigEdit::SetPath { segments, value } => {
                assert_eq!(
                    segments,
                    vec!["tui".to_string(), "show_tooltips".to_string()]
                );
                assert_eq!(value.to_string(), "false");
            }
            other => panic!("unexpected config edit: {other:?}"),
        }
    }

    #[test]
    fn transcript_visibility_edits_target_tui_display_preferences() {
        match display_preference_edit(DisplayPreferenceKey::ToolResults, /*enabled*/ false) {
            ConfigEdit::SetPath { segments, value } => {
                assert_eq!(
                    segments,
                    vec![
                        "tui".to_string(),
                        "display_preferences".to_string(),
                        "show_tool_results".to_string(),
                    ]
                );
                assert_eq!(value.to_string(), "false");
            }
            other => panic!("unexpected config edit: {other:?}"),
        }

        match display_preference_edit(DisplayPreferenceKey::PatchDiffs, /*enabled*/ false) {
            ConfigEdit::SetPath { segments, value } => {
                assert_eq!(
                    segments,
                    vec![
                        "tui".to_string(),
                        "display_preferences".to_string(),
                        "show_patch_diffs".to_string(),
                    ]
                );
                assert_eq!(value.to_string(), "false");
            }
            other => panic!("unexpected config edit: {other:?}"),
        }
    }
}
