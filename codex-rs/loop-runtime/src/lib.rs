use codex_loop::LoopContextMode;
use codex_loop::LoopSecurityMode;
use codex_loop::PersistedLoopExecutionSettings;
use codex_protocol::protocol::ReadOnlyAccess;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::Path;

#[derive(Debug)]
pub struct LoopRuntimeOverrides {
    pub cwd: Option<AbsolutePathBuf>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub developer_instructions: String,
}

pub fn build_loop_runtime_overrides(
    security_mode: LoopSecurityMode,
    settings: &PersistedLoopExecutionSettings,
    workspace_cwd: &Path,
    inherited_network_access: bool,
) -> Result<LoopRuntimeOverrides, String> {
    let cwd = settings
        .cwd
        .as_ref()
        .map(|cwd| resolve_absolute_path(cwd, workspace_cwd))
        .transpose()?;

    let sandbox_policy = if matches!(security_mode, LoopSecurityMode::SpecifiedDirectory) {
        if settings.writable_roots.is_empty() {
            return Err(
                "Loop security mode `specified_directory` requires at least one writable directory."
                    .to_string(),
            );
        }

        let writable_roots = settings
            .writable_roots
            .iter()
            .map(|path| resolve_absolute_path(path, workspace_cwd))
            .collect::<Result<Vec<_>, _>>()?;

        Some(SandboxPolicy::WorkspaceWrite {
            writable_roots,
            read_only_access: ReadOnlyAccess::FullAccess,
            network_access: inherited_network_access,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        })
    } else {
        None
    };

    Ok(LoopRuntimeOverrides {
        cwd,
        sandbox_policy,
        developer_instructions: loop_developer_instructions(security_mode, settings),
    })
}

pub fn build_loop_phase_input(
    context_mode: LoopContextMode,
    prompt: &str,
    recent_main_messages: &[String],
    current_user_turn: Option<&str>,
    last_assistant_message: Option<&str>,
) -> String {
    let mut sections = Vec::new();
    if matches!(
        context_mode,
        LoopContextMode::Ephemeral | LoopContextMode::Persistent
    ) && !recent_main_messages.is_empty()
    {
        sections.push(format!(
            "Recent main-thread messages:\n{}",
            recent_main_messages.join("\n\n")
        ));
    }
    if let Some(current_user_turn) = current_user_turn.filter(|text| !text.trim().is_empty()) {
        sections.push(format!(
            "Current main-thread user turn:\n{current_user_turn}"
        ));
    }
    if let Some(last_assistant_message) =
        last_assistant_message.filter(|text| !text.trim().is_empty())
    {
        sections.push(format!(
            "Latest main-thread assistant response:\n{last_assistant_message}"
        ));
    }
    sections.push(format!("Original loop prompt:\n{prompt}"));
    sections.join("\n\n")
}

fn loop_developer_instructions(
    security_mode: LoopSecurityMode,
    settings: &PersistedLoopExecutionSettings,
) -> String {
    let mut parts = vec![
        "This is a hidden `/loop` execution thread.".to_string(),
        "Use the current main-thread context only as background.".to_string(),
        "Keep work scoped to this scheduled task.".to_string(),
    ];
    if let Some(cwd) = &settings.cwd {
        parts.push(format!(
            "Use `{}` as the execution working directory.",
            cwd.display()
        ));
    } else {
        parts.push(
            "Use the same working directory as the parent thread unless the runtime overrides it."
                .to_string(),
        );
    }
    match security_mode {
        LoopSecurityMode::Inherited => {
            parts.push(
                "Use the same permissions and tool access as the parent thread unless the runtime overrides them."
                    .to_string(),
            );
        }
        LoopSecurityMode::SpecifiedDirectory => {
            let writable_roots = settings
                .writable_roots
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "Only write files inside these directories: {writable_roots}."
            ));
        }
    }
    parts.join(" ")
}

fn resolve_absolute_path(path: &Path, workspace_cwd: &Path) -> Result<AbsolutePathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_cwd.join(path)
    };
    let absolute = std::fs::canonicalize(&absolute)
        .map_err(|err| format!("Path `{}` is unavailable: {err}", absolute.display()))?;
    AbsolutePathBuf::from_absolute_path(absolute.clone())
        .map_err(|err| format!("Invalid path `{}`: {err}", absolute.display()))
}

#[cfg(test)]
mod tests {
    use super::LoopRuntimeOverrides;
    use super::build_loop_phase_input;
    use super::build_loop_runtime_overrides;
    use codex_loop::LoopContextMode;
    use codex_loop::LoopSecurityMode;
    use codex_loop::PersistedLoopExecutionSettings;
    use codex_protocol::protocol::ReadOnlyAccess;
    use codex_protocol::protocol::SandboxPolicy;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn build_loop_runtime_overrides_requires_writable_roots_for_specified_directory() {
        let workspace = tempdir().expect("tempdir");
        let err = build_loop_runtime_overrides(
            LoopSecurityMode::SpecifiedDirectory,
            &PersistedLoopExecutionSettings::default(),
            workspace.path(),
            false,
        )
        .expect_err("specified directory should require writable roots");

        assert_eq!(
            err,
            "Loop security mode `specified_directory` requires at least one writable directory."
        );
    }

    #[test]
    fn build_loop_runtime_overrides_returns_workspace_write_policy() {
        let workspace = tempdir().expect("tempdir");
        std::fs::create_dir_all(workspace.path().join("src")).expect("mkdir");
        let overrides = build_loop_runtime_overrides(
            LoopSecurityMode::SpecifiedDirectory,
            &PersistedLoopExecutionSettings {
                cwd: Some("src".into()),
                writable_roots: vec!["src".into()],
            },
            workspace.path(),
            true,
        )
        .expect("runtime overrides");

        let expected = LoopRuntimeOverrides {
            cwd: Some(
                codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
                    workspace.path().join("src").canonicalize().expect("canonical cwd"),
                )
                .expect("absolute cwd"),
            ),
            sandbox_policy: Some(SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![
                    codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
                        workspace.path().join("src").canonicalize().expect("canonical root"),
                    )
                    .expect("absolute root"),
                ],
                read_only_access: ReadOnlyAccess::FullAccess,
                network_access: true,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            }),
            developer_instructions:
                "This is a hidden `/loop` execution thread. Use the current main-thread context only as background. Keep work scoped to this scheduled task. Use `src` as the execution working directory. Only write files inside these directories: src.".to_string(),
        };

        assert_eq!(overrides.cwd, expected.cwd);
        assert_eq!(overrides.sandbox_policy, expected.sandbox_policy);
        assert_eq!(
            overrides.developer_instructions,
            expected.developer_instructions
        );
    }

    #[test]
    fn build_loop_phase_input_skips_main_thread_history_for_embed() {
        let input = build_loop_phase_input(
            LoopContextMode::Embed,
            "review progress",
            &["user: hi".to_string(), "assistant: hello".to_string()],
            Some("continue"),
            Some("done"),
        );

        assert_eq!(
            input,
            "Current main-thread user turn:\ncontinue\n\nLatest main-thread assistant response:\ndone\n\nOriginal loop prompt:\nreview progress"
        );
    }
}
