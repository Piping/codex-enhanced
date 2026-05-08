use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_tools::ToolName;
use serde::Deserialize;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::SystemTime;
use tokio::process::Command;

pub struct GrepFilesHandler;

const DEFAULT_LIMIT: usize = 100;

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

#[derive(Debug, Deserialize)]
struct GrepFilesArgs {
    pattern: String,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

impl ToolHandler for GrepFilesHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("grep_files")
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { payload, turn, .. } = invocation;
        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "grep_files handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: GrepFilesArgs = parse_arguments(&arguments)?;
        if args.limit == 0 {
            return Err(FunctionCallError::RespondToModel(
                "limit must be greater than zero".to_string(),
            ));
        }

        let search_root = args
            .path
            .map(PathBuf::from)
            .map(|path| crate::util::resolve_path(turn.cwd.as_path(), &path))
            .unwrap_or_else(|| turn.cwd.to_path_buf());

        let matches = run_rg_search(
            &args.pattern,
            args.include.as_deref(),
            search_root.as_path(),
            args.limit,
            turn.cwd.as_path(),
        )
        .await?;

        let output = if matches.is_empty() {
            "No matching files found".to_string()
        } else {
            matches.join("\n")
        };
        Ok(FunctionToolOutput::from_text(output, Some(true)))
    }
}

async fn run_rg_search(
    pattern: &str,
    include: Option<&str>,
    path: &Path,
    limit: usize,
    cwd: &Path,
) -> Result<Vec<String>, FunctionCallError> {
    let mut command = Command::new("rg");
    command
        .arg("--files-with-matches")
        .arg("--no-messages")
        .arg("--color")
        .arg("never")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(cwd);
    if let Some(include) = include {
        command.arg("--glob").arg(include);
    }
    command.arg(pattern).arg(path);

    let output = command
        .output()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("failed to run rg: {err}")))?;

    match output.status.code() {
        Some(0) => {
            let mut results = parse_results(&output.stdout, usize::MAX)
                .into_iter()
                .map(|result| crate::util::resolve_path(cwd, &PathBuf::from(result)))
                .collect::<Vec<_>>();
            results.sort_by(|left, right| {
                let left_modified = modified_time(left.as_path());
                let right_modified = modified_time(right.as_path());
                right_modified
                    .cmp(&left_modified)
                    .then_with(|| left.cmp(right))
            });
            results.truncate(limit);
            Ok(results
                .into_iter()
                .map(|path| path.display().to_string())
                .collect())
        }
        Some(1) => Ok(Vec::new()),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let details = if stderr.is_empty() {
                format!("rg exited with status {}", output.status)
            } else {
                stderr
            };
            Err(FunctionCallError::RespondToModel(format!(
                "grep_files search failed: {details}"
            )))
        }
    }
}

fn modified_time(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn parse_results(stdout: &[u8], limit: usize) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .take(limit)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[path = "grep_files_tests.rs"]
mod tests;
