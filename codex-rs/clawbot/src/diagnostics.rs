use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use serde::Serialize;

use crate::model::CLAWBOT_DIAGNOSTICS_RELATIVE_PATH;

#[derive(Debug, Serialize)]
struct DiagnosticEvent<T> {
    ts_ms: i64,
    kind: String,
    payload: T,
}

pub fn append_diagnostic_event<T>(workspace_root: &Path, kind: &str, payload: T) -> Result<()>
where
    T: Serialize,
{
    let path = workspace_root.join(CLAWBOT_DIAGNOSTICS_RELATIVE_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let event = DiagnosticEvent {
        ts_ms: unix_timestamp_ms_now()?,
        kind: kind.to_string(),
        payload,
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::to_writer(&mut file, &event)
        .context("failed to encode clawbot diagnostic event")?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to append {}", path.display()))
}

fn unix_timestamp_ms_now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_millis() as i64)
}
