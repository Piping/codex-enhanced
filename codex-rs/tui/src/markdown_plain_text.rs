use pulldown_cmark::Event;
use pulldown_cmark::Options;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;
use regex_lite::Regex;
use std::path::Path;
use std::sync::LazyLock;

static PARENTHESIZED_ABSOLUTE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*\((/[^)\n]+)\)")
        .unwrap_or_else(|error| panic!("invalid absolute path regex: {error}"))
});

pub(crate) fn markdown_to_plain_text(input: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(input, options);
    let mut plain_text = String::new();
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut link_stack: Vec<(String, usize)> = Vec::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote | Tag::CodeBlock(_) => {
                    ensure_block_boundary(&mut plain_text);
                }
                Tag::List(start) => list_stack.push(start),
                Tag::Item => start_list_item(&mut plain_text, &mut list_stack),
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    link_stack.push((dest_url.to_string(), plain_text.len()));
                }
                Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::HtmlBlock
                | Tag::FootnoteDefinition(_)
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell
                | Tag::MetadataBlock(_) => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote
                | TagEnd::CodeBlock
                | TagEnd::List(_) => ensure_block_boundary(&mut plain_text),
                TagEnd::Item => ensure_line_boundary(&mut plain_text),
                TagEnd::Link | TagEnd::Image => {
                    if let Some((dest, start)) = link_stack.pop() {
                        let rendered = plain_text[start..].trim();
                        if !dest.is_empty() && rendered != dest {
                            if rendered.is_empty() {
                                plain_text.push_str(&dest);
                            } else {
                                plain_text.push_str(" (");
                                plain_text.push_str(&dest);
                                plain_text.push(')');
                            }
                        }
                    }
                }
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::HtmlBlock
                | TagEnd::FootnoteDefinition
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell
                | TagEnd::MetadataBlock(_) => {}
            },
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                plain_text.push_str(&text);
            }
            Event::SoftBreak | Event::HardBreak => ensure_line_boundary(&mut plain_text),
            Event::Rule => {
                ensure_block_boundary(&mut plain_text);
                plain_text.push_str("---");
                ensure_block_boundary(&mut plain_text);
            }
            Event::FootnoteReference(reference) => plain_text.push_str(&reference),
            Event::TaskListMarker(checked) => {
                plain_text.push('[');
                plain_text.push(if checked { 'x' } else { ' ' });
                plain_text.push_str("] ");
            }
        }
    }

    strip_parenthesized_absolute_paths(&plain_text)
}

fn start_list_item(plain_text: &mut String, list_stack: &mut [Option<u64>]) {
    ensure_line_boundary(plain_text);

    let depth = list_stack.len().saturating_sub(1);
    plain_text.push_str(&"  ".repeat(depth));

    match list_stack.last_mut() {
        Some(Some(next_number)) => {
            let current_number = *next_number;
            *next_number += 1;
            plain_text.push_str(&format!("{current_number}. "));
        }
        Some(None) => plain_text.push_str("• "),
        None => {}
    }
}

fn ensure_line_boundary(plain_text: &mut String) {
    trim_trailing_horizontal_whitespace(plain_text);
    if !plain_text.is_empty() && !plain_text.ends_with('\n') {
        plain_text.push('\n');
    }
}

fn ensure_block_boundary(plain_text: &mut String) {
    trim_trailing_horizontal_whitespace(plain_text);
    if plain_text.is_empty() || plain_text.ends_with("\n\n") {
        return;
    }
    if plain_text.ends_with('\n') {
        plain_text.push('\n');
    } else {
        plain_text.push_str("\n\n");
    }
}

fn trim_trailing_horizontal_whitespace(plain_text: &mut String) {
    while matches!(plain_text.chars().last(), Some(' ' | '\t')) {
        plain_text.pop();
    }
}

fn strip_parenthesized_absolute_paths(plain_text: &str) -> String {
    let mut sanitized = plain_text.trim().to_string();

    while let Some(captures) = PARENTHESIZED_ABSOLUTE_PATH_RE.captures(&sanitized) {
        let Some(full_match) = captures.get(0) else {
            break;
        };
        let Some(path_match) = captures.get(1) else {
            break;
        };
        if !looks_like_absolute_path(path_match.as_str()) {
            break;
        }

        sanitized.replace_range(full_match.start()..full_match.end(), "");
    }

    sanitized
}

fn looks_like_absolute_path(path_text: &str) -> bool {
    let path_without_hash_suffix = path_text
        .split_once('#')
        .map_or(path_text, |(path, _suffix)| path);
    let path_without_location_suffix = strip_colon_location_suffix(path_without_hash_suffix);
    Path::new(path_without_location_suffix).is_absolute()
}

fn strip_colon_location_suffix(path_text: &str) -> &str {
    let bytes = path_text.as_bytes();
    let mut index = bytes.len();

    while index > 0 && bytes[index - 1].is_ascii_digit() {
        index -= 1;
    }
    if index == bytes.len() || index == 0 || bytes[index - 1] != b':' {
        return path_text;
    }

    index -= 1;
    let mut candidate_start = index;

    while candidate_start > 0 && bytes[candidate_start - 1].is_ascii_digit() {
        candidate_start -= 1;
    }
    if candidate_start > 0 && bytes[candidate_start - 1] == b':' {
        candidate_start -= 1;
        while candidate_start > 0 && bytes[candidate_start - 1].is_ascii_digit() {
            candidate_start -= 1;
        }
    }

    let candidate = &path_text[candidate_start..];
    if candidate.starts_with(':')
        && candidate[1..]
            .split(':')
            .all(|segment| !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit()))
    {
        &path_text[..candidate_start]
    } else {
        path_text
    }
}

#[cfg(test)]
mod tests {
    use super::markdown_to_plain_text;
    use pretty_assertions::assert_eq;

    #[test]
    fn strips_heading_list_and_link_markdown() {
        let markdown = "# Title\n\n- item with **bold** text\n- [docs](https://example.com)\n";

        assert_eq!(
            markdown_to_plain_text(markdown),
            "Title\n\n• item with bold text\n• docs (https://example.com)"
        );
    }

    #[test]
    fn preserves_ordered_lists_and_task_markers() {
        let markdown = "1. first\n2. second\n\n- [x] done\n- [ ] todo\n";

        assert_eq!(
            markdown_to_plain_text(markdown),
            "1. first\n2. second\n\n  • [x] done\n  • [ ] todo"
        );
    }

    #[test]
    fn strips_parenthesized_absolute_file_paths_from_plain_text_copy() {
        let markdown = "[src/lib.rs](/Users/example/code/codex/codex-rs/tui/src/lib.rs:12:3)\n\
            [docs](https://example.com/docs)\n";

        assert_eq!(
            markdown_to_plain_text(markdown),
            "src/lib.rs\ndocs (https://example.com/docs)"
        );
    }
}
