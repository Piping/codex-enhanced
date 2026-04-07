use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::LazyLock;

use regex_lite::Regex;
use shlex::Shlex;

use super::custom_prompts::CustomPrompt;
use super::custom_prompts::PROMPTS_CMD_PREFIX;
use codex_protocol::user_input::ByteRange;
use codex_protocol::user_input::TextElement;

static PROMPT_ARG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$[A-Z][A-Z0-9_]*").unwrap_or_else(|_| std::process::abort()));

/// Parse a first-line slash command of the form `/name <rest>`.
/// Returns `(name, rest_after_name, rest_offset)` if the line begins with `/`
/// and contains a non-empty name; otherwise returns `None`.
///
/// `rest_offset` is the byte index into the original line where `rest_after_name`
/// starts after trimming leading whitespace (so `line[rest_offset..] == rest_after_name`).
pub fn parse_slash_name(line: &str) -> Option<(&str, &str, usize)> {
    let stripped = line.strip_prefix('/')?;
    let mut name_end_in_stripped = stripped.len();
    for (idx, ch) in stripped.char_indices() {
        if ch.is_whitespace() {
            name_end_in_stripped = idx;
            break;
        }
    }
    let name = &stripped[..name_end_in_stripped];
    if name.is_empty() {
        return None;
    }
    let rest_untrimmed = &stripped[name_end_in_stripped..];
    let rest = rest_untrimmed.trim_start();
    let rest_start_in_stripped = name_end_in_stripped + (rest_untrimmed.len() - rest.len());
    let rest_offset = rest_start_in_stripped + 1;
    Some((name, rest, rest_offset))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PromptArg {
    pub(crate) text: String,
    pub(crate) text_elements: Vec<TextElement>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PromptExpansion {
    pub(crate) text: String,
    pub(crate) text_elements: Vec<TextElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptSelectionMode {
    Completion,
    Submit,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PromptSelectionAction {
    Insert {
        text: String,
        cursor: Option<usize>,
    },
    Submit {
        text: String,
        text_elements: Vec<TextElement>,
    },
}

#[derive(Debug)]
pub(crate) enum PromptArgsError {
    MissingAssignment { token: String },
    MissingKey { token: String },
}

impl PromptArgsError {
    fn describe(&self, command: &str) -> String {
        match self {
            PromptArgsError::MissingAssignment { token } => format!(
                "Could not parse {command}: expected key=value but found '{token}'. Wrap values in double quotes if they contain spaces."
            ),
            PromptArgsError::MissingKey { token } => {
                format!("Could not parse {command}: expected a name before '=' in '{token}'.")
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum PromptExpansionError {
    Args {
        command: String,
        error: PromptArgsError,
    },
    MissingArgs {
        command: String,
        missing: Vec<String>,
    },
}

impl PromptExpansionError {
    pub(crate) fn user_message(&self) -> String {
        match self {
            PromptExpansionError::Args { command, error } => error.describe(command),
            PromptExpansionError::MissingArgs { command, missing } => {
                let list = missing.join(", ");
                format!(
                    "Missing required args for {command}: {list}. Provide as key=value (quote values with spaces)."
                )
            }
        }
    }
}

pub(crate) fn prompt_argument_names(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for matched in PROMPT_ARG_REGEX.find_iter(content) {
        if matched.start() > 0 && content.as_bytes()[matched.start() - 1] == b'$' {
            continue;
        }
        let name = &content[matched.start() + 1..matched.end()];
        if name == "ARGUMENTS" {
            continue;
        }
        let name = name.to_string();
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
    names
}

pub(crate) fn prompt_has_numeric_placeholders(content: &str) -> bool {
    if content.contains("$ARGUMENTS") {
        return true;
    }
    let bytes = content.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'$' && (b'1'..=b'9').contains(&bytes[index + 1]) {
            return true;
        }
        index += 1;
    }
    false
}

pub(crate) fn prompt_selection_action(
    prompt: &CustomPrompt,
    first_line: &str,
    mode: PromptSelectionMode,
    text_elements: &[TextElement],
) -> PromptSelectionAction {
    let named_args = prompt_argument_names(&prompt.content);
    let has_numeric = prompt_has_numeric_placeholders(&prompt.content);

    match mode {
        PromptSelectionMode::Completion => {
            if !named_args.is_empty() {
                let (text, cursor) =
                    prompt_command_with_arg_placeholders(&prompt.name, &named_args);
                return PromptSelectionAction::Insert {
                    text,
                    cursor: Some(cursor),
                };
            }
            let text = format!("/{PROMPTS_CMD_PREFIX}:{} ", prompt.name);
            PromptSelectionAction::Insert {
                cursor: Some(text.len()),
                text,
            }
        }
        PromptSelectionMode::Submit => {
            if !named_args.is_empty() {
                let (text, cursor) =
                    prompt_command_with_arg_placeholders(&prompt.name, &named_args);
                return PromptSelectionAction::Insert {
                    text,
                    cursor: Some(cursor),
                };
            }
            if has_numeric {
                if let Some(expanded) =
                    expand_if_numeric_with_positional_args(prompt, first_line, text_elements)
                {
                    return PromptSelectionAction::Submit {
                        text: expanded.text,
                        text_elements: expanded.text_elements,
                    };
                }
                return PromptSelectionAction::Insert {
                    text: format!("/{PROMPTS_CMD_PREFIX}:{} ", prompt.name),
                    cursor: None,
                };
            }
            PromptSelectionAction::Insert {
                text: prompt.content.clone(),
                cursor: Some(prompt.content.len()),
            }
        }
    }
}

pub(crate) fn expand_custom_prompt(
    text: &str,
    text_elements: &[TextElement],
    custom_prompts: &[CustomPrompt],
) -> Result<Option<PromptExpansion>, PromptExpansionError> {
    let Some((name, rest, rest_offset)) = parse_slash_name(text) else {
        return Ok(None);
    };
    let Some(prompt_name) = name.strip_prefix(&format!("{PROMPTS_CMD_PREFIX}:")) else {
        return Ok(None);
    };
    let Some(prompt) = custom_prompts
        .iter()
        .find(|prompt| prompt.name == prompt_name)
    else {
        return Ok(None);
    };

    let required = prompt_argument_names(&prompt.content);
    let local_elements: Vec<TextElement> = text_elements
        .iter()
        .filter_map(|element| {
            let mut shifted = shift_text_element_left(element, rest_offset)?;
            if shifted.byte_range.start >= rest.len() {
                return None;
            }
            shifted.byte_range.end = shifted.byte_range.end.min(rest.len());
            (shifted.byte_range.start < shifted.byte_range.end).then_some(shifted)
        })
        .collect();

    if !required.is_empty() {
        let inputs = parse_prompt_inputs(rest, &local_elements).map_err(|error| {
            PromptExpansionError::Args {
                command: format!("/{name}"),
                error,
            }
        })?;
        let missing: Vec<String> = required
            .into_iter()
            .filter(|key| !inputs.contains_key(key))
            .collect();
        if !missing.is_empty() {
            return Err(PromptExpansionError::MissingArgs {
                command: format!("/{name}"),
                missing,
            });
        }
        let (expanded_text, expanded_elements) =
            expand_named_placeholders_with_elements(&prompt.content, &inputs);
        return Ok(Some(PromptExpansion {
            text: expanded_text,
            text_elements: expanded_elements,
        }));
    }

    let positional_args = parse_positional_args(rest, &local_elements);
    Ok(Some(expand_numeric_placeholders(
        &prompt.content,
        &positional_args,
    )))
}

pub(crate) fn expand_if_numeric_with_positional_args(
    prompt: &CustomPrompt,
    first_line: &str,
    text_elements: &[TextElement],
) -> Option<PromptExpansion> {
    if !prompt_argument_names(&prompt.content).is_empty() {
        return None;
    }
    if !prompt_has_numeric_placeholders(&prompt.content) {
        return None;
    }
    let args = extract_positional_args_for_prompt_line(first_line, &prompt.name, text_elements);
    if args.is_empty() {
        return None;
    }
    Some(expand_numeric_placeholders(&prompt.content, &args))
}

fn parse_positional_args(rest: &str, text_elements: &[TextElement]) -> Vec<PromptArg> {
    parse_tokens_with_elements(rest, text_elements)
}

fn parse_prompt_inputs(
    rest: &str,
    text_elements: &[TextElement],
) -> Result<HashMap<String, PromptArg>, PromptArgsError> {
    let mut inputs = HashMap::new();
    if rest.trim().is_empty() {
        return Ok(inputs);
    }

    for token in parse_tokens_with_elements(rest, text_elements) {
        let Some((key, value)) = token.text.split_once('=') else {
            return Err(PromptArgsError::MissingAssignment { token: token.text });
        };
        if key.is_empty() {
            return Err(PromptArgsError::MissingKey { token: token.text });
        }
        let value_start = key.len() + 1;
        let value_elements = token
            .text_elements
            .iter()
            .filter_map(|element| shift_text_element_left(element, value_start))
            .collect();
        inputs.insert(
            key.to_string(),
            PromptArg {
                text: value.to_string(),
                text_elements: value_elements,
            },
        );
    }
    Ok(inputs)
}

fn parse_tokens_with_elements(rest: &str, text_elements: &[TextElement]) -> Vec<PromptArg> {
    let mut elements = text_elements.to_vec();
    elements.sort_by_key(|element| element.byte_range.start);
    let (text_for_shlex, replacements) = replace_text_elements_with_sentinels(rest, &elements);
    Shlex::new(&text_for_shlex)
        .map(|token| apply_replacements_to_token(token, &replacements))
        .collect()
}

fn replace_text_elements_with_sentinels(
    rest: &str,
    elements: &[TextElement],
) -> (String, Vec<ElementReplacement>) {
    let mut out = String::with_capacity(rest.len());
    let mut replacements = Vec::new();
    let mut cursor = 0;

    for (index, element) in elements.iter().enumerate() {
        let start = element.byte_range.start;
        let end = element.byte_range.end;
        out.push_str(&rest[cursor..start]);
        let mut sentinel = format!("__CODEX_ELEM_{index}__");
        while rest.contains(&sentinel) {
            sentinel.push('_');
        }
        out.push_str(&sentinel);
        replacements.push(ElementReplacement {
            sentinel,
            text: rest[start..end].to_string(),
            placeholder: element.placeholder(rest).map(str::to_string),
        });
        cursor = end;
    }

    out.push_str(&rest[cursor..]);
    (out, replacements)
}

fn apply_replacements_to_token(token: String, replacements: &[ElementReplacement]) -> PromptArg {
    if replacements.is_empty() {
        return PromptArg {
            text: token,
            text_elements: Vec::new(),
        };
    }

    let mut out = String::with_capacity(token.len());
    let mut out_elements = Vec::new();
    let mut cursor = 0;

    while cursor < token.len() {
        let Some((offset, replacement)) = next_replacement(&token, cursor, replacements) else {
            out.push_str(&token[cursor..]);
            break;
        };
        let start_in_token = cursor + offset;
        out.push_str(&token[cursor..start_in_token]);
        let start = out.len();
        out.push_str(&replacement.text);
        let end = out.len();
        if start < end {
            out_elements.push(TextElement::new(
                ByteRange { start, end },
                replacement.placeholder.clone(),
            ));
        }
        cursor = start_in_token + replacement.sentinel.len();
    }

    PromptArg {
        text: out,
        text_elements: out_elements,
    }
}

fn next_replacement<'a>(
    token: &str,
    cursor: usize,
    replacements: &'a [ElementReplacement],
) -> Option<(usize, &'a ElementReplacement)> {
    let slice = &token[cursor..];
    let mut best = None;
    for replacement in replacements {
        if let Some(position) = slice.find(&replacement.sentinel) {
            match best {
                Some((best_position, _)) if best_position <= position => {}
                _ => best = Some((position, replacement)),
            }
        }
    }
    best
}

fn expand_named_placeholders_with_elements(
    content: &str,
    args: &HashMap<String, PromptArg>,
) -> (String, Vec<TextElement>) {
    let mut out = String::with_capacity(content.len());
    let mut out_elements = Vec::new();
    let mut cursor = 0;

    for matched in PROMPT_ARG_REGEX.find_iter(content) {
        let start = matched.start();
        let end = matched.end();
        if start > 0 && content.as_bytes()[start - 1] == b'$' {
            out.push_str(&content[cursor..end]);
            cursor = end;
            continue;
        }
        out.push_str(&content[cursor..start]);
        cursor = end;
        let key = &content[start + 1..end];
        if let Some(arg) = args.get(key) {
            append_arg_with_elements(&mut out, &mut out_elements, arg);
        } else {
            out.push_str(&content[start..end]);
        }
    }

    out.push_str(&content[cursor..]);
    (out, out_elements)
}

fn expand_numeric_placeholders(content: &str, args: &[PromptArg]) -> PromptExpansion {
    let mut out = String::with_capacity(content.len());
    let mut out_elements = Vec::new();
    let mut index = 0;

    while let Some(offset) = content[index..].find('$') {
        let placeholder_start = index + offset;
        out.push_str(&content[index..placeholder_start]);
        let rest = &content[placeholder_start..];
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 {
            match bytes[1] {
                b'$' => {
                    out.push_str("$$");
                    index = placeholder_start + 2;
                    continue;
                }
                b'1'..=b'9' => {
                    let arg_index = (bytes[1] - b'1') as usize;
                    if let Some(arg) = args.get(arg_index) {
                        append_arg_with_elements(&mut out, &mut out_elements, arg);
                    }
                    index = placeholder_start + 2;
                    continue;
                }
                _ => {}
            }
        }
        if rest.len() > "ARGUMENTS".len() && rest[1..].starts_with("ARGUMENTS") {
            if !args.is_empty() {
                append_joined_args_with_elements(&mut out, &mut out_elements, args);
            }
            index = placeholder_start + 1 + "ARGUMENTS".len();
            continue;
        }
        out.push('$');
        index = placeholder_start + 1;
    }

    out.push_str(&content[index..]);
    PromptExpansion {
        text: out,
        text_elements: out_elements,
    }
}

fn extract_positional_args_for_prompt_line(
    line: &str,
    prompt_name: &str,
    text_elements: &[TextElement],
) -> Vec<PromptArg> {
    let trimmed = line.trim_start();
    let trim_offset = line.len() - trimmed.len();
    let Some((name, rest, rest_offset)) = parse_slash_name(trimmed) else {
        return Vec::new();
    };
    let Some(after_prefix) = name.strip_prefix(&format!("{PROMPTS_CMD_PREFIX}:")) else {
        return Vec::new();
    };
    if after_prefix != prompt_name {
        return Vec::new();
    }
    let rest_trimmed_start = rest.trim_start();
    let args_str = rest_trimmed_start.trim_end();
    if args_str.is_empty() {
        return Vec::new();
    }
    let args_offset = trim_offset + rest_offset + (rest.len() - rest_trimmed_start.len());
    let local_elements: Vec<TextElement> = text_elements
        .iter()
        .filter_map(|element| {
            let mut shifted = shift_text_element_left(element, args_offset)?;
            if shifted.byte_range.start >= args_str.len() {
                return None;
            }
            shifted.byte_range.end = shifted.byte_range.end.min(args_str.len());
            (shifted.byte_range.start < shifted.byte_range.end).then_some(shifted)
        })
        .collect();
    parse_positional_args(args_str, &local_elements)
}

fn shift_text_element_left(element: &TextElement, offset: usize) -> Option<TextElement> {
    if element.byte_range.end <= offset {
        return None;
    }
    let start = element.byte_range.start.saturating_sub(offset);
    let end = element.byte_range.end.saturating_sub(offset);
    (start < end).then_some(element.map_range(|_| ByteRange { start, end }))
}

fn append_arg_with_elements(
    out: &mut String,
    out_elements: &mut Vec<TextElement>,
    arg: &PromptArg,
) {
    let start = out.len();
    out.push_str(&arg.text);
    if arg.text_elements.is_empty() {
        return;
    }
    out_elements.extend(arg.text_elements.iter().map(|element| {
        element.map_range(|range| ByteRange {
            start: start + range.start,
            end: start + range.end,
        })
    }));
}

fn append_joined_args_with_elements(
    out: &mut String,
    out_elements: &mut Vec<TextElement>,
    args: &[PromptArg],
) {
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        append_arg_with_elements(out, out_elements, arg);
    }
}

fn prompt_command_with_arg_placeholders(name: &str, args: &[String]) -> (String, usize) {
    let mut text = format!("/{PROMPTS_CMD_PREFIX}:{name}");
    let mut cursor = text.len();
    for (index, arg) in args.iter().enumerate() {
        text.push_str(format!(" {arg}=\"\"").as_str());
        if index == 0 {
            cursor = text.len() - 1;
        }
    }
    (text, cursor)
}

#[derive(Debug, Clone)]
struct ElementReplacement {
    sentinel: String,
    text: String,
    placeholder: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn prompt(name: &str, content: &str) -> CustomPrompt {
        CustomPrompt {
            name: name.to_string(),
            path: format!("/tmp/{name}.md").into(),
            content: content.to_string(),
            description: None,
            argument_hint: None,
        }
    }

    #[test]
    fn expand_named_arguments() {
        let prompts = vec![prompt("review", "Review $USER changes on $BRANCH")];
        let expanded =
            expand_custom_prompt("/prompts:review USER=Alice BRANCH=main", &[], &prompts)
                .expect("expand custom prompt");
        assert_eq!(
            expanded,
            Some(PromptExpansion {
                text: "Review Alice changes on main".to_string(),
                text_elements: Vec::new(),
            })
        );
    }

    #[test]
    fn expand_numeric_arguments() {
        let prompts = vec![prompt("rewrite", "Rewrite $1 as $2")];
        let expanded = expand_custom_prompt("/prompts:rewrite draft polished", &[], &prompts)
            .expect("expand custom prompt");
        assert_eq!(
            expanded,
            Some(PromptExpansion {
                text: "Rewrite draft as polished".to_string(),
                text_elements: Vec::new(),
            })
        );
    }

    #[test]
    fn missing_required_args_reports_error() {
        let prompts = vec![prompt("review", "Review $USER changes on $BRANCH")];
        let err = expand_custom_prompt("/prompts:review USER=Alice", &[], &prompts)
            .expect_err("missing args should fail")
            .user_message();
        assert!(err.contains("BRANCH"));
    }

    #[test]
    fn prompt_selection_submit_inserts_plain_prompt_body() {
        let action = prompt_selection_action(
            &prompt("rewrite", "Please rewrite this draft"),
            "/prompts:rewrite",
            PromptSelectionMode::Submit,
            &[],
        );
        assert_eq!(
            action,
            PromptSelectionAction::Insert {
                text: "Please rewrite this draft".to_string(),
                cursor: Some("Please rewrite this draft".len()),
            }
        );
    }

    #[test]
    fn prompt_selection_completion_for_named_args_inserts_placeholders() {
        let action = prompt_selection_action(
            &prompt("review", "Review $USER changes on $BRANCH"),
            "/review",
            PromptSelectionMode::Completion,
            &[],
        );
        assert_eq!(
            action,
            PromptSelectionAction::Insert {
                text: "/prompts:review USER=\"\" BRANCH=\"\"".to_string(),
                cursor: Some("/prompts:review USER=\"".len()),
            }
        );
    }
}
