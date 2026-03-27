use async_trait::async_trait;
use codex_loop::LoopContextMode;
use codex_loop::LoopResponseMode;
use codex_loop::LoopSecurityMode;
use codex_loop_runtime::CreateLoopRequest;
use codex_loop_runtime::CreateLoopResult;
use codex_loop_runtime::CreateLoopServiceError;
use codex_loop_runtime::CreateLoopTriggerRequest;
use codex_loop_runtime::DeleteLoopResult;
use codex_loop_runtime::LoopInfo;
use codex_loop_runtime::LoopSummary;
use codex_loop_runtime::UpdateLoopRequest;
use codex_loop_runtime::create_loop;
use codex_loop_runtime::delete_loop;
use codex_loop_runtime::get_loop;
use codex_loop_runtime::list_loops;
use codex_loop_runtime::update_loop;
use serde::Deserialize;
use serde::Serialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct LoopToolHandler;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LoopToolOp {
    Create,
    List,
    Info,
    Update,
    Delete,
}

#[derive(Debug, Deserialize)]
struct LoopToolArgs {
    op: LoopToolOp,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    create: Option<CreateLoopRequest>,
    #[serde(default)]
    update: Option<LoopToolUpdateArgs>,
}

#[derive(Debug, Deserialize)]
struct LoopToolUpdateArgs {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    action: Option<Option<String>>,
    #[serde(default)]
    context_mode: Option<LoopContextMode>,
    #[serde(default)]
    response_mode: Option<LoopResponseMode>,
    #[serde(default)]
    security_mode: Option<LoopSecurityMode>,
    #[serde(default)]
    cwd: Option<Option<String>>,
    #[serde(default)]
    writable_roots: Option<Vec<String>>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    trigger_bindings: Option<Vec<CreateLoopTriggerRequest>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum LoopToolResult {
    Create { created: CreateLoopResult },
    List { loops: Vec<LoopSummary> },
    Info { info: LoopInfo },
    Update { updated: LoopInfo },
    Delete { deleted: DeleteLoopResult },
}

#[async_trait]
impl ToolHandler for LoopToolHandler {
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
                    "loop handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: LoopToolArgs = parse_arguments(&arguments)?;
        let result = match args.op {
            LoopToolOp::Create => {
                let Some(create_request) = args.create else {
                    return Err(FunctionCallError::RespondToModel(
                        "loop op=create requires the create field".to_string(),
                    ));
                };
                LoopToolResult::Create {
                    created: create_loop(create_request, turn.cwd.as_path()).map_err(loop_error)?,
                }
            }
            LoopToolOp::List => LoopToolResult::List {
                loops: list_loops(turn.cwd.as_path()).map_err(loop_error)?,
            },
            LoopToolOp::Info => {
                let id = required_loop_id(args.id, "info")?;
                LoopToolResult::Info {
                    info: get_loop(&id, turn.cwd.as_path()).map_err(loop_error)?,
                }
            }
            LoopToolOp::Update => {
                let id = required_loop_id(args.id, "update")?;
                let Some(update) = args.update else {
                    return Err(FunctionCallError::RespondToModel(
                        "loop op=update requires the update field".to_string(),
                    ));
                };
                LoopToolResult::Update {
                    updated: update_loop(
                        UpdateLoopRequest {
                            id,
                            prompt: update.prompt,
                            action: update.action,
                            context_mode: update.context_mode,
                            response_mode: update.response_mode,
                            security_mode: update.security_mode,
                            cwd: update.cwd,
                            writable_roots: update.writable_roots,
                            enabled: update.enabled,
                            trigger_bindings: update.trigger_bindings,
                        },
                        turn.cwd.as_path(),
                    )
                    .map_err(loop_error)?,
                }
            }
            LoopToolOp::Delete => {
                let id = required_loop_id(args.id, "delete")?;
                LoopToolResult::Delete {
                    deleted: delete_loop(&id, turn.cwd.as_path()).map_err(loop_error)?,
                }
            }
        };

        let content = serde_json::to_string(&result).map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize loop tool result: {err}"))
        })?;
        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

fn required_loop_id(id: Option<String>, op_name: &str) -> Result<String, FunctionCallError> {
    let id = id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(format!("loop op={op_name} requires the id field"))
        })?;
    Ok(id.to_string())
}

fn loop_error(err: CreateLoopServiceError) -> FunctionCallError {
    FunctionCallError::RespondToModel(err.to_string())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::CreateLoopTriggerRequest;
    use super::LoopToolUpdateArgs;

    #[test]
    fn update_request_supports_clearing_optional_fields() {
        let update = serde_json::from_value::<LoopToolUpdateArgs>(serde_json::json!({
            "action": null,
            "cwd": null,
            "enabled": true,
            "trigger_bindings": [
                {
                    "kind": "timer",
                    "schedule": "10m"
                }
            ]
        }))
        .expect("deserialize update");

        assert_eq!(update.action, Some(None));
        assert_eq!(update.cwd, Some(None));
        assert_eq!(update.enabled, Some(true));
        assert_eq!(
            update.trigger_bindings,
            Some(vec![CreateLoopTriggerRequest::Timer {
                schedule: "10m".to_string()
            }])
        );
    }
}
