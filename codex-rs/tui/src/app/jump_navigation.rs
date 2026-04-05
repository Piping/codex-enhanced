use std::sync::Arc;

use ratatui::text::Line;

use crate::history_cell::AgentMessageCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::McpToolCallCell;
use crate::history_cell::ReasoningSummaryCell;
use crate::history_cell::UserHistoryCell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JumpTargetKind {
    UserMessage,
    AgentMessage,
    Reasoning,
    ToolCall,
    Event,
}

impl JumpTargetKind {
    fn title(self) -> &'static str {
        match self {
            Self::UserMessage => "User Message",
            Self::AgentMessage => "Assistant Message",
            Self::Reasoning => "Reasoning",
            Self::ToolCall => "Tool Call",
            Self::Event => "Event",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JumpTarget {
    pub(crate) cell_index: usize,
    pub(crate) ordinal: usize,
    pub(crate) kind: JumpTargetKind,
    pub(crate) title: String,
    pub(crate) preview: String,
}

impl JumpTarget {
    fn new(cell_index: usize, ordinal: usize, kind: JumpTargetKind, preview: String) -> Self {
        Self {
            cell_index,
            ordinal,
            kind,
            title: format!("{} {ordinal}", kind.title()),
            preview,
        }
    }

    pub(crate) fn search_value(&self) -> String {
        format!("{} {}", self.title, self.preview)
    }
}

pub(crate) fn build_jump_targets(cells: &[Arc<dyn HistoryCell>]) -> Vec<JumpTarget> {
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

    targets
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
    use pretty_assertions::assert_eq;
    use ratatui::text::Line;
    use std::sync::Arc;

    use super::JumpTargetKind;
    use super::build_jump_targets;
    use crate::history_cell::AgentMessageCell;
    use crate::history_cell::PlainHistoryCell;
    use crate::history_cell::UserHistoryCell;

    #[test]
    fn build_jump_targets_classifies_cells_and_skips_empty_entries() {
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
                /*is_first_line*/ true,
            )),
        ];

        let targets = build_jump_targets(&cells);

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].kind, JumpTargetKind::UserMessage);
        assert_eq!(targets[0].title, "User Message 1");
        assert_eq!(targets[0].preview, "first question");
        assert_eq!(targets[1].kind, JumpTargetKind::AgentMessage);
        assert_eq!(targets[1].title, "Assistant Message 2");
        assert_eq!(targets[1].preview, "first answer");
    }
}
