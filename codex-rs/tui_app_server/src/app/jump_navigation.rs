use crate::history_cell::AgentMessageCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::McpToolCallCell;
use crate::history_cell::ReasoningSummaryCell;
use crate::history_cell::UserHistoryCell;
use codex_ext::JumpCatalog;
use codex_ext::JumpTarget;
use codex_ext::JumpTargetKind;
use ratatui::text::Line;
use std::sync::Arc;

pub(crate) fn build_jump_catalog(cells: &[Arc<dyn HistoryCell>]) -> JumpCatalog {
    let mut targets = Vec::new();
    let mut ordinal = 1usize;

    for (cell_index, cell) in cells.iter().enumerate() {
        let preview = preview_from_lines(cell.transcript_lines(u16::MAX));
        if preview.is_empty() {
            continue;
        }

        targets.push(JumpTarget::new(
            cell_index,
            ordinal,
            classify_history_cell(cell.as_ref()),
            preview,
        ));
        ordinal += 1;
    }

    JumpCatalog::new(targets)
}

fn classify_history_cell(cell: &dyn HistoryCell) -> JumpTargetKind {
    if cell.as_any().is::<UserHistoryCell>() {
        JumpTargetKind::UserMessage
    } else if cell.as_any().is::<AgentMessageCell>() {
        JumpTargetKind::AgentMessage
    } else if cell.as_any().is::<ReasoningSummaryCell>() {
        JumpTargetKind::Reasoning
    } else if cell.as_any().is::<McpToolCallCell>() {
        JumpTargetKind::ToolCall
    } else {
        JumpTargetKind::Event
    }
}

fn preview_from_lines(lines: Vec<Line<'static>>) -> String {
    lines
        .into_iter()
        .map(line_to_plain_text)
        .map(|line| {
            line.trim()
                .trim_start_matches(['•', '-', '>', '›'])
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

fn line_to_plain_text(line: Line<'static>) -> String {
    line.spans
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::build_jump_catalog;
    use crate::history_cell::AgentMessageCell;
    use crate::history_cell::PlainHistoryCell;
    use crate::history_cell::UserHistoryCell;
    use codex_ext::JumpTargetKind;
    use pretty_assertions::assert_eq;
    use ratatui::text::Line;
    use std::sync::Arc;

    #[test]
    fn build_jump_catalog_classifies_cells_and_skips_empty_entries() {
        let cells = vec![
            Arc::new(UserHistoryCell {
                message: "first question".to_string(),
                text_elements: Vec::new(),
                local_image_paths: Vec::new(),
                remote_image_urls: Vec::new(),
            }) as Arc<dyn crate::history_cell::HistoryCell>,
            Arc::new(PlainHistoryCell::new(vec![Line::from("   ")])),
            Arc::new(AgentMessageCell::new(
                vec![Line::from("first answer")],
                true,
            )),
        ];

        let catalog = build_jump_catalog(&cells);

        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.targets[0].kind, JumpTargetKind::UserMessage);
        assert_eq!(catalog.targets[0].preview, "first question");
        assert_eq!(catalog.targets[1].kind, JumpTargetKind::AgentMessage);
        assert_eq!(catalog.targets[1].preview, "first answer");
    }
}
