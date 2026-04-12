use serde_yaml::Mapping;
use serde_yaml::Value as YamlValue;

pub(crate) fn serialize_yaml_value(value: &YamlValue) -> Result<String, String> {
    let mut output = String::new();
    write_yaml_value(&mut output, value, 0)?;
    Ok(output)
}

fn write_yaml_value(output: &mut String, value: &YamlValue, indent: usize) -> Result<(), String> {
    match value {
        YamlValue::Mapping(mapping) => write_mapping(output, mapping, indent),
        YamlValue::Sequence(sequence) => write_sequence(output, sequence, indent),
        YamlValue::String(text) if text.contains('\n') => {
            write_block_scalar(output, "", text, indent);
            Ok(())
        }
        _ => {
            output.push_str(&" ".repeat(indent));
            output.push_str(&serialize_inline_value(value)?);
            output.push('\n');
            Ok(())
        }
    }
}

fn write_mapping(output: &mut String, mapping: &Mapping, indent: usize) -> Result<(), String> {
    for (key, value) in mapping {
        write_mapping_entry(output, key, value, indent)?;
    }
    Ok(())
}

fn write_mapping_entry(
    output: &mut String,
    key: &YamlValue,
    value: &YamlValue,
    indent: usize,
) -> Result<(), String> {
    let prefix = format!("{}{}:", " ".repeat(indent), serialize_yaml_key(key)?);
    match value {
        YamlValue::Mapping(mapping) if mapping.is_empty() => {
            output.push_str(&prefix);
            output.push_str(" {}\n");
        }
        YamlValue::Sequence(sequence) if sequence.is_empty() => {
            output.push_str(&prefix);
            output.push_str(" []\n");
        }
        YamlValue::Mapping(mapping) => {
            output.push_str(&prefix);
            output.push('\n');
            write_mapping(output, mapping, indent + 2)?;
        }
        YamlValue::Sequence(sequence) => {
            output.push_str(&prefix);
            output.push('\n');
            write_sequence(output, sequence, indent + 2)?;
        }
        YamlValue::String(text) if text.contains('\n') => {
            write_block_scalar(output, &prefix, text, indent + 2);
        }
        _ => {
            output.push_str(&prefix);
            output.push(' ');
            output.push_str(&serialize_inline_value(value)?);
            output.push('\n');
        }
    }
    Ok(())
}

fn write_sequence(
    output: &mut String,
    sequence: &[YamlValue],
    indent: usize,
) -> Result<(), String> {
    for value in sequence {
        match value {
            YamlValue::Mapping(mapping) => write_sequence_mapping_item(output, mapping, indent)?,
            YamlValue::Sequence(sequence) if sequence.is_empty() => {
                output.push_str(&" ".repeat(indent));
                output.push_str("- []\n");
            }
            YamlValue::Sequence(sequence) => {
                output.push_str(&" ".repeat(indent));
                output.push_str("-\n");
                write_sequence(output, sequence, indent + 2)?;
            }
            YamlValue::String(text) if text.contains('\n') => {
                write_block_scalar(
                    output,
                    &format!("{}-", " ".repeat(indent)),
                    text,
                    indent + 2,
                );
            }
            _ => {
                output.push_str(&" ".repeat(indent));
                output.push_str("- ");
                output.push_str(&serialize_inline_value(value)?);
                output.push('\n');
            }
        }
    }
    Ok(())
}

fn write_sequence_mapping_item(
    output: &mut String,
    mapping: &Mapping,
    indent: usize,
) -> Result<(), String> {
    if mapping.is_empty() {
        output.push_str(&" ".repeat(indent));
        output.push_str("- {}\n");
        return Ok(());
    }

    let mut entries = mapping.iter();
    let Some((first_key, first_value)) = entries.next() else {
        return Ok(());
    };
    let prefix = format!(
        "{}- {}:",
        " ".repeat(indent),
        serialize_yaml_key(first_key)?
    );
    match first_value {
        YamlValue::Mapping(mapping) if mapping.is_empty() => {
            output.push_str(&prefix);
            output.push_str(" {}\n");
        }
        YamlValue::Sequence(sequence) if sequence.is_empty() => {
            output.push_str(&prefix);
            output.push_str(" []\n");
        }
        YamlValue::Mapping(mapping) => {
            output.push_str(&prefix);
            output.push('\n');
            write_mapping(output, mapping, indent + 4)?;
        }
        YamlValue::Sequence(sequence) => {
            output.push_str(&prefix);
            output.push('\n');
            write_sequence(output, sequence, indent + 4)?;
        }
        YamlValue::String(text) if text.contains('\n') => {
            write_block_scalar(output, &prefix, text, indent + 4);
        }
        _ => {
            output.push_str(&prefix);
            output.push(' ');
            output.push_str(&serialize_inline_value(first_value)?);
            output.push('\n');
        }
    }

    for (key, value) in entries {
        write_mapping_entry(output, key, value, indent + 2)?;
    }

    Ok(())
}

fn write_block_scalar(output: &mut String, prefix: &str, text: &str, content_indent: usize) {
    if !prefix.is_empty() {
        output.push_str(prefix);
        output.push(' ');
    }
    output.push_str(block_scalar_header(text));
    output.push('\n');

    let trailing_newlines = text.chars().rev().take_while(|ch| *ch == '\n').count();
    let content = if trailing_newlines == 0 {
        text
    } else {
        &text[..text.len() - trailing_newlines]
    };

    if content.is_empty() {
        output.push_str(&" ".repeat(content_indent));
        output.push('\n');
        return;
    }

    for line in content.split('\n') {
        output.push_str(&" ".repeat(content_indent));
        output.push_str(line);
        output.push('\n');
    }
}

fn block_scalar_header(text: &str) -> &'static str {
    let trailing_newlines = text.chars().rev().take_while(|ch| *ch == '\n').count();
    match trailing_newlines {
        0 => "|-",
        1 => "|",
        _ => "|+",
    }
}

fn serialize_yaml_key(value: &YamlValue) -> Result<String, String> {
    match value {
        YamlValue::String(text) if text.contains('\n') => {
            Err("workflow yaml keys cannot be multiline strings".to_string())
        }
        _ => serialize_inline_value(value),
    }
}

fn serialize_inline_value(value: &YamlValue) -> Result<String, String> {
    let rendered = serde_yaml::to_string(value).map_err(|err| err.to_string())?;
    Ok(rendered.trim_start_matches("---\n").trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_yaml::Value as YamlValue;

    use super::serialize_yaml_value;

    #[test]
    fn serializes_multiline_strings_as_block_scalars() {
        let value: YamlValue = serde_yaml::from_str(
            r#"steps:
  - prompt: "line one\nline two"
"#,
        )
        .expect("yaml");

        assert_eq!(
            serialize_yaml_value(&value).expect("serialize"),
            concat!(
                "steps:\n",
                "  - prompt: |-\n",
                "      line one\n",
                "      line two\n",
            )
        );
    }
}
