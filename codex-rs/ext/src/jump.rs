use serde::Deserialize;
use serde::Serialize;

const MAX_PREVIEW_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JumpTargetKind {
    UserMessage,
    AgentMessage,
    Reasoning,
    ToolCall,
    Event,
}

impl JumpTargetKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::UserMessage => "User Message",
            Self::AgentMessage => "Agent Message",
            Self::Reasoning => "Reasoning",
            Self::ToolCall => "Tool Call",
            Self::Event => "Event",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JumpTarget {
    pub cell_index: usize,
    pub ordinal: usize,
    pub kind: JumpTargetKind,
    pub title: String,
    pub preview: String,
}

impl JumpTarget {
    pub fn new(
        cell_index: usize,
        ordinal: usize,
        kind: JumpTargetKind,
        preview: impl Into<String>,
    ) -> Self {
        let preview = normalize_preview(preview.into());
        Self {
            cell_index,
            ordinal,
            kind,
            title: format!("{} {ordinal}", kind.display_name()),
            preview,
        }
    }

    pub fn search_value(&self) -> String {
        format!("{} {}", self.title, self.preview)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JumpCatalog {
    pub targets: Vec<JumpTarget>,
}

impl JumpCatalog {
    pub fn new(targets: Vec<JumpTarget>) -> Self {
        Self { targets }
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }
}

fn normalize_preview(preview: String) -> String {
    let collapsed = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut normalized = collapsed.trim().to_string();
    if normalized.len() > MAX_PREVIEW_CHARS {
        normalized.truncate(MAX_PREVIEW_CHARS);
        normalized.push_str("...");
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::JumpCatalog;
    use super::JumpTarget;
    use super::JumpTargetKind;
    use pretty_assertions::assert_eq;

    #[test]
    fn jump_target_normalizes_preview_whitespace() {
        let target = JumpTarget::new(
            4,
            2,
            JumpTargetKind::AgentMessage,
            "  first line\n   second   line  ",
        );

        assert_eq!(target.title, "Agent Message 2");
        assert_eq!(target.preview, "first line second line");
        assert_eq!(
            target.search_value(),
            "Agent Message 2 first line second line"
        );
    }

    #[test]
    fn jump_catalog_reports_size() {
        let catalog = JumpCatalog::new(vec![JumpTarget::new(
            0,
            1,
            JumpTargetKind::UserMessage,
            "hello",
        )]);

        assert_eq!(catalog.len(), 1);
        assert!(!catalog.is_empty());
    }
}
