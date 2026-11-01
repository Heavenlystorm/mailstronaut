//! Shared bounded native activation and preference writes. No GUI or secret values.

use core::fmt;
use std::path::Path;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    Config, native, process,
    source::{self, WriteMode},
};

/// A static diagnostic; source text and evaluator details are never included.
#[derive(Debug)]
pub enum Error {
    /// A typed candidate failed canonical schema validation; fields are static.
    Validation(Vec<crate::ValidationError>),
    /// A preference patch could not be deserialized as the typed schema.
    Shape,
    /// Filesystem ownership, conflict, bounds or commit failure.
    Source(source::Error),
    /// The resource-limited child failed.
    Evaluation(process::EvaluationError),
    /// Standard serialization rejected the data or its expanded size.
    Serialization(native::Error),
    /// A declared/current root must be an attribute set.
    Root,
    /// UI state was based on a different valid configuration.
    Changed,
    /// A blocking task did not finish.
    Task,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(errors) => match errors.first() {
                Some(error) => write!(f, "{}: {}", error.field, error.message),
                None => f.write_str("configuration is invalid"),
            },
            Self::Shape => f.write_str("a preference has an unexpected type or value"),
            Self::Source(error) => error.fmt(f),
            Self::Evaluation(error) => error.fmt(f),
            Self::Serialization(error) => error.fmt(f),
            Self::Root => f.write_str("configuration must be an attribute set"),
            Self::Changed => {
                f.write_str("configuration changed on disk; reload before saving preferences")
            }
            Self::Task => f.write_str("configuration operation was interrupted"),
        }
    }
}
impl std::error::Error for Error {}

/// Reapply exactly the declared attributes before defaults, preserving omissions.
/// # Errors
/// Refuses invalid, conflicting and unowned sources without replacing them.
pub async fn reconcile(
    executable: &Path,
    declared: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let declaration = source::read(declared)
        .map_err(Error::Source)?
        .ok_or(Error::Source(source::Error::Read))?;
    let declared = process::evaluate_raw(executable, &declaration, cancel)
        .await
        .map_err(Error::Evaluation)?;
    if !declared.is_object() {
        return Err(Error::Root);
    }
    let edit = source::Edit::begin(path).map_err(Error::Source)?;
    let current = match edit.current() {
        Some(text) => process::evaluate_raw(executable, text, cancel)
            .await
            .map_err(Error::Evaluation)?,
        None => serde_json::json!({ "schemaVersion": 1 }),
    };
    candidate(
        executable,
        edit,
        current,
        declared,
        WriteMode::Activation,
        cancel,
    )
    .await
    .map(|_| ())
}

/// Save preference changes against the exact effective configuration loaded by UI.
/// The raw source is patched before defaults; accounts and unrelated keys survive.
/// # Errors
/// Refuses managed/readonly/unadopted files, stale UI state and invalid candidates.
pub async fn preferences(
    executable: &Path,
    path: &Path,
    expected: &Config,
    changes: Value,
    cancel: &CancellationToken,
) -> Result<Config, Error> {
    let edit = source::Edit::begin(path).map_err(Error::Source)?;
    if expected.managed || edit.ownership().map_err(Error::Source)? != source::Ownership::AppOwned {
        return Err(Error::Source(source::Error::Ownership));
    }
    let text = edit.current().ok_or(Error::Source(source::Error::Read))?;
    let current = process::evaluate(executable, text, cancel)
        .await
        .map_err(Error::Evaluation)?;
    if serde_json::to_value(&current).map_err(|_| Error::Task)?
        != serde_json::to_value(expected).map_err(|_| Error::Task)?
    {
        return Err(Error::Changed);
    }
    let mut typed = serde_json::to_value(&current).map_err(|_| Error::Task)?;
    native::merge(&mut typed, changes.clone());
    let typed: Config = serde_json::from_value(typed).map_err(|_| Error::Shape)?;
    let errors = typed.validate();
    if !errors.is_empty() {
        return Err(Error::Validation(errors));
    }
    let raw = process::evaluate_raw(executable, text, cancel)
        .await
        .map_err(Error::Evaluation)?;
    candidate(
        executable,
        edit,
        raw,
        changes,
        WriteMode::Application,
        cancel,
    )
    .await
}

async fn candidate(
    executable: &Path,
    edit: source::Edit,
    mut current: Value,
    patch: Value,
    mode: WriteMode,
    cancel: &CancellationToken,
) -> Result<Config, Error> {
    if !current.is_object() || !patch.is_object() {
        return Err(Error::Root);
    }
    native::merge(&mut current, patch);
    if !native::set_managed(&mut current, false) {
        return Err(Error::Root);
    }
    let text = native::to_nix(&current).map_err(Error::Serialization)?;
    let config = process::evaluate(executable, &text, cancel)
        .await
        .map_err(Error::Evaluation)?;
    if cancel.is_cancelled() {
        return Err(Error::Evaluation(process::EvaluationError::Cancelled));
    }
    tokio::task::spawn_blocking(move || edit.commit(&text, mode))
        .await
        .map_err(|_| Error::Task)?
        .map_err(Error::Source)?;
    Ok(config)
}
