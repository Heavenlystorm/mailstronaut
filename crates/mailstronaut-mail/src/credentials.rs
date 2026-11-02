//! Scoped, redacted credential access using the standard oo7 backend.

use core::{error::Error as StdError, fmt, time::Duration};
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;

const APPLICATION: &str = include_str!("../../../assets/app-id");
/// Maximum stored password or refresh token size.
pub const MAX_SECRET_BYTES: usize = 64 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Separate password and OAuth refresh-token namespaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    /// IMAP/SMTP password or app password.
    Password,
    /// OAuth refresh token, never a browser authorization code.
    RefreshToken,
}

/// Exact application/account/reference identity; no credential material.
#[derive(Clone, Debug)]
pub struct Key {
    account: String,
    reference: String,
    purpose: Purpose,
    oauth_context: Option<String>,
}
impl Key {
    /// Validate a stable config identity before any backend access.
    ///
    /// # Errors
    /// Rejects empty, oversized or malformed identifiers.
    pub fn new(account: &str, reference: &str, purpose: Purpose) -> Result<Self, Error> {
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 256
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        };
        if !valid(account) || !valid(reference) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            account: account.into(),
            reference: reference.into(),
            purpose,
            oauth_context: None,
        })
    }

    /// Bind a refresh credential to a SHA-256 digest of its public OAuth metadata.
    /// Different clients/issuers/scopes cannot retrieve each other's stored token.
    ///
    /// # Errors
    /// Only refresh credentials and lowercase 64-hex context hashes are accepted.
    pub fn bind_oauth(mut self, context: String) -> Result<Self, Error> {
        if self.purpose != Purpose::RefreshToken
            || context.len() != 64
            || !context
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(Error::Invalid);
        }
        self.oauth_context = Some(context);
        Ok(self)
    }

    fn attributes(&self) -> Vec<(&str, &str)> {
        let mut attributes = vec![
            ("application", APPLICATION),
            ("account", &self.account),
            ("reference", &self.reference),
            (
                "purpose",
                match self.purpose {
                    Purpose::Password => "password",
                    Purpose::RefreshToken => "oauth-refresh",
                },
            ),
        ];
        if let Some(context) = &self.oauth_context {
            attributes.push(("oauth-context", context));
        }
        attributes
    }
}

/// Redacted failures. Backend messages and credential contents are never retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// More than one matching item exists; user reconciliation is required.
    Ambiguous,
    /// Invalid identity or unsupported secret encoding/size.
    Invalid,
    /// Foreground unlock is required.
    Locked,
    /// The operation exceeded its time budget.
    TimedOut,
    /// The encrypted backend is unavailable or refused the request.
    Unavailable,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ambiguous => "multiple stored credentials match; reconcile them in the keyring",
            Self::Invalid => "invalid credential identity, encoding or size",
            Self::Locked => "keyring locked; unlock it in the foreground",
            Self::TimedOut => "keyring operation timed out",
            Self::Unavailable => "encrypted credential storage unavailable or access refused",
        })
    }
}
impl StdError for Error {}

/// Standard encrypted backend. Debug output deliberately omits backend state.
#[derive(Clone)]
pub struct Store(Arc<oo7::Keyring>);
impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialStore([REDACTED])")
    }
}
impl Store {
    /// Connect only to an existing, unlocked native Secret Service collection.
    ///
    /// This never creates a collection or requests unlock. Native automatic work
    /// may use it before the UI opens. Sandboxed callers must instead retain a
    /// store initialized through the foreground portal; this native-only method
    /// does not select or initialize the sandbox backend.
    ///
    /// # Errors
    /// Fails closed for missing/locked storage, access refusal or a timeout.
    pub async fn connect_native_background() -> Result<Self, Error> {
        timeout(IO_TIMEOUT, async {
            let service = oo7::dbus::Service::encrypted()
                .await
                .map_err(|_| Error::Unavailable)?;
            let collection = service
                .with_alias("default")
                .await
                .map_err(|_| Error::Unavailable)?
                .ok_or(Error::Unavailable)?;
            let store = Self(Arc::new(oo7::Keyring::DBus(collection)));
            store.ready().await?;
            Ok(store)
        })
        .await
        .map_err(|_| Error::TimedOut)?
    }

    /// Connect after an explicit foreground user action. oo7 may request a portal
    /// secret or create a missing default collection, both potentially interactive.
    /// Do not invoke this constructor from automatic background retries.
    ///
    /// # Errors
    /// Fails closed if encrypted storage is unavailable, denied or times out.
    pub async fn connect_foreground() -> Result<Self, Error> {
        let backend = timeout(PROMPT_TIMEOUT, oo7::Keyring::new())
            .await
            .map_err(|_| Error::TimedOut)?
            .map_err(|_| Error::Unavailable)?;
        Ok(Self(Arc::new(backend)))
    }

    /// Explicitly request foreground unlock. Reads and writes never call this.
    ///
    /// # Errors
    /// Returns a redacted error if unlocking fails, is cancelled or times out.
    pub async fn unlock_foreground(&self) -> Result<(), Error> {
        timeout(PROMPT_TIMEOUT, self.0.unlock())
            .await
            .map_err(|_| Error::TimedOut)?
            .map_err(|_| Error::Unavailable)
    }

    async fn ready(&self) -> Result<(), Error> {
        if self.0.is_locked().await.map_err(|_| Error::Unavailable)? {
            Err(Error::Locked)
        } else {
            Ok(())
        }
    }

    /// Retrieve one exact credential without prompting or creating fallback files.
    /// A missing credential is distinct from a locked or unavailable backend.
    ///
    /// # Errors
    /// Rejects ambiguity, locked storage, invalid/oversized values and backend failures.
    pub async fn get(&self, key: &Key) -> Result<Option<SecretString>, Error> {
        timeout(IO_TIMEOUT, async {
            self.ready().await?;
            let items = self
                .0
                .search_items(&key.attributes())
                .await
                .map_err(|_| Error::Unavailable)?;
            if items.len() > 1 {
                return Err(Error::Ambiguous);
            }
            let Some(item) = items.first() else {
                return Ok(None);
            };
            if item.is_locked().await.map_err(|_| Error::Unavailable)? {
                return Err(Error::Locked);
            }
            let secret = item.secret().await.map_err(|_| Error::Unavailable)?;
            let value = core::str::from_utf8(secret.as_bytes()).map_err(|_| Error::Invalid)?;
            if value.is_empty() || value.len() > MAX_SECRET_BYTES {
                return Err(Error::Invalid);
            }
            Ok(Some(SecretString::from(value)))
        })
        .await
        .map_err(|_| Error::TimedOut)?
    }

    /// Store or rotate one exact credential; the caller retains a zeroizing value.
    /// No account removal/config reload invokes this operation implicitly.
    ///
    /// # Errors
    /// Rejects invalid values, ambiguity, locked storage and backend failures.
    pub async fn put(&self, key: &Key, value: &SecretString) -> Result<(), Error> {
        if value.expose_secret().is_empty() || value.expose_secret().len() > MAX_SECRET_BYTES {
            return Err(Error::Invalid);
        }
        timeout(IO_TIMEOUT, async {
            self.ready().await?;
            if self
                .0
                .search_items(&key.attributes())
                .await
                .map_err(|_| Error::Unavailable)?
                .len()
                > 1
            {
                return Err(Error::Ambiguous);
            }
            self.0
                .create_item(
                    "Mailstronaut credential",
                    &key.attributes(),
                    oo7::Secret::text(value.expose_secret()),
                    true,
                )
                .await
                .map_err(|_| Error::Unavailable)
        })
        .await
        .map_err(|_| Error::TimedOut)?
    }

    /// Delete this scoped credential on explicit removal or confirmed provider revocation.
    ///
    /// # Errors
    /// Rejects ambiguity and locked/unavailable storage; never broadens the query.
    pub async fn delete(&self, key: &Key) -> Result<(), Error> {
        timeout(IO_TIMEOUT, async {
            self.ready().await?;
            let items = self
                .0
                .search_items(&key.attributes())
                .await
                .map_err(|_| Error::Unavailable)?;
            if items.len() > 1 {
                return Err(Error::Ambiguous);
            }
            if let Some(item) = items.first() {
                if item.is_locked().await.map_err(|_| Error::Unavailable)? {
                    return Err(Error::Locked);
                }
                item.delete().await.map_err(|_| Error::Unavailable)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::TimedOut)?
    }
}
