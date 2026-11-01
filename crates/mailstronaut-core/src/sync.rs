//! Read-only, resumable work units. No scheduler, credential prompt or UI lives here.

use core::{error::Error as StdError, fmt, num::NonZeroU32};

use mailstronaut_mail::imap::{self, Connection, Metadata};
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{
    store::mailbox::{Flags, Folder, FolderRole, GmailId, Membership, UidSnapshot},
    worker,
};

const UID_WINDOW: u32 = 128;
const BODY_BUDGET: u32 = 16 * 1024 * 1024;

/// A paging cursor is meaningful only inside its exact mailbox UID namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    /// Hash of the authenticated public configuration; discovery cannot make a
    /// prior server/principal's cursor valid merely by rebinding the account.
    pub context: [u8; 32],
    /// Local folder identity, scoped to the owning database.
    pub folder: i64,
    /// SELECT generation when this cursor was committed.
    pub uid_validity: NonZeroU32,
    /// Exclusive upper UID bound for the next page.
    pub before: NonZeroU32,
}

/// One explicitly requested account/folder page. Account workers serialize work
/// for their selected connection; callers bound the number of concurrent workers.
pub struct Request<'a> {
    /// Stable enabled configuration account ID.
    pub account: &'a str,
    /// Fingerprint of the authenticated public account configuration.
    pub context: &'a str,
    /// Opaque server wire mailbox name.
    pub folder: &'a str,
    /// This folder participates in unified inbox queries.
    pub inbox: bool,
    /// Exclusive upper UID bound; None starts from SELECT's current UIDNEXT.
    pub before: Option<Position>,
    /// Skip downloading older originals, while still reconciling cached flags.
    pub earliest: i64,
    /// User's configured original-size budget, no greater than 8 MiB.
    pub max_original: u32,
}

/// Counts describe this page only. None means the UID namespace has been
/// scanned to its beginning, not that every message/attachment is cached.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Page {
    /// Continue strictly before this UID; only advanced after committed work.
    pub next_before: Option<Position>,
    /// Newly committed original messages.
    pub downloaded: u32,
    /// Existing originals whose observed flags were refreshed.
    pub refreshed: u32,
    /// Originals omitted because of age, size, unsupported MIME or concurrent expunge.
    pub skipped: u32,
}

/// Redacted transport/storage errors; no credentials, bodies or server diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Protocol, TLS, cancellation or network failure.
    Transport(imap::Error),
    /// Durable cache operation failed.
    Storage(worker::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::Storage(error) => error.fmt(f),
        }
    }
}
impl StdError for Error {}
impl From<imap::Error> for Error {
    fn from(value: imap::Error) -> Self {
        Self::Transport(value)
    }
}
impl From<worker::Error> for Error {
    fn from(value: worker::Error) -> Self {
        Self::Storage(value)
    }
}

const ROLES: [FolderRole; 5] = [
    FolderRole::Sent,
    FolderRole::Drafts,
    FolderRole::Trash,
    FolderRole::Archive,
    FolderRole::Junk,
];

/// Choose at most one folder per role.
///
/// An explicit configuration override wins, then the server's SPECIAL-USE attribute,
/// then a conventional name for servers without that extension. Wire names are matched,
/// never rewritten.
#[must_use]
pub fn folder_roles(
    folders: &[imap::Folder],
    overrides: &mailstronaut_config::Folders,
) -> Vec<(String, FolderRole)> {
    let selectable = || folders.iter().filter(|folder| folder.selectable);
    ROLES
        .into_iter()
        .filter_map(|role| {
            let (declared, attribute, names): (_, _, &[&str]) = match role {
                FolderRole::Sent => (
                    &overrides.sent,
                    "\\sent",
                    &[
                        "sent",
                        "sent mail",
                        "sent items",
                        "sent messages",
                        "envoyés",
                        "inviata",
                        "posta inviata",
                    ],
                ),
                FolderRole::Drafts => (
                    &overrides.drafts,
                    "\\drafts",
                    &["drafts", "draft", "brouillons", "bozze"],
                ),
                FolderRole::Trash => (
                    &overrides.trash,
                    "\\trash",
                    &[
                        "trash",
                        "deleted",
                        "deleted items",
                        "deleted messages",
                        "corbeille",
                        "cestino",
                    ],
                ),
                FolderRole::Archive => (
                    &overrides.archive,
                    "\\archive",
                    &["archive", "archives", "archivio"],
                ),
                FolderRole::Junk => (
                    &None,
                    "\\junk",
                    &[
                        "junk",
                        "spam",
                        "junk e-mail",
                        "indésirables",
                        "posta indesiderata",
                    ],
                ),
            };
            let leaf = |folder: &&imap::Folder| {
                let delimiter = folder.delimiter.as_deref().unwrap_or("/");
                let name = folder
                    .wire_name
                    .rsplit(delimiter)
                    .next()
                    .unwrap_or_default()
                    .to_lowercase();
                names.contains(&name.as_str())
            };
            declared
                .as_deref()
                .filter(|name| selectable().any(|folder| folder.wire_name == *name))
                .map(str::to_owned)
                .or_else(|| {
                    selectable()
                        .find(|folder| {
                            folder
                                .attributes
                                .iter()
                                .any(|value| value.eq_ignore_ascii_case(attribute))
                        })
                        .or_else(|| selectable().find(leaf))
                        .map(|folder| folder.wire_name.clone())
                })
                .map(|name| (name, role))
        })
        .collect()
}

/// Discover selectable folders over an already authenticated connection. Persist
/// exact wire identities and roles without translating paths or fetching their mail.
/// # Errors
/// Returns redacted transport/storage errors; incomplete LIST results are not saved.
pub async fn discover(
    store: &worker::Handle,
    connection: &mut Connection,
    account: &str,
    context: &str,
    inbox: &str,
    overrides: &mailstronaut_config::Folders,
    cancel: &CancellationToken,
) -> Result<Vec<(String, FolderRole)>, Error> {
    let folders = connection.folders(cancel).await?;
    store.ensure_account(account.into()).await?;
    store
        .bind_mailbox_context(account.into(), context.into())
        .await?;
    let roles = folder_roles(&folders, overrides);
    let names = folders
        .into_iter()
        .filter(|folder| folder.selectable)
        .map(|folder| folder.wire_name)
        .collect();
    store
        .remember_folders(account.into(), names, inbox.into())
        .await?;
    store
        .assign_folder_roles(account.into(), roles.clone())
        .await?;
    Ok(roles)
}

/// Reconcile one closed UID window and fetch bounded PEEK originals serially.
///
/// No local/network transaction spans an await. A disconnected/cancelled page
/// can be retried: cached UID bindings make repeated originals idempotent.
///
/// # Errors
/// Returns redacted connection/storage failures. Its cursor is returned only
/// after every preceding storage operation commits. No persistent sync cursor
/// is claimed; callers may restart from newest safely after interruption.
pub async fn page(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<Page, Error> {
    if request.max_original == 0 || request.max_original > 8 * 1024 * 1024 {
        return Err(imap::Error::Configuration.into());
    }
    let (folder, validity, upper) = prepare(store, connection, request, cancel).await?;
    let Some(last) = NonZeroU32::new(upper) else {
        return Ok(Page::default());
    };
    let first = NonZeroU32::new(upper.saturating_sub(UID_WINDOW - 1).max(1))
        .ok_or(imap::Error::Protocol)?;
    let uids = connection.search(first, last, None, cancel).await?;
    store
        .reconcile_span(
            request.account.into(),
            UidSnapshot {
                folder: folder.id,
                uid_validity: validity,
                first,
                last,
                present: uids.clone(),
            },
        )
        .await?;
    let mut page = Page {
        next_before: (first.get() > 1).then_some(Position {
            context: Sha256::digest(request.context.as_bytes()).into(),
            folder: folder.id,
            uid_validity: validity,
            before: first,
        }),
        ..Page::default()
    };
    let mut downloaded_bytes = 0;
    // Most recent UIDs first. Metadata is owned and bounded to 50 entries; no
    // borrowed protocol stream survives while the blocking store is awaited.
    let descending = uids.into_iter().rev().collect::<Vec<_>>();
    for batch in descending.chunks(50) {
        let mut metadata = connection.metadata(batch, cancel).await?;
        metadata.sort_unstable_by_key(|entry| core::cmp::Reverse(entry.uid));
        for observed in metadata {
            if cancel.is_cancelled() {
                return Err(imap::Error::Cancelled.into());
            }
            let binding = membership(folder.id, validity, &observed);
            if store
                .observe_membership(request.account.into(), binding)
                .await?
            {
                page.refreshed += 1;
                continue;
            }
            if observed.received_at < request.earliest
                || observed.size == 0
                || observed.size > request.max_original
            {
                page.skipped += 1;
                continue;
            }
            if downloaded_bytes + observed.size > BODY_BUDGET {
                // Include this unprocessed UID when the next page resumes.
                page.next_before = observed
                    .uid
                    .get()
                    .checked_add(1)
                    .and_then(NonZeroU32::new)
                    .map(|before| Position {
                        context: Sha256::digest(request.context.as_bytes()).into(),
                        folder: folder.id,
                        uid_validity: validity,
                        before,
                    });
                return Ok(page);
            }
            let Some(message) = connection.peek(&observed, cancel).await? else {
                page.skipped += 1;
                continue;
            };
            downloaded_bytes += observed.size;
            match store
                .cache_message(
                    request.account.into(),
                    membership(folder.id, validity, &message.metadata),
                    message.metadata.received_at,
                    message.metadata.gmail_id.map(GmailId),
                    message.original,
                )
                .await
            {
                Ok(_) => page.downloaded += 1,
                Err(worker::Error::UnsupportedMessage) => page.skipped += 1,
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(page)
}
async fn prepare(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<(Folder, NonZeroU32, u32), Error> {
    let selected = connection.select(request.folder, cancel).await?;
    let validity = selected
        .uid_validity
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    let next = selected
        .uid_next
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    store.ensure_account(request.account.into()).await?;
    let changed = store
        .bind_mailbox_context(request.account.into(), request.context.into())
        .await?;
    let folder = store
        .ensure_folder(request.account.into(), request.folder.into(), request.inbox)
        .await?;
    store
        .select_folder(request.account.into(), folder.id, validity)
        .await?;
    let upper = request
        .before
        .filter(|position| {
            !changed
                && position.context == <[u8; 32]>::from(Sha256::digest(request.context.as_bytes()))
                && position.folder == folder.id
                && position.uid_validity == validity
        })
        .map_or_else(
            || next.get(),
            |position| position.before.get().min(next.get()),
        )
        .saturating_sub(1);
    Ok((folder, validity, upper))
}

const fn membership(folder: i64, validity: NonZeroU32, metadata: &Metadata) -> Membership {
    Membership {
        folder,
        uid_validity: validity,
        uid: metadata.uid,
        flags: Flags {
            seen: metadata.seen,
            starred: metadata.starred,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{FolderRole, folder_roles, imap};
    use mailstronaut_config::Folders;

    fn folder(name: &str, attributes: &[&str]) -> imap::Folder {
        imap::Folder {
            wire_name: name.into(),
            selectable: true,
            delimiter: Some("/".into()),
            attributes: attributes.iter().map(|value| (*value).into()).collect(),
        }
    }

    #[test]
    fn special_use_beats_conventional_names_and_an_override_beats_both() {
        let folders = [
            folder("INBOX", &[]),
            folder("Sent", &[]),
            folder("[Gmail]/Sent Mail", &["\\Sent"]),
            folder("Custom outgoing", &[]),
        ];
        assert_eq!(
            folder_roles(&folders, &Folders::default()),
            [("[Gmail]/Sent Mail".to_owned(), FolderRole::Sent)]
        );
        let declared: Folders =
            serde_json::from_str(r#"{"sent":"Custom outgoing"}"#).expect("declared folders");
        assert_eq!(
            folder_roles(&folders, &declared),
            [("Custom outgoing".to_owned(), FolderRole::Sent)]
        );
    }

    #[test]
    fn names_are_a_fallback_and_unselectable_or_unknown_folders_gain_no_role() {
        let mut parent = folder("Trash", &["\\Trash"]);
        parent.selectable = false;
        let folders = [
            parent,
            folder("INBOX/Envoyés", &[]),
            folder("Corbeille", &[]),
            folder("Reports", &[]),
        ];
        let declared: Folders =
            serde_json::from_str(r#"{"archive":"Missing"}"#).expect("declared folders");
        assert_eq!(
            folder_roles(&folders, &declared),
            [
                ("INBOX/Envoyés".to_owned(), FolderRole::Sent),
                ("Corbeille".to_owned(), FolderRole::Trash),
            ]
        );
    }
}
