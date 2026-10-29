//! Bounded file loading and atomic adoption of a validated configuration.

use core::{error::Error as StdError, fmt};
use std::{
    fs,
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio_util::sync::CancellationToken;

use crate::{
    Config, MAX_SOURCE_BYTES,
    process::{self, EvaluationError},
};

/// A deliberately redacted loading failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The selected source is not a regular, readable UTF-8 file.
    Read,
    /// The source exceeds the input budget.
    TooLarge,
    /// The limited helper could not produce a valid configuration.
    Evaluation(EvaluationError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => f.write_str("configuration must be a readable regular UTF-8 file"),
            Self::TooLarge => f.write_str("configuration exceeds the source size limit"),
            Self::Evaluation(error) => error.fmt(f),
        }
    }
}
impl StdError for Error {}

/// Read a native source, following Home Manager symlinks without rewriting them.
///
/// # Errors
/// Returns a redacted error for a non-file, inaccessible/invalid text or excess size.
pub fn read(path: &Path) -> Result<Option<String>, Error> {
    match fs::metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling managed link is a broken source, rather than new setup.
            return if fs::symlink_metadata(path).is_ok() {
                Err(Error::Read)
            } else {
                Ok(None)
            };
        }
        Ok(metadata) if metadata.is_file() => {}
        _ => return Err(Error::Read),
    }
    // Opening a FIFO swapped in after metadata must not stall a blocking worker.
    let file = fs::File::from(
        rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| Error::Read)?,
    );
    if !file.metadata().map_err(|_| Error::Read)?.is_file() {
        return Err(Error::Read);
    }
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Read)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(Error::TooLarge);
    }
    String::from_utf8(bytes).map(Some).map_err(|_| Error::Read)
}

/// Read off the UI thread and evaluate using the bounded same-binary helper.
/// No accounts, credentials, Nix daemon, or network services are contacted.
///
/// # Errors
/// Returns only redacted file, evaluator or cancellation failures.
pub async fn load(
    path: PathBuf,
    executable: &Path,
    cancellation: &CancellationToken,
) -> Result<Option<Config>, Error> {
    let source = tokio::task::spawn_blocking(move || read(&path))
        .await
        .map_err(|_| Error::Read)??;
    match source {
        Some(source) => process::evaluate(executable, &source, cancellation)
            .await
            .map(Some)
            .map_err(Error::Evaluation),
        None => Ok(None),
    }
}

/// Last-valid account declaration. Invalid reloads never partially replace it.
/// Existing files are treated as user-managed; this type never writes a source.
#[derive(Default)]
pub struct Runtime {
    active: Option<Arc<Config>>,
    error: Option<Error>,
    revision: u64,
}

impl Runtime {
    /// Adopt a fully evaluated result. A missing file after startup is an error,
    /// so removing a managed symlink cannot silently disconnect all accounts.
    ///
    /// Returns true only when a new declaration was adopted.
    pub fn apply(&mut self, result: Result<Option<Config>, Error>) -> bool {
        match result {
            Ok(Some(config)) if config.validate().is_empty() => {
                self.active = Some(Arc::new(config));
                self.error = None;
                self.revision = self.revision.saturating_add(1);
                true
            }
            Ok(Some(_)) => {
                self.error = Some(Error::Evaluation(EvaluationError::Rejected));
                false
            }
            Ok(None) if self.active.is_none() => {
                self.error = None;
                false
            }
            Ok(None) => {
                self.error = Some(Error::Read);
                false
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    /// Current validated declaration, shared immutably with account workers.
    #[must_use]
    pub const fn active(&self) -> Option<&Arc<Config>> {
        self.active.as_ref()
    }
    /// Most recent failure; the active declaration is retained separately.
    #[must_use]
    pub const fn error(&self) -> Option<Error> {
        self.error
    }
    /// Successful configuration generation, initially zero.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}
