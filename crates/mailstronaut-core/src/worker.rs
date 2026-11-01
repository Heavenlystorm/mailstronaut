//! A bounded channel to one dedicated blocking SQLite owner, not a daemon/RPC service.

use core::{error::Error as StdError, fmt};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    thread,
};

use tokio::sync::{mpsc, oneshot};

use crate::{
    mime::{self, Projection},
    store::{
        self, Store,
        mailbox::{Cursor, Folder, GmailId, Membership, Summary, UidSnapshot},
        outbox::{self, Draft, DraftSummary, Submission},
    },
};

/// Maximum queued store operations. Full-size originals are separately bounded
/// to 8 MiB, so this queue cannot retain an unbounded mailbox in RAM.
pub const QUEUE_CAPACITY: usize = 16;

/// Redacted store-worker failures that can cross the UI/engine channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A draft revision or submission intent changed before it was applied.
    Conflict,
    /// Another live application owner holds this profile's advisory lock.
    ProfileInUse,
    /// The owner stopped or could not be started.
    Closed,
    /// A UID/provider identity conflicts with cached mail.
    IdentityConflict,
    /// A malformed or oversized request was rejected.
    InvalidRequest,
    /// The original is missing or failed integrity checks.
    MessageUnavailable,
    /// The original cannot be projected within the MIME budget.
    UnsupportedMessage,
    /// The database requires a different application/schema or migration repair.
    Schema,
    /// The mailbox generation changed; resynchronize before retrying.
    StaleMailbox,
    /// Local filesystem/database access failed.
    Storage,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Conflict => "mail operation changed; refresh before retrying",
            Self::ProfileInUse => "another Mailstronaut instance is using this profile",
            Self::Closed => "mail storage worker unavailable",
            Self::IdentityConflict => "server identity conflicts with cached mail",
            Self::InvalidRequest => "invalid or oversized mail storage request",
            Self::MessageUnavailable => "cached message unavailable or damaged",
            Self::UnsupportedMessage => "message exceeds supported MIME limits",
            Self::Schema => "mail database upgrade or recovery required",
            Self::StaleMailbox => "mailbox generation changed; resynchronization required",
            Self::Storage => "mail storage operation failed",
        })
    }
}
impl StdError for Error {}
impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        match error {
            store::Error::OperationConflict => Self::Conflict,
            store::Error::IdentityConflict => Self::IdentityConflict,
            store::Error::InvalidInput => Self::InvalidRequest,
            store::Error::MissingMessage | store::Error::CorruptBlob => Self::MessageUnavailable,
            store::Error::StaleMailbox => Self::StaleMailbox,
            store::Error::UnsupportedSchema | store::Error::Migration(_) => Self::Schema,
            store::Error::Mime(_) => Self::UnsupportedMessage,
            store::Error::Io(_) | store::Error::Sql(_) | store::Error::Metadata(_) => Self::Storage,
        }
    }
}

use crate::store::flags::{Attempt, Counts};
use mailstronaut_mail::imap::{FlagChange, Metadata};

type Reply<T> = oneshot::Sender<Result<T, Error>>;

enum Command {
    ReminderAction(
        Vec<String>,
        String,
        i64,
        Reply<Option<store::reminders::Reminder>>,
    ),
    Reminders(Vec<String>, Reply<Vec<store::reminders::Reminder>>),
    ReminderFor(String, i64, Reply<Option<store::reminders::Reminder>>),
    NextReminder(Vec<String>, Reply<Option<i64>>),
    ScheduleReminder(
        store::reminders::Schedule,
        Reply<store::reminders::Reminder>,
    ),
    EditReminder(String, String, i64, bool, Reply<()>),
    ClaimReminder(Vec<String>, i64, Reply<Option<store::reminders::Attempt>>),
    FinishReminder(store::reminders::Attempt, bool, i64, Reply<()>),
    ReminderWithdrawals(Vec<String>, Reply<Vec<store::reminders::Reminder>>),
    WithdrewReminder(store::reminders::Reminder, Reply<()>),

    ConversationSnapshot(String, Reply<Option<store::conversations::Snapshot>>),
    CommitConversations(
        store::conversations::Snapshot,
        crate::threads::Groups,
        Reply<()>,
    ),
    ConversationPage(
        Vec<String>,
        store::browse::Scope,
        String,
        Option<Cursor>,
        u32,
        Reply<Vec<store::conversations::Summary>>,
    ),
    Folders(Vec<String>, Reply<Vec<Folder>>),
    RememberFolders(String, Vec<String>, String, Reply<()>),
    ConversationMessages(String, i64, Reply<Vec<Summary>>),
    ConversationMemberPage(String, i64, Option<Cursor>, u32, Reply<Vec<Summary>>),
    BindContext(String, String, Reply<bool>),
    UndoFlags(String, i64, Reply<()>),
    SetFlag(String, i64, FlagChange, bool, String, Reply<()>),
    BeginFlag(String, String, Reply<Option<Attempt>>),
    ConfirmFlag(Attempt, Metadata, Reply<()>),
    FailFlag(Attempt, bool, Reply<()>),
    RetryFlags(String, String, Reply<()>),
    FlagCounts(Vec<String>, Reply<Counts>),
    RecoverDraft(String, String, Reply<Draft>),
    SaveDraft(Draft, Reply<()>),
    AddAttachments(
        Draft,
        Vec<mailstronaut_mail::compose::AttachmentData>,
        Reply<Draft>,
    ),
    RemoveAttachment(Draft, String, Reply<Draft>),
    CopyDraft(Draft, String, String, Reply<Draft>),
    Draft(String, String, Reply<Draft>),
    Drafts(Vec<String>, Reply<Vec<DraftSummary>>),
    QueueDraft(String, String, u64, bool, Reply<outbox::Summary>),
    BeginSubmission(String, String, Reply<Submission>),
    FinishSubmission(Submission, mailstronaut_mail::smtp::Outcome, Reply<()>),
    RetrySubmission(String, String, bool, Reply<()>),
    CancelSubmission(String, String, Reply<()>),
    Outbox(Vec<String>, Reply<Vec<outbox::Summary>>),
    Account(String, Reply<()>),
    Folder(String, String, bool, Reply<Folder>),
    Select(String, i64, core::num::NonZeroU32, Reply<()>),
    Cache {
        account: String,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        original: Vec<u8>,
        reply: Reply<i64>,
    },
    Inbox(Vec<String>, Option<Cursor>, u32, Reply<Vec<Summary>>),
    Message(String, i64, Reply<Projection>),
    Observe(String, Membership, Reply<bool>),
    Reconcile(String, UidSnapshot, Reply<()>),
    Shutdown(oneshot::Sender<()>),
}

/// Cloneable request handle; no SQLite connection or filesystem work runs here.
#[derive(Clone)]
pub struct Handle(mpsc::Sender<Command>);
impl Handle {
    /// Resolve only a current notification action in enabled accounts.
    /// # Errors
    /// Rejects invalid scope and storage failures.
    pub async fn reminder_action(
        &self,
        accounts: Vec<String>,
        notification: String,
        generation: i64,
    ) -> Result<Option<store::reminders::Reminder>, Error> {
        self.request(|reply| Command::ReminderAction(accounts, notification, generation, reply))
            .await
    }

    /// List at most 100 reminder records for enabled accounts.
    /// # Errors
    /// Returns request/storage failures.
    pub async fn reminders(
        &self,
        accounts: Vec<String>,
    ) -> Result<Vec<store::reminders::Reminder>, Error> {
        self.request(|reply| Command::Reminders(accounts, reply))
            .await
    }
    /// Resolve one exact original's reminder, including cancelled entries.
    /// # Errors
    /// Returns request/storage failures.
    pub async fn reminder_for(
        &self,
        account: String,
        message: i64,
    ) -> Result<Option<store::reminders::Reminder>, Error> {
        self.request(|reply| Command::ReminderFor(account, message, reply))
            .await
    }
    /// Read the earliest pending instant independently of the visible page.
    /// # Errors
    /// Returns request/storage failures.
    pub async fn next_reminder(&self, accounts: Vec<String>) -> Result<Option<i64>, Error> {
        self.request(|reply| Command::NextReminder(accounts, reply))
            .await
    }
    /// Durably create/reschedule with exact generation comparison.
    /// # Errors
    /// Rejects stale edits, invalid times and unavailable targets.
    pub async fn schedule_reminder(
        &self,
        request: store::reminders::Schedule,
    ) -> Result<store::reminders::Reminder, Error> {
        self.request(|reply| Command::ScheduleReminder(request, reply))
            .await
    }
    /// Cancel or explicitly retry the exact current reminder revision.
    /// # Errors
    /// Rejects stale/foreign edits and storage failures.
    pub async fn edit_reminder(
        &self,
        account: String,
        id: String,
        generation: i64,
        retry: bool,
    ) -> Result<(), Error> {
        self.request(|reply| Command::EditReminder(account, id, generation, retry, reply))
            .await
    }
    /// Commit a fenced due notification claim before desktop I/O.
    /// # Errors
    /// Returns request/storage failures.
    pub async fn claim_reminder(
        &self,
        accounts: Vec<String>,
        now: i64,
    ) -> Result<Option<store::reminders::Attempt>, Error> {
        self.request(|reply| Command::ClaimReminder(accounts, now, reply))
            .await
    }
    /// Commit the exact desktop attempt's outcome.
    /// # Errors
    /// Rejects stale callbacks and storage failures.
    pub async fn finish_reminder(
        &self,
        attempt: store::reminders::Attempt,
        accepted: bool,
        now: i64,
    ) -> Result<(), Error> {
        self.request(|reply| Command::FinishReminder(attempt, accepted, now, reply))
            .await
    }
    /// Read bounded crash-safe withdrawal work, including cancelled records.
    /// # Errors
    /// Returns request/storage failures.
    pub async fn reminder_withdrawals(
        &self,
        accounts: Vec<String>,
    ) -> Result<Vec<store::reminders::Reminder>, Error> {
        self.request(|reply| Command::ReminderWithdrawals(accounts, reply))
            .await
    }
    /// Commit a completed withdrawal for exactly this generation.
    /// # Errors
    /// Rejects stale callbacks and storage failures.
    pub async fn withdrew_reminder(&self, record: store::reminders::Reminder) -> Result<(), Error> {
        self.request(|reply| Command::WithdrewReminder(record, reply))
            .await
    }

    /// Read stale account metadata for an isolated RFC projection, without bodies.
    /// # Errors
    /// Rejects metadata budgets, corrupt indexes or unavailable storage.
    pub async fn conversation_snapshot(
        &self,
        account: String,
    ) -> Result<Option<store::conversations::Snapshot>, Error> {
        self.request(|reply| Command::ConversationSnapshot(account, reply))
            .await
    }
    /// Commit exact complete helper output only if its account revision still matches.
    /// # Errors
    /// Rejects stale/mixed/incomplete output and storage failures.
    pub async fn commit_conversations(
        &self,
        snapshot: store::conversations::Snapshot,
        groups: crate::threads::Groups,
    ) -> Result<(), Error> {
        self.request(|reply| Command::CommitConversations(snapshot, groups, reply))
            .await
    }
    /// Read at most 100 account-local conversation rows with unread/star aggregates.
    /// # Errors
    /// Rejects excessive account/page selections and storage failures.
    pub async fn conversation_inbox(
        &self,
        accounts: Vec<String>,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<store::conversations::Summary>, Error> {
        self.conversation_page(
            accounts,
            store::browse::Scope::Inbox,
            String::new(),
            after,
            limit,
        )
        .await
    }
    /// Read a bounded local conversation page in an enabled account/folder scope.
    /// # Errors
    /// Rejects scope/query/page limits and storage failures.
    pub async fn conversation_page(
        &self,
        accounts: Vec<String>,
        scope: store::browse::Scope,
        query: String,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<store::conversations::Summary>, Error> {
        self.request(|reply| Command::ConversationPage(accounts, scope, query, after, limit, reply))
            .await
    }
    /// List known folders for enabled accounts without contacting a mail server.
    /// # Errors
    /// Rejects excessive results and storage failures.
    pub async fn folders(&self, accounts: Vec<String>) -> Result<Vec<Folder>, Error> {
        self.request(|reply| Command::Folders(accounts, reply))
            .await
    }
    /// Persist a bounded authenticated LIST result while retaining offline folders.
    /// # Errors
    /// Rejects invalid names and failed durable writes.
    pub async fn remember_folders(
        &self,
        account: String,
        names: Vec<String>,
        inbox: String,
    ) -> Result<(), Error> {
        self.request(|reply| Command::RememberFolders(account, names, inbox, reply))
            .await
    }
    /// Read bounded cached conversation members, including Sent/Archive ancestry.
    /// # Errors
    /// Rejects unknown/cross-account messages and unavailable storage.
    pub async fn conversation_messages(
        &self,
        account: String,
        message: i64,
    ) -> Result<Vec<Summary>, Error> {
        self.request(|reply| Command::ConversationMessages(account, message, reply))
            .await
    }

    /// Read a bounded keyset page of cached originals, without opening any body.
    /// # Errors
    /// Rejects invalid/foreign identities, excessive pages and storage failures.
    pub async fn conversation_member_page(
        &self,
        account: String,
        message: i64,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<Summary>, Error> {
        self.request(|reply| Command::ConversationMemberPage(account, message, after, limit, reply))
            .await
    }

    /// Bind cached namespaces after authenticating this configuration.
    /// # Errors
    /// Returns invalid-context, account or storage failures.
    pub async fn bind_mailbox_context(
        &self,
        account: String,
        context: String,
    ) -> Result<bool, Error> {
        self.request(|reply| Command::BindContext(account, context, reply))
            .await
    }
    /// Persist an offline read/star intention before acknowledging the UI.
    /// # Errors
    /// Rejects stale server provenance, missing messages and storage failures.
    pub async fn set_flag(
        &self,
        account: String,
        message: i64,
        flag: FlagChange,
        desired: bool,
        context: String,
    ) -> Result<(), Error> {
        self.request(|reply| Command::SetFlag(account, message, flag, desired, context, reply))
            .await
    }
    /// Undo a message's local flag intentions only before any network attempt.
    /// # Errors
    /// Rejects attempted/missing work and storage failures.
    pub async fn undo_flags(&self, account: String, message: i64) -> Result<(), Error> {
        self.request(|reply| Command::UndoFlags(account, message, reply))
            .await
    }
    /// Commit and claim one exact UID flag assignment.
    /// # Errors
    /// Rejects concurrent claims, invalid context and storage failures.
    pub async fn begin_flag(
        &self,
        account: String,
        context: String,
    ) -> Result<Option<Attempt>, Error> {
        self.request(|reply| Command::BeginFlag(account, context, reply))
            .await
    }
    /// Persist a fresh server confirmation for the exact attempt token.
    /// # Errors
    /// Rejects stale/mismatched confirmations and storage failures.
    pub async fn confirm_flag(&self, attempt: Attempt, observed: Metadata) -> Result<(), Error> {
        self.request(|reply| Command::ConfirmFlag(attempt, observed, reply))
            .await
    }
    /// Retain a failed/retired flag without implicitly retrying it.
    /// # Errors
    /// Rejects stale callbacks and storage failures.
    pub async fn fail_flag(&self, attempt: Attempt, conflict: bool) -> Result<(), Error> {
        self.request(|reply| Command::FailFlag(attempt, conflict, reply))
            .await
    }
    /// Explicitly retry failed idempotent flags, never SMTP submissions.
    /// # Errors
    /// Rejects invalid context and storage failures.
    pub async fn retry_flags(&self, account: String, context: String) -> Result<(), Error> {
        self.request(|reply| Command::RetryFlags(account, context, reply))
            .await
    }
    /// Read enabled accounts' pending/failed/retired flag counts.
    /// # Errors
    /// Rejects oversized selections and storage failures.
    pub async fn flag_counts(&self, accounts: Vec<String>) -> Result<Counts, Error> {
        self.request(|reply| Command::FlagCounts(accounts, reply))
            .await
    }

    /// Recover a definite SMTP rejection as a new editable draft, cancelling its
    /// old immutable intent. Unknown/accepted mail is never implicitly reopened.
    ///
    /// # Errors
    /// Returns a phase/account/integrity or storage failure.
    pub async fn recover_draft(&self, account: String, id: String) -> Result<Draft, Error> {
        self.request(|reply| Command::RecoverDraft(account, id, reply))
            .await
    }
    /// Persist one editor revision without blocking the UI.
    ///
    /// # Errors
    /// Rejects oversized input, stale/closed drafts and unavailable storage.
    pub async fn save_draft(&self, draft: Draft) -> Result<(), Error> {
        if draft.content.body.len() > mailstronaut_mail::compose::MAX_BODY_BYTES
            || draft.content.references.len() > 256
        {
            return Err(Error::InvalidRequest);
        }
        let size = [
            &draft.content.from,
            &draft.content.to,
            &draft.content.cc,
            &draft.content.bcc,
            &draft.content.subject,
        ]
        .into_iter()
        .chain(&draft.content.references)
        .try_fold(draft.content.body.len(), |total, value| {
            total.checked_add(value.len())
        });
        if size.is_none_or(|size| size > outbox::MAX_DRAFT_BYTES) {
            return Err(Error::InvalidRequest);
        }
        self.request(|reply| Command::SaveDraft(draft, reply)).await
    }
    /// Add owned files on the blocking storage owner.
    /// # Errors
    /// Rejects stale drafts, oversized files and storage failures.
    pub async fn add_attachments(
        &self,
        draft: Draft,
        files: Vec<mailstronaut_mail::compose::AttachmentData>,
    ) -> Result<Draft, Error> {
        mailstronaut_mail::compose::validate_attachments(&files)
            .map_err(|_| Error::InvalidRequest)?;
        self.request(|reply| Command::AddAttachments(draft, files, reply))
            .await
    }
    /// Remove one exact draft-local file.
    /// # Errors
    /// Rejects stale drafts, unknown identifiers and storage failures.
    pub async fn remove_attachment(&self, draft: Draft, id: String) -> Result<Draft, Error> {
        self.request(|reply| Command::RemoveAttachment(draft, id, reply))
            .await
    }
    /// Retain file snapshots when the user switches sending accounts.
    /// # Errors
    /// Rejects stale drafts, unknown accounts and storage failures.
    pub async fn copy_draft(
        &self,
        draft: Draft,
        account: String,
        from: String,
    ) -> Result<Draft, Error> {
        self.request(|reply| Command::CopyDraft(draft, account, from, reply))
            .await
    }
    /// Load an editable, account-scoped saved draft.
    ///
    /// # Errors
    /// Returns a missing, invalid or unavailable-storage error.
    pub async fn draft(&self, account: String, id: String) -> Result<Draft, Error> {
        self.request(|reply| Command::Draft(account, id, reply))
            .await
    }
    /// List at most 100 drafts belonging to enabled accounts.
    ///
    /// # Errors
    /// Returns a request/metadata/storage error.
    pub async fn drafts(&self, accounts: Vec<String>) -> Result<Vec<DraftSummary>, Error> {
        self.request(|reply| Command::Drafts(accounts, reply)).await
    }
    /// Freeze exactly the displayed, committed editor revision.
    ///
    /// # Errors
    /// Rejects stale/closed/invalid drafts and failed durable writes.
    pub async fn queue_draft(
        &self,
        account: String,
        id: String,
        revision: u64,
        server_saves_sent: bool,
    ) -> Result<outbox::Summary, Error> {
        self.request(|reply| Command::QueueDraft(account, id, revision, server_saves_sent, reply))
            .await
    }
    /// Persist submission intent before returning the original to the network task.
    ///
    /// # Errors
    /// Rejects non-pending/wrong-account/damaged records.
    pub async fn begin_submission(&self, account: String, id: String) -> Result<Submission, Error> {
        self.request(|reply| Command::BeginSubmission(account, id, reply))
            .await
    }
    /// Persist only this claimed attempt's result; never retry Unknown implicitly.
    ///
    /// # Errors
    /// Rejects stale attempts and failed durable writes.
    pub async fn finish_submission(
        &self,
        submission: Submission,
        outcome: mailstronaut_mail::smtp::Outcome,
    ) -> Result<(), Error> {
        self.request(|reply| Command::FinishSubmission(submission, outcome, reply))
            .await
    }
    /// Explicit retry of the same frozen original, with separate Unknown consent.
    ///
    /// # Errors
    /// Rejects accepted, active or unconfirmed unknown records.
    pub async fn retry_submission(
        &self,
        account: String,
        id: String,
        confirm_unknown: bool,
    ) -> Result<(), Error> {
        self.request(|reply| Command::RetrySubmission(account, id, confirm_unknown, reply))
            .await
    }
    /// Cancel a pending intent and recover its draft before network submission.
    ///
    /// # Errors
    /// Rejects in-flight, accepted or unknown records.
    pub async fn cancel_submission(&self, account: String, id: String) -> Result<(), Error> {
        self.request(|reply| Command::CancelSubmission(account, id, reply))
            .await
    }
    /// List bounded outgoing state for enabled accounts.
    ///
    /// # Errors
    /// Returns a request/metadata/storage error.
    pub async fn outbox(&self, accounts: Vec<String>) -> Result<Vec<outbox::Summary>, Error> {
        self.request(|reply| Command::Outbox(accounts, reply)).await
    }
    /// Apply server flags without rereading a cached original.
    ///
    /// # Errors
    /// Returns a redacted identity, generation or worker failure.
    pub async fn observe_membership(
        &self,
        account: String,
        membership: Membership,
    ) -> Result<bool, Error> {
        self.request(|reply| Command::Observe(account, membership, reply))
            .await
    }
    /// Retire memberships absent from a complete, unfiltered UID search.
    ///
    /// # Errors
    /// Returns a redacted invalid/stale snapshot or storage failure.
    pub async fn reconcile_span(
        &self,
        account: String,
        snapshot: UidSnapshot,
    ) -> Result<(), Error> {
        self.request(|reply| Command::Reconcile(account, snapshot, reply))
            .await
    }
    async fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, Error> {
        let (reply, response) = oneshot::channel();
        self.0
            .send(command(reply))
            .await
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }

    /// Register an active account without deleting disabled accounts' data.
    ///
    /// # Errors
    /// Returns a redacted worker or storage error.
    pub async fn ensure_account(&self, account: String) -> Result<(), Error> {
        self.request(|reply| Command::Account(account, reply)).await
    }
    /// Cache a discovered folder, preserving a previously selected generation.
    ///
    /// # Errors
    /// Returns a redacted worker, identity or storage error.
    pub async fn ensure_folder(
        &self,
        account: String,
        name: String,
        inbox: bool,
    ) -> Result<Folder, Error> {
        self.request(|reply| Command::Folder(account, name, inbox, reply))
            .await
    }
    /// Apply SELECT's UID namespace before body/flag work is accepted.
    ///
    /// # Errors
    /// Returns a redacted worker, identity or storage error.
    pub async fn select_folder(
        &self,
        account: String,
        folder: i64,
        validity: core::num::NonZeroU32,
    ) -> Result<(), Error> {
        self.request(|reply| Command::Select(account, folder, validity, reply))
            .await
    }
    /// Enqueue one bounded original. Backpressure suspends the caller without
    /// blocking the GUI or retaining an unbounded number of message bodies.
    ///
    /// # Errors
    /// Returns a redacted size, UID generation, identity or storage error.
    pub async fn cache_message(
        &self,
        account: String,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        original: Vec<u8>,
    ) -> Result<i64, Error> {
        if original.len() > mime::MAX_INPUT_BYTES {
            return Err(Error::InvalidRequest);
        }
        self.request(|reply| Command::Cache {
            account,
            membership,
            received_at,
            gmail,
            original,
            reply,
        })
        .await
    }
    /// Obtain an immutable, bounded page for currently enabled accounts.
    ///
    /// # Errors
    /// Returns a redacted request, storage or worker error.
    pub async fn inbox(
        &self,
        accounts: Vec<String>,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<Summary>, Error> {
        self.request(|reply| Command::Inbox(accounts, after, limit, reply))
            .await
    }
    /// Read a verified original for exactly this account/message identity.
    ///
    /// # Errors
    /// Returns a redacted integrity, storage or worker error.
    pub async fn message(&self, account: String, id: i64) -> Result<Projection, Error> {
        self.request(|reply| Command::Message(account, id, reply))
            .await
    }
}

/// Lifetime of the single blocking database owner.
///
/// Retain this in the engine;
/// give only its Handle to consumers. Call shutdown before the Tokio driver ends.
/// Dropping the last handle closes the channel and lets the thread finish queued
/// work; explicit shutdown also waits for that thread off the GUI thread.
pub struct Worker {
    handle: Handle,
    thread: thread::JoinHandle<()>,
}
impl Worker {
    /// Open/migrate the store on its owning thread. No raw connection crosses
    /// the async boundary; a failed startup is joined before returning the error.
    ///
    /// # Errors
    /// Returns a redacted open/migration/thread error without resetting data.
    pub async fn open(root: PathBuf) -> Result<Self, Error> {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (ready, opened) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("mailstronaut-store".into())
            .spawn(move || match open_profile(&root) {
                Ok((_profile, mut store)) => {
                    if ready.send(Ok(())).is_ok() {
                        run(&mut store, receiver);
                    }
                }
                Err(error) => {
                    let _sent = ready.send(Err(error));
                }
            })
            .map_err(|_| Error::Closed)?;
        match opened.await.unwrap_or(Err(Error::Closed)) {
            Ok(()) => Ok(Self {
                handle: Handle(sender),
                thread,
            }),
            Err(error) => {
                let _joined = tokio::task::spawn_blocking(move || thread.join()).await;
                Err(error)
            }
        }
    }
    /// A lightweight clone for the engine and immutable view queries.
    #[must_use]
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    /// Stop after previously queued commands and await the owner thread. Any
    /// consumer requests queued after Shutdown fail closed when its receiver drops.
    ///
    /// # Errors
    /// Returns Closed if the owner panicked or had already exited.
    pub async fn shutdown(self) -> Result<(), Error> {
        let (reply, response) = oneshot::channel();
        let sent = self.handle.0.send(Command::Shutdown(reply)).await.is_ok();
        let acknowledged = sent && response.await.is_ok();
        tokio::task::spawn_blocking(move || self.thread.join())
            .await
            .map_err(|_| Error::Closed)?
            .map_err(|_| Error::Closed)?;
        if acknowledged {
            Ok(())
        } else {
            Err(Error::Closed)
        }
    }
}

// The application owner holds this lock before migration/recovery and until its
// queue drains. Independent Store handles used for integrity/GC retain their
// SQLite transaction coordination, but cannot become a second app worker.
fn open_profile(root: &Path) -> Result<(fs::File, Store), Error> {
    fs::create_dir_all(root).map_err(|_| Error::Storage)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Storage)?;
    let lock = fs::File::from(
        rustix::fs::open(
            root.join("profile.lock"),
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::from_bits_truncate(0o600),
        )
        .map_err(|_| Error::Storage)?,
    );
    if !lock.metadata().map_err(|_| Error::Storage)?.is_file() {
        return Err(Error::Storage);
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(
        |error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                Error::ProfileInUse
            } else {
                Error::Storage
            }
        },
    )?;
    let store = Store::open(root).map_err(Error::from)?;
    store.recover_submissions().map_err(Error::from)?;
    store.recover_flags().map_err(Error::from)?;
    store.recover_reminders().map_err(Error::from)?;
    Ok((lock, store))
}

fn respond<T>(reply: Reply<T>, operation: impl FnOnce() -> Result<T, store::Error>) {
    // A discarded read/cache request need not perform expensive parsing or I/O.
    // Once a transaction starts it completes atomically even if its caller goes.
    if !reply.is_closed() {
        let _sent = reply.send(operation().map_err(Error::from));
    }
}
fn commit<T>(reply: Reply<T>, operation: impl FnOnce() -> Result<T, store::Error>) {
    // Once a durable edit/result is queued, closing its UI waiter must not lose
    // the draft or a known SMTP acknowledgment. The channel remains bounded.
    let _sent = reply.send(operation().map_err(Error::from));
}
fn claim(store: &mut Store, account: &str, id: &str, reply: Reply<Submission>) {
    if reply.is_closed() {
        return;
    }
    let result = store.begin_submission(account, id).map_err(Error::from);
    if let Err(Ok(submission)) = reply.send(result) {
        // The network task never obtained the claim, so no DATA could be sent.
        let _cancelled = store.finish_submission(
            &submission,
            mailstronaut_mail::smtp::Outcome::CancelledBeforeSubmission,
        );
    }
}
fn claim_flag(store: &mut Store, account: &str, context: &str, reply: Reply<Option<Attempt>>) {
    if reply.is_closed() {
        return;
    }
    let result = store.begin_flag(account, context).map_err(Error::from);
    if let Err(Ok(Some(attempt))) = reply.send(result) {
        // No network caller obtained this claim. Clear its in-flight fence;
        // explicit retry remains safe and no account stays permanently busy.
        let _failed = store.fail_flag(&attempt, false);
    }
}
#[expect(
    clippy::too_many_lines,
    reason = "Keep the single serialized command dispatch visible; individual operations live in store modules."
)]
fn run(store: &mut Store, mut receiver: mpsc::Receiver<Command>) {
    while let Some(command) = receiver.blocking_recv() {
        match command {
            Command::ReminderAction(accounts, notification, generation, reply) => {
                respond(reply, || {
                    store.reminder_action(&accounts, &notification, generation)
                });
            }
            Command::Reminders(accounts, reply) => respond(reply, || store.reminders(&accounts)),
            Command::ReminderFor(account, message, reply) => {
                respond(reply, || store.reminder_for(&account, message));
            }
            Command::NextReminder(accounts, reply) => {
                respond(reply, || store.next_reminder(&accounts));
            }
            Command::ScheduleReminder(request, reply) => {
                commit(reply, || store.schedule_reminder(&request));
            }
            Command::EditReminder(account, id, generation, retry, reply) => commit(reply, || {
                store.edit_reminder(&account, &id, generation, retry)
            }),
            Command::ClaimReminder(accounts, now, reply) => {
                if !reply.is_closed() {
                    let result = store.claim_reminder(&accounts, now).map_err(Error::from);
                    if let Err(Ok(Some(attempt))) = reply.send(result) {
                        let _released = store.release_reminder(&attempt);
                    }
                }
            }
            Command::FinishReminder(attempt, accepted, now, reply) => {
                commit(reply, || store.finish_reminder(&attempt, accepted, now));
            }
            Command::ReminderWithdrawals(accounts, reply) => {
                respond(reply, || store.reminder_withdrawals(&accounts));
            }
            Command::WithdrewReminder(record, reply) => {
                commit(reply, || store.withdrew_reminder(&record));
            }
            Command::ConversationSnapshot(account, reply) => {
                respond(reply, || store.conversation_snapshot(&account));
            }
            Command::CommitConversations(snapshot, groups, reply) => {
                commit(reply, || store.commit_conversations(&snapshot, &groups));
            }
            Command::ConversationPage(accounts, scope, query, after, limit, reply) => {
                respond(reply, || {
                    store.conversation_page(&accounts, &scope, &query, after, limit)
                });
            }
            Command::Folders(accounts, reply) => respond(reply, || store.folders(&accounts)),
            Command::RememberFolders(account, names, inbox, reply) => {
                commit(reply, || store.remember_folders(&account, &names, &inbox));
            }
            Command::ConversationMessages(account, message, reply) => {
                respond(reply, || store.conversation_messages(&account, message));
            }
            Command::ConversationMemberPage(account, message, after, limit, reply) => {
                _ = reply.send(
                    store
                        .conversation_member_page(&account, message, after, limit)
                        .map_err(Into::into),
                );
            }
            Command::BindContext(account, context, reply) => {
                commit(reply, || store.bind_mailbox_context(&account, &context));
            }
            Command::UndoFlags(account, message, reply) => {
                commit(reply, || store.undo_flags(&account, message));
            }
            Command::SetFlag(account, message, flag, desired, context, reply) => {
                commit(reply, || {
                    store.set_flag(&account, message, flag, desired, &context)
                });
            }
            Command::BeginFlag(account, context, reply) => {
                claim_flag(store, &account, &context, reply);
            }
            Command::ConfirmFlag(attempt, observed, reply) => {
                commit(reply, || store.confirm_flag(&attempt, &observed));
            }
            Command::FailFlag(attempt, conflict, reply) => {
                commit(reply, || store.fail_flag(&attempt, conflict));
            }
            Command::RetryFlags(account, context, reply) => {
                commit(reply, || store.retry_flags(&account, &context));
            }
            Command::FlagCounts(accounts, reply) => respond(reply, || store.flag_counts(&accounts)),
            Command::RecoverDraft(account, id, reply) => {
                commit(reply, || store.recover_draft(&account, &id));
            }
            Command::SaveDraft(draft, reply) => commit(reply, || store.save_draft(&draft)),
            Command::AddAttachments(draft, files, reply) => {
                commit(reply, || store.add_draft_attachments(&draft, files));
            }
            Command::RemoveAttachment(draft, id, reply) => {
                commit(reply, || store.remove_draft_attachment(&draft, &id));
            }
            Command::CopyDraft(draft, account, from, reply) => {
                commit(reply, || store.copy_draft(&draft, &account, from));
            }
            Command::Draft(account, id, reply) => respond(reply, || store.draft(&account, &id)),
            Command::Drafts(accounts, reply) => respond(reply, || store.drafts(&accounts)),
            Command::QueueDraft(account, id, revision, server_saves_sent, reply) => {
                commit(reply, || {
                    store.queue_draft(&account, &id, revision, server_saves_sent)
                });
            }
            Command::BeginSubmission(account, id, reply) => {
                claim(store, &account, &id, reply);
            }
            Command::FinishSubmission(submission, outcome, reply) => {
                commit(reply, || store.finish_submission(&submission, outcome));
            }
            Command::RetrySubmission(account, id, confirm, reply) => {
                commit(reply, || store.retry_submission(&account, &id, confirm));
            }
            Command::CancelSubmission(account, id, reply) => {
                commit(reply, || store.cancel_submission(&account, &id));
            }
            Command::Outbox(accounts, reply) => respond(reply, || store.outbox(&accounts)),
            Command::Observe(account, membership, reply) => {
                respond(reply, || store.observe_membership(&account, membership));
            }
            Command::Reconcile(account, snapshot, reply) => {
                respond(reply, || store.reconcile_span(&account, &snapshot));
            }
            Command::Account(account, reply) => respond(reply, || store.ensure_account(&account)),
            Command::Folder(account, name, inbox, reply) => {
                respond(reply, || store.ensure_folder(&account, &name, inbox));
            }
            Command::Select(account, folder, validity, reply) => {
                respond(reply, || store.select_folder(&account, folder, validity));
            }
            Command::Cache {
                account,
                membership,
                received_at,
                gmail,
                original,
                reply,
            } => respond(reply, || {
                store.cache_message(&account, membership, received_at, gmail, &original)
            }),
            Command::Inbox(accounts, cursor, limit, reply) => {
                respond(reply, || store.inbox(&accounts, cursor, limit));
            }
            Command::Message(account, id, reply) => respond(reply, || store.message(&account, id)),
            Command::Shutdown(reply) => {
                let _sent = reply.send(());
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn accepted_draft_and_smtp_result_commands_survive_dropped_ui_waiters() {
        let root = tempfile::tempdir().expect("fixture");
        let owner = Worker::open(root.path().to_owned()).await.expect("owner");
        let handle = owner.handle();
        handle.ensure_account("a".into()).await.expect("account");
        let mut draft = Draft::new("a".into(), "sender@example.test".into());
        draft.content.to = "to@example.test".into();
        draft.content.body = "Durable synthetic edit.".into();
        let (reply, waiter) = oneshot::channel();
        drop(waiter);
        handle
            .0
            .send(Command::SaveDraft(draft.clone(), reply))
            .await
            .expect("accepted save command");
        assert_eq!(
            handle
                .draft("a".into(), draft.id.clone())
                .await
                .expect("FIFO barrier")
                .content
                .body,
            "Durable synthetic edit."
        );
        let queued = handle
            .queue_draft("a".into(), draft.id, draft.revision, false)
            .await
            .expect("freeze");
        let submission = handle
            .begin_submission("a".into(), queued.id.clone())
            .await
            .expect("claim");
        let (reply, waiter) = oneshot::channel();
        drop(waiter);
        handle
            .0
            .send(Command::FinishSubmission(
                submission,
                mailstronaut_mail::smtp::Outcome::Accepted,
                reply,
            ))
            .await
            .expect("accepted delivery result");
        assert_eq!(
            handle.outbox(vec!["a".into()]).await.expect("FIFO barrier")[0].phase,
            outbox::Phase::Accepted,
            "known DATA acknowledgment must not turn ambiguous because a view closed"
        );
        owner.shutdown().await.expect("shutdown");
    }
}
