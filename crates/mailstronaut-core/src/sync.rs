//! Read-only, resumable work units. No scheduler, credential prompt or UI lives here.

use core::{error::Error as StdError, fmt, num::NonZeroU32};

use mailstronaut_mail::imap::{self, Connection, Metadata};
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{
    store::{
        backfill::Pass,
        mailbox::{Flags, Folder, FolderRole, GmailId, Membership, UidSnapshot},
    },
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
    /// Larger originals are not kept: only their headers and text are, no greater than 8 MiB.
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
    /// UIDs of this window whose metadata was reviewed, kept or not. A window cut short by
    /// the body budget counts only the UIDs above its stopping point.
    pub reviewed: u32,
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
    let selected = select_folder(store, connection, request, cancel).await?;
    let upper = upper(request, &selected);
    window(store, connection, request, &selected, upper, cancel).await
}

/// Reconcile and fetch one closed window of UIDs at or below `upper`, newest first.
async fn window(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    selected: &Selected,
    upper: u32,
    cancel: &CancellationToken,
) -> Result<Page, Error> {
    let folder = &selected.folder;
    let validity = selected.validity;
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
        next_before: (first.get() > 1).then(|| position(request, folder.id, validity, first)),
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
            if observed.received_at < request.earliest || observed.size == 0 {
                page.skipped += 1;
                continue;
            }
            if observed.size > request.max_original {
                // Too large to keep, but it still arrived: show what can be read.
                if cache_reduced(
                    store, connection, request, folder.id, validity, &observed, cancel,
                )
                .await?
                {
                    page.downloaded += 1;
                } else {
                    page.skipped += 1;
                }
                continue;
            }
            if downloaded_bytes + observed.size > BODY_BUDGET {
                // Include this unprocessed UID when the next page resumes.
                page.next_before = observed
                    .uid
                    .get()
                    .checked_add(1)
                    .and_then(NonZeroU32::new)
                    .map(|before| position(request, folder.id, validity, before));
                page.reviewed = u32::try_from(
                    descending
                        .iter()
                        .take_while(|uid| **uid > observed.uid)
                        .count(),
                )
                .unwrap_or(u32::MAX);
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
    page.reviewed = u32::try_from(descending.len()).unwrap_or(u32::MAX);
    Ok(page)
}

fn position(
    request: &Request<'_>,
    folder: i64,
    uid_validity: NonZeroU32,
    before: NonZeroU32,
) -> Position {
    Position {
        context: digest(request.context),
        folder,
        uid_validity,
        before,
    }
}

/// The fingerprint of a mailbox context, as positions and stored passes record it.
fn digest(context: &str) -> [u8; 32] {
    Sha256::digest(context.as_bytes()).into()
}

/// Returns whether a reduced copy was stored. An expunged message or one whose headers
/// the parser refuses is simply not cached, like any other unsupported message.
async fn cache_reduced(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    folder: i64,
    validity: NonZeroU32,
    observed: &Metadata,
    cancel: &CancellationToken,
) -> Result<bool, Error> {
    let Some(outline) = connection.outline(observed, cancel).await? else {
        return Ok(false);
    };
    let Some((reduced, omitted)) = crate::outline::reduce(&outline) else {
        return Ok(false);
    };
    match store
        .cache_reduced(
            request.account.into(),
            membership(folder, validity, &outline.metadata),
            outline.metadata.received_at,
            outline.metadata.gmail_id.map(GmailId),
            reduced,
            omitted,
        )
        .await
    {
        Ok(_) => Ok(true),
        Err(worker::Error::UnsupportedMessage) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// The folder as SELECT left it, after its UID namespace has been recorded.
struct Selected {
    folder: Folder,
    validity: NonZeroU32,
    /// UIDNEXT when the folder was selected: the namespace's exclusive upper bound.
    next: u32,
    /// EXISTS when the folder was selected.
    exists: u32,
    /// The mailbox context differs from the one the store last recorded.
    changed: bool,
}

async fn select_folder(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<Selected, Error> {
    let mailbox = connection.select(request.folder, cancel).await?;
    let validity = mailbox
        .uid_validity
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    let next = mailbox
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
    Ok(Selected {
        folder,
        validity,
        next: next.get(),
        exists: mailbox.exists,
        changed,
    })
}

/// The exclusive bound of the next window. A continuation is honoured only inside the
/// exact namespace and mailbox context that produced it.
fn upper(request: &Request<'_>, selected: &Selected) -> u32 {
    request
        .before
        .filter(|position| {
            !selected.changed
                && position.context == digest(request.context)
                && position.folder == selected.folder.id
                && position.uid_validity == selected.validity
        })
        .map_or_else(
            || selected.next,
            |position| position.before.get().min(selected.next),
        )
        .saturating_sub(1)
}

/// How far one folder's history pass has got, as reported while it runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress {
    /// Messages reviewed by the current pass, whether or not their originals were kept.
    pub completed: u32,
    /// Messages present when the pass began; None when SELECT did not report them.
    pub total: Option<u32>,
}

/// The outcome of one window of a folder's history pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Step {
    /// Originals newly committed by this window.
    pub downloaded: u32,
    /// The pass has reached the beginning of the folder, now or in an earlier step.
    pub finished: bool,
    /// Where the pass stands after this step.
    pub progress: Progress,
}

/// What a stored pass must still match before its cursor may be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fence {
    /// UIDVALIDITY of the namespace the cursor must belong to.
    pub uid_validity: u32,
    /// Fingerprint of the mailbox context the pass must have started under.
    pub context: [u8; 32],
    /// The age cap the pass must have been started with.
    pub cap_days: Option<u32>,
}

/// What a folder's pass does before its next window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Plan {
    /// Finished recently enough that nothing under this fence needs another walk.
    Rest(Pass),
    /// Resume from the stored cursor.
    Continue(Pass),
    /// Begin again from the newest UID, since the stored cursor can no longer be trusted.
    Start(Pass),
}

/// A finished pass is walked again after this long. That is how flags on old mail and
/// gaps are reconciled without rescanning the whole history on every poll.
pub const REWALK_SECONDS: i64 = 6 * 60 * 60;

/// Decide what a folder's next window does, from its stored pass.
///
/// Only a pass started under the same namespace, context and age cap may continue. A UID
/// reset, a changed account or a changed limit starts from the top instead of trusting
/// old UIDs.
#[must_use]
pub fn plan(stored: Option<Pass>, fence: Fence, next: u32, exists: u32, now: i64) -> Plan {
    let fresh = Pass {
        uid_validity: fence.uid_validity,
        context: fence.context,
        cap_days: fence.cap_days,
        next_before: Some(next),
        total: Some(exists),
        reviewed: 0,
        finished_at: None,
    };
    match stored {
        Some(pass)
            if pass.uid_validity == fence.uid_validity
                && pass.context == fence.context
                && pass.cap_days == fence.cap_days =>
        {
            match pass.finished_at {
                None => Plan::Continue(pass),
                Some(finished) if now.saturating_sub(finished) < REWALK_SECONDS => Plan::Rest(pass),
                Some(_) => Plan::Start(fresh),
            }
        }
        _ => Plan::Start(fresh),
    }
}

/// Advance one folder's history pass by one window, from its durable cursor.
///
/// Every window is reviewed to the bottom of the folder, and reviewing a window that
/// downloads nothing does not end the pass: cached messages and UID gaps are normal.
/// A cursor is saved only after its window's originals committed, so an interrupted step
/// repeats safely. Age caps decide what is downloaded, never what is deleted.
///
/// # Errors
/// Returns redacted connection or storage failures. The stored cursor then names the last
/// committed window.
pub async fn backfill_step(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cap_days: Option<u32>,
    now: i64,
    cancel: &CancellationToken,
) -> Result<Step, Error> {
    if request.max_original == 0 || request.max_original > 8 * 1024 * 1024 {
        return Err(imap::Error::Configuration.into());
    }
    let selected = select_folder(store, connection, request, cancel).await?;
    let fence = Fence {
        uid_validity: selected.validity.get(),
        context: digest(request.context),
        cap_days,
    };
    let account = request.account.to_owned();
    let stored = store
        .backfill_pass(account.clone(), selected.folder.id)
        .await?;
    let mut pass = match plan(stored, fence, selected.next, selected.exists, now) {
        Plan::Rest(pass) => {
            return Ok(Step {
                downloaded: 0,
                finished: true,
                progress: progress(&pass),
            });
        }
        Plan::Start(pass) => {
            store
                .save_backfill(account.clone(), selected.folder.id, pass)
                .await?;
            pass
        }
        Plan::Continue(pass) => pass,
    };
    // The cursor is exclusive. A namespace that grew since the pass began is walked from
    // its newest UID, which the recent sync also covers.
    let upper = pass
        .next_before
        .unwrap_or(0)
        .min(selected.next)
        .saturating_sub(1);
    if upper == 0 {
        pass.next_before = None;
        pass.finished_at = Some(now);
        store
            .save_backfill(account, selected.folder.id, pass)
            .await?;
        return Ok(Step {
            downloaded: 0,
            finished: true,
            progress: progress(&pass),
        });
    }
    let page = window(store, connection, request, &selected, upper, cancel).await?;
    pass.reviewed = pass.reviewed.saturating_add(page.reviewed);
    pass.next_before = page.next_before.map(|position| position.before.get());
    pass.finished_at = pass.next_before.is_none().then_some(now);
    store
        .save_backfill(account, selected.folder.id, pass)
        .await?;
    Ok(Step {
        downloaded: page.downloaded,
        finished: pass.next_before.is_none(),
        progress: progress(&pass),
    })
}

const fn progress(pass: &Pass) -> Progress {
    Progress {
        completed: pass.reviewed,
        total: pass.total,
    }
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

#[cfg(test)]
mod history_tests {
    use super::{Fence, Pass, Plan, REWALK_SECONDS, plan};

    const FENCE: Fence = Fence {
        uid_validity: 42,
        context: [9; 32],
        cap_days: None,
    };

    /// A pass 200 UIDs into a 300-message folder: `finished` decides whether it is done.
    fn pass(finished_at: Option<i64>) -> Pass {
        Pass {
            uid_validity: 42,
            context: [9; 32],
            cap_days: None,
            next_before: finished_at.is_none().then_some(200),
            total: Some(300),
            reviewed: 100,
            finished_at,
        }
    }

    #[test]
    fn a_partial_pass_resumes_exactly_where_it_stopped() {
        assert_eq!(
            plan(Some(pass(None)), FENCE, 300, 300, 0),
            Plan::Continue(pass(None))
        );
    }

    #[test]
    fn a_new_folder_starts_from_its_newest_uid_with_a_total() {
        let fresh = Pass {
            next_before: Some(300),
            total: Some(300),
            reviewed: 0,
            ..pass(None)
        };
        assert_eq!(plan(None, FENCE, 300, 300, 0), Plan::Start(fresh));
    }

    #[test]
    fn a_uid_reset_never_resumes_the_old_namespaces_cursor() {
        let reset = Fence {
            uid_validity: 43,
            ..FENCE
        };
        let restarted = Pass {
            uid_validity: 43,
            next_before: Some(500),
            total: Some(480),
            reviewed: 0,
            ..pass(None)
        };
        assert_eq!(
            plan(Some(pass(None)), reset, 500, 480, 0),
            Plan::Start(restarted)
        );
    }

    #[test]
    fn a_changed_mailbox_context_or_age_cap_starts_again() {
        let other_account = Fence {
            context: [1; 32],
            ..FENCE
        };
        assert!(matches!(
            plan(Some(pass(None)), other_account, 300, 300, 0),
            Plan::Start(_)
        ));
        let narrowed = Fence {
            cap_days: Some(30),
            ..FENCE
        };
        assert!(matches!(
            plan(Some(pass(None)), narrowed, 300, 300, 0),
            Plan::Start(_)
        ));
    }

    #[test]
    fn a_finished_pass_rests_until_the_rewalk_interval_has_passed() {
        let finished = pass(Some(1_000));
        assert_eq!(
            plan(Some(finished), FENCE, 300, 300, 1_000 + REWALK_SECONDS - 1),
            Plan::Rest(finished)
        );
        assert!(matches!(
            plan(Some(finished), FENCE, 300, 300, 1_000 + REWALK_SECONDS),
            Plan::Start(_)
        ));
    }
}
