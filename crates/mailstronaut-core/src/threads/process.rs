//! Cancellable and reaped parent-side lifecycle for the RFC grouping helper.

use core::time::Duration;
use std::{path::Path, process::Stdio};

use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

use super::{Error, Groups, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, Message};

/// Wall-time ceiling for input/output and child exit, in addition to CPU limits.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Project metadata in this application's bounded child, never on the SQLite/UI owner.
///
/// # Errors
/// Returns static errors; cancellation/timeout/failure kill and reap the child.
/// Task abortion retains Tokio's kill-on-drop fallback; explicit cancellation
/// should be preferred while the Tokio process driver remains alive.
pub async fn project(
    executable: &Path,
    messages: &[Message],
    cancel: &CancellationToken,
) -> Result<Groups, Error> {
    super::validate(messages)?;
    if !executable.is_absolute() {
        return Err(Error::Invalid);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let input = serde_json::to_vec(messages).map_err(|_| Error::Invalid)?;
    if input.len() > MAX_INPUT_BYTES {
        return Err(Error::TooLarge);
    }
    let mut child = Command::new(executable)
        .arg("--internal-project-threads")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Error::Unavailable)?;
    let result = cancel
        .run_until_cancelled(tokio::time::timeout(TIMEOUT, exchange(&mut child, &input)))
        .await
        .map_or(Err(Error::Cancelled), |result| {
            result.unwrap_or(Err(Error::TimedOut))
        });
    let result = result.and_then(|groups| {
        groups.validate(messages)?;
        Ok(groups)
    });
    if result.is_err() {
        let _killed = child.kill().await;
        let _reaped = child.wait().await;
    }
    result
}
async fn exchange(child: &mut Child, input: &[u8]) -> Result<Groups, Error> {
    let mut stdin = child.stdin.take().ok_or(Error::Unavailable)?;
    let stdout = child.stdout.take().ok_or(Error::Unavailable)?;
    let write = async {
        stdin
            .write_all(input)
            .await
            .map_err(|_| Error::Unavailable)?;
        stdin.shutdown().await.map_err(|_| Error::Unavailable)?;
        drop(stdin);
        Ok(())
    };
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(u64::try_from(MAX_OUTPUT_BYTES + 1).map_err(|_| Error::Invalid)?)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Error::Unavailable)?;
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err(Error::TooLarge);
        }
        serde_json::from_slice::<Groups>(&bytes).map_err(|_| Error::Invalid)
    };
    let wait = async {
        if child
            .wait()
            .await
            .map_err(|_| Error::Unavailable)?
            .success()
        {
            Ok(())
        } else {
            Err(Error::Unavailable)
        }
    };
    let ((), groups, ()) = tokio::try_join!(write, read, wait)?;
    Ok(groups)
}
