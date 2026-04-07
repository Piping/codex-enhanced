use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Base namespace for custom prompt slash commands (without trailing colon).
pub(crate) const PROMPTS_CMD_PREFIX: &str = "prompts";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CustomPrompt {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) content: String,
    pub(crate) description: Option<String>,
    pub(crate) argument_hint: Option<String>,
}

pub(crate) fn discover_prompts_in(dir: &Path) -> Vec<CustomPrompt> {
    discover_prompts_in_excluding(dir, &HashSet::new())
}

fn discover_prompts_in_excluding(dir: &Path, exclude: &HashSet<String>) -> Vec<CustomPrompt> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut prompts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        if !is_md {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if exclude.contains(name) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let (description, argument_hint, body) = parse_frontmatter(&content);
        prompts.push(CustomPrompt {
            name: name.to_string(),
            path,
            content: body,
            description,
            argument_hint,
        });
    }

    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    prompts
}

fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>, String) {
    let mut segments = content.split_inclusive('\n');
    let Some(first_segment) = segments.next() else {
        return (None, None, String::new());
    };
    let first_line = first_segment.trim_end_matches(['\r', '\n']);
    if first_line.trim() != "---" {
        return (None, None, content.to_string());
    }

    let mut description = None;
    let mut argument_hint = None;
    let mut frontmatter_closed = false;
    let mut consumed = first_segment.len();

    for segment in segments {
        let line = segment.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();

        if trimmed == "---" {
            frontmatter_closed = true;
            consumed += segment.len();
            break;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            consumed += segment.len();
            continue;
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            let normalized_key = key.trim().to_ascii_lowercase();
            let mut normalized_value = value.trim().to_string();
            if normalized_value.len() >= 2 {
                let bytes = normalized_value.as_bytes();
                let first = bytes[0];
                let last = bytes[bytes.len() - 1];
                if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                    normalized_value = normalized_value[1..normalized_value.len() - 1].to_string();
                }
            }
            match normalized_key.as_str() {
                "description" => description = Some(normalized_value),
                "argument-hint" | "argument_hint" => argument_hint = Some(normalized_value),
                _ => {}
            }
        }

        consumed += segment.len();
    }

    if !frontmatter_closed {
        return (None, None, content.to_string());
    }

    let body = if consumed >= content.len() {
        String::new()
    } else {
        content[consumed..].to_string()
    };
    (description, argument_hint, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_frontmatter_extracts_description_and_hint() {
        let content = "---\n\
description: review prompt\n\
argument-hint: USER=\"...\"\n\
---\n\
Body text\n";

        let (description, argument_hint, body) = parse_frontmatter(content);
        assert_eq!(description, Some("review prompt".to_string()));
        assert_eq!(argument_hint, Some("USER=\"...\"".to_string()));
        assert_eq!(body, "Body text\n".to_string());
    }

    #[test]
    fn parse_frontmatter_leaves_plain_markdown_unchanged() {
        let content = "# Prompt\n\nBody\n";
        let (description, argument_hint, body) = parse_frontmatter(content);
        assert_eq!(description, None);
        assert_eq!(argument_hint, None);
        assert_eq!(body, content.to_string());
    }
}
