use crate::ToolName;
use crate::ToolSpec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodeModeToolKind {
    Function,
    Freeform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeModeToolDefinition {
    pub name: String,
    pub tool_name: ToolName,
    pub description: String,
    pub kind: CodeModeToolKind,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeModeToolNamespaceDescription {
    pub name: String,
    pub description: String,
}

/// Leaves tool descriptions unchanged when code-mode support is not compiled in.
pub fn augment_tool_spec_for_code_mode(spec: ToolSpec) -> ToolSpec {
    spec
}

pub fn tool_spec_to_code_mode_tool_definition(_spec: &ToolSpec) -> Option<CodeModeToolDefinition> {
    None
}

pub fn collect_code_mode_tool_definitions<'a>(
    _specs: impl IntoIterator<Item = &'a ToolSpec>,
) -> Vec<CodeModeToolDefinition> {
    Vec::new()
}

pub fn collect_code_mode_exec_prompt_tool_definitions<'a>(
    _specs: impl IntoIterator<Item = &'a ToolSpec>,
) -> Vec<CodeModeToolDefinition> {
    Vec::new()
}

pub fn code_mode_name_for_tool_name(tool_name: &ToolName) -> String {
    match tool_name.namespace.as_deref() {
        Some(namespace) if namespace.ends_with('_') || tool_name.name.starts_with('_') => {
            format!("{namespace}{}", tool_name.name)
        }
        Some(namespace) => format!("{namespace}_{}", tool_name.name),
        None => tool_name.name.clone(),
    }
}

pub fn is_code_mode_nested_tool(tool_name: &str) -> bool {
    tool_name != CODE_MODE_EXEC_TOOL_NAME && tool_name != CODE_MODE_WAIT_TOOL_NAME
}

pub const CODE_MODE_EXEC_TOOL_NAME: &str = "exec";
pub const CODE_MODE_WAIT_TOOL_NAME: &str = "wait";
