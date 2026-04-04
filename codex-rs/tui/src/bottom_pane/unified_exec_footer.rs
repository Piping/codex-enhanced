//! Renders and formats unified-exec background session summary text.
//!
//! This module provides one canonical summary string so the bottom pane can
//! either render a dedicated footer row or reuse the same text inline in the
//! status row without duplicating copy/grammar logic.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::live_wrap::take_prefix_by_width;
use crate::render::renderable::Renderable;

/// Tracks active unified-exec processes and workflows and renders a compact summary.
pub(crate) struct UnifiedExecFooter {
    terminals: Vec<String>,
    workflows: Vec<String>,
}

impl UnifiedExecFooter {
    pub(crate) fn new() -> Self {
        Self {
            terminals: Vec::new(),
            workflows: Vec::new(),
        }
    }

    pub(crate) fn set_activity(&mut self, terminals: Vec<String>, workflows: Vec<String>) -> bool {
        if self.terminals == terminals && self.workflows == workflows {
            return false;
        }
        self.terminals = terminals;
        self.workflows = workflows;
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.terminals.is_empty() && self.workflows.is_empty()
    }

    /// Returns the unindented summary text used by both footer and status-row rendering.
    ///
    /// The returned string intentionally omits leading spaces and separators so
    /// callers can choose layout-specific framing (inline separator vs. row
    /// indentation). Returning `None` means there is nothing to surface.
    pub(crate) fn summary_text(&self) -> Option<String> {
        let terminal_count = self.terminals.len();
        let workflow_count = self.workflows.len();
        if terminal_count == 0 && workflow_count == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if terminal_count > 0 {
            let plural = if terminal_count == 1 { "" } else { "s" };
            parts.push(format!(
                "{terminal_count} background terminal{plural} running"
            ));
        }
        if workflow_count > 0 {
            let plural = if workflow_count == 1 { "" } else { "s" };
            parts.push(format!(
                "{workflow_count} background workflow{plural} running"
            ));
        }
        Some(format!(
            "{} · /ps to view · /stop to close",
            parts.join(" · ")
        ))
    }

    fn render_lines(&self, width: u16) -> Vec<Line<'static>> {
        if width < 4 {
            return Vec::new();
        }
        let Some(summary) = self.summary_text() else {
            return Vec::new();
        };
        let message = format!("  {summary}");
        let (truncated, _, _) = take_prefix_by_width(&message, width as usize);
        vec![Line::from(truncated.dim())]
    }
}

impl Renderable for UnifiedExecFooter {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        Paragraph::new(self.render_lines(area.width)).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.render_lines(width).len() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;

    #[test]
    fn desired_height_empty() {
        let footer = UnifiedExecFooter::new();
        assert_eq!(footer.desired_height(/*width*/ 40), 0);
    }

    #[test]
    fn render_more_sessions() {
        let mut footer = UnifiedExecFooter::new();
        footer.set_activity(vec!["rg \"foo\" src".to_string()], Vec::new());
        let width = 50;
        let height = footer.desired_height(width);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        footer.render(Rect::new(0, 0, width, height), &mut buf);
        assert_snapshot!("render_more_sessions", format!("{buf:?}"));
    }

    #[test]
    fn render_many_sessions() {
        let mut footer = UnifiedExecFooter::new();
        footer.set_activity(
            (0..123).map(|idx| format!("cmd {idx}")).collect(),
            Vec::new(),
        );
        let width = 50;
        let height = footer.desired_height(width);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        footer.render(Rect::new(0, 0, width, height), &mut buf);
        assert_snapshot!("render_many_sessions", format!("{buf:?}"));
    }

    #[test]
    fn summary_text_includes_workflows() {
        let mut footer = UnifiedExecFooter::new();
        footer.set_activity(
            vec!["rg \"foo\" src".to_string()],
            vec!["director · after_turn".to_string()],
        );

        assert_eq!(
            footer.summary_text(),
            Some(
                "1 background terminal running · 1 background workflow running · /ps to view · /stop to close"
                    .to_string()
            )
        );
    }
}
