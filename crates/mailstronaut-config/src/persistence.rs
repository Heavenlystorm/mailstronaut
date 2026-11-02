//! Shared bounded native activation and preference writes. No GUI or secret values.

use core::fmt;
use std::path::Path;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    Account, Config, native, process,
    source::{self, WriteMode},
};

/// A static diagnostic; source text and evaluator details are never included.
#[derive(Debug)]
pub enum Error {
    /// Initialization cannot adopt or replace an existing source.
    Exists,
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
            Self::Exists => f.write_str("configuration already exists; reload it instead"),
            Self::Validation(errors) => match errors.first() {
                Some(error) => write!(f, "{}: {}", error.field, error.message),
                None => f.write_str("configuration is invalid"),
            },
            Self::Shape => f.write_str("a preference has an unexpected type or value"),
            Self::Source(error) => error.fmt(f),
            Self::Evaluation(error) => error.fmt(f),
            Self::Serialization(error) => error.fmt(f),
            Self::Root => f.write_str("configuration must be an attribute set"),
            Self::Changed => f.write_str("configuration changed on disk; reload before saving"),
            Self::Task => f.write_str("configuration operation was interrupted"),
        }
    }
}
impl std::error::Error for Error {}

/// Create an inert native source without replacing any existing path.
///
/// CLI and onboarding share the same ownership marker, writer lock and no-clobber
/// publication. The constant contains no account or secret declaration.
/// # Errors
/// Refuses busy writers, existing files/links and filesystem failures.
pub fn initialize(path: &Path) -> Result<(), Error> {
    let edit = source::Edit::begin(path).map_err(Error::Source)?;
    if edit.current().is_some() {
        return Err(Error::Exists);
    }
    edit.commit(
        "{\n  schemaVersion = 1;\n  accounts = [ ];\n}\n",
        WriteMode::Initialize,
    )
    .map_err(Error::Source)
}

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
    let (edit, current) = checked_edit(executable, path, expected, cancel).await?;
    let text = edit.current().ok_or(Error::Source(source::Error::Read))?;
    let mut typed = serde_json::to_value(&current).map_err(|_| Error::Task)?;
    native::merge(&mut typed, changes.clone());
    let typed: Config = serde_json::from_value(typed).map_err(|_| Error::Shape)?;
    validate(&typed)?;
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

/// Append one account while preserving explicit keys and omissions in existing
/// account declarations. No credentials are accepted or retrieved here.
/// # Errors
/// Refuses stale/unowned sources, duplicate IDs and invalid complete candidates.
pub async fn add_account(
    executable: &Path,
    path: &Path,
    expected: &Config,
    account: Account,
    cancel: &CancellationToken,
) -> Result<Config, Error> {
    let (edit, mut current) = checked_edit(executable, path, expected, cancel).await?;
    current.accounts.push(account.clone());
    validate(&current)?;
    let text = edit.current().ok_or(Error::Source(source::Error::Read))?;
    let mut raw = process::evaluate_raw(executable, text, cancel)
        .await
        .map_err(Error::Evaluation)?;
    let accounts = raw
        .as_object_mut()
        .ok_or(Error::Root)?
        .entry("accounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or(Error::Shape)?;
    accounts.push(serde_json::to_value(account).map_err(|_| Error::Shape)?);
    candidate(
        executable,
        edit,
        raw,
        serde_json::json!({}),
        WriteMode::Application,
        cancel,
    )
    .await
}

/// Update login names in one account while retaining the remaining raw declaration.
///
/// # Errors
/// Rejects stale or unowned configuration and invalid login names.
pub async fn account_logins(
    executable: &Path,
    path: &Path,
    expected: &Config,
    id: &str,
    incoming: String,
    outgoing: Option<String>,
    cancel: &CancellationToken,
) -> Result<Config, Error> {
    let (edit, mut current) = checked_edit(executable, path, expected, cancel).await?;
    let account = current
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or(Error::Shape)?;
    account.user_name.clone_from(&incoming);
    account.smtp_user_name.clone_from(&outgoing);
    validate(&current)?;
    let text = edit.current().ok_or(Error::Source(source::Error::Read))?;
    let mut raw = process::evaluate_raw(executable, text, cancel)
        .await
        .map_err(Error::Evaluation)?;
    let accounts = raw
        .get_mut("accounts")
        .and_then(Value::as_array_mut)
        .ok_or(Error::Shape)?;
    let account = accounts
        .iter_mut()
        .find(|account| account.get("id").and_then(Value::as_str) == Some(id))
        .and_then(Value::as_object_mut)
        .ok_or(Error::Shape)?;
    account.insert("userName".into(), Value::String(incoming));
    if let Some(outgoing) = outgoing {
        account.insert("smtpUserName".into(), Value::String(outgoing));
    } else {
        account.remove("smtpUserName");
    }
    candidate(
        executable,
        edit,
        raw,
        serde_json::json!({}),
        WriteMode::Application,
        cancel,
    )
    .await
}

fn validate(config: &Config) -> Result<(), Error> {
    let errors = config.validate();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(errors))
    }
}

async fn checked_edit(
    executable: &Path,
    path: &Path,
    expected: &Config,
    cancel: &CancellationToken,
) -> Result<(source::Edit, Config), Error> {
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
    Ok((edit, current))
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
