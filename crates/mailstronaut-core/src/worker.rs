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
        mailbox::{Cursor, Folder, FolderRole, GmailId, Membership, Summary, UidSnapshot},
        moves::MoveAttempt,
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
    FilteredPage(
        store::browse::FilteredPage,
        Reply<Vec<store::conversations::Summary>>,
    ),
    UnreadCounts(Vec<String>, Reply<Vec<(i64, u32)>>),
    RankedPage(
        Vec<String>,
        store::browse::Scope,
        Vec<i64>,
        Option<Cursor>,
        u32,
        Reply<Vec<store::conversations::Summary>>,
    ),
    SearchPending(u32, Reply<store::search::SearchBatch>),
    SearchIndexed(i64, Reply<()>),
    RequeueSearch(Reply<()>),
    SemanticPending(u32, Reply<store::semantic::Batch>),
    SemanticIndexed(i64, Reply<()>),
    RequeueSemantic(Reply<()>),
    SemanticCandidates(
        Vec<String>,
        store::browse::Scope,
        store::browse::SearchFilter,
        Reply<std::collections::BTreeMap<i64, String>>,
    ),
    AnalysisPending(
        Vec<String>,
        u32,
        i64,
        u32,
        Reply<Vec<store::analysis::AnalysisInput>>,
    ),
    StoreAnalysis(Vec<store::analysis::AnalysisResult>, Reply<()>),
    PriorityItems(
        Vec<String>,
        i64,
        u32,
        Reply<Vec<store::analysis::PriorityItem>>,
    ),
    DismissPriority(i64, Reply<()>),
    AnalysisData(i64, Reply<Option<String>>),
    NewInboxMail(
        String,
        Option<i64>,
        i64,
        u32,
        Reply<(i64, Vec<store::analysis::NewMail>)>,
    ),
    CacheReduced {
        account: String,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        reduced: Vec<u8>,
        omitted: crate::outline::Omitted,
        reply: Reply<i64>,
    },
    Omitted(String, i64, Reply<Option<crate::outline::Omitted>>),
    Location(
        String,
        i64,
        Reply<Option<(String, core::num::NonZeroU32, core::num::NonZeroU32)>>,
    ),
    RememberFolders(String, Vec<String>, String, Reply<()>),
    AssignFolderRoles(String, Vec<(String, FolderRole)>, Reply<()>),
    QueueMove(String, i64, Option<i64>, String, String, i64, Reply<u32>),
    UndoMove(String, i64, Reply<()>),
    BeginMove(String, String, Reply<Option<MoveAttempt>>),
    FinishMove(MoveAttempt, bool, Reply<()>),
    AbandonMove(MoveAttempt, Reply<()>),
    RetryMoves(String, Reply<()>),
    PendingSentCopy(String, Reply<Option<outbox::SentCopy>>),
    FinishSentCopy(String, String, Reply<()>),
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
    DiscardDraft(String, String, Reply<()>),
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
    BackfillPass(String, i64, Reply<Option<store::backfill::Pass>>),
    Coverage(Vec<String>, Reply<Vec<store::backfill::FolderCoverage>>),
    SaveBackfill(String, i64, store::backfill::Pass, Reply<()>),
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
    /// Read a conversation page of messages an index has matched, in the given order.
    /// # Errors
    /// Rejects scope and page limits and storage failures.
    pub async fn ranked_conversation_page(
        &self,
        accounts: Vec<String>,
        scope: store::browse::Scope,
        hits: Vec<i64>,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<store::conversations::Summary>, Error> {
        self.request(|reply| Command::RankedPage(accounts, scope, hits, after, limit, reply))
            .await
    }
    /// Read originals matching explicit metadata restrictions and optional index hits.
    /// # Errors
    /// Rejects invalid filters, scopes, limits and storage failures.
    pub async fn filtered_conversation_page(
        &self,
        request: store::browse::FilteredPage,
    ) -> Result<Vec<store::conversations::Summary>, Error> {
        self.request(|reply| Command::FilteredPage(request, reply))
            .await
    }
    /// Read the oldest changes waiting for the search index.
    /// # Errors
    /// Reports storage failures.
    pub async fn search_pending(&self, limit: u32) -> Result<store::search::SearchBatch, Error> {
        self.request(|reply| Command::SearchPending(limit, reply))
            .await
    }
    /// Forget changes the search index has committed.
    /// # Errors
    /// Reports storage failures.
    pub async fn search_indexed(&self, through: i64) -> Result<(), Error> {
        self.request(|reply| Command::SearchIndexed(through, reply))
            .await
    }
    /// Queue every message again, for a search index that had to be recreated.
    /// # Errors
    /// Reports storage failures.
    pub async fn requeue_search(&self) -> Result<(), Error> {
        self.request(Command::RequeueSearch).await
    }
    /// Read bounded changes for the independent semantic consumer.
    /// # Errors
    /// Rejects invalid limits and reports storage failures.
    pub async fn semantic_pending(&self, limit: u32) -> Result<store::semantic::Batch, Error> {
        self.request(|reply| Command::SemanticPending(limit, reply))
            .await
    }
    /// Acknowledge a durably persisted embedding batch without consuming lexical work.
    /// # Errors
    /// Reports storage failures.
    pub async fn semantic_indexed(&self, through: i64) -> Result<(), Error> {
        self.request(|reply| Command::SemanticIndexed(through, reply))
            .await
    }
    /// Requeue originals after a model/index policy change.
    /// # Errors
    /// Reports storage failures without touching drafts or reminders.
    pub async fn requeue_semantic(&self) -> Result<(), Error> {
        self.request(Command::RequeueSemantic).await
    }
    /// Canonical allowed original revisions, scoped before any vector result limit.
    /// # Errors
    /// Rejects invalid scope/filters and reports storage failures.
    pub async fn semantic_candidates(
        &self,
        accounts: Vec<String>,
        scope: store::browse::Scope,
        filter: store::browse::SearchFilter,
    ) -> Result<std::collections::BTreeMap<i64, String>, Error> {
        self.request(|reply| Command::SemanticCandidates(accounts, scope, filter, reply))
            .await
    }
    /// Unread inbox mail not yet analyzed by this policy.
    /// # Errors
    /// Rejects invalid limits and reports storage failures.
    pub async fn analysis_pending(
        &self,
        accounts: Vec<String>,
        policy: u32,
        since: i64,
        limit: u32,
    ) -> Result<Vec<store::analysis::AnalysisInput>, Error> {
        self.request(|reply| Command::AnalysisPending(accounts, policy, since, limit, reply))
            .await
    }
    /// Store priority results. A dismissal survives any later result.
    /// # Errors
    /// Reports storage failures.
    pub async fn store_analysis(
        &self,
        results: Vec<store::analysis::AnalysisResult>,
    ) -> Result<(), Error> {
        self.request(|reply| Command::StoreAnalysis(results, reply))
            .await
    }
    /// The priority group right now, soonest to expire first.
    /// # Errors
    /// Rejects invalid limits and reports storage failures.
    pub async fn priority_items(
        &self,
        accounts: Vec<String>,
        now: i64,
        limit: u32,
    ) -> Result<Vec<store::analysis::PriorityItem>, Error> {
        self.request(|reply| Command::PriorityItems(accounts, now, limit, reply))
            .await
    }
    /// The account's newest message, and unread inbox mail after a given one.
    /// # Errors
    /// Rejects an empty limit and reports storage failures.
    pub async fn new_inbox_mail(
        &self,
        account: String,
        after: Option<i64>,
        since: i64,
        limit: u32,
    ) -> Result<(i64, Vec<store::analysis::NewMail>), Error> {
        self.request(|reply| Command::NewInboxMail(account, after, since, limit, reply))
            .await
    }
    /// The complete stored priority result for a message, if it has one.
    /// # Errors
    /// Reports storage failures.
    pub async fn analysis_data(&self, message: i64) -> Result<Option<String>, Error> {
        self.request(|reply| Command::AnalysisData(message, reply))
            .await
    }
    /// Take a message out of the priority group for good, without reading it.
    /// # Errors
    /// Reports storage failures.
    pub async fn dismiss_priority(&self, message: i64) -> Result<(), Error> {
        self.request(|reply| Command::DismissPriority(message, reply))
            .await
    }
    /// Cache the small copy of a message too large to keep, with what it leaves out.
    /// # Errors
    /// As [`Self::cache_message`].
    pub async fn cache_reduced(
        &self,
        account: String,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        reduced: Vec<u8>,
        omitted: crate::outline::Omitted,
    ) -> Result<i64, Error> {
        if reduced.len() > mime::MAX_INPUT_BYTES {
            return Err(Error::InvalidRequest);
        }
        self.request(|reply| Command::CacheReduced {
            account,
            membership,
            received_at,
            gmail,
            reduced,
            omitted,
            reply,
        })
        .await
    }
    /// What a stored message leaves on the server, if anything.
    /// # Errors
    /// Reports storage failures.
    pub async fn omitted(
        &self,
        account: String,
        message: i64,
    ) -> Result<Option<crate::outline::Omitted>, Error> {
        self.request(|reply| Command::Omitted(account, message, reply))
            .await
    }
    /// Where the server has a message now.
    /// # Errors
    /// Reports storage failures.
    pub async fn location(
        &self,
        account: String,
        message: i64,
    ) -> Result<Option<(String, core::num::NonZeroU32, core::num::NonZeroU32)>, Error> {
        self.request(|reply| Command::Location(account, message, reply))
            .await
    }
    /// List known folders for enabled accounts without contacting a mail server.
    /// # Errors
    /// Rejects excessive results and storage failures.
    pub async fn folders(&self, accounts: Vec<String>) -> Result<Vec<Folder>, Error> {
        self.request(|reply| Command::Folders(accounts, reply))
            .await
    }
    /// Unread messages per folder, by folder ID.
    /// # Errors
    /// Rejects excessive account lists and storage failures.
    pub async fn unread_counts(&self, accounts: Vec<String>) -> Result<Vec<(i64, u32)>, Error> {
        self.request(|reply| Command::UnreadCounts(accounts, reply))
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
    /// Queue moving a conversation out of a folder (or the inbox), hiding it locally. No
    /// sync pass may claim it before `not_before`, which is the Undo window.
    /// # Errors
    /// Rejects unknown destinations, nothing to move and stale context.
    pub async fn queue_move(
        &self,
        account: String,
        message: i64,
        source: Option<i64>,
        destination: String,
        context: String,
        not_before: i64,
    ) -> Result<u32, Error> {
        self.request(|reply| {
            Command::QueueMove(
                account,
                message,
                source,
                destination,
                context,
                not_before,
                reply,
            )
        })
        .await
    }
    /// Cancel a conversation's queued moves if none reached the server.
    /// # Errors
    /// Rejects moves already attempted and storage failures.
    pub async fn undo_move(&self, account: String, message: i64) -> Result<(), Error> {
        self.request(|reply| Command::UndoMove(account, message, reply))
            .await
    }
    /// Claim the next queued move.
    /// # Errors
    /// Rejects concurrent claims and storage failures.
    pub async fn begin_move(
        &self,
        account: String,
        context: String,
    ) -> Result<Option<MoveAttempt>, Error> {
        self.request(|reply| Command::BeginMove(account, context, reply))
            .await
    }
    /// Commit the exact attempt's result.
    /// # Errors
    /// Rejects stale attempts and storage failures.
    pub async fn finish_move(&self, attempt: MoveAttempt, moved: bool) -> Result<(), Error> {
        self.request(|reply| Command::FinishMove(attempt, moved, reply))
            .await
    }
    /// Give up on a move this server can never do.
    /// # Errors
    /// Rejects stale attempts and storage failures.
    pub async fn abandon_move(&self, attempt: MoveAttempt) -> Result<(), Error> {
        self.request(|reply| Command::AbandonMove(attempt, reply))
            .await
    }
    /// Requeue this account's failed moves.
    /// # Errors
    /// Returns storage failures.
    pub async fn retry_moves(&self, account: String) -> Result<(), Error> {
        self.request(|reply| Command::RetryMoves(account, reply))
            .await
    }
    /// The next accepted message still owing a copy in the Sent folder.
    /// # Errors
    /// Rejects damaged originals and storage failures.
    pub async fn pending_sent_copy(
        &self,
        account: String,
    ) -> Result<Option<outbox::SentCopy>, Error> {
        self.request(|reply| Command::PendingSentCopy(account, reply))
            .await
    }
    /// Record a completed Sent copy, without touching SMTP delivery state.
    /// # Errors
    /// Rejects a record that owes no copy and storage failures.
    pub async fn finish_sent_copy(&self, account: String, id: String) -> Result<(), Error> {
        self.request(|reply| Command::FinishSentCopy(account, id, reply))
            .await
    }
    /// Record which folders serve the standard mail roles.
    /// # Errors
    /// Rejects duplicate roles and failed durable writes.
    pub async fn assign_folder_roles(
        &self,
        account: String,
        roles: Vec<(String, FolderRole)>,
    ) -> Result<(), Error> {
        self.request(|reply| Command::AssignFolderRoles(account, roles, reply))
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
    /// Close an editable draft without sending it, retaining its tombstone.
    ///
    /// # Errors
    /// Rejects missing/closed/foreign drafts and storage failures.
    pub async fn discard_draft(&self, account: String, id: String) -> Result<(), Error> {
        self.request(|reply| Command::DiscardDraft(account, id, reply))
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
    /// The durable history pass of one folder, if one was started.
    ///
    /// # Errors
    /// Returns a redacted worker or storage error.
    pub async fn backfill_pass(
        &self,
        account: String,
        folder: i64,
    ) -> Result<Option<store::backfill::Pass>, Error> {
        self.request(|reply| Command::BackfillPass(account, folder, reply))
            .await
    }
    /// How much of each folder's history is cached, for the accounts asked about.
    ///
    /// # Errors
    /// Returns a redacted worker or storage error.
    pub async fn coverage(
        &self,
        accounts: Vec<String>,
    ) -> Result<Vec<store::backfill::FolderCoverage>, Error> {
        self.request(|reply| Command::Coverage(accounts, reply))
            .await
    }

    /// Store a history pass once the window it describes has committed.
    ///
    /// # Errors
    /// Returns a redacted worker or storage error.
    pub async fn save_backfill(
        &self,
        account: String,
        folder: i64,
        pass: store::backfill::Pass,
    ) -> Result<(), Error> {
        self.request(|reply| Command::SaveBackfill(account, folder, pass, reply))
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
    store.recover_moves().map_err(Error::from)?;
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
            Command::FilteredPage(request, reply) => {
                respond(reply, || store.filtered_conversation_page(&request));
            }
            Command::UnreadCounts(accounts, reply) => {
                respond(reply, || store.unread_counts(&accounts));
            }
            Command::RankedPage(accounts, scope, hits, after, limit, reply) => {
                respond(reply, || {
                    store.ranked_conversation_page(&accounts, &scope, &hits, after, limit)
                });
            }
            Command::SearchPending(limit, reply) => {
                respond(reply, || store.search_pending(limit));
            }
            Command::SearchIndexed(through, reply) => {
                commit(reply, || store.search_indexed(through));
            }
            Command::RequeueSearch(reply) => commit(reply, || store.requeue_search()),
            Command::SemanticPending(limit, reply) => {
                respond(reply, || store.semantic_pending(limit));
            }
            Command::SemanticIndexed(through, reply) => {
                commit(reply, || store.semantic_indexed(through));
            }
            Command::RequeueSemantic(reply) => commit(reply, || store.requeue_semantic()),
            Command::SemanticCandidates(accounts, scope, filter, reply) => respond(reply, || {
                store.semantic_candidates(&accounts, &scope, &filter)
            }),
            Command::AnalysisPending(accounts, policy, since, limit, reply) => {
                respond(reply, || {
                    store.analysis_pending(&accounts, policy, since, limit)
                });
            }
            Command::StoreAnalysis(results, reply) => {
                commit(reply, || store.store_analysis(&results));
            }
            Command::PriorityItems(accounts, now, limit, reply) => {
                respond(reply, || store.priority_items(&accounts, now, limit));
            }
            Command::DismissPriority(message, reply) => {
                commit(reply, || store.dismiss_priority(message));
            }
            Command::AnalysisData(message, reply) => {
                respond(reply, || store.analysis_data(message));
            }
            Command::NewInboxMail(account, after, since, limit, reply) => {
                respond(reply, || {
                    store.new_inbox_mail(&account, after, since, limit)
                });
            }
            Command::CacheReduced {
                account,
                membership,
                received_at,
                gmail,
                reduced,
                omitted,
                reply,
            } => {
                commit(reply, || {
                    store.cache_reduced(
                        &account,
                        membership,
                        received_at,
                        gmail,
                        &reduced,
                        &omitted,
                    )
                });
            }
            Command::Omitted(account, message, reply) => {
                respond(reply, || store.omitted(&account, message));
            }
            Command::Location(account, message, reply) => {
                respond(reply, || store.location(&account, message));
            }
            Command::RememberFolders(account, names, inbox, reply) => {
                commit(reply, || store.remember_folders(&account, &names, &inbox));
            }
            Command::QueueMove(
                account,
                message,
                source,
                destination,
                context,
                not_before,
                reply,
            ) => {
                commit(reply, || {
                    store.queue_move_after(
                        &account,
                        message,
                        source,
                        &destination,
                        &context,
                        not_before,
                    )
                });
            }
            Command::UndoMove(account, message, reply) => {
                commit(reply, || store.undo_move(&account, message));
            }
            Command::BeginMove(account, context, reply) => {
                if !reply.is_closed() {
                    let result = store.begin_move(&account, &context).map_err(Error::from);
                    if let Err(Ok(Some(attempt))) = reply.send(result) {
                        let _failed = store.finish_move(&attempt, false);
                    }
                }
            }
            Command::FinishMove(attempt, moved, reply) => {
                commit(reply, || store.finish_move(&attempt, moved));
            }
            Command::AbandonMove(attempt, reply) => {
                commit(reply, || store.abandon_move(&attempt));
            }
            Command::RetryMoves(account, reply) => {
                commit(reply, || store.retry_moves(&account));
            }
            Command::PendingSentCopy(account, reply) => {
                respond(reply, || store.pending_sent_copy(&account));
            }
            Command::FinishSentCopy(account, id, reply) => {
                commit(reply, || store.finish_sent_copy(&account, &id));
            }
            Command::AssignFolderRoles(account, roles, reply) => {
                commit(reply, || store.assign_folder_roles(&account, &roles));
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
            Command::DiscardDraft(account, id, reply) => {
                commit(reply, || store.discard_draft(&account, &id));
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
            Command::BackfillPass(account, folder, reply) => {
                respond(reply, || store.backfill_pass(&account, folder));
            }
            Command::Coverage(accounts, reply) => {
                respond(reply, || store.coverage(&accounts));
            }
            Command::SaveBackfill(account, folder, pass, reply) => {
                respond(reply, || store.save_backfill(&account, folder, &pass));
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
