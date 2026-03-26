use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;

pub(crate) fn control_panel_thread_item() -> SelectionItem {
    SelectionItem {
        name: "Thread".to_string(),
        description: None,
        selected_description: Some(
            "Open thread-specific actions for the current conversation.".to_string(),
        ),
        actions: vec![Box::new(|tx| tx.send(AppEvent::OpenThreadPanel))],
        dismiss_on_select: false,
        ..Default::default()
    }
}

pub(crate) fn thread_panel_items() -> Vec<SelectionItem> {
    vec![
        SelectionItem {
            name: "Fork Current Session".to_string(),
            description: None,
            selected_description: Some("Fork the current thread into a new session.".to_string()),
            actions: vec![Box::new(|tx| tx.send(AppEvent::ForkCurrentSession))],
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "Jump To Message".to_string(),
            description: None,
            selected_description: Some(
                "Search committed transcript entries and open the transcript overlay.".to_string(),
            ),
            actions: vec![Box::new(|tx| tx.send(AppEvent::OpenJumpToMessagePanel))],
            dismiss_on_select: false,
            ..Default::default()
        },
        SelectionItem {
            name: "Undo Last User Message".to_string(),
            description: None,
            selected_description: Some(
                "Restore the last sent input and roll back one turn.".to_string(),
            ),
            actions: vec![Box::new(|tx| tx.send(AppEvent::UndoLastUserMessage))],
            dismiss_on_select: true,
            ..Default::default()
        },
    ]
}
