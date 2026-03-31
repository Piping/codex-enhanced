use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::display_preferences::DisplayPreferenceKey;
use crate::display_preferences::DisplayPreferences;

pub(crate) fn control_panel_show_hide_item() -> SelectionItem {
    SelectionItem {
        name: "UI".to_string(),
        description: None,
        selected_description: Some(
            "Configure local TUI-only transcript visibility and pending UI settings.".to_string(),
        ),
        actions: vec![Box::new(|tx| {
            tx.send(AppEvent::OpenDisplayPreferencesPanel)
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}

pub(crate) fn display_preferences_items(
    display_preferences: &DisplayPreferences,
) -> Vec<SelectionItem> {
    std::iter::once(text_accent_color_wip_item())
        .chain(
            [
                DisplayPreferenceKey::RawThinking,
                DisplayPreferenceKey::StartupTooltips,
                DisplayPreferenceKey::ToolResults,
                DisplayPreferenceKey::ExecCommands,
                DisplayPreferenceKey::WaitedMessages,
                DisplayPreferenceKey::PatchDiffs,
            ]
            .into_iter()
            .map(|key| display_preference_item(display_preferences, key)),
        )
        .collect()
}

fn text_accent_color_wip_item() -> SelectionItem {
    SelectionItem {
        name: "Text Accent Color".to_string(),
        description: Some("WIP. Accent palette remapping is not available yet.".to_string()),
        selected_description: Some(
            "Planned local TUI text-accent palette control. Not wired yet.".to_string(),
        ),
        is_disabled: true,
        disabled_reason: Some("WIP".to_string()),
        dismiss_on_select: false,
        ..Default::default()
    }
}

fn display_preference_item(
    display_preferences: &DisplayPreferences,
    key: DisplayPreferenceKey,
) -> SelectionItem {
    let enabled = display_preferences.is_enabled(key);
    let (name, description) = match (key, enabled) {
        (DisplayPreferenceKey::RawThinking, true) => (
            "Hide Raw Thinking",
            "Currently visible. Hide raw reasoning text while keeping summaries.",
        ),
        (DisplayPreferenceKey::RawThinking, false) => (
            "Show Raw Thinking",
            "Currently hidden. Reveal raw reasoning text in this TUI only.",
        ),
        (DisplayPreferenceKey::StartupTooltips, true) => (
            "Hide Startup Tooltips",
            "Currently visible. Hide welcome and session tooltip hints.",
        ),
        (DisplayPreferenceKey::StartupTooltips, false) => (
            "Show Startup Tooltips",
            "Currently hidden. Reveal welcome and session tooltip hints.",
        ),
        (DisplayPreferenceKey::ToolResults, true) => (
            "Hide Tool Results",
            "Currently visible. Keep tool invocations but collapse result details.",
        ),
        (DisplayPreferenceKey::ToolResults, false) => (
            "Show Tool Results",
            "Currently hidden. Reveal tool result details in transcript cells.",
        ),
        (DisplayPreferenceKey::ExecCommands, true) => (
            "Hide Command Execution",
            "Currently visible. Hide command execution cells and keep model replies.",
        ),
        (DisplayPreferenceKey::ExecCommands, false) => (
            "Show Command Execution",
            "Currently hidden. Reveal command execution cells in this TUI only.",
        ),
        (DisplayPreferenceKey::WaitedMessages, true) => (
            "Hide Waited Messages",
            "Currently visible. Hide 'Waited for ...' background terminal messages.",
        ),
        (DisplayPreferenceKey::WaitedMessages, false) => (
            "Show Waited Messages",
            "Currently hidden. Reveal 'Waited for ...' background terminal messages.",
        ),
        (DisplayPreferenceKey::PatchDiffs, true) => (
            "Hide Patch / Edit Diff",
            "Currently visible. Collapse patch and edit diff summaries.",
        ),
        (DisplayPreferenceKey::PatchDiffs, false) => (
            "Show Patch / Edit Diff",
            "Currently hidden. Reveal patch and edit diff summaries.",
        ),
    };

    SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::ToggleDisplayPreference(key));
        })],
        dismiss_on_select: false,
        ..Default::default()
    }
}
