use codex_core::config::Config;
use codex_protocol::protocol::ReadOnlyAccess;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedLoopExecutionSettings {
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub writable_roots: Vec<PathBuf>,
}

pub fn apply_loop_execution_settings(
    config: &mut Config,
    settings: &PersistedLoopExecutionSettings,
    workspace_cwd: &Path,
) -> Result<String, String> {
    let resolved_cwd = settings
        .cwd
        .as_ref()
        .map(|cwd| resolve_absolute_path(cwd, workspace_cwd))
        .transpose()?;
    if let Some(cwd) = resolved_cwd {
        config.cwd = cwd;
    }

    if !settings.writable_roots.is_empty() {
        let writable_roots = resolve_writable_roots(settings, workspace_cwd)?;
        let network_access = config
            .permissions
            .sandbox_policy
            .get()
            .has_full_network_access();
        config
            .permissions
            .sandbox_policy
            .set(SandboxPolicy::WorkspaceWrite {
                writable_roots,
                read_only_access: ReadOnlyAccess::FullAccess,
                network_access,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            })
            .map_err(|err| format!("Failed to configure `/loop` writable roots: {err}"))?;
    }

    Ok(loop_developer_instructions(settings))
}

pub fn loop_execution_summary(
    settings: &PersistedLoopExecutionSettings,
    session_cwd: &Path,
) -> String {
    let cwd = settings
        .cwd
        .as_ref()
        .map(|path| format!("`{}`", path.display()))
        .unwrap_or_else(|| format!("session default (`{}`)", session_cwd.display()));
    let writable_scope = if settings.writable_roots.is_empty() {
        "session default".to_string()
    } else {
        settings
            .writable_roots
            .iter()
            .map(|path| format!("`{}`", path.display()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("CWD: {cwd}. Writable scope: {writable_scope}.")
}

pub fn writable_roots_editor_text(settings: &PersistedLoopExecutionSettings) -> String {
    settings
        .writable_roots
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn cwd_editor_text(settings: &PersistedLoopExecutionSettings, session_cwd: &Path) -> String {
    settings
        .cwd
        .clone()
        .unwrap_or_else(|| session_cwd.to_path_buf())
        .display()
        .to_string()
}

pub fn parse_loop_writable_roots(
    input: &str,
    workspace_cwd: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut seen = BTreeSet::new();
    let mut writable_roots = Vec::new();

    for raw_line in input.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let absolute = resolve_pathbuf(PathBuf::from(trimmed), workspace_cwd)?;
        let metadata = fs::metadata(&absolute)
            .map_err(|err| format!("Writable directory `{trimmed}` is unavailable: {err}"))?;
        if !metadata.is_dir() {
            return Err(format!(
                "Writable directory `{trimmed}` is not a directory."
            ));
        }

        let persisted = normalize_persisted_path(absolute.as_path(), workspace_cwd);
        if seen.insert(persisted.clone()) {
            writable_roots.push(persisted);
        }
    }

    Ok(writable_roots)
}

pub fn parse_loop_cwd(input: &str, workspace_cwd: &Path) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Working directory cannot be empty.".to_string());
    }
    let absolute = resolve_pathbuf(PathBuf::from(trimmed), workspace_cwd)?;
    let metadata = fs::metadata(&absolute)
        .map_err(|err| format!("Working directory `{trimmed}` is unavailable: {err}"))?;
    if !metadata.is_dir() {
        return Err(format!("Working directory `{trimmed}` is not a directory."));
    }
    Ok(normalize_persisted_path(absolute.as_path(), workspace_cwd))
}

fn loop_developer_instructions(settings: &PersistedLoopExecutionSettings) -> String {
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
    if settings.writable_roots.is_empty() {
        parts.push(
            "Use the same permissions and tool access as the parent thread unless the runtime overrides them."
                .to_string(),
        );
    } else {
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
    parts.join(" ")
}

fn resolve_writable_roots(
    settings: &PersistedLoopExecutionSettings,
    workspace_cwd: &Path,
) -> Result<Vec<AbsolutePathBuf>, String> {
    settings
        .writable_roots
        .iter()
        .map(|path| resolve_absolute_path(path, workspace_cwd))
        .collect()
}

fn resolve_absolute_path(path: &Path, workspace_cwd: &Path) -> Result<AbsolutePathBuf, String> {
    let absolute = resolve_pathbuf(path.to_path_buf(), workspace_cwd)?;
    AbsolutePathBuf::from_absolute_path(absolute.clone())
        .map_err(|err| format!("Invalid path `{}`: {err}", absolute.display()))
}

fn resolve_pathbuf(path: PathBuf, workspace_cwd: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path
    } else {
        workspace_cwd.join(path)
    };
    fs::canonicalize(&absolute)
        .map_err(|err| format!("Path `{}` is unavailable: {err}", absolute.display()))
}

fn normalize_persisted_path(path: &Path, workspace_cwd: &Path) -> PathBuf {
    if let Ok(relative) = path.strip_prefix(workspace_cwd) {
        if relative.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            relative.to_path_buf()
        }
    } else {
        path.to_path_buf()
    }
}
