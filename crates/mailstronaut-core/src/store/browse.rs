//! Local account/folder navigation and literal full-text queries.

use super::{
    Error, Store,
    mailbox::{Cursor, Folder},
};
use rusqlite::params;

/// One explicit local scope. All reads remain restricted to enabled accounts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Scope {
    /// Configured inbox memberships across enabled accounts.
    #[default]
    Inbox,
    /// All currently cached folder memberships in this account.
    Account(String),
    /// One account-bound opaque server mailbox identity.
    Folder {
        /// Enabled configuration account identity.
        account: String,
        /// Local folder identity belonging to that account.
        folder: i64,
    },
}
impl Scope {
    pub(super) fn parts<'a>(
        &'a self,
        accounts: &[String],
    ) -> Result<(Option<&'a str>, Option<i64>, bool), Error> {
        let (account, folder, inbox) = match self {
            Self::Inbox => (None, None, true),
            Self::Account(account) => (Some(account.as_str()), None, false),
            Self::Folder { account, folder } if *folder > 0 => {
                (Some(account.as_str()), Some(*folder), false)
            }
            Self::Folder { .. } => return Err(Error::InvalidInput),
        };
        if account.is_some_and(|account| !accounts.iter().any(|enabled| enabled == account)) {
            return Err(Error::InvalidInput);
        }
        Ok((account, folder, inbox))
    }
}

/// Explicit metadata filters, applied to the same original as the text query.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchFilter {
    /// Inclusive arrival instant, in UTC seconds.
    pub received_from: Option<i64>,
    /// Exclusive arrival instant, in UTC seconds.
    pub received_before: Option<i64>,
    /// Literal substring of parsed From names/addresses; ASCII case-insensitive.
    pub sender: String,
}

impl SearchFilter {
    /// Whether no metadata restriction was requested.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.sender.len() > 256
            || self.sender.chars().any(char::is_control)
            || matches!((self.received_from, self.received_before), (Some(from), Some(before)) if from >= before)
        {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}

/// One bounded filtered conversation read. Ranked hits come from the local index.
#[derive(Clone)]
pub struct FilteredPage {
    /// Enabled configuration account identities.
    pub accounts: Vec<String>,
    /// Selected account/folder/inbox scope.
    pub scope: Scope,
    /// Literal fallback query, used only without ranked hits.
    pub query: String,
    /// Ranked originals, if the local search index answered.
    pub hits: Option<Vec<i64>>,
    /// Original-level metadata restrictions.
    pub filter: SearchFilter,
    /// Last returned original from the previous batch.
    pub after: Option<Cursor>,
    /// Batch size, at most 100.
    pub limit: u32,
}

/// Plain whitespace-separated terms, each quoted as an FTS5 phrase. Users do not
/// supply MATCH operators/column selectors. SQLite owns Unicode tokenization.
fn search_expression(input: &str) -> Result<Option<String>, Error> {
    if input.len() > 1024 || input.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidInput);
    }
    let terms = input.split_whitespace().collect::<Vec<_>>();
    if terms.len() > 32 {
        return Err(Error::InvalidInput);
    }
    Ok((!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    }))
}

/// A message's copies within the requested scope, for a query over `{m}`.
fn scoped_copies(message: &str, account: &str) -> String {
    format!(
        "effective_memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
         WHERE s.message_id={message} AND s.account_id={account} AND (?6 IS NULL OR f.id=?6) AND (?8=0 OR f.is_inbox=1)"
    )
}

/// A page with no search: the conversations whose newest in-scope message comes next.
/// Walking messages newest first stops as soon as the page is full, where the complete
/// query has to rank a whole mailbox first. `page_without_a_search_agrees_with_the_complete_query`
/// holds the two together.
fn browse_page() -> String {
    let scoped = scoped_copies("m.id", "m.account_id");
    let newer = scoped_copies("newer.id", "newer.account_id");
    let own = scoped_copies("p.id", "p.account_id");
    let member = scoped_copies("g.id", "p.account_id");
    let members = "(SELECT other.message_id AS id FROM conversation_members other WHERE other.account_id=p.account_id AND other.conversation_id=p.conversation UNION ALL SELECT p.id WHERE p.conversation IS NULL) g";
    format!("
WITH page AS (
 SELECT m.id,m.account_id,m.subject,m.sender,m.preview,m.received_at,
 CASE WHEN a.projected_thread_revision=a.thread_revision THEN cm.conversation_id END AS conversation
 FROM messages m INDEXED BY inbox_order
 JOIN accounts a ON a.id=m.account_id
 LEFT JOIN conversation_members cm ON cm.account_id=m.account_id AND cm.message_id=m.id
 WHERE m.account_id IN (SELECT value FROM json_each(?1)) AND (?5 IS NULL OR m.account_id=?5)
 AND ?9 IS NULL AND ?10 IS NULL AND ?11=''
 AND (?2 IS NULL OR m.received_at<?2 OR (m.received_at=?2 AND m.id<?3))
 AND EXISTS(SELECT 1 FROM {scoped})
 AND NOT EXISTS(
  SELECT 1 FROM conversation_members other JOIN messages newer ON newer.id=other.message_id
  WHERE a.projected_thread_revision=a.thread_revision
  AND other.account_id=cm.account_id AND other.conversation_id=cm.conversation_id
  AND (newer.received_at>m.received_at OR (newer.received_at=m.received_at AND newer.id>m.id))
  AND EXISTS(SELECT 1 FROM {newer}))
 ORDER BY m.received_at DESC,m.id DESC LIMIT ?4
)
SELECT p.id,p.account_id,p.subject,p.sender,p.preview,p.received_at,
 (SELECT MIN(s.seen) FROM {own}),(SELECT MAX(s.starred) FROM {own}),p.conversation,
 CASE WHEN p.conversation IS NULL THEN 1 ELSE (
 SELECT COUNT(*) FROM conversation_members cm WHERE cm.account_id=p.account_id AND cm.conversation_id=p.conversation
 AND EXISTS(SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity WHERE s.account_id=cm.account_id AND s.message_id=cm.message_id)) END,
 (SELECT COUNT(*) FROM {members} WHERE EXISTS(SELECT 1 FROM {member} AND s.seen=0)),
 (SELECT COALESCE(MAX(EXISTS(SELECT 1 FROM {member} AND s.starred=1)),0) FROM {members}),
 (SELECT MAX(f.is_inbox) FROM {own})
FROM page p ORDER BY p.received_at DESC,p.id DESC")
}

/// One conversation page, by how it selects and orders the matching messages.
struct PageQuery {
    hits: &'static str,
    source: &'static str,
    matches: &'static str,
    join: &'static str,
    member_order: &'static str,
    after: &'static str,
    order: &'static str,
}
const LITERAL_PAGE: PageQuery = PageQuery {
    hits: "",
    source: "messages m",
    matches: "CASE WHEN ?7 IS NULL THEN 1 ELSE m.id IN (SELECT rowid FROM message_search WHERE message_search MATCH ?7) END AS matches,0 AS hit_rank",
    join: "",
    member_order: "",
    after: "(?2 IS NULL OR received_at<?2 OR (received_at=?2 AND id<?3))",
    order: "received_at DESC,id DESC",
};
const RANKED_PAGE: PageQuery = PageQuery {
    // Only the hits and their conversations, rather than ranking a whole mailbox.
    hits: "hits(id,hit_rank) AS MATERIALIZED (SELECT value,key FROM json_each(?7)),
 wanted(id) AS MATERIALIZED (
  SELECT id FROM hits UNION
  SELECT others.message_id FROM hits h CROSS JOIN messages hit ON hit.id=h.id
  CROSS JOIN conversation_members mine ON mine.account_id=hit.account_id AND mine.message_id=hit.id
  CROSS JOIN conversation_members others ON others.account_id=mine.account_id AND others.conversation_id=mine.conversation_id
 ),",
    source: "wanted w CROSS JOIN messages m ON m.id=w.id",
    matches: "h.id IS NOT NULL AS matches,h.hit_rank AS hit_rank",
    join: "LEFT JOIN hits h ON h.id=m.id",
    member_order: "hit_rank ASC,",
    after: "(?2 IS NULL OR hit_rank>COALESCE((SELECT hit_rank FROM hits WHERE id=?3),-1))",
    order: "hit_rank ASC",
};
impl PageQuery {
    fn sql(&self) -> String {
        format!("
WITH {hits}inbox AS (
 SELECT m.id,m.account_id,m.subject,m.sender,m.preview,m.received_at,MIN(s.seen) AS seen,MAX(s.starred) AS starred,MAX(f.is_inbox) AS inbox,
 ((?9 IS NULL OR m.received_at>=?9) AND (?10 IS NULL OR m.received_at<?10)
 AND CASE WHEN ?11='' THEN 1 ELSE instr(lower({sender_text}),lower(?11))>0 END) AND {matches}
 FROM {source} JOIN effective_memberships s ON s.message_id=m.id AND s.account_id=m.account_id
 JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
 {join}
 WHERE m.account_id IN (SELECT value FROM json_each(?1))
 AND (?5 IS NULL OR m.account_id=?5) AND (?6 IS NULL OR f.id=?6)
 AND (?8=0 OR f.is_inbox=1) GROUP BY m.id
), tagged AS (
 SELECT i.*,CASE WHEN a.projected_thread_revision=a.thread_revision THEN cm.conversation_id ELSE NULL END AS conversation,
 CASE WHEN a.projected_thread_revision=a.thread_revision AND cm.conversation_id IS NOT NULL THEN cm.conversation_id ELSE -i.id END AS group_key
 FROM inbox i JOIN accounts a ON a.id=i.account_id LEFT JOIN conversation_members cm ON cm.account_id=i.account_id AND cm.message_id=i.id
), ranked AS (
 SELECT *,SUM(NOT seen) OVER(PARTITION BY account_id,group_key) AS unread_count,
 MAX(starred) OVER(PARTITION BY account_id,group_key) AS thread_starred,
 MAX(matches) OVER(PARTITION BY account_id,group_key) AS any_match,
 ROW_NUMBER() OVER(PARTITION BY account_id,group_key ORDER BY matches DESC,{member_order}received_at DESC,id DESC) AS position FROM tagged
)
SELECT id,account_id,subject,sender,preview,received_at,seen,starred,conversation,
 CASE WHEN conversation IS NULL THEN 1 ELSE (
 SELECT COUNT(*) FROM conversation_members cm WHERE cm.account_id=r.account_id AND cm.conversation_id=r.conversation
 AND EXISTS(SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity WHERE s.account_id=cm.account_id AND s.message_id=cm.message_id)) END,
 unread_count,thread_starred,inbox FROM ranked r
WHERE position=1 AND any_match=1 AND {after}
ORDER BY {order} LIMIT ?4",
            hits = self.hits,
            source = self.source,
            matches = self.matches,
            join = self.join,
            member_order = self.member_order,
            after = self.after,
            order = self.order,
            sender_text = super::search::SENDER_TEXT,
        )
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 4096 && !name.contains(['\0', '\r', '\n'])
}

impl Store {
    /// Remember selectable mailboxes from a successful authenticated LIST. Existing
    /// cached/removed folders are retained for offline use; only the configured
    /// inbox participates in the unified inbox after discovery.
    /// # Errors
    /// Rejects invalid/excessive names and storage failures. Caller binds the
    /// authenticated mailbox context first; this API never authenticates itself.
    pub fn remember_folders(
        &mut self,
        account: &str,
        names: &[String],
        inbox: &str,
    ) -> Result<(), Error> {
        if names.len() > 4096
            || names.iter().map(String::len).sum::<usize>() > 4 * 1024 * 1024
            || !valid_name(inbox)
            || names.iter().any(|name| !valid_name(name))
        {
            return Err(Error::InvalidInput);
        }
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE folders SET is_inbox=0 WHERE account_id=?1",
            [account],
        )?;
        for name in names {
            let is_inbox = name == inbox
                || (inbox.eq_ignore_ascii_case("INBOX") && name.eq_ignore_ascii_case("INBOX"));
            tx.execute("INSERT INTO folders(account_id,name,is_inbox) VALUES(?1,?2,?3) ON CONFLICT(account_id,name) DO UPDATE SET is_inbox=excluded.is_inbox",params![account,name,is_inbox])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Enumerate at most 4096 known folders, including discovered uncached ones.
    /// Wire names are display text only and never filesystem paths.
    /// # Errors
    /// Rejects excessive selection/results and storage failures.
    pub fn folders(&self, accounts: &[String]) -> Result<Vec<Folder>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let mut query=self.connection.prepare_cached("SELECT id,account_id,name,uidvalidity,is_inbox,role FROM folders WHERE account_id IN (SELECT value FROM json_each(?1)) ORDER BY account_id,is_inbox DESC,CASE role WHEN 'sent' THEN 0 WHEN 'drafts' THEN 1 WHEN 'archive' THEN 2 WHEN 'junk' THEN 3 WHEN 'trash' THEN 4 ELSE 5 END,name LIMIT 4097")?;
        let rows = query
            .query_map(
                [serde_json::to_string(accounts)?],
                super::mailbox::read_folder,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() > 4096 {
            return Err(Error::InvalidInput);
        }
        Ok(rows)
    }

    /// Unread messages per folder as the user currently sees them: a pending read
    /// counts as read, and a message being moved away no longer counts.
    /// # Errors
    /// Rejects excessive account lists and storage failures.
    pub fn unread_counts(&self, accounts: &[String]) -> Result<Vec<(i64, u32)>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let mut query = self.connection.prepare_cached(
            "SELECT f.id,COUNT(*) FROM effective_memberships s
             JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
             WHERE s.seen=0 AND s.account_id IN (SELECT value FROM json_each(?1))
             GROUP BY f.id",
        )?;
        query
            .query_map([serde_json::to_string(accounts)?], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// Page local conversations in a configured inbox/account/folder scope.
    /// Search opens the newest matching member while counts describe the complete
    /// selected scope. Search never calls mail transports or accesses remote data.
    /// # Errors
    /// Rejects disabled accounts, foreign folders, query/page limits and storage errors.
    pub fn conversation_page(
        &self,
        accounts: &[String],
        scope: &Scope,
        query: &str,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<super::conversations::Summary>, Error> {
        let search = search_expression(query)?;
        if search.is_none() {
            return self.page(accounts, scope, &browse_page(), None, after, limit);
        }
        self.page(
            accounts,
            scope,
            &LITERAL_PAGE.sql(),
            search.as_deref(),
            after,
            limit,
        )
    }

    /// Like [`Self::conversation_page`], for messages an index has already matched: they
    /// keep the given order, and the page after a cursor continues from its place in it.
    /// # Errors
    /// Rejects disabled accounts, foreign folders, page limits and storage errors.
    pub fn ranked_conversation_page(
        &self,
        accounts: &[String],
        scope: &Scope,
        hits: &[i64],
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<super::conversations::Summary>, Error> {
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let hits = serde_json::to_string(hits)?;
        self.page(
            accounts,
            scope,
            &RANKED_PAGE.sql(),
            Some(&hits),
            after,
            limit,
        )
    }

    fn page(
        &self,
        accounts: &[String],
        scope: &Scope,
        statement: &str,
        search: Option<&str>,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<super::conversations::Summary>, Error> {
        self.filtered_page(
            accounts,
            scope,
            statement,
            search,
            after,
            limit,
            &SearchFilter::default(),
        )
    }

    /// Read filtered originals before conversation grouping, preserving complete
    /// in-scope unread counts and selecting a matching original for opening.
    /// # Errors
    /// Rejects invalid ranges, limits, scopes and storage failures.
    pub fn filtered_conversation_page(
        &self,
        request: &FilteredPage,
    ) -> Result<Vec<super::conversations::Summary>, Error> {
        request.filter.validate()?;
        let (statement, search) = match &request.hits {
            Some(hits) if hits.is_empty() => return Ok(Vec::new()),
            Some(hits) => (RANKED_PAGE.sql(), Some(serde_json::to_string(hits)?)),
            None => (LITERAL_PAGE.sql(), search_expression(&request.query)?),
        };
        self.filtered_page(
            &request.accounts,
            &request.scope,
            &statement,
            search.as_deref(),
            request.after,
            request.limit,
            &request.filter,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Internal SQL bindings share the existing bounded page inputs and metadata filters."
    )]
    fn filtered_page(
        &self,
        accounts: &[String],
        scope: &Scope,
        statement: &str,
        search: Option<&str>,
        after: Option<Cursor>,
        limit: u32,
        filter: &SearchFilter,
    ) -> Result<Vec<super::conversations::Summary>, Error> {
        if accounts.len() > 256 || !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let (account, folder, inbox_only) = scope.parts(accounts)?;
        if let Some(folder) = folder {
            let known: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM folders WHERE account_id=?1 AND id=?2)",
                params![account, folder],
                |row| row.get(0),
            )?;
            if !known {
                return Err(Error::InvalidInput);
            }
        }
        let selected = serde_json::to_string(accounts)?;
        let mut statement = self.connection.prepare_cached(statement)?;
        statement
            .query_map(
                params![
                    selected,
                    after.map(|cursor| cursor.received_at),
                    after.map(|cursor| cursor.message_id),
                    limit,
                    account,
                    folder,
                    search,
                    inbox_only,
                    filter.received_from,
                    filter.received_before,
                    filter.sender
                ],
                |row| {
                    Ok(super::conversations::Summary {
                        latest: {
                            let mut message = super::mailbox::read_summary(row)?;
                            message.inbox = row.get(12)?;
                            message
                        },
                        conversation: row.get(8)?,
                        cached_messages: row.get(9)?,
                        unread: row.get(10)?,
                        starred: row.get(11)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::mailbox::{Flags, FolderRole, Membership},
        threads,
    };
    use core::num::NonZeroU32;
    use mailstronaut_mail::imap::FlagChange;
    fn cache(
        store: &mut Store,
        account: &str,
        folder: i64,
        uid: u32,
        parent: bool,
        body: &str,
    ) -> i64 {
        let raw = format!(
            "From: Named Sender <sender@example.test>\r\nTo: reader@example.test\r\nSubject: Café dossier\r\nMessage-ID: <{uid}@{account}.test>\r\nReferences: <root@{account}.test>\r\n\r\n{body}"
        );
        store
            .cache_message(
                account,
                Membership {
                    folder,
                    uid_validity: NonZeroU32::new(42).expect("namespace"),
                    uid: NonZeroU32::new(uid).expect("UID"),
                    flags: Flags {
                        seen: parent,
                        starred: false,
                    },
                },
                i64::from(uid),
                None,
                raw.as_bytes(),
            )
            .expect("message")
    }
    fn project(store: &mut Store, account: &str) {
        let snapshot = store
            .conversation_snapshot(account)
            .expect("snapshot")
            .expect("stale");
        let groups = threads::project(&snapshot.messages).expect("synthetic grouping");
        store
            .commit_conversations(&snapshot, &groups)
            .expect("commit");
    }
    fn fixture() -> (tempfile::TempDir, Store, [i64; 4], [i64; 1]) {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        for account in ["a", "b"] {
            store.ensure_account(account).expect("account");
        }
        let inbox = store.ensure_folder("a", "INBOX", true).expect("inbox").id;
        let sent = store.ensure_folder("a", "Sent", false).expect("sent").id;
        let other = store.ensure_folder("b", "INBOX", true).expect("other").id;
        for (account, folder) in [("a", inbox), ("a", sent), ("b", other)] {
            store
                .select_folder(account, folder, NonZeroU32::new(42).expect("namespace"))
                .expect("select");
        }
        let needle = cache(&mut store, "a", inbox, 1, false, "Needle questionnaire");
        let latest = cache(&mut store, "a", inbox, 2, false, "Later ordinary reply");
        let ancestor = cache(&mut store, "a", sent, 3, true, "Private sent marker");
        let foreign = cache(
            &mut store,
            "b",
            other,
            4,
            false,
            "Needle other-account marker",
        );
        project(&mut store, "a");
        project(&mut store, "b");
        (root, store, [needle, latest, ancestor, foreign], [sent])
    }
    #[test]
    fn metadata_filters_select_matching_originals_without_clearing_unread_siblings() {
        let (_root, store, [needle, latest, _, foreign], _) = fixture();
        for hits in [None, Some(vec![foreign, latest, needle])] {
            let mut request = FilteredPage {
                accounts: vec!["a".into(), "b".into()],
                scope: Scope::Inbox,
                query: String::new(),
                hits,
                filter: SearchFilter {
                    received_from: Some(1),
                    received_before: Some(2),
                    sender: "SENDER@EXAMPLE.TEST".into(),
                },
                after: None,
                limit: 100,
            };
            let rows = store
                .filtered_conversation_page(&request)
                .expect("filtered read");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].latest.id, needle);
            assert_eq!(rows[0].latest.sender, "Named Sender");
            assert_eq!(rows[0].unread, 2, "unmatched sibling still unread");
            assert!(!rows[0].latest.seen);
            request.filter.received_from = Some(2);
            assert!(
                store.filtered_conversation_page(&request).is_err(),
                "empty/reversed interval rejected"
            );
            request.filter = SearchFilter {
                sender: "sender%_@example.test".into(),
                ..SearchFilter::default()
            };
            assert!(
                store
                    .filtered_conversation_page(&request)
                    .expect("literal sender")
                    .is_empty(),
                "wildcards are literal"
            );
            request.filter.sender = "reader@example.test".into();
            assert!(
                store
                    .filtered_conversation_page(&request)
                    .expect("recipient")
                    .is_empty(),
                "a From restriction never matches To"
            );
        }
    }

    #[test]
    fn sender_only_filters_page_and_scope_without_hiding_thread_members() {
        let (_root, store, [needle, latest, ancestor, foreign], _) = fixture();
        let mut request = FilteredPage {
            accounts: vec!["a".into(), "b".into()],
            scope: Scope::Inbox,
            query: String::new(),
            hits: None,
            filter: SearchFilter {
                sender: "SENDER@EXAMPLE.TEST".into(),
                ..SearchFilter::default()
            },
            after: None,
            limit: 1,
        };
        let first = store.filtered_conversation_page(&request).expect("first");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].latest.id, foreign);
        request.after = Some(Cursor {
            received_at: first[0].latest.received_at,
            message_id: foreign,
        });
        let second = store.filtered_conversation_page(&request).expect("second");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].latest.id, latest);
        assert_eq!(second[0].unread, 2);
        request.after = None;
        request.scope = Scope::Account("a".into());
        assert_eq!(
            store.filtered_conversation_page(&request).expect("scope")[0]
                .latest
                .id,
            ancestor
        );
        request.query = "Needle".into();
        assert_eq!(
            store.filtered_conversation_page(&request).expect("query")[0]
                .latest
                .id,
            needle
        );
        request.filter.received_from = Some(2);
        assert!(
            store
                .filtered_conversation_page(&request)
                .expect("same original")
                .is_empty(),
            "text and date must match the same original, not separate thread members"
        );
    }

    #[test]
    fn local_search_opens_a_matching_member_and_respects_account_folder_and_namespace() {
        let (root, mut store, [needle, latest, ancestor, foreign], [sent]) = fixture();
        let accounts = vec!["a".into(), "b".into()];
        let results = store
            .conversation_page(&accounts, &Scope::Inbox, "needle", None, 100)
            .expect("search");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].latest.id, foreign);
        assert_eq!(results[1].latest.id, needle);
        assert_eq!(
            results[1].unread, 2,
            "counts include the nonmatching sibling"
        );
        assert_eq!(
            store
                .conversation_page(&accounts, &Scope::Inbox, "", None, 100)
                .expect("latest")[1]
                .latest
                .id,
            latest
        );
        let result = store
            .conversation_page(
                &accounts,
                &Scope::Account("a".into()),
                "sent marker",
                None,
                100,
            )
            .expect("account search");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].latest.id, ancestor);
        assert!(!result[0].latest.inbox);
        assert_eq!(
            store
                .conversation_page(&accounts, &Scope::Inbox, "sent marker", None, 100)
                .expect("inbox excludes Sent"),
            Vec::new()
        );
        let scope = Scope::Folder {
            account: "a".into(),
            folder: sent,
        };
        assert_eq!(
            store
                .conversation_page(&accounts, &scope, "cafe", None, 100)
                .expect("Unicode search")[0]
                .latest
                .id,
            ancestor
        );
        assert!(matches!(
            store.conversation_page(&["a".into()], &Scope::Account("b".into()), "", None, 100),
            Err(Error::InvalidInput)
        ));
        assert!(matches!(
            store.conversation_page(
                &accounts,
                &Scope::Folder {
                    account: "b".into(),
                    folder: sent
                },
                "",
                None,
                100
            ),
            Err(Error::InvalidInput)
        ));
        assert_eq!(
            store.folders(&["a".into()]).expect("known folders").len(),
            2
        );
        store
            .select_folder("a", sent, NonZeroU32::new(43).expect("changed namespace"))
            .expect("retire old Sent");
        assert_eq!(
            store
                .conversation_page(&accounts, &scope, "sent marker", None, 100)
                .expect("no stale membership"),
            Vec::new()
        );
        drop(store);
        let store = Store::open(root.path()).expect("offline restart");
        assert_eq!(
            store
                .conversation_page(&["a".into()], &Scope::Inbox, "questionnaire", None, 100)
                .expect("persistent index")[0]
                .latest
                .id,
            needle
        );
    }
    #[test]
    fn authenticated_catalog_keeps_wire_names_and_offline_history_but_replaces_the_inbox_role() {
        let (_root, mut store, _, [sent]) = fixture();
        let names = vec!["Custom inbox".into(), "&AMk-tudes".into(), "INBOX".into()];
        store
            .remember_folders("a", &names, "Custom inbox")
            .expect("catalog");
        let folders = store.folders(&["a".into(), "b".into()]).expect("folders");
        assert!(
            folders.iter().any(|folder| folder.account == "a"
                && folder.name == "Custom inbox"
                && folder.inbox)
        );
        assert!(
            folders
                .iter()
                .any(|folder| folder.account == "a" && folder.name == "&AMk-tudes")
        );
        assert!(folders.iter().any(|folder| folder.account == "a"
            && folder.id == sent
            && folder.uid_validity.is_some()));
        assert!(
            !folders
                .iter()
                .any(|folder| folder.account == "a" && folder.name == "INBOX" && folder.inbox)
        );
        assert!(
            folders
                .iter()
                .any(|folder| folder.account == "b" && folder.inbox)
        );
        assert!(
            store
                .remember_folders("a", &["new".into(), "bad\r\nSELECT".into()], "INBOX")
                .is_err()
        );
        assert_eq!(
            store
                .folders(&["a".into(), "b".into()])
                .expect("unchanged catalog"),
            folders
        );
    }
    /// Everything the list shows for a row.
    fn summary(row: &crate::store::conversations::Summary) -> impl PartialEq + core::fmt::Debug {
        (
            (
                row.latest.id,
                row.latest.account.clone(),
                row.latest.subject.clone(),
                row.latest.sender.clone(),
                row.latest.preview.clone(),
                row.latest.received_at,
            ),
            (
                row.latest.seen,
                row.latest.starred,
                row.latest.inbox,
                row.conversation,
                row.cached_messages,
                row.unread,
                row.starred,
            ),
        )
    }

    fn agree(store: &Store, accounts: &[String], scope: &Scope) {
        for limit in [100, 7] {
            let mut after = None;
            loop {
                let fast = store
                    .conversation_page(accounts, scope, "", after, limit)
                    .expect("page");
                let complete = store
                    .page(accounts, scope, &LITERAL_PAGE.sql(), None, after, limit)
                    .expect("complete");
                assert_eq!(
                    fast.iter().map(summary).collect::<Vec<_>>(),
                    complete.iter().map(summary).collect::<Vec<_>>(),
                    "{scope:?} after {after:?}"
                );
                let Some(last) = fast.last() else { break };
                after = Some(Cursor {
                    received_at: last.latest.received_at,
                    message_id: last.latest.id,
                });
            }
        }
    }

    /// Intentions that are not on the server yet change what the list shows.
    fn add_pending_intentions(store: &mut Store, messages: &[(&str, i64)]) {
        const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for (index, (account, id)) in messages.iter().enumerate() {
            match index % 11 {
                0 => store
                    .set_flag(
                        account,
                        *id,
                        mailstronaut_mail::imap::FlagChange::Seen,
                        true,
                        CONTEXT,
                    )
                    .expect("read"),
                3 => store
                    .set_flag(
                        account,
                        *id,
                        mailstronaut_mail::imap::FlagChange::Starred,
                        true,
                        CONTEXT,
                    )
                    .expect("star"),
                7 => {
                    // A message with nothing left to move is fine here.
                    _ = store.queue_move(account, *id, None, "Archive", CONTEXT);
                }
                _ => {}
            }
        }
    }

    /// Random conversations, copies, ties, pending flags and moves in two accounts.
    fn random_mailbox(seed: u64) -> (tempfile::TempDir, Store, Vec<(&'static str, i64)>) {
        use crate::store::mailbox::FolderRole;
        const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        let mut folders = Vec::new();
        for account in ["a", "b"] {
            store.ensure_account(account).expect("account");
            store
                .bind_mailbox_context(account, CONTEXT)
                .expect("context");
            for (name, inbox) in [("INBOX", true), ("Archive", false), ("Sent", false)] {
                let id = store
                    .ensure_folder(account, name, inbox)
                    .expect("folder")
                    .id;
                store
                    .select_folder(account, id, NonZeroU32::new(42).expect("namespace"))
                    .expect("generation");
                folders.push((account, id));
            }
            store
                .assign_folder_roles(account, &[("Archive".into(), FolderRole::Archive)])
                .expect("role");
        }
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut messages = Vec::new();
        for uid in 1..=180_u32 {
            let (account, folder) = folders[usize::try_from(next() % 6).expect("index")];
            // Equal dates, so ties have to break the same way too.
            let received_at = i64::try_from(next() % 40).expect("date");
            let thread = next() % 12;
            let raw = format!(
                "From: sender@example.test\r\nTo: reader@example.test\r\nSubject: Thread {thread}\r\nMessage-ID: <{uid}@{account}.test>\r\nReferences: <root{thread}@{account}.test>\r\n\r\nBody {uid}."
            );
            let flags = Flags {
                seen: next() % 3 == 0,
                starred: next() % 5 == 0,
            };
            let id = store
                .cache_message(
                    account,
                    Membership {
                        folder,
                        uid_validity: NonZeroU32::new(42).expect("namespace"),
                        uid: NonZeroU32::new(uid).expect("UID"),
                        flags,
                    },
                    received_at,
                    None,
                    raw.as_bytes(),
                )
                .expect("message");
            // Some originals have a second copy elsewhere with its own flags.
            if next() % 4 == 0 {
                let copy = folders
                    .iter()
                    .find(|(owner, other)| *owner == account && *other != folder)
                    .expect("another folder");
                store
                    .cache_message(
                        account,
                        Membership {
                            folder: copy.1,
                            uid_validity: NonZeroU32::new(42).expect("namespace"),
                            uid: NonZeroU32::new(uid + 1000).expect("UID"),
                            flags: Flags {
                                seen: next() % 2 == 0,
                                starred: false,
                            },
                        },
                        received_at,
                        None,
                        raw.as_bytes(),
                    )
                    .expect("copy");
            }
            messages.push((account, id));
        }
        project(&mut store, "a");
        // Leave account b with a stale projection on odd seeds, so it lists ungrouped.
        if seed.is_multiple_of(2) {
            project(&mut store, "b");
        }
        add_pending_intentions(&mut store, &messages);
        (root, store, folders)
    }

    #[test]
    fn page_without_a_search_agrees_with_the_complete_query() {
        for seed in [1_u64, 2, 3, 4] {
            let (_root, store, folders) = random_mailbox(seed);
            let accounts = ["a".to_owned(), "b".to_owned()];
            agree(&store, &accounts, &Scope::Inbox);
            agree(&store, &accounts, &Scope::Account("a".into()));
            agree(&store, &accounts, &Scope::Account("b".into()));
            for (account, folder) in &folders {
                agree(
                    &store,
                    &accounts,
                    &Scope::Folder {
                        account: (*account).to_owned(),
                        folder: *folder,
                    },
                );
            }
            agree(&store, &["a".to_owned()], &Scope::Inbox);
        }
    }

    #[test]
    fn ranked_pages_keep_the_given_order_group_conversations_and_respect_scope() {
        let (_root, store, [needle, latest, ancestor, foreign], [sent]) = fixture();
        let accounts = ["a".to_owned(), "b".to_owned()];
        let page = |scope: &Scope, hits: &[i64], after, limit| {
            store
                .ranked_conversation_page(&accounts, scope, hits, after, limit)
                .expect("ranked page")
                .into_iter()
                .map(|row| row.latest.id)
                .collect::<Vec<_>>()
        };
        let all = Scope::Inbox;
        // Account a's three messages are one conversation: one row, at its best-ranked member.
        assert_eq!(page(&all, &[latest, needle], None, 100), [latest]);
        assert_eq!(page(&all, &[needle, latest], None, 100), [needle]);
        // The given order wins, whatever the dates say.
        assert_eq!(page(&all, &[foreign, needle], None, 100), [foreign, needle]);
        assert_eq!(page(&all, &[needle, foreign], None, 100), [needle, foreign]);
        // Scope still applies, and is checked here rather than trusted from the index.
        let account = Scope::Account("a".into());
        assert_eq!(page(&account, &[foreign, needle], None, 100), [needle]);
        let folder = Scope::Folder {
            account: "a".into(),
            folder: sent,
        };
        assert_eq!(page(&folder, &[needle, ancestor], None, 100), [ancestor]);
        // Unknown IDs simply do not appear, and no hits means no rows.
        assert_eq!(page(&all, &[999_999, needle], None, 100), [needle]);
        assert_eq!(page(&all, &[], None, 100), Vec::<i64>::new());
        // Paging continues from the cursor's place in the order.
        assert_eq!(page(&all, &[foreign, needle], None, 1), [foreign]);
        let cursor = |message_id| {
            Some(Cursor {
                received_at: 0,
                message_id,
            })
        };
        assert_eq!(page(&all, &[foreign, needle], cursor(foreign), 1), [needle]);
        assert_eq!(
            page(&all, &[foreign, needle], cursor(needle), 1),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn unread_counts_follow_what_the_user_sees_in_each_folder() {
        const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let root = tempfile::tempdir().expect("fixture");
        let mut store = Store::open(root.path()).expect("open");
        // Binding resets folder generations, so it comes first.
        store.ensure_account("a").expect("account");
        store.bind_mailbox_context("a", CONTEXT).expect("context");
        let mut folder = |account: &str, name: &str, inbox: bool| {
            store.ensure_account(account).expect("account");
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
        let other = folder("b", "INBOX", true);
        store
            .assign_folder_roles("a", &[("Archive".into(), FolderRole::Archive)])
            .expect("role");
        let first = cache(&mut store, "a", inbox, 1, false, "one");
        cache(&mut store, "a", inbox, 2, false, "two");
        let third = cache(&mut store, "a", inbox, 3, false, "three");
        cache(&mut store, "a", inbox, 4, true, "already read");
        cache(&mut store, "a", archive, 5, false, "archived");
        cache(&mut store, "b", other, 1, false, "another account");
        let counts = |store: &Store, accounts: &[&str]| {
            let accounts: Vec<String> = accounts.iter().map(|&name| name.to_owned()).collect();
            let mut counts = store.unread_counts(&accounts).expect("counts");
            counts.sort_unstable();
            counts
        };
        assert_eq!(counts(&store, &["a"]), [(inbox, 3), (archive, 1)]);
        assert_eq!(
            counts(&store, &["a", "b"]),
            [(inbox, 3), (archive, 1), (other, 1)]
        );
        store
            .set_flag("a", first, FlagChange::Seen, true, CONTEXT)
            .expect("explicit read");
        assert_eq!(
            counts(&store, &["a"]),
            [(inbox, 2), (archive, 1)],
            "a read not yet on the server already counts as read"
        );
        store
            .queue_move("a", third, None, "Archive", CONTEXT)
            .expect("move");
        assert_eq!(
            counts(&store, &["a"]),
            [(inbox, 1), (archive, 1)],
            "a message being moved away no longer counts where it was"
        );
        store
            .select_folder("a", inbox, NonZeroU32::new(43).expect("namespace"))
            .expect("new generation");
        assert_eq!(
            counts(&store, &["a"]),
            [(archive, 1)],
            "copies from an old UIDVALIDITY are not current mail"
        );
    }

    #[test]
    fn query_text_cannot_supply_fts_operators_or_sql() {
        let (_root, store, _, _) = fixture();
        let accounts = vec!["a".into(), "b".into()];
        assert_eq!(
            store
                .conversation_page(&accounts, &Scope::Inbox, "needle OR missing", None, 100)
                .expect("literal OR"),
            Vec::new()
        );
        for query in [
            "\"",
            "*",
            "subject:needle",
            "needle) OR *",
            "' ; DELETE FROM messages; --",
        ] {
            let _literal = store
                .conversation_page(&accounts, &Scope::Inbox, query, None, 100)
                .expect("punctuation stays literal");
        }
    }
    #[test]
    fn local_search_is_bounded_and_pages_equal_dates_without_duplicates() {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        store.ensure_account("a").expect("account");
        let folder = store.ensure_folder("a", "INBOX", true).expect("folder").id;
        store
            .select_folder("a", folder, NonZeroU32::new(42).expect("namespace"))
            .expect("select");
        for uid in 1..=12 {
            let id = cache(&mut store, "a", folder, uid, false, "Synthetic page");
            store
                .connection
                .execute(
                    "UPDATE messages SET thread_references='[]',received_at=1 WHERE id=?1",
                    [id],
                )
                .expect("independent messages with equal dates");
        }
        project(&mut store, "a");
        let mut after = None;
        let mut ids = Vec::new();
        loop {
            let page = store
                .conversation_page(&["a".into()], &Scope::Inbox, "synthetic", after, 3)
                .expect("page");
            if page.is_empty() {
                break;
            }
            after = page.last().map(|row| row.latest.cursor());
            ids.extend(page.iter().map(|row| row.latest.id));
        }
        assert_eq!(ids.len(), 12);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 12);
        assert!(search_expression(&"x".repeat(1025)).is_err());
        assert!(search_expression(&"term ".repeat(33)).is_err());
        assert_eq!(search_expression(" \t ").expect("empty"), None);
    }
}
