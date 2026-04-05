use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_utils_string::take_bytes_at_char_boundary;
use serde::Deserialize;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs;

pub struct ReadFileHandler;

const DEFAULT_LIMIT: usize = 200;
const DEFAULT_OFFSET: usize = 1;
const MAX_LINE_LENGTH: usize = 2_000;

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

fn default_offset() -> usize {
    DEFAULT_OFFSET
}

fn default_max_levels() -> usize {
    1
}

fn default_include_header() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReadFileMode {
    #[default]
    Slice,
    Indentation,
}

#[derive(Clone, Debug, Deserialize)]
struct IndentationArgs {
    #[serde(default)]
    anchor_line: Option<usize>,
    #[serde(default)]
    include_siblings: bool,
    #[serde(default = "default_max_levels")]
    max_levels: usize,
    #[serde(default = "default_include_header")]
    include_header: bool,
    #[serde(default)]
    max_lines: Option<usize>,
}

impl Default for IndentationArgs {
    fn default() -> Self {
        Self {
            anchor_line: None,
            include_siblings: false,
            max_levels: default_max_levels(),
            include_header: default_include_header(),
            max_lines: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReadFileArgs {
    file_path: String,
    #[serde(default = "default_offset")]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    mode: ReadFileMode,
    #[serde(default)]
    indentation: Option<IndentationArgs>,
}

impl ToolHandler for ReadFileHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { payload, turn, .. } = invocation;
        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "read_file handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: ReadFileArgs = parse_arguments(&arguments)?;
        validate_offset_limit(args.offset, args.limit)?;

        let path = crate::util::resolve_path(turn.cwd.as_path(), &PathBuf::from(args.file_path));
        let lines = match args.mode {
            ReadFileMode::Slice => slice::read(path.as_path(), args.offset, args.limit).await?,
            ReadFileMode::Indentation => {
                indentation::read_block(
                    path.as_path(),
                    args.offset,
                    args.limit,
                    args.indentation.unwrap_or_default(),
                )
                .await?
            }
        };

        Ok(FunctionToolOutput::from_text(lines.join("\n"), Some(true)))
    }
}

#[derive(Clone, Debug)]
struct LineRecord {
    text: String,
    indent: usize,
    is_blank: bool,
}

impl LineRecord {
    fn new(text: String) -> Self {
        let is_blank = text.trim().is_empty();
        let indent = text
            .chars()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .map(|ch| if ch == '\t' { 4 } else { 1 })
            .sum();
        Self {
            text,
            indent,
            is_blank,
        }
    }

    fn trimmed(&self) -> &str {
        self.text.trim()
    }
}

#[derive(Clone, Copy, Debug)]
struct BlockRange {
    start: usize,
    actual_start: usize,
    end: usize,
}

async fn load_lines(path: &Path) -> Result<Vec<LineRecord>, FunctionCallError> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("failed to read file: {err}")))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text
        .split('\n')
        .map(|line| LineRecord::new(line.trim_end_matches('\r').to_string()))
        .collect::<Vec<_>>();
    if matches!(lines.last(), Some(last) if last.text.is_empty()) {
        lines.pop();
    }
    Ok(lines)
}

fn validate_offset_limit(offset: usize, limit: usize) -> Result<(), FunctionCallError> {
    if offset == 0 {
        return Err(FunctionCallError::RespondToModel(
            "offset must be a 1-indexed line number".to_string(),
        ));
    }
    if limit == 0 {
        return Err(FunctionCallError::RespondToModel(
            "limit must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn format_output(
    lines: &[LineRecord],
    start: usize,
    end_exclusive: usize,
    max_lines: usize,
) -> Vec<String> {
    lines[start..end_exclusive]
        .iter()
        .take(max_lines)
        .enumerate()
        .map(|(offset, line)| format_line(start + offset + 1, line.text.as_str()))
        .collect()
}

fn format_line(line_number: usize, line: &str) -> String {
    let truncated = if line.len() > MAX_LINE_LENGTH {
        take_bytes_at_char_boundary(line, MAX_LINE_LENGTH).to_string()
    } else {
        line.to_string()
    };
    format!("L{line_number}: {truncated}")
}

fn resolve_anchor_index(
    lines: &[LineRecord],
    anchor_line: usize,
) -> Result<usize, FunctionCallError> {
    if anchor_line == 0 {
        return Err(FunctionCallError::RespondToModel(
            "anchor_line must be a 1-indexed line number".to_string(),
        ));
    }
    let requested = anchor_line - 1;
    if requested >= lines.len() {
        return Err(FunctionCallError::RespondToModel(
            "anchor_line exceeds file length".to_string(),
        ));
    }
    if !lines[requested].is_blank {
        return Ok(requested);
    }
    if let Some(previous) = (0..requested).rev().find(|index| !lines[*index].is_blank) {
        return Ok(previous);
    }
    ((requested + 1)..lines.len())
        .find(|index| !lines[*index].is_blank)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "cannot infer indentation from an empty file".to_string(),
            )
        })
}

fn find_ancestor_start(lines: &[LineRecord], anchor_index: usize, levels: usize) -> Option<usize> {
    let mut current_index = anchor_index;
    let mut current_indent = lines[anchor_index].indent;
    let mut found = None;
    for _ in 0..levels {
        let next = (0..current_index)
            .rev()
            .find(|index| !lines[*index].is_blank && lines[*index].indent < current_indent)?;
        found = Some(next);
        current_index = next;
        current_indent = lines[next].indent;
    }
    found
}

fn is_closing_line(line: &str) -> bool {
    matches!(line.chars().next(), Some('}') | Some(']') | Some(')'))
}

fn is_header_line(line: &str) -> bool {
    line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with('*')
        || line.starts_with("#[")
        || line.starts_with("#!")
        || line.starts_with("///")
        || line.starts_with("//!")
        || line.starts_with('@')
}

fn extend_header_upwards(lines: &[LineRecord], actual_start: usize) -> usize {
    let indent = lines[actual_start].indent;
    let mut start = actual_start;
    while start > 0 {
        let previous = &lines[start - 1];
        if previous.is_blank || previous.indent != indent || !is_header_line(previous.trimmed()) {
            break;
        }
        start -= 1;
    }
    start
}

fn block_range(
    lines: &[LineRecord],
    actual_start: usize,
    section_end: usize,
    include_header: bool,
) -> BlockRange {
    let start = if include_header {
        extend_header_upwards(lines, actual_start)
    } else {
        actual_start
    };
    let end = find_block_end(lines, actual_start, section_end);
    BlockRange {
        start,
        actual_start,
        end,
    }
}

fn find_block_end(lines: &[LineRecord], actual_start: usize, section_end: usize) -> usize {
    let start_indent = lines[actual_start].indent;
    let mut saw_nested = false;
    let mut last_included = actual_start;

    for (index, line) in lines
        .iter()
        .enumerate()
        .take(section_end + 1)
        .skip(actual_start + 1)
    {
        if line.is_blank {
            last_included = index;
            continue;
        }
        if line.indent > start_indent {
            saw_nested = true;
            last_included = index;
            continue;
        }
        if line.indent == start_indent && is_closing_line(line.trimmed()) {
            return index;
        }
        if saw_nested {
            return last_included;
        }
        return actual_start;
    }

    last_included
}

fn qualifies_as_block(lines: &[LineRecord], start: usize, section_end: usize) -> bool {
    let start_indent = lines[start].indent;
    for line in lines.iter().take(section_end + 1).skip(start + 1) {
        if line.is_blank {
            continue;
        }
        return line.indent > start_indent;
    }
    false
}

mod slice {
    use super::FunctionCallError;
    use super::LineRecord;
    use super::Path;
    use super::format_output;
    use super::load_lines;
    use super::validate_offset_limit;

    pub(super) async fn read(
        path: &Path,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<String>, FunctionCallError> {
        let lines = load_lines(path).await?;
        read_loaded(lines.as_slice(), offset, limit)
    }

    pub(super) fn read_loaded(
        lines: &[LineRecord],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<String>, FunctionCallError> {
        validate_offset_limit(offset, limit)?;
        let start = offset - 1;
        if start >= lines.len() {
            return Err(FunctionCallError::RespondToModel(
                "offset exceeds file length".to_string(),
            ));
        }
        let end = (start + limit).min(lines.len());
        Ok(format_output(lines, start, end, limit))
    }
}

mod indentation {
    use super::BlockRange;
    use super::FunctionCallError;
    use super::IndentationArgs;
    use super::LineRecord;
    use super::Path;
    use super::block_range;
    use super::find_ancestor_start;
    use super::format_output;
    use super::is_header_line;
    use super::load_lines;
    use super::qualifies_as_block;
    use super::resolve_anchor_index;
    use super::slice;
    use super::validate_offset_limit;

    pub(super) async fn read_block(
        path: &Path,
        offset: usize,
        limit: usize,
        options: IndentationArgs,
    ) -> Result<Vec<String>, FunctionCallError> {
        validate_offset_limit(offset, limit)?;
        if options.max_levels == 0 {
            return Err(FunctionCallError::RespondToModel(
                "indentation.max_levels must be greater than zero".to_string(),
            ));
        }

        let max_lines = options.max_lines.unwrap_or(limit);
        if max_lines == 0 {
            return Err(FunctionCallError::RespondToModel(
                "indentation.max_lines must be greater than zero".to_string(),
            ));
        }

        let lines = load_lines(path).await?;
        if lines.is_empty() || offset > lines.len() {
            return Err(FunctionCallError::RespondToModel(
                "offset exceeds file length".to_string(),
            ));
        }

        let anchor_index = resolve_anchor_index(&lines, options.anchor_line.unwrap_or(offset))?;
        let Some(actual_start) = find_ancestor_start(&lines, anchor_index, options.max_levels)
        else {
            return slice::read_loaded(lines.as_slice(), offset, max_lines);
        };

        let section_end = lines.len() - 1;
        let mut selection = block_range(&lines, actual_start, section_end, options.include_header);
        if options.include_siblings
            && let Some(parent_start) = find_ancestor_start(&lines, actual_start, 1)
        {
            let parent_scope = block_range(
                &lines,
                parent_start,
                section_end,
                /*include_header*/ false,
            );
            selection =
                expand_with_siblings(&lines, parent_scope, selection, options.include_header);
        }

        let end_exclusive = (selection.end + 1).min(lines.len());
        Ok(format_output(
            &lines,
            selection.start,
            end_exclusive,
            max_lines,
        ))
    }

    fn expand_with_siblings(
        lines: &[LineRecord],
        parent_scope: BlockRange,
        selected: BlockRange,
        include_header: bool,
    ) -> BlockRange {
        let items = collect_scope_items(
            lines,
            parent_scope,
            lines[selected.actual_start].indent,
            include_header,
        );
        let Some(position) = items.iter().position(|item| match item {
            ScopeItem::Block(block) => block.actual_start == selected.actual_start,
            ScopeItem::Barrier => false,
        }) else {
            return selected;
        };

        let mut start = selected.start;
        let mut end = selected.end;

        let mut left = position;
        while left > 0 {
            match items[left - 1] {
                ScopeItem::Block(block) => {
                    start = block.start;
                    left -= 1;
                }
                ScopeItem::Barrier => break,
            }
        }

        let mut right = position;
        while right + 1 < items.len() {
            match items[right + 1] {
                ScopeItem::Block(block) => {
                    end = block.end;
                    right += 1;
                }
                ScopeItem::Barrier => break,
            }
        }

        BlockRange {
            start,
            actual_start: selected.actual_start,
            end,
        }
    }

    fn collect_scope_items(
        lines: &[LineRecord],
        parent_scope: BlockRange,
        indent: usize,
        include_header: bool,
    ) -> Vec<ScopeItem> {
        let mut items = Vec::new();
        let mut index = parent_scope.actual_start.saturating_add(1);
        while index <= parent_scope.end {
            let line = &lines[index];
            if line.is_blank || line.indent != indent {
                index += 1;
                continue;
            }

            if is_header_line(line.trimmed()) {
                let header_start = index;
                let mut actual_start = index;
                while actual_start <= parent_scope.end
                    && !lines[actual_start].is_blank
                    && lines[actual_start].indent == indent
                    && is_header_line(lines[actual_start].trimmed())
                {
                    actual_start += 1;
                }
                if actual_start <= parent_scope.end
                    && lines[actual_start].indent == indent
                    && qualifies_as_block(lines, actual_start, parent_scope.end)
                {
                    let mut block =
                        block_range(lines, actual_start, parent_scope.end, include_header);
                    block.start = header_start;
                    items.push(ScopeItem::Block(block));
                    index = block.end + 1;
                    continue;
                }
                items.push(ScopeItem::Barrier);
                index = actual_start;
                continue;
            }

            if qualifies_as_block(lines, index, parent_scope.end) {
                let block = block_range(lines, index, parent_scope.end, include_header);
                items.push(ScopeItem::Block(block));
                index = block.end + 1;
            } else {
                items.push(ScopeItem::Barrier);
                index += 1;
            }
        }
        items
    }

    #[derive(Clone, Copy, Debug)]
    enum ScopeItem {
        Block(BlockRange),
        Barrier,
    }
}

#[cfg(test)]
#[path = "read_file_tests.rs"]
mod tests;
