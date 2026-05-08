use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;

/// Returns JSON values that are compatible with function calling in the
/// Chat Completions API.
pub fn create_tools_json_for_chat_completions_api(
    tools: &[ToolSpec],
) -> Result<Vec<Value>, serde_json::Error> {
    let mut tools_json = Vec::new();

    for tool in tools {
        match tool {
            ToolSpec::Function(function) => {
                tools_json.push(function_tool_to_chat_json(function)?);
            }
            ToolSpec::Freeform(freeform) if freeform.name == "apply_patch" => {
                let function = ResponsesApiTool {
                    name: freeform.name.clone(),
                    description: freeform.description.clone(),
                    strict: false,
                    defer_loading: None,
                    parameters: JsonSchema::object(
                        BTreeMap::from([(
                            "input".to_string(),
                            JsonSchema::string(Some(
                                "The entire contents of the apply_patch command".to_string(),
                            )),
                        )]),
                        Some(vec!["input".to_string()]),
                        Some(false.into()),
                    ),
                    output_schema: None,
                };
                tools_json.push(function_tool_to_chat_json(&function)?);
            }
            ToolSpec::Freeform(_)
            | ToolSpec::Namespace(_)
            | ToolSpec::ImageGeneration { .. }
            | ToolSpec::LocalShell {}
            | ToolSpec::ToolSearch { .. }
            | ToolSpec::WebSearch { .. } => {}
        }
    }

    Ok(tools_json)
}

fn function_tool_to_chat_json(tool: &ResponsesApiTool) -> Result<Value, serde_json::Error> {
    responses_function_to_chat_json(&tool.name, tool)
}

fn responses_function_to_chat_json(
    name: &str,
    tool: &ResponsesApiTool,
) -> Result<Value, serde_json::Error> {
    Ok(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": tool.description,
            "parameters": serde_json::to_value(&tool.parameters)?,
            "strict": tool.strict,
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FreeformTool;
    use crate::FreeformToolFormat;
    use crate::JsonSchema;
    use crate::ResponsesApiNamespaceTool;
    use crate::ResponsesApiTool;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    fn test_function(name: &str) -> ResponsesApiTool {
        ResponsesApiTool {
            name: name.to_string(),
            description: format!("{name} description"),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(BTreeMap::new(), None, Some(false.into())),
            output_schema: None,
        }
    }

    #[test]
    fn converts_function_and_apply_patch_tools() {
        let tools = vec![
            ToolSpec::Function(test_function("shell")),
            ToolSpec::Freeform(FreeformTool {
                name: "apply_patch".to_string(),
                description: "freeform apply_patch".to_string(),
                format: FreeformToolFormat {
                    r#type: "grammar".to_string(),
                    syntax: "lark".to_string(),
                    definition: "start: /.+/".to_string(),
                },
            }),
        ];

        let got = create_tools_json_for_chat_completions_api(&tools).expect("tools should encode");

        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["type"], "function");
        assert_eq!(got[0]["function"]["name"], "shell");
        assert_eq!(got[1]["type"], "function");
        assert_eq!(got[1]["function"]["name"], "apply_patch");
    }

    #[test]
    fn drops_unsupported_tools_including_namespaces() {
        let tools = vec![
            ToolSpec::Namespace(crate::ResponsesApiNamespace {
                name: "mcp__calendar__".to_string(),
                description: "calendar".to_string(),
                tools: vec![ResponsesApiNamespaceTool::Function(test_function("create"))],
            }),
            ToolSpec::LocalShell {},
            ToolSpec::Freeform(FreeformTool {
                name: "js_repl".to_string(),
                description: "freeform js".to_string(),
                format: FreeformToolFormat {
                    r#type: "grammar".to_string(),
                    syntax: "lark".to_string(),
                    definition: "start: /.+/".to_string(),
                },
            }),
        ];

        let got = create_tools_json_for_chat_completions_api(&tools).expect("tools should encode");

        assert_eq!(got, Vec::<Value>::new());
    }
}
