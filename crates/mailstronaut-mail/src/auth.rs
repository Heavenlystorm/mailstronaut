//! One encrypted credential owner and serialized OAuth refresh state per account/reference.

use core::{error::Error as StdError, fmt, time::Duration};
use std::{collections::HashMap, sync::Arc, time::Instant};

use mailstronaut_config::{Account, Auth, Sasl};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    credentials::{self, Key, Purpose, Store},
    imap::Authentication,
    oauth::{self, Grant, Provider},
};

/// Static/redacted account authentication failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The account's encrypted credential needs foreground attention.
    Credential(credentials::Error),
    /// Authorization was refused, revoked, cancelled or could not be completed.
    OAuth(oauth::Error),
    /// No password exists for the configured reference.
    Missing,
    /// Invalid account/auth configuration or exhausted bounded credential slots.
    Configuration,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Credential(error) => error.fmt(f),
            Self::OAuth(error) => error.fmt(f),
            Self::Missing => f.write_str("No stored password; save it in Accounts."),
            Self::Configuration => f.write_str("Invalid account authentication configuration."),
        }
    }
}
impl StdError for Error {}
impl From<credentials::Error> for Error {
    fn from(error: credentials::Error) -> Self {
        Self::Credential(error)
    }
}
impl From<oauth::Error> for Error {
    fn from(error: oauth::Error) -> Self {
        Self::OAuth(error)
    }
}

/// A short-lived borrowed wire credential. Debug never exposes its secret.
pub struct Credential {
    secret: SecretString,
    mechanism: Option<Sasl>,
}
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<private>)")
    }
}
impl Credential {
    /// Borrow for a single required-TLS mail connection; no token enters configuration.
    #[must_use]
    pub const fn authentication(&self) -> Authentication<'_> {
        match self.mechanism {
            Some(mechanism) => Authentication::OAuth2 {
                token: &self.secret,
                mechanism,
            },
            None => Authentication::Password(&self.secret),
        }
    }
}
struct Cached {
    secret: SecretString,
    until: Instant,
}
#[derive(Default)]
struct State {
    fingerprint: String,
    provider: Option<Provider>,
    access: Option<Cached>,
    blocked: bool,
}
type Entries = Mutex<HashMap<(String, String), Arc<Mutex<State>>>>;
struct Inner {
    store: Store,
    entries: Entries,
}

/// Cloneable foreground-created credential session.
///
/// Incoming and outgoing calls
/// share the same refresh lock when their account/reference match. Creating a
/// session does not connect/unlock the keyring or contact any network service.
#[derive(Clone)]
pub struct Session(Arc<Inner>);
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthenticationSession(<private>)")
    }
}
impl Session {
    /// Retain the already explicitly unlocked standard encrypted backend.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self(Arc::new(Inner {
            store,
            entries: Mutex::default(),
        }))
    }
    /// Resolve one protocol credential.
    ///
    /// Refreshes serialize; rotated refresh tokens
    /// commit before access is published. Reads never unlock/prompt or retry.
    ///
    /// # Errors
    /// Reports missing/locked/revoked credentials, cancellation and static transport errors.
    pub async fn resolve(
        &self,
        account: &Account,
        outgoing: bool,
        cancel: &CancellationToken,
    ) -> Result<Credential, Error> {
        let auth = selected(account, outgoing);
        if let Auth::Password { secret_ref } = auth {
            let key = Key::new(&account.id, secret_ref, Purpose::Password)?;
            let secret = cancel
                .run_until_cancelled(self.0.store.get(&key))
                .await
                .ok_or(oauth::Error::Cancelled)??
                .ok_or(Error::Missing)?;
            return Ok(Credential {
                secret,
                mechanism: None,
            });
        }
        let (key, entry, fingerprint) = self.entry(account, outgoing).await?;
        let mut state = cancel
            .run_until_cancelled(entry.lock())
            .await
            .ok_or(oauth::Error::Cancelled)?;
        if cancel.is_cancelled() {
            return Err(oauth::Error::Cancelled.into());
        }
        state.configure(fingerprint);
        if state.blocked {
            return Err(oauth::Error::Reauthenticate.into());
        }
        // Even a cached access token cannot bypass a locked/deleted keyring item.
        let refresh = cancel
            .run_until_cancelled(self.0.store.get(&key))
            .await
            .ok_or(oauth::Error::Cancelled)??
            .ok_or(oauth::Error::Reauthenticate)?;
        if state.provider.is_none() {
            state.provider = Some(Provider::prepare(auth, &account.tls).await?);
        }
        let provider = state.provider.as_ref().ok_or(Error::Configuration)?.clone();
        if let Some(access) = &state.access
            && access.until > Instant::now()
        {
            return Ok(Credential {
                secret: access.secret.clone(),
                mechanism: Some(provider.mechanism()),
            });
        }
        state.access = None;
        let grant = match provider.refresh(&refresh, cancel).await {
            Ok(grant) => grant,
            Err(oauth::Error::Reauthenticate) => {
                state.blocked = true;
                // Known revoked credentials must not trigger retries after restart.
                // Failure to delete still leaves this live session blocked.
                let _deleted = self.0.store.delete(&key).await;
                return Err(oauth::Error::Reauthenticate.into());
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(rotated) = grant.refresh()
            && let Err(error) = self.0.store.put(&key, rotated).await
        {
            state.blocked = true;
            return Err(error.into());
        }
        let secret = state.cache(&grant);
        drop(state);
        Ok(Credential {
            secret,
            mechanism: Some(provider.mechanism()),
        })
    }
    /// Persist an explicitly authorized offline grant.
    ///
    /// This operation finishes its
    /// bounded encrypted write once a provider issued new tokens; do not cancel it
    /// between rotation and persistence. No access token is written to disk.
    ///
    /// # Errors
    /// Refuses grants without a refresh token and encrypted-backend/metadata failures.
    pub async fn accept(
        &self,
        account: &Account,
        outgoing: bool,
        grant: Grant,
    ) -> Result<(), Error> {
        let refresh = grant.refresh().ok_or(oauth::Error::Reauthenticate)?;
        let (key, entry, fingerprint) = self.entry(account, outgoing).await?;
        let mut state = entry.lock().await;
        state.configure(fingerprint);
        if let Err(error) = self.0.store.put(&key, refresh).await {
            state.access = None;
            state.blocked = true;
            return Err(error.into());
        }
        state.blocked = false;
        let _access = state.cache(&grant);
        drop(state);
        Ok(())
    }
    /// Invalidate a rejected cached bearer token before a later explicit reconnect.
    /// No network request or credential deletion happens here.
    pub async fn invalidate(&self, account: &Account, outgoing: bool) {
        let Auth::Oauth2 { secret_ref, .. } = selected(account, outgoing) else {
            return;
        };
        let entry = self
            .0
            .entries
            .lock()
            .await
            .get(&(account.id.clone(), secret_ref.clone()))
            .cloned();
        if let Some(entry) = entry {
            entry.lock().await.access = None;
        }
    }
    async fn entry(
        &self,
        account: &Account,
        outgoing: bool,
    ) -> Result<(Key, Arc<Mutex<State>>, String), Error> {
        let auth = selected(account, outgoing);
        let Auth::Oauth2 { secret_ref, .. } = auth else {
            return Err(Error::Configuration);
        };
        let context = serde_json::to_vec(auth).map_err(|_| Error::Configuration)?;
        let context = format!("{:x}", Sha256::digest(context));
        let key = Key::new(&account.id, secret_ref, Purpose::RefreshToken)?.bind_oauth(context)?;
        let fingerprint =
            serde_json::to_string(&(auth, &account.tls)).map_err(|_| Error::Configuration)?;
        let mut entries = self.0.entries.lock().await;
        let identity = (account.id.clone(), secret_ref.clone());
        if !entries.contains_key(&identity) && entries.len() >= 512 {
            return Err(Error::Configuration);
        }
        let entry = entries.entry(identity).or_default().clone();
        drop(entries);
        Ok((key, entry, fingerprint))
    }
}
impl State {
    fn configure(&mut self, fingerprint: String) {
        if self.fingerprint != fingerprint {
            *self = Self {
                fingerprint,
                ..Self::default()
            };
        }
    }
    fn cache(&mut self, grant: &Grant) -> SecretString {
        let secret = grant.access().clone();
        self.access = grant
            .expires_in()
            .and_then(|lifetime| {
                Instant::now().checked_add(lifetime.saturating_sub(Duration::from_secs(60)))
            })
            .map(|until| Cached {
                secret: secret.clone(),
                until,
            });
        secret
    }
}
fn selected(account: &Account, outgoing: bool) -> &Auth {
    if outgoing {
        account.smtp_auth.as_ref().unwrap_or(&account.auth)
    } else {
        &account.auth
    }
}
