//! Parent-side lifecycle for the limited same-executable configuration helper.

use core::{error::Error as ErrorTrait, fmt, time::Duration};
use std::{path::Path, process::Stdio};

use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    process::{Child, Command},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::{Config, MAX_SOURCE_BYTES, helper};

/// Wall time for the complete exchange, including pipe writes and process exit.
pub const EVALUATION_TIMEOUT: Duration = Duration::from_secs(2);

/// Redacted failures: no source, evaluator diagnostics or configuration values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EvaluationError {
    /// Cancellation was requested; the child was killed and waited for.
    Cancelled,
    /// The internal response could not be decoded.
    InvalidResponse,
    /// Spawning requires an absolute path to this installed executable.
    RelativeExecutable,
    /// Evaluation or schema validation rejected the source.
    Rejected,
    /// Source exceeded the documented input limit.
    SourceTooLarge,
    /// The operating system could not start the helper.
    Spawn,
    /// The helper exceeded the wall-time budget and was killed and waited for.
    TimedOut,
    /// Pipe I/O or process exit failed.
    Transport,
}

impl fmt::Display for EvaluationError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match *self {
            Self::Cancelled => "configuration evaluation cancelled",
            Self::InvalidResponse => "invalid configuration helper response",
            Self::RelativeExecutable => "configuration helper requires an absolute executable path",
            Self::Rejected => "invalid configuration expression or schema",
            Self::SourceTooLarge => "configuration exceeds the source size limit",
            Self::Spawn => "unable to start configuration helper",
            Self::TimedOut => "configuration evaluation exceeded its time limit",
            Self::Transport => "configuration helper failed",
        })
    }
}

impl ErrorTrait for EvaluationError {}

/// Evaluate native Nix in the limited helper without blocking the async runtime.
///
/// `executable` must be this application's absolute executable path. The child
/// receives only the internal flag, never source text in arguments. Stderr is
/// discarded; stdout is bounded internal IPC. Explicit cancellation, timeouts and
/// transport errors all kill and reap the child before returning. Aborting the
/// calling task uses Tokio's kill-on-drop fallback; prefer the cancellation token
/// so cleanup can be awaited. The caller must keep Tokio's process driver alive.
///
/// # Errors
/// Returns a redacted error for oversized input, cancellation, timeout, process
/// failure, invalid IPC or rejected configuration. No host Nix is invoked.
#[inline]
pub async fn evaluate(
    executable: &Path,
    source: &str,
    cancellation: &CancellationToken,
) -> Result<Config, EvaluationError> {
    match run(
        executable,
        "--internal-evaluate-config",
        source,
        cancellation,
    )
    .await?
    {
        helper::Response::Success(config) if config.validate().is_empty() => Ok(*config),
        helper::Response::Success(_) | helper::Response::Rejected => Err(EvaluationError::Rejected),
        helper::Response::Raw(_) => Err(EvaluationError::InvalidResponse),
    }
}

/// Evaluate native Nix to plain data, before schema defaults are applied, in the same
/// limited helper. Reconciling declarations needs to see what was actually declared.
///
/// # Errors
/// As [`evaluate`], except that no schema validation has happened yet.
pub async fn evaluate_raw(
    executable: &Path,
    source: &str,
    cancellation: &CancellationToken,
) -> Result<serde_json::Value, EvaluationError> {
    match run(executable, "--internal-evaluate-raw", source, cancellation).await? {
        helper::Response::Raw(value) => Ok(value),
        helper::Response::Rejected => Err(EvaluationError::Rejected),
        helper::Response::Success(_) => Err(EvaluationError::InvalidResponse),
    }
}

async fn run(
    executable: &Path,
    flag: &str,
    source: &str,
    cancellation: &CancellationToken,
) -> Result<helper::Response, EvaluationError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(EvaluationError::SourceTooLarge);
    }
    if !executable.is_absolute() {
        return Err(EvaluationError::RelativeExecutable);
    }
    if cancellation.is_cancelled() {
        return Err(EvaluationError::Cancelled);
    }
    let mut child = Command::new(executable)
        .arg(flag)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| EvaluationError::Spawn)?;
    let result = cancellation
        .run_until_cancelled(time::timeout(
            EVALUATION_TIMEOUT,
            exchange(&mut child, source),
        ))
        .await
        .map_or(Err(EvaluationError::Cancelled), |response| {
            response.unwrap_or(Err(EvaluationError::TimedOut))
        });
    if result.is_err() {
        // kill() also waits on Unix. An already reaped child is harmless here.
        let _cleanup = child.kill().await;
        let _reaped = child.wait().await;
    }
    result
}

/// Exchange bounded data concurrently so neither pipe can deadlock the parent.
async fn exchange(child: &mut Child, source: &str) -> Result<helper::Response, EvaluationError> {
    let mut input = child.stdin.take().ok_or(EvaluationError::Transport)?;
    let output = child.stdout.take().ok_or(EvaluationError::Transport)?;
    let limit = u64::try_from(helper::MAX_RESPONSE_BYTES)
        .map_err(|_| EvaluationError::Transport)?
        .saturating_add(1);
    let write = async move {
        input
            .write_all(source.as_bytes())
            .await
            .map_err(|_| EvaluationError::Transport)?;
        input
            .shutdown()
            .await
            .map_err(|_| EvaluationError::Transport)?;
        drop(input);
        Ok(())
    };
    let read = async move {
        let mut bytes = Vec::new();
        output
            .take(limit)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| EvaluationError::Transport)?;
        if bytes.len() > helper::MAX_RESPONSE_BYTES {
            return Err(EvaluationError::InvalidResponse);
        }
        Ok(bytes)
    };
    let wait = async {
        let status = child.wait().await.map_err(|_| EvaluationError::Transport)?;
        if status.success() {
            Ok(())
        } else {
            Err(EvaluationError::Transport)
        }
    };
    let ((), bytes, ()) = tokio::try_join!(write, read, wait)?;
    serde_json::from_slice(&bytes).map_err(|_| EvaluationError::InvalidResponse)
}
