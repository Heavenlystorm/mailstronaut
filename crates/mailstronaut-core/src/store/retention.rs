//! Explicit age-based eviction of server-backed mail from one local account cache.

use rusqlite::{TransactionBehavior, params};

use super::{Error, OWNED, Store};

/// Result of a committed local cache eviction.
#[derive(Debug, Eq, PartialEq)]
pub struct Evicted {
    /// Number of cached originals removed.
    pub messages: usize,
    /// Filesystem cleanup failed after the database commit and can be retried.
    pub cleanup_pending: bool,
}

impl Store {
    /// Evict old, server-backed originals from this authenticated mailbox context.
    /// Derived indexes receive deletion work in the same transaction. Keep history
    /// cursors because the removed mail is outside the current sync age limit.
    ///
    /// # Errors
    /// Rejects a stale mailbox context or failed database transaction. A failure
    /// collecting unreferenced blobs after commit is reported in the result.
    pub fn evict_cached_before(
        &mut self,
        account: &str,
        context: &str,
        cutoff: i64,
    ) -> Result<Evicted, Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1 AND imap_context=?2)",
            params![account, context],
            |row| row.get(0),
        )?;
        if !current {
            return Err(Error::StaleMailbox);
        }
        // Freeze the candidates before removing any memberships. An original
        // with no current server copy may be the only copy the user has.
        tx.execute_batch("CREATE TEMP TABLE cache_eviction(id INTEGER PRIMARY KEY);")?;
        tx.execute(
            &format!(
                "INSERT INTO cache_eviction
                 SELECT m.id FROM messages m
                 WHERE m.account_id=?1 AND m.received_at>0 AND m.received_at<?2
                 AND m.id NOT IN ({OWNED})
                 AND EXISTS(
                   SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id
                   WHERE s.message_id=m.id AND f.uidvalidity=s.uidvalidity
                 )
                 AND NOT EXISTS(SELECT 1 FROM analysis WHERE message_id=m.id AND dismissed=1)
                 AND NOT EXISTS(
                   SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id
                   WHERE s.message_id=m.id AND
                     (f.role='drafts' OR s.starred=1 OR (m.read_cleared=1 AND s.seen=0))
                 )"
            ),
            params![account, cutoff],
        )?;
        tx.execute_batch(
            "DELETE FROM memberships WHERE message_id IN cache_eviction;
             DELETE FROM provider_ids WHERE message_id IN cache_eviction;
             DELETE FROM message_search WHERE rowid IN cache_eviction;",
        )?;
        let messages = tx.execute("DELETE FROM messages WHERE id IN cache_eviction", [])?;
        tx.execute_batch("DROP TABLE cache_eviction;")?;
        tx.commit()?;
        let cleanup_pending = self.collect_orphan_blobs().is_err();
        Ok(Evicted {
            messages,
            cleanup_pending,
        })
    }
}
