use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSpawnRequest {
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSpawnResolution {
    pub cwd: PathBuf,
    pub inherited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSpawnError {
    EmptyPath,
    MissingDirectory(PathBuf),
    NotADirectory(PathBuf),
}

impl fmt::Display for WorkspaceSpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "workspace path cannot be empty"),
            Self::MissingDirectory(path) => {
                write!(f, "workspace {} does not exist", path.display())
            }
            Self::NotADirectory(path) => {
                write!(f, "workspace {} is not a directory", path.display())
            }
        }
    }
}

impl std::error::Error for WorkspaceSpawnError {}

pub fn resolve_workspace_spawn(
    parent_cwd: &Path,
    request: &WorkspaceSpawnRequest,
) -> Result<WorkspaceSpawnResolution, WorkspaceSpawnError> {
    let Some(requested_cwd) = request.cwd.as_deref() else {
        return Ok(WorkspaceSpawnResolution {
            cwd: parent_cwd.to_path_buf(),
            inherited: true,
        });
    };

    let requested_cwd = requested_cwd.trim();
    if requested_cwd.is_empty() {
        return Err(WorkspaceSpawnError::EmptyPath);
    }

    let resolved = PathBuf::from(requested_cwd);
    let resolved = if resolved.is_absolute() {
        resolved
    } else {
        parent_cwd.join(resolved)
    };

    if !resolved.exists() {
        return Err(WorkspaceSpawnError::MissingDirectory(resolved));
    }
    if !resolved.is_dir() {
        return Err(WorkspaceSpawnError::NotADirectory(resolved));
    }

    Ok(WorkspaceSpawnResolution {
        cwd: resolved,
        inherited: false,
    })
}

#[cfg(test)]
mod tests {
    use super::WorkspaceSpawnError;
    use super::WorkspaceSpawnRequest;
    use super::resolve_workspace_spawn;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn resolve_workspace_spawn_inherits_parent_cwd_by_default() {
        let tempdir = tempdir().expect("tempdir");

        let resolution =
            resolve_workspace_spawn(tempdir.path(), &WorkspaceSpawnRequest { cwd: None })
                .expect("resolution");

        assert_eq!(resolution.cwd, tempdir.path());
        assert!(resolution.inherited);
    }

    #[test]
    fn resolve_workspace_spawn_resolves_relative_path() {
        let tempdir = tempdir().expect("tempdir");
        let child = tempdir.path().join("worker-a");
        std::fs::create_dir(&child).expect("create dir");

        let resolution = resolve_workspace_spawn(
            tempdir.path(),
            &WorkspaceSpawnRequest {
                cwd: Some("worker-a".to_string()),
            },
        )
        .expect("resolution");

        assert_eq!(resolution.cwd, child);
        assert!(!resolution.inherited);
    }

    #[test]
    fn resolve_workspace_spawn_rejects_missing_directory() {
        let tempdir = tempdir().expect("tempdir");

        let err = resolve_workspace_spawn(
            tempdir.path(),
            &WorkspaceSpawnRequest {
                cwd: Some("missing".to_string()),
            },
        )
        .expect_err("missing path should fail");

        assert_eq!(
            err,
            WorkspaceSpawnError::MissingDirectory(tempdir.path().join("missing"))
        );
    }
}
