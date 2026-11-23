//! Stored priority results, and which messages are in the priority group right now.
//!
//! The store owns final eligibility, not whatever produced a result: reading, dismissing
//! and expiry are decided here, so that a late result can never bring a message back.

use chrono::{Days, LocalResult, NaiveDate, NaiveTime, TimeZone as _};
use chrono_tz::Tz;
use rusqlite::{OptionalExtension as _, params};
use serde::{Deserialize, Serialize};

use super::{Error, Store};

pub(super) const SCHEMA: &str = "
ALTER TABLE analysis ADD COLUMN policy INTEGER NOT NULL DEFAULT 0;
ALTER TABLE analysis ADD COLUMN expires_at INTEGER;
ALTER TABLE analysis ADD COLUMN due TEXT;
ALTER TABLE analysis ADD COLUMN dismissed INTEGER NOT NULL DEFAULT 0 CHECK(dismissed IN (0, 1));
CREATE INDEX analysis_expiry ON analysis(expires_at) WHERE expires_at IS NOT NULL;
";

pub(super) const DEADLINE_CORRECTIONS_SCHEMA: &str = "
CREATE TABLE deadline_corrections (
 message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
 data TEXT NOT NULL,
 due TEXT NOT NULL,
 priority_until INTEGER
);
";

/// A deadline explicitly entered by the reader.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeadlineCorrection {
    /// The selected civil date.
    pub date: NaiveDate,
    /// An explicit time, or the end of the selected date.
    pub time: Option<NaiveTime>,
    /// The timezone in effect when the user made the correction.
    pub zone: String,
    /// The fixed expiration instant in Unix seconds.
    pub ends_at: i64,
}

/// One explicit correction to the deadline of a cached original.
#[derive(Clone, Debug)]
pub struct CorrectDeadline {
    /// The original's owning account.
    pub account: String,
    /// Canonical message ID.
    pub message: i64,
    /// The selected civil date.
    pub date: NaiveDate,
    /// An explicit time, or the end of the selected date.
    pub time: Option<NaiveTime>,
    /// The timezone used to interpret the selection.
    pub zone: Tz,
    /// When the user submitted the correction, in Unix seconds.
    pub now: i64,
}

impl CorrectDeadline {
    /// Resolve the civil time. An ambiguous or nonexistent explicit time has no
    /// single instant and must be corrected at the input.
    #[must_use]
    pub fn resolve(&self) -> Option<DeadlineCorrection> {
        let ends_at = if let Some(time) = self.time {
            self.zone
                .from_local_datetime(&self.date.and_time(time))
                .single()?
        } else {
            let next = self.date.checked_add_days(Days::new(1))?;
            let midnight = next.and_hms_opt(0, 0, 0)?;
            let boundary = match self.zone.from_local_datetime(&midnight) {
                LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => at,
                LocalResult::None => self
                    .zone
                    .from_local_datetime(&self.date.and_hms_opt(23, 59, 59)?)
                    .single()?
                    .checked_add_signed(chrono::TimeDelta::seconds(1))?,
            };
            // A calendar date skipped entirely by this zone has no deadline.
            if boundary
                .checked_sub_signed(chrono::TimeDelta::seconds(1))?
                .date_naive()
                != self.date
            {
                return None;
            }
            boundary
        };
        Some(DeadlineCorrection {
            date: self.date,
            time: self.time,
            zone: self.zone.name().into(),
            ends_at: ends_at.timestamp(),
        })
    }
}

/// A newly arrived message, as far as announcing it needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewMail {
    /// Canonical message ID.
    pub id: i64,
    /// Displayed sender.
    pub sender: String,
    /// Decoded subject.
    pub subject: String,
    /// When the message arrived, in Unix seconds.
    pub received_at: i64,
}

/// A message waiting for analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisInput {
    /// Canonical message ID.
    pub id: i64,
    /// Owning account.
    pub account: String,
    /// Decoded subject.
    pub subject: String,
    /// Readable text, or `None` if there is none, as with encrypted mail.
    pub body: Option<String>,
    /// Primary recipients' addresses.
    pub to: Vec<String>,
    /// The Date header.
    pub authored: Option<i64>,
    /// When the server received it.
    pub received_at: i64,
}

/// One message's result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisResult {
    /// Canonical message ID.
    pub id: i64,
    /// Version of the rules or model that produced this.
    pub policy: u32,
    /// When priority ends, or `None` if this message does not qualify.
    pub expires_at: Option<i64>,
    /// The accepted deadline as an ISO local date, with a time if one was stated.
    pub due: Option<String>,
    /// The complete versioned result.
    pub data: String,
}

/// A message in the priority group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriorityItem {
    /// Canonical message ID.
    pub id: i64,
    /// When priority ends.
    pub expires_at: i64,
    /// The accepted deadline, if there is one.
    pub due: Option<String>,
}

const IN_INBOX: &str = "EXISTS(SELECT 1 FROM effective_memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity WHERE s.message_id=m.id AND s.account_id=m.account_id AND f.is_inbox=1 AND s.seen=0)";

impl Store {
    /// Save user evidence independently of automatic analysis. A correction can
    /// change an active priority deadline but cannot start or restart priority.
    /// # Errors
    /// Rejects invalid civil times and originals no longer in the declared account.
    pub fn correct_deadline(
        &mut self,
        edit: &CorrectDeadline,
    ) -> Result<DeadlineCorrection, Error> {
        let corrected = edit.resolve().ok_or(Error::InvalidInput)?;
        let due = corrected.time.map_or_else(
            || corrected.date.to_string(),
            |time| format!("{} {}", corrected.date, time.format("%H:%M")),
        );
        let count = self.connection.execute(
            &format!(
                "INSERT INTO deadline_corrections(message_id,data,due,priority_until)
                 SELECT m.id,?3,?4,CASE WHEN m.read_cleared=0 AND a.dismissed=0 AND {IN_INBOX}
                   AND (CASE WHEN c.message_id IS NULL THEN a.expires_at ELSE c.priority_until END)>?5
                   THEN ?6 END
                 FROM messages m LEFT JOIN analysis a ON a.message_id=m.id
                 LEFT JOIN deadline_corrections c ON c.message_id=m.id
                 WHERE m.account_id=?1 AND m.id=?2
                 ON CONFLICT(message_id) DO UPDATE SET data=excluded.data,due=excluded.due,
                   priority_until=excluded.priority_until"
            ),
            params![edit.account, edit.message, serde_json::to_string(&corrected)?, due, edit.now, corrected.ends_at],
        )?;
        if count == 0 {
            return Err(Error::MissingMessage);
        }
        Ok(corrected)
    }

    /// Read the user's correction for an original in the specified account.
    /// # Errors
    /// Reports storage or decoding failures.
    pub fn deadline_correction(
        &self,
        account: &str,
        message: i64,
    ) -> Result<Option<DeadlineCorrection>, Error> {
        let data: Option<String> = self
            .connection
            .query_row(
                "SELECT c.data FROM deadline_corrections c JOIN messages m ON m.id=c.message_id
             WHERE m.account_id=?1 AND m.id=?2",
                params![account, message],
                |row| row.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }

    /// Unread inbox mail received since a moment and not yet analyzed by this policy.
    /// # Errors
    /// Rejects invalid limits and reports storage failures.
    pub fn analysis_pending(
        &self,
        accounts: &[String],
        policy: u32,
        since: i64,
        limit: u32,
    ) -> Result<Vec<AnalysisInput>, Error> {
        if accounts.len() > 256 || limit == 0 {
            return Err(Error::InvalidInput);
        }
        let mut query = self.connection.prepare_cached(&format!(
            "SELECT m.id,m.account_id,m.subject,t.body,m.metadata,m.received_at FROM messages m
             LEFT JOIN analysis a ON a.message_id=m.id
             LEFT JOIN message_search t ON t.rowid=m.id
             WHERE m.account_id IN (SELECT value FROM json_each(?1)) AND m.read_cleared=0 AND m.received_at>=?2
             AND (a.message_id IS NULL OR a.policy<?3) AND {IN_INBOX}
             ORDER BY m.id LIMIT ?4"
        ))?;
        let rows = query.query_map(
            params![serde_json::to_string(accounts)?, since, policy, limit],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (id, account, subject, body, metadata, received_at) = row?;
            let metadata = metadata
                .as_deref()
                .map(serde_json::from_str::<super::mailbox::Metadata>)
                .transpose()?;
            let unreadable = metadata
                .as_ref()
                .is_some_and(|metadata| metadata.status.encrypted_unsupported);
            let headers = metadata.map(|metadata| metadata.headers);
            Ok(AnalysisInput {
                id,
                account,
                subject,
                body: body.filter(|text| !unreadable && !text.trim().is_empty()),
                to: headers.as_ref().map_or_else(Vec::new, |headers| {
                    headers
                        .to
                        .iter()
                        .filter_map(|mailbox| mailbox.address.clone())
                        .collect()
                }),
                authored: headers.and_then(|headers| headers.date),
                received_at,
            })
        })
        .collect()
    }

    /// Store results. A dismissal survives any later result for the same message, and a
    /// message that has since gone away is ignored.
    /// # Errors
    /// Reports storage failures, leaving none of the batch stored.
    pub fn store_analysis(&mut self, results: &[AnalysisResult]) -> Result<(), Error> {
        self.store_analysis_if(results, || true)
    }

    pub(crate) fn store_analysis_if(
        &mut self,
        results: &[AnalysisResult],
        active: impl Fn() -> bool,
    ) -> Result<(), Error> {
        if !active() {
            return Ok(());
        }
        let tx = self.connection.transaction()?;
        for result in results {
            tx.execute(
                "INSERT INTO analysis(message_id,data,policy,expires_at,due)
                 SELECT id,?2,?3,?4,?5 FROM messages WHERE id=?1
                 ON CONFLICT(message_id) DO UPDATE SET data=excluded.data,policy=excluded.policy,expires_at=excluded.expires_at,due=excluded.due",
                params![result.id, result.data, result.policy, result.expires_at, result.due],
            )?;
        }
        // A write may have waited for SQLite while the account was reconfigured.
        if active() {
            tx.commit()?;
        }
        Ok(())
    }

    /// The priority group right now, soonest to expire first. Reading is final: an original
    /// marked read once stays out of the group even if it is later marked unread.
    /// # Errors
    /// Rejects invalid limits and reports storage failures.
    pub fn priority_items(
        &self,
        accounts: &[String],
        now: i64,
        limit: u32,
    ) -> Result<Vec<PriorityItem>, Error> {
        if accounts.len() > 256 || limit == 0 {
            return Err(Error::InvalidInput);
        }
        let mut query = self.connection.prepare_cached(&format!(
            "WITH active(message_id,ends_at,due) AS (
               SELECT a.message_id,a.expires_at,a.due FROM analysis a
               WHERE a.expires_at>?2 AND a.dismissed=0
                 AND NOT EXISTS(SELECT 1 FROM deadline_corrections c WHERE c.message_id=a.message_id)
               UNION ALL
               SELECT c.message_id,c.priority_until,c.due FROM deadline_corrections c
                JOIN analysis a ON a.message_id=c.message_id
               WHERE c.priority_until>?2 AND a.dismissed=0
             )
             SELECT m.id,active.ends_at,active.due FROM active JOIN messages m ON m.id=active.message_id
             WHERE m.read_cleared=0
             AND m.account_id IN (SELECT value FROM json_each(?1)) AND {IN_INBOX}
             ORDER BY active.ends_at,m.id DESC LIMIT ?3"
        ))?;
        query
            .query_map(
                params![serde_json::to_string(accounts)?, now, limit],
                |row| {
                    Ok(PriorityItem {
                        id: row.get(0)?,
                        expires_at: row.get(1)?,
                        due: row.get(2)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// Take a message out of the priority group for good, without reading it.
    /// # Errors
    /// Reports storage failures; dismissing a message with no result does nothing.
    pub fn dismiss_priority(&mut self, message: i64) -> Result<(), Error> {
        self.connection.execute(
            "UPDATE analysis SET dismissed=1 WHERE message_id=?1",
            [message],
        )?;
        Ok(())
    }

    /// The account's newest message, and any unread inbox mail after a given one that
    /// arrived since a moment, newest first. The moment keeps older mail fetched on request
    /// from being announced as new. With nothing to compare against, it lists nothing.
    /// # Errors
    /// Rejects an empty limit and reports storage failures.
    pub fn new_inbox_mail(
        &self,
        account: &str,
        after: Option<i64>,
        since: i64,
        limit: u32,
    ) -> Result<(i64, Vec<NewMail>), Error> {
        if limit == 0 {
            return Err(Error::InvalidInput);
        }
        let newest: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(id),0) FROM messages WHERE account_id=?1",
            [account],
            |row| row.get(0),
        )?;
        let Some(after) = after else {
            return Ok((newest, Vec::new()));
        };
        let mut query = self.connection.prepare_cached(&format!(
            "SELECT m.id,m.sender,m.subject,m.received_at FROM messages m
             WHERE m.account_id=?1 AND m.id>?2 AND m.id<=?3 AND m.received_at>=?4 AND m.read_cleared=0
             AND {IN_INBOX} ORDER BY m.received_at DESC,m.id DESC LIMIT ?5"
        ))?;
        let mail = query
            .query_map(params![account, after, newest, since, limit], |row| {
                Ok(NewMail {
                    id: row.get(0)?,
                    sender: row.get(1)?,
                    subject: row.get(2)?,
                    received_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((newest, mail))
    }

    /// The complete stored result for a message, if it has one.
    /// # Errors
    /// Reports storage failures.
    pub fn analysis_data(&self, message: i64) -> Result<Option<String>, Error> {
        self.connection
            .query_row(
                "SELECT data FROM analysis WHERE message_id=?1",
                [message],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU32;

    use super::super::browse::Scope;
    use super::super::mailbox::{Flags, Membership};
    use super::*;

    const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct Fixture {
        _root: tempfile::TempDir,
        store: Store,
        inbox: i64,
        archive: i64,
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().expect("fixture");
        let mut store = Store::open(root.path()).expect("open");
        store.ensure_account("a").expect("account");
        store.ensure_account("b").expect("account");
        store.bind_mailbox_context("a", CONTEXT).expect("context");
        let mut folder = |account: &str, name: &str, inbox: bool| {
            let id = store
                .ensure_folder(account, name, inbox)
                .expect("folder")
                .id;
            store
                .select_folder(account, id, NonZeroU32::new(42).expect("namespace"))
                .expect("generation");
            id
        };
        let inbox = folder("a", "INBOX", true);
        let archive = folder("a", "Archive", false);
        folder("b", "INBOX", true);
        Fixture {
            _root: root,
            store,
            inbox,
            archive,
        }
    }

    fn cache(fixture: &mut Fixture, folder: i64, uid: u32, seen: bool, received_at: i64) -> i64 {
        fixture
            .store
            .cache_message(
                "a",
                Membership {
                    folder,
                    uid_validity: NonZeroU32::new(42).expect("namespace"),
                    uid: NonZeroU32::new(uid).expect("UID"),
                    flags: Flags {
                        seen,
                        starred: false,
                    },
                },
                received_at,
                None,
                format!(
                    "From: Sender <sender@example.test>\r\nTo: Reader <reader@example.test>\r\nDate: Wed, 4 Nov 2026 09:00:00 +0100\r\nSubject: Subject {uid}\r\nMessage-ID: <{uid}@example.test>\r\n\r\nBody {uid}."
                )
                .as_bytes(),
            )
            .expect("message")
    }

    fn accounts() -> [String; 1] {
        ["a".to_owned()]
    }

    fn pending(fixture: &Fixture, policy: u32) -> Vec<i64> {
        fixture
            .store
            .analysis_pending(&accounts(), policy, 0, 100)
            .expect("pending")
            .into_iter()
            .map(|input| input.id)
            .collect()
    }

    fn give_priority(fixture: &mut Fixture, id: i64, expires_at: i64) {
        fixture
            .store
            .store_analysis(&[AnalysisResult {
                id,
                policy: 1,
                expires_at: Some(expires_at),
                due: Some("2026-11-15".into()),
                data: "{}".into(),
            }])
            .expect("result");
    }

    #[test]
    fn retiring_rules_before_commit_rolls_back_the_entire_analysis_batch() {
        let mut f = fixture();
        let inbox = f.inbox;
        let ids = [cache(&mut f, inbox, 1, false, 100), cache(&mut f, inbox, 2, false, 100)];
        let results = ids.map(|id| AnalysisResult {
            id, policy: 1, expires_at: Some(500), due: None, data: "{}".into(),
        });
        let active = std::cell::Cell::new(true);
        f.store.store_analysis_if(&results, || active.replace(false)).expect("retired batch");
        assert_eq!(pending(&f, 1), ids);
        f.store.store_analysis(&results).expect("replacement batch");
        assert!(pending(&f, 1).is_empty());
    }

    fn group(fixture: &Fixture, now: i64) -> Vec<i64> {
        fixture
            .store
            .priority_items(&accounts(), now, 100)
            .expect("items")
            .into_iter()
            .map(|item| item.id)
            .collect()
    }

    #[test]
    fn only_unread_recent_inbox_mail_waits_and_it_carries_what_analysis_needs() {
        let mut fixture = fixture();
        let (inbox, archive) = (fixture.inbox, fixture.archive);
        let wanted = cache(&mut fixture, inbox, 1, false, 1_000);
        cache(&mut fixture, inbox, 2, true, 1_000);
        cache(&mut fixture, inbox, 3, false, 10);
        cache(&mut fixture, archive, 4, false, 1_000);
        let found = fixture
            .store
            .analysis_pending(&accounts(), 1, 500, 100)
            .expect("pending");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, wanted);
        assert_eq!(found[0].account, "a");
        assert_eq!(found[0].subject, "Subject 1");
        assert!(
            found[0]
                .body
                .as_deref()
                .is_some_and(|body| body.contains("Body 1."))
        );
        assert_eq!(found[0].to, ["reader@example.test"]);
        assert_eq!(found[0].authored, Some(1_793_779_200 + 7200 - 3600 - 3600));
        assert_eq!(found[0].received_at, 1_000);
    }

    #[test]
    fn a_stored_result_stops_waiting_until_the_policy_changes() {
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let id = cache(&mut fixture, inbox, 1, false, 1_000);
        assert_eq!(pending(&fixture, 1), [id]);
        give_priority(&mut fixture, id, 5_000);
        assert_eq!(pending(&fixture, 1), Vec::<i64>::new());
        assert_eq!(pending(&fixture, 2), [id]);
        assert_eq!(
            fixture.store.analysis_data(id).expect("data").as_deref(),
            Some("{}")
        );
    }

    #[test]
    fn the_group_is_ordered_by_what_ends_soonest_and_only_while_it_lasts() {
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let later = cache(&mut fixture, inbox, 1, false, 1_000);
        let sooner = cache(&mut fixture, inbox, 2, false, 1_000);
        give_priority(&mut fixture, later, 9_000);
        give_priority(&mut fixture, sooner, 5_000);
        assert_eq!(group(&fixture, 1_000), [sooner, later]);
        assert_eq!(group(&fixture, 4_999), [sooner, later]);
        assert_eq!(group(&fixture, 5_000), [later], "the end is exclusive");
        assert_eq!(group(&fixture, 9_000), Vec::<i64>::new());
        let item = &fixture
            .store
            .priority_items(&accounts(), 1_000, 1)
            .expect("items")[0];
        assert_eq!(item.due.as_deref(), Some("2026-11-15"));
    }

    #[test]
    fn reading_is_final_even_after_marking_unread_again() {
        use mailstronaut_mail::imap::FlagChange;
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let id = cache(&mut fixture, inbox, 1, false, 1_000);
        let sibling = cache(&mut fixture, inbox, 2, false, 1_000);
        give_priority(&mut fixture, id, 5_000);
        assert_eq!(group(&fixture, 1_000), [id]);
        fixture
            .store
            .set_flag("a", id, FlagChange::Seen, true, CONTEXT)
            .expect("read");
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new());
        // Reading one message says nothing about another that qualifies later.
        give_priority(&mut fixture, sibling, 5_000);
        assert_eq!(group(&fixture, 1_000), [sibling]);
        fixture.store.dismiss_priority(sibling).expect("dismiss");
        fixture
            .store
            .set_flag("a", id, FlagChange::Seen, false, CONTEXT)
            .expect("unread");
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new());
        // A result arriving late cannot bring it back either.
        give_priority(&mut fixture, id, 9_000);
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new());
    }

    #[test]
    fn dismissing_is_final_and_does_not_read_the_message() {
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let id = cache(&mut fixture, inbox, 1, false, 1_000);
        give_priority(&mut fixture, id, 5_000);
        fixture.store.dismiss_priority(id).expect("dismiss");
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new());
        give_priority(&mut fixture, id, 9_000);
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new(), "late result");
        let rows = fixture
            .store
            .conversation_page(&accounts(), &Scope::Inbox, "", None, 100)
            .expect("inbox");
        assert!(!rows[0].latest.seen);
        fixture.store.dismiss_priority(999_999).expect("no result");
    }

    #[test]
    fn leaving_the_inbox_or_being_another_accounts_mail_ends_it() {
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let id = cache(&mut fixture, inbox, 1, false, 1_000);
        give_priority(&mut fixture, id, 5_000);
        assert_eq!(
            fixture
                .store
                .priority_items(&["b".to_owned()], 1_000, 100)
                .expect("items"),
            []
        );
        fixture
            .store
            .assign_folder_roles(
                "a",
                &[("Archive".into(), super::super::mailbox::FolderRole::Archive)],
            )
            .expect("role");
        fixture
            .store
            .queue_move("a", id, None, "Archive", CONTEXT)
            .expect("move");
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new());
    }

    #[test]
    fn a_result_for_a_message_that_has_gone_is_ignored() {
        let mut fixture = fixture();
        fixture
            .store
            .store_analysis(&[AnalysisResult {
                id: 999_999,
                policy: 1,
                expires_at: Some(5_000),
                due: None,
                data: "{}".into(),
            }])
            .expect("ignored");
        assert_eq!(group(&fixture, 1_000), Vec::<i64>::new());
    }

    #[test]
    fn new_mail_is_what_arrived_after_a_baseline_and_is_still_unread_in_the_inbox() {
        use mailstronaut_mail::imap::FlagChange;
        let mut fixture = fixture();
        let (inbox, archive) = (fixture.inbox, fixture.archive);
        let existing = cache(&mut fixture, inbox, 1, false, 1_000);
        let ids = |(_, mail): (i64, Vec<NewMail>)| {
            mail.into_iter().map(|item| item.id).collect::<Vec<_>>()
        };
        // The first look only records where things stand.
        let (baseline, mail) = fixture
            .store
            .new_inbox_mail("a", None, 0, 100)
            .expect("baseline");
        assert_eq!(baseline, existing);
        assert_eq!(mail, []);
        let new = cache(&mut fixture, inbox, 2, false, 2_000);
        let newer = cache(&mut fixture, inbox, 3, false, 3_000);
        cache(&mut fixture, inbox, 4, true, 3_000);
        cache(&mut fixture, archive, 5, false, 3_000);
        let found = |fixture: &Fixture, since| {
            ids(fixture
                .store
                .new_inbox_mail("a", Some(baseline), since, 100)
                .expect("new mail"))
        };
        assert_eq!(found(&fixture, 0), [newer, new], "newest first");
        // Older mail fetched on request gets a new ID, and is still not new mail.
        assert_eq!(found(&fixture, 2_500), [newer]);
        fixture
            .store
            .set_flag("a", newer, FlagChange::Seen, true, CONTEXT)
            .expect("read");
        assert_eq!(found(&fixture, 0), [new], "already read elsewhere");
        assert_eq!(
            ids(fixture
                .store
                .new_inbox_mail("b", Some(0), 0, 100)
                .expect("other account")),
            Vec::<i64>::new()
        );
        assert!(fixture.store.new_inbox_mail("a", None, 0, 0).is_err());
    }

    fn correction(message: i64, now: i64) -> CorrectDeadline {
        CorrectDeadline {
            account: "a".into(),
            message,
            date: NaiveDate::from_ymd_opt(2026, 10, 25).expect("date"),
            time: None,
            zone: chrono_tz::Europe::Paris,
            now,
        }
    }

    #[test]
    fn a_correction_controls_active_expiry_and_survives_reanalysis_and_cache_clear() {
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let now = 1_792_908_000;
        let id = cache(&mut fixture, inbox, 1, false, now);
        let unowned = cache(&mut fixture, inbox, 2, false, now);
        give_priority(&mut fixture, id, now + 100);
        let edit = correction(id, now);
        let corrected = fixture
            .store
            .correct_deadline(&edit)
            .expect("correct deadline");
        let end = chrono::DateTime::parse_from_rfc3339("2026-10-26T00:00:00+01:00")
            .expect("first instant after the long local day")
            .timestamp();
        assert_eq!(corrected.ends_at, end);
        give_priority(&mut fixture, id, now + 200);
        let items = fixture
            .store
            .priority_items(&accounts(), now + 300, 10)
            .expect("group");
        assert_eq!(
            items,
            [PriorityItem {
                id,
                expires_at: end,
                due: Some("2026-10-25".into())
            }]
        );
        assert_eq!(group(&fixture, end), Vec::<i64>::new());
        assert_eq!(
            fixture.store.deadline_correction("b", id).expect("foreign"),
            None
        );
        assert!(
            fixture
                .store
                .correct_deadline(&CorrectDeadline {
                    account: "b".into(),
                    ..edit
                })
                .is_err()
        );

        fixture.store.clear_cache("a").expect("clear cache");
        assert_eq!(
            fixture
                .store
                .deadline_correction("a", id)
                .expect("saved correction"),
            Some(corrected)
        );
        assert!(matches!(
            fixture.store.message("a", unowned),
            Err(Error::MissingMessage)
        ));
    }

    #[test]
    fn corrections_never_start_priority_for_expired_dismissed_or_read_originals() {
        let mut fixture = fixture();
        let inbox = fixture.inbox;
        let now = 1_792_908_000;
        let expired = cache(&mut fixture, inbox, 1, false, now - 200);
        let dismissed = cache(&mut fixture, inbox, 2, false, now);
        let read = cache(&mut fixture, inbox, 3, true, now);
        give_priority(&mut fixture, expired, now - 1);
        give_priority(&mut fixture, dismissed, now + 100);
        give_priority(&mut fixture, read, now + 100);
        fixture.store.dismiss_priority(dismissed).expect("dismiss");
        for id in [expired, dismissed, read] {
            fixture
                .store
                .correct_deadline(&correction(id, now))
                .expect("save evidence");
            give_priority(&mut fixture, id, now + 9000);
        }
        assert_eq!(group(&fixture, now), Vec::<i64>::new());
    }

    #[test]
    fn an_explicit_time_in_a_dst_fold_or_gap_is_rejected() {
        let mut edit = correction(1, 0);
        edit.time = NaiveTime::from_hms_opt(2, 30, 0);
        assert_eq!(edit.resolve(), None);
        edit.date = NaiveDate::from_ymd_opt(2026, 3, 29).expect("gap date");
        assert_eq!(edit.resolve(), None);
    }

    #[test]
    fn invalid_limits_are_refused() {
        let fixture = fixture();
        assert!(
            fixture
                .store
                .analysis_pending(&accounts(), 1, 0, 0)
                .is_err()
        );
        assert!(fixture.store.priority_items(&accounts(), 0, 0).is_err());
    }
}
