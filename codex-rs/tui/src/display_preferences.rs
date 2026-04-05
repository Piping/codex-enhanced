use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use codex_core::config::Config;
use codex_core::config::edit::ConfigEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayPreferenceKey {
    StartupTooltips,
    RawThinking,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DisplayPreferences {
    show_startup_tooltips: Arc<AtomicBool>,
    show_raw_thinking: Arc<AtomicBool>,
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
        }
    }

    pub(crate) fn show_startup_tooltips(&self) -> bool {
        self.show_startup_tooltips.load(Ordering::Relaxed)
    }

    pub(crate) fn show_raw_thinking(&self) -> bool {
        self.show_raw_thinking.load(Ordering::Relaxed)
    }

    pub(crate) fn sync_from_config(&self, config: &Config) {
        self.show_startup_tooltips
            .store(config.show_tooltips, Ordering::Relaxed);
        self.show_raw_thinking
            .store(config.show_raw_agent_reasoning, Ordering::Relaxed);
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

        preferences.set_enabled(DisplayPreferenceKey::StartupTooltips, true);
        assert!(preferences.show_startup_tooltips());

        set_display_preference_in_config(&mut config, DisplayPreferenceKey::StartupTooltips, true);
        assert!(config.show_tooltips);
    }

    #[test]
    fn startup_tooltips_edit_targets_tui_show_tooltips() {
        match display_preference_edit(DisplayPreferenceKey::StartupTooltips, false) {
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
}
