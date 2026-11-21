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

/// How much of one folder's history is cached, as its pass last recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Coverage {
    /// No pass has started for this folder.
    NotStarted,
    /// A pass is still walking the folder.
    Partial {
        /// Messages examined so far.
        reviewed: u32,
        /// Messages the folder held when the pass began; None when the server did not say.
        total: Option<u32>,
    },
    /// The pass reached the beginning of the folder, within the age cap it was given, if any.
    Complete {
        /// The declared age limit in days, or None for the complete history.
        cap_days: Option<u32>,
    },
}

/// The coverage of one discovered folder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderCoverage {
    /// Stable local folder identity.
    pub id: i64,
    /// The configured account identifier.
    pub account: String,
    /// The folder's server name, as stored.
    pub folder: String,
    /// Whether this folder participates in the unified inbox.
    pub inbox: bool,
    /// What the folder's pass has cached so far.
    pub coverage: Coverage,
}

impl Store {
    /// The history coverage of every folder of these accounts, by account and folder name.
    ///
    /// # Errors
    /// Rejects too many accounts and reports storage failures.
    pub fn coverage(&self, accounts: &[String]) -> Result<Vec<FolderCoverage>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let selected = serde_json::to_string(accounts)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT f.account_id, f.name, b.reviewed, b.total, b.finished_at, b.cap_days,
                    b.account_id IS NOT NULL, f.id, f.is_inbox
             FROM folders f LEFT JOIN backfill b ON b.account_id=f.account_id AND b.folder_id=f.id
             WHERE f.account_id IN (SELECT value FROM json_each(?1))
             ORDER BY f.account_id, f.name",
        )?;
        let rows = statement
            .query_map(params![selected], |row| {
                let started: bool = row.get(6)?;
                let coverage = if !started {
                    Coverage::NotStarted
                } else if row.get::<_, Option<i64>>(4)?.is_some() {
                    Coverage::Complete {
                        cap_days: row.get(5)?,
                    }
                } else {
                    Coverage::Partial {
                        reviewed: row.get(2)?,
                        total: row.get(3)?,
                    }
                };
                Ok(FolderCoverage {
                    id: row.get(7)?,
                    account: row.get(0)?,
                    folder: row.get(1)?,
                    inbox: row.get(8)?,
                    coverage,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

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
    use super::{Coverage, FolderCoverage, Pass, Store};

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
    fn coverage_reports_each_folder_of_the_asked_accounts_without_inventing_progress() {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        store.ensure_account("a").expect("account a");
        store.ensure_account("b").expect("account b");
        let inbox = store.ensure_folder("a", "INBOX", true).expect("inbox").id;
        let archive = store
            .ensure_folder("a", "Archive", false)
            .expect("archive")
            .id;
        let trash = store.ensure_folder("a", "Trash", false).expect("trash").id;
        let other = store.ensure_folder("b", "INBOX", true).expect("other").id;
        store
            .save_backfill(
                "a",
                inbox,
                &Pass {
                    reviewed: 128,
                    total: Some(300),
                    ..pass(Some(173))
                },
            )
            .expect("partial pass");
        store
            .save_backfill(
                "a",
                archive,
                &Pass {
                    cap_days: Some(30),
                    reviewed: 12,
                    total: Some(12),
                    finished_at: Some(1_000),
                    ..pass(None)
                },
            )
            .expect("finished pass");
        store
            .save_backfill("b", other, &pass(None))
            .expect("another account's pass");
        assert_eq!(
            store.coverage(&["a".into()]).expect("coverage"),
            [
                FolderCoverage {
                    id: archive,
                    account: "a".into(),
                    folder: "Archive".into(),
                    inbox: false,
                    coverage: Coverage::Complete { cap_days: Some(30) },
                },
                FolderCoverage {
                    id: inbox,
                    account: "a".into(),
                    folder: "INBOX".into(),
                    inbox: true,
                    coverage: Coverage::Partial {
                        reviewed: 128,
                        total: Some(300),
                    },
                },
                FolderCoverage {
                    id: trash,
                    account: "a".into(),
                    folder: "Trash".into(),
                    inbox: false,
                    coverage: Coverage::NotStarted,
                },
            ]
        );
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
