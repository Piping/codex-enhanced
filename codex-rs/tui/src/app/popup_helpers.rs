use super::App;
use crate::bottom_pane::SelectionViewParams;

impl App {
    pub(super) fn open_selection_popup_for_view<F>(
        &mut self,
        view_id: &'static str,
        build: F,
    ) -> Option<usize>
    where
        F: FnOnce(&Self, Option<usize>) -> SelectionViewParams,
    {
        let active_selected_idx = self.chat_widget.selected_index_for_active_view(view_id);
        let params = build(self, active_selected_idx);
        if active_selected_idx.is_some() {
            let _ = self
                .chat_widget
                .replace_selection_view_if_active(view_id, params);
        } else {
            self.chat_widget.show_selection_view(params);
        }
        active_selected_idx
    }

    pub(super) fn refresh_selection_popup_if_active<F>(
        &mut self,
        view_id: &'static str,
        build: F,
    ) -> bool
    where
        F: FnOnce(&Self, usize) -> SelectionViewParams,
    {
        let Some(initial_selected_idx) = self.chat_widget.selected_index_for_active_view(view_id)
        else {
            return false;
        };
        let params = build(self, initial_selected_idx);
        self.chat_widget
            .replace_selection_view_if_active(view_id, params)
    }
}
