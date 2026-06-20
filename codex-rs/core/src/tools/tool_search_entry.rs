use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tools::LoadableToolSpec;
use codex_tools::ToolsConfig;
use codex_tools::dynamic_tool_to_loadable_tool_spec;

#[derive(Clone)]
pub(crate) struct ToolSearchEntry {
    pub(crate) search_text: String,
    pub(crate) output: LoadableToolSpec,
    pub(crate) limit_bucket: Option<String>,
}

pub(crate) fn build_tool_search_entries(dynamic_tools: &[DynamicToolSpec]) -> Vec<ToolSearchEntry> {
    let mut entries = Vec::new();

    let mut dynamic_tools = dynamic_tools.iter().collect::<Vec<_>>();
    dynamic_tools.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    for tool in dynamic_tools {
        match dynamic_tool_search_entry(tool) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                tracing::error!(
                    "Failed to convert deferred dynamic tool {:?} to OpenAI tool: {error:?}",
                    tool.name
                );
            }
        }
    }

    entries
}

pub(crate) fn build_tool_search_entries_for_config(
    config: &ToolsConfig,
    dynamic_tools: &[DynamicToolSpec],
) -> Vec<ToolSearchEntry> {
    let dynamic_tools = dynamic_tools
        .iter()
        .filter(|tool| config.namespace_tools || tool.namespace.is_none())
        .cloned()
        .collect::<Vec<_>>();
    build_tool_search_entries(&dynamic_tools)
}

fn dynamic_tool_search_entry(tool: &DynamicToolSpec) -> Result<ToolSearchEntry, serde_json::Error> {
    Ok(ToolSearchEntry {
        search_text: build_dynamic_search_text(tool),
        output: dynamic_tool_to_loadable_tool_spec(tool)?,
        limit_bucket: None,
    })
}

fn build_dynamic_search_text(tool: &DynamicToolSpec) -> String {
    let mut parts = vec![
        tool.name.clone(),
        tool.name.replace('_', " "),
        tool.description.clone(),
    ];

    if let Some(namespace) = &tool.namespace {
        parts.push(namespace.clone());
    }

    parts.extend(
        tool.input_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .map(|map| map.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    );

    parts.join(" ")
}
