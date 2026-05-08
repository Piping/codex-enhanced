use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::display_preferences::DisplayPreferenceKey;
use crate::display_preferences::DisplayPreferences;
use ratatui::style::Stylize;

pub(crate) const DISPLAY_PREFERENCES_SELECTION_VIEW_ID: &str = "display-preferences-panel";

pub(crate) fn display_preferences_items(
    display_preferences: &DisplayPreferences,
) -> Vec<SelectionItem> {
    [
        DisplayPreferenceKey::StartupTooltips,
        DisplayPreferenceKey::RawThinking,
        DisplayPreferenceKey::ToolResults,
        DisplayPreferenceKey::HookOutput,
        DisplayPreferenceKey::PatchDiffs,
    ]
    .into_iter()
    .map(|key| display_preference_item(display_preferences, key))
    .collect()
}

pub(crate) fn display_preferences_panel_params(
    display_preferences: &DisplayPreferences,
    initial_selected_idx: Option<usize>,
) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(DISPLAY_PREFERENCES_SELECTION_VIEW_ID),
        title: Some("UI".to_string()),
        subtitle: Some("These settings only affect local TUI rendering.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        footer_note: Some(
            "Model context and persisted rollout history are unchanged."
                .dim()
                .into(),
        ),
        items: display_preferences_items(display_preferences),
        initial_selected_idx,
        ..Default::default()
    }
}

fn display_preference_item(
    display_preferences: &DisplayPreferences,
    key: DisplayPreferenceKey,
) -> SelectionItem {
    let enabled = display_preferences.is_enabled(key);
    let (name, description) = match (key, enabled) {
        (DisplayPreferenceKey::StartupTooltips, true) => (
            "Hide Startup Tooltips",
            "Currently visible. Hide first-run and local startup tooltip hints in this TUI.",
        ),
        (DisplayPreferenceKey::StartupTooltips, false) => (
            "Show Startup Tooltips",
            "Currently hidden. Restore first-run and local startup tooltip hints in this TUI.",
        ),
        (DisplayPreferenceKey::RawThinking, true) => (
            "Hide Raw Thinking",
            "Currently visible. Hide raw reasoning text while keeping summaries.",
        ),
        (DisplayPreferenceKey::RawThinking, false) => (
            "Show Raw Thinking",
            "Currently hidden. Reveal raw reasoning text in this TUI only.",
        ),
        (DisplayPreferenceKey::ToolResults, true) => (
            "Hide Tool Activity",
            "Currently visible. Hide tool calls and result details in transcript cells.",
        ),
        (DisplayPreferenceKey::ToolResults, false) => (
            "Show Tool Activity",
            "Currently hidden. Reveal tool calls and result details in transcript cells.",
        ),
        (DisplayPreferenceKey::HookOutput, true) => (
            "Hide Hook Activity",
            "Currently visible. Hide hook lifecycle messages and hook output details in the transcript.",
        ),
        (DisplayPreferenceKey::HookOutput, false) => (
            "Show Hook Activity",
            "Currently hidden. Reveal hook lifecycle messages and hook output details in the transcript.",
        ),
        (DisplayPreferenceKey::PatchDiffs, true) => (
            "Hide Patch Diffs",
            "Currently visible. Collapse patch/edit diff summaries in transcript cells.",
        ),
        (DisplayPreferenceKey::PatchDiffs, false) => (
            "Show Patch Diffs",
            "Currently hidden. Reveal patch/edit diff summaries in transcript cells.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    use crate::app_event_sender::AppEventSender;
    use crate::bottom_pane::ListSelectionView;
    use crate::render::renderable::Renderable;

    fn render_lines(view: &ListSelectionView) -> String {
        let width = 48;
        let height = view.desired_height(width);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        (0..area.height)
            .map(|row| {
                let mut line = String::new();
                for col in 0..area.width {
                    let symbol = buf[(area.x + col, area.y + row)].symbol();
                    if symbol.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(symbol);
                    }
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn display_preferences_menu_snapshot() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = ListSelectionView::new(
            display_preferences_panel_params(
                &DisplayPreferences::default(),
                /*initial_selected_idx*/ None,
            ),
            tx,
            crate::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!("display_preferences_menu", render_lines(&view));
    }
}
