//! Bounded file loading and atomic adoption of a validated configuration.

use core::{error::Error as StdError, fmt};
use std::{
    ffi::OsString,
    fs,
    io::{self, Read as _},
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _},
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
    /// A cooperating writer holds the path, or its snapshot changed.
    Conflict,
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
            Self::Conflict => {
                f.write_str("configuration changed or is being edited; reload and retry")
            }
            Self::Read => f.write_str("configuration must be a readable regular UTF-8 file"),
            Self::TooLarge => f.write_str("configuration exceeds the source size limit"),
            Self::Evaluation(error) => error.fmt(f),
        }
    }
}
impl StdError for Error {}

/// One cooperating configuration writer, with a checked pre-edit snapshot.
/// The lock is a persistent sibling; never unlink it while another writer may use it.
pub struct Edit {
    lock: fs::File,
    path: PathBuf,
    identity: Option<(u64, u64)>,
    text: Option<String>,
}

impl Drop for Edit {
    fn drop(&mut self) {
        // A concurrently spawned child can briefly inherit the open description
        // before CLOEXEC closes it. Explicitly unlock instead of relying solely
        // on closing the parent's descriptor to release the cooperating writer.
        _ = self.lock.unlock();
    }
}

impl Edit {
    /// Lock a native source before reading it. All application writers share this.
    /// # Errors
    /// Rejects non-regular/symlink lock files, busy writers and unstable snapshots.
    pub fn begin(path: &Path) -> Result<Self, Error> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|_| Error::Read)?;
        let mut name = OsString::from(".");
        name.push(path.file_name().ok_or(Error::Read)?);
        name.push(".lock");
        let lock = fs::File::from(
            rustix::fs::open(
                parent.join(name),
                rustix::fs::OFlags::CREATE
                    | rustix::fs::OFlags::RDWR
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::NONBLOCK,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .map_err(|_| Error::Read)?,
        );
        if !lock.metadata().map_err(|_| Error::Read)?.is_file() {
            return Err(Error::Read);
        }
        lock.try_lock().map_err(|_| Error::Conflict)?;
        let mut edit = Self {
            lock,
            path: path.into(),
            identity: None,
            text: None,
        };
        edit.identity = identity(path)?;
        edit.text = read(path)?;
        if edit.identity != identity(path)? {
            return Err(Error::Conflict);
        }
        Ok(edit)
    }

    /// The bounded source captured while acquiring the path.
    #[must_use]
    pub fn current(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Detect atomic replacements and in-place edits before committing.
    /// External editors must use this lock for atomic cooperation with app writes.
    /// # Errors
    /// Rejects a changed source/identity or unreadable replacement.
    pub fn verify(&self) -> Result<(), Error> {
        if self.identity != identity(&self.path)?
            || self.text != read(&self.path)?
            || self.identity != identity(&self.path)?
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

fn identity(path: &Path) -> Result<Option<(u64, u64)>, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some((metadata.dev(), metadata.ino()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Read),
    }
}

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
