//! Durable history passes, with one cursor per account folder.
//!
//! A cursor is fenced by the UID namespace, the mailbox context and the age cap it started
//! under. Nothing here deletes mail: a pass only decides what is downloaded.

use rusqlite::{OptionalExtension as _, params};

use super::{Error, Store};

/// Cursor of one folder's history pass, which walks from newest to oldest UID.
pub(super) const SCHEMA: &str = "
CREATE TABLE backfill (
 account_id TEXT NOT NULL,
 folder_id INTEGER NOT NULL,
 uidvalidity INTEGER NOT NULL,
 context BLOB NOT NULL CHECK(length(context) = 32),
 cap_days INTEGER,
 next_before INTEGER,
 total INTEGER,
 reviewed INTEGER NOT NULL DEFAULT 0,
 finished_at INTEGER,
 PRIMARY KEY(account_id, folder_id),
 FOREIGN KEY(account_id, folder_id) REFERENCES folders(account_id, id) ON DELETE CASCADE
);";

/// Progress of one folder's pass, as last committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pass {
    /// UIDVALIDITY of the namespace this cursor belongs to.
    pub uid_validity: u32,
    /// SHA-256 of the mailbox context the pass started under.
    pub context: [u8; 32],
    /// Oldest fetched age in days, or None for the complete history.
    pub cap_days: Option<u32>,
    /// Exclusive upper UID bound of the next window; None once UID 1 has been reviewed.
    pub next_before: Option<u32>,
    /// Messages present when the pass started, when SELECT reported them.
    pub total: Option<u32>,
    /// Messages reviewed by this pass so far, whether or not they were downloaded.
    pub reviewed: u32,
    /// Unix seconds when the pass reached the beginning of the folder.
    pub finished_at: Option<i64>,
}

impl Store {
    /// The stored pass of one folder, if one was started.
    ///
    /// # Errors
    /// Returns an error if SQLite cannot read the cursor.
    pub fn backfill_pass(&self, account: &str, folder: i64) -> Result<Option<Pass>, Error> {
        let pass = self
            .connection
            .query_row(
                "SELECT uidvalidity,context,cap_days,next_before,total,reviewed,finished_at
                 FROM backfill WHERE account_id=?1 AND folder_id=?2",
                params![account, folder],
                |row| {
                    let context: Vec<u8> = row.get(1)?;
                    Ok(Pass {
                        uid_validity: row.get(0)?,
                        context: <[u8; 32]>::try_from(context.as_slice())
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        cap_days: row.get(2)?,
                        next_before: row.get(3)?,
                        total: row.get(4)?,
                        reviewed: row.get(5)?,
                        finished_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(pass)
    }

    /// Record how far a pass has committed. Written only after the window it describes.
    ///
    /// # Errors
    /// Returns an error if SQLite cannot save the cursor, including for an unknown folder.
    pub fn save_backfill(&self, account: &str, folder: i64, pass: &Pass) -> Result<(), Error> {
        self.connection.execute(
            "INSERT INTO backfill (account_id,folder_id,uidvalidity,context,cap_days,next_before,total,reviewed,finished_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(account_id,folder_id) DO UPDATE SET
              uidvalidity=excluded.uidvalidity, context=excluded.context, cap_days=excluded.cap_days,
              next_before=excluded.next_before, total=excluded.total, reviewed=excluded.reviewed,
              finished_at=excluded.finished_at",
            params![
                account,
                folder,
                pass.uid_validity,
                pass.context.as_slice(),
                pass.cap_days,
                pass.next_before,
                pass.total,
                pass.reviewed,
                pass.finished_at,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Pass, Store};

    fn pass(next_before: Option<u32>) -> Pass {
        Pass {
            uid_validity: 42,
            context: [7; 32],
            cap_days: None,
            next_before,
            total: Some(300),
            reviewed: 0,
            finished_at: None,
        }
    }

    #[test]
    fn a_pass_survives_reopening_and_each_save_replaces_the_last() {
        let root = tempfile::tempdir().expect("profile");
        let folder = {
            let mut store = Store::open(root.path()).expect("store");
            store.ensure_account("a").expect("account");
            let folder = store.ensure_folder("a", "INBOX", true).expect("inbox").id;
            assert_eq!(store.backfill_pass("a", folder).expect("empty"), None);
            store
                .save_backfill("a", folder, &pass(Some(257)))
                .expect("first save");
            store
                .save_backfill(
                    "a",
                    folder,
                    &Pass {
                        reviewed: 128,
                        ..pass(Some(129))
                    },
                )
                .expect("second save");
            folder
        };
        let store = Store::open(root.path()).expect("reopened");
        let stored = store
            .backfill_pass("a", folder)
            .expect("read")
            .expect("saved pass");
        assert_eq!(stored.next_before, Some(129));
        assert_eq!(stored.reviewed, 128);
        assert_eq!(stored.context, [7; 32]);
        assert_eq!(stored.total, Some(300));
    }

    #[test]
    fn a_pass_belongs_to_one_account_and_folder_and_needs_a_real_folder() {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        store.ensure_account("a").expect("account a");
        store.ensure_account("b").expect("account b");
        let folder = store.ensure_folder("a", "INBOX", true).expect("inbox").id;
        store
            .save_backfill("a", folder, &pass(None))
            .expect("saved");
        assert_eq!(
            store.backfill_pass("b", folder).expect("other account"),
            None
        );
        assert!(
            store
                .save_backfill("a", folder + 1000, &pass(None))
                .is_err()
        );
    }
}
