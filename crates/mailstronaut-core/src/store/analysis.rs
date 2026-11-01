//! Stored priority results, and which messages are in the priority group right now.
//!
//! The store owns final eligibility, not whatever produced a result: reading, dismissing
//! and expiry are decided here, so that a late result can never bring a message back.

use rusqlite::{OptionalExtension as _, params};

use super::{Error, Store};

pub(super) const SCHEMA: &str = "
ALTER TABLE analysis ADD COLUMN policy INTEGER NOT NULL DEFAULT 0;
ALTER TABLE analysis ADD COLUMN expires_at INTEGER;
ALTER TABLE analysis ADD COLUMN due TEXT;
ALTER TABLE analysis ADD COLUMN dismissed INTEGER NOT NULL DEFAULT 0 CHECK(dismissed IN (0, 1));
CREATE INDEX analysis_expiry ON analysis(expires_at) WHERE expires_at IS NOT NULL;
";

/// A newly arrived message, as far as announcing it needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewMail {
    /// Canonical message ID.
    pub id: i64,
    /// Displayed sender.
    pub sender: String,
    /// Decoded subject.
    pub subject: String,
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
        let tx = self.connection.transaction()?;
        for result in results {
            tx.execute(
                "INSERT INTO analysis(message_id,data,policy,expires_at,due)
                 SELECT id,?2,?3,?4,?5 FROM messages WHERE id=?1
                 ON CONFLICT(message_id) DO UPDATE SET data=excluded.data,policy=excluded.policy,expires_at=excluded.expires_at,due=excluded.due",
                params![result.id, result.data, result.policy, result.expires_at, result.due],
            )?;
        }
        tx.commit()?;
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
            "SELECT m.id,a.expires_at,a.due FROM analysis a JOIN messages m ON m.id=a.message_id
             WHERE a.expires_at>?2 AND a.dismissed=0 AND m.read_cleared=0
             AND m.account_id IN (SELECT value FROM json_each(?1)) AND {IN_INBOX}
             ORDER BY a.expires_at,m.id DESC LIMIT ?3"
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
            "SELECT m.id,m.sender,m.subject FROM messages m
             WHERE m.account_id=?1 AND m.id>?2 AND m.id<=?3 AND m.received_at>=?4 AND m.read_cleared=0
             AND {IN_INBOX} ORDER BY m.received_at DESC,m.id DESC LIMIT ?5"
        ))?;
        let mail = query
            .query_map(params![account, after, newest, since, limit], |row| {
                Ok(NewMail {
                    id: row.get(0)?,
                    sender: row.get(1)?,
                    subject: row.get(2)?,
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
