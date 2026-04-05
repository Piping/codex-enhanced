use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use codex_core::config::Config;
use codex_core::config::edit::ConfigEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisplayPreferenceKey {
    RawThinking,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DisplayPreferences {
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
            DisplayPreferenceKey::RawThinking => self.show_raw_thinking(),
        }
    }

    pub(crate) fn set_enabled(&self, key: DisplayPreferenceKey, enabled: bool) {
        match key {
            DisplayPreferenceKey::RawThinking => {
                self.show_raw_thinking.store(enabled, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn show_raw_thinking(&self) -> bool {
        self.show_raw_thinking.load(Ordering::Relaxed)
    }

    pub(crate) fn sync_from_config(&self, config: &Config) {
        self.show_raw_thinking
            .store(config.show_raw_agent_reasoning, Ordering::Relaxed);
    }
}

pub(crate) fn display_preference_edit(key: DisplayPreferenceKey, enabled: bool) -> ConfigEdit {
    match key {
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
        DisplayPreferenceKey::RawThinking => config.show_raw_agent_reasoning = enabled,
    }
}
