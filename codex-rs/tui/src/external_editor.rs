use std::env;
use std::fs;
use std::path::Path;
use std::process::Stdio;

use color_eyre::eyre::Report;
use color_eyre::eyre::Result;
use tempfile::Builder;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub(crate) enum EditorError {
    #[error("no usable editor found in VISUAL, EDITOR, or vim")]
    MissingEditor,
    #[cfg(not(windows))]
    #[error("failed to parse editor command")]
    ParseFailed,
    #[error("editor command is empty")]
    EmptyCommand,
}

/// Tries to resolve the full path to a Windows program, respecting PATH + PATHEXT.
/// Falls back to the original program name if resolution fails.
#[cfg(windows)]
fn resolve_windows_program(program: &str) -> std::path::PathBuf {
    // On Windows, `Command::new("code")` will not resolve `code.cmd` shims on PATH.
    // Use `which` so we respect PATH + PATHEXT (e.g., `code` -> `code.cmd`).
    which::which(program).unwrap_or_else(|_| std::path::PathBuf::from(program))
}

/// Resolve editor commands from environment variables, then fall back to `vim`.
/// Prefers `VISUAL` over `EDITOR`, but will try each configured editor before `vim`.
pub(crate) fn resolve_editor_commands() -> std::result::Result<Vec<Vec<String>>, EditorError> {
    let mut commands = Vec::new();
    let mut last_error = None;

    for key in ["VISUAL", "EDITOR"] {
        let Ok(raw) = env::var(key) else {
            continue;
        };
        let parts = {
            #[cfg(windows)]
            {
                winsplit::split(&raw)
            }
            #[cfg(not(windows))]
            {
                match shlex::split(&raw) {
                    Some(parts) => parts,
                    None => {
                        last_error = Some(EditorError::ParseFailed);
                        continue;
                    }
                }
            }
        };
        if parts.is_empty() {
            last_error = Some(EditorError::EmptyCommand);
            continue;
        }
        if !commands.contains(&parts) {
            commands.push(parts);
        }
    }

    let vim = vec!["vim".to_string()];
    if !commands.contains(&vim) {
        commands.push(vim);
    }

    if commands.is_empty() {
        return Err(last_error.unwrap_or(EditorError::MissingEditor));
    }

    Ok(commands)
}

/// Write `seed` to a temp file, launch an editor, and return the updated content.
pub(crate) async fn run_editor(seed: &str, editor_cmds: &[Vec<String>]) -> Result<String> {
    run_editor_with_suffix(seed, editor_cmds, ".md").await
}

/// Write `seed` to a temp file with a custom suffix, launch an editor,
/// and return the updated content.
pub(crate) async fn run_editor_with_suffix(
    seed: &str,
    editor_cmds: &[Vec<String>],
    suffix: &str,
) -> Result<String> {
    if editor_cmds.is_empty() {
        return Err(Report::msg("editor command list is empty"));
    }

    // Convert to TempPath immediately so no file handle stays open on Windows.
    let temp_path = Builder::new().suffix(suffix).tempfile()?.into_temp_path();
    fs::write(&temp_path, seed)?;
    launch_editor_for_path(&temp_path, editor_cmds).await?;
    let contents = fs::read_to_string(&temp_path)?;
    Ok(contents)
}

/// Launch the editor against an existing file path.
pub(crate) async fn edit_file(path: &Path, editor_cmds: &[Vec<String>]) -> Result<()> {
    if editor_cmds.is_empty() {
        return Err(Report::msg("editor command list is empty"));
    }
    launch_editor_for_path(path, editor_cmds).await
}

async fn launch_editor_for_path(path: &Path, editor_cmds: &[Vec<String>]) -> Result<()> {
    let mut failures = Vec::new();

    for editor_cmd in editor_cmds {
        if editor_cmd.is_empty() {
            failures.push("editor command is empty".to_string());
            continue;
        }

        let mut cmd = {
            #[cfg(windows)]
            {
                // handles .cmd/.bat shims
                Command::new(resolve_windows_program(&editor_cmd[0]))
            }
            #[cfg(not(windows))]
            {
                Command::new(&editor_cmd[0])
            }
        };
        if editor_cmd.len() > 1 {
            cmd.args(&editor_cmd[1..]);
        }
        let command = shlex::try_join(editor_cmd.iter().map(String::as_str))
            .unwrap_or_else(|_| editor_cmd.join(" "));
        match cmd
            .arg(path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => failures.push(format!("`{command}` exited with status {status}")),
            Err(err) => failures.push(format!("`{command}`: {err}")),
        }
    }

    Err(Report::msg(format!(
        "failed to open any editor: {}",
        failures.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serial_test::serial;
    use tempfile::tempdir;

    struct EnvGuard {
        visual: Option<String>,
        editor: Option<String>,
        path: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self {
                visual: env::var("VISUAL").ok(),
                editor: env::var("EDITOR").ok(),
                path: env::var("PATH").ok(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            restore_env("VISUAL", self.visual.take());
            restore_env("EDITOR", self.editor.take());
            restore_env("PATH", self.path.take());
        }
    }

    fn restore_env(key: &str, value: Option<String>) {
        match value {
            Some(val) => unsafe { env::set_var(key, val) },
            None => unsafe { env::remove_var(key) },
        }
    }

    #[test]
    #[serial]
    fn resolve_editor_prefers_visual() {
        let _guard = EnvGuard::new();
        unsafe {
            env::set_var("VISUAL", "vis");
            env::set_var("EDITOR", "ed");
        }
        let commands = resolve_editor_commands().unwrap();
        assert_eq!(commands[0], vec!["vis".to_string()]);
    }

    #[test]
    #[serial]
    fn resolve_editor_falls_back_to_vim_when_unset() {
        let _guard = EnvGuard::new();
        unsafe {
            env::remove_var("VISUAL");
            env::remove_var("EDITOR");
        }
        let commands = resolve_editor_commands().unwrap();
        assert_eq!(commands, vec![vec!["vim".to_string()]]);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_editor_returns_updated_content() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let script_path = dir.path().join("edit.sh");
        fs::write(&script_path, "#!/bin/sh\nprintf \"edited\" > \"$1\"\n").unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();

        let commands = vec![vec![script_path.to_string_lossy().to_string()]];
        let result = run_editor("seed", &commands).await.unwrap();
        assert_eq!(result, "edited".to_string());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn edit_file_updates_existing_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let script_path = dir.path().join("edit.sh");
        fs::write(&script_path, "#!/bin/sh\nprintf \"updated\" > \"$1\"\n").unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();

        let file_path = dir.path().join("workflow.yaml");
        fs::write(&file_path, "seed").unwrap();

        let commands = vec![vec![script_path.to_string_lossy().to_string()]];
        edit_file(&file_path, &commands).await.unwrap();
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "updated");
    }

    #[tokio::test]
    #[cfg(unix)]
    #[serial]
    async fn run_editor_falls_back_when_primary_editor_fails() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = EnvGuard::new();
        let dir = tempdir().unwrap();

        let failing_editor = dir.path().join("broken-editor.sh");
        fs::write(&failing_editor, "#!/bin/sh\nexit 1\n").unwrap();
        let mut perms = fs::metadata(&failing_editor).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&failing_editor, perms).unwrap();

        let vim_path = dir.path().join("vim");
        fs::write(&vim_path, "#!/bin/sh\nprintf \"edited\" > \"$1\"\n").unwrap();
        let mut perms = fs::metadata(&vim_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&vim_path, perms).unwrap();

        unsafe {
            env::set_var("VISUAL", failing_editor.as_os_str());
            env::remove_var("EDITOR");
            env::set_var("PATH", dir.path());
        }

        let commands = resolve_editor_commands().unwrap();
        let result = run_editor("seed", &commands).await.unwrap();
        assert_eq!(result, "edited");
    }
}
