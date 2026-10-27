//! SQLite schema and content-addressed message blobs.

use core::time::Duration;
use std::{
    fs, io,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, params};
use rusqlite_migration::{M, Migrations};
use sha2::{Digest as _, Sha256};

/// Initial numbered migration.
const SCHEMA: &str = "
CREATE TABLE accounts (id TEXT PRIMARY KEY NOT NULL);
CREATE TABLE folders (id INTEGER PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id), name TEXT NOT NULL, uidvalidity INTEGER, is_inbox INTEGER NOT NULL DEFAULT 0, UNIQUE(account_id, name), UNIQUE(account_id, id));
CREATE TABLE messages (id INTEGER PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id), message_id TEXT, subject TEXT NOT NULL DEFAULT '', preview TEXT NOT NULL DEFAULT '', blob_hash TEXT NOT NULL, UNIQUE(account_id, id));
CREATE TABLE memberships (account_id TEXT NOT NULL, folder_id INTEGER NOT NULL, uidvalidity INTEGER NOT NULL, uid INTEGER NOT NULL, message_id INTEGER NOT NULL, seen INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(account_id, folder_id, uidvalidity, uid), FOREIGN KEY(account_id, folder_id) REFERENCES folders(account_id, id), FOREIGN KEY(account_id, message_id) REFERENCES messages(account_id, id));
CREATE TABLE provider_ids (account_id TEXT NOT NULL, provider_id TEXT NOT NULL, message_id INTEGER NOT NULL, PRIMARY KEY(account_id, provider_id), FOREIGN KEY(account_id, message_id) REFERENCES messages(account_id, id));
CREATE TABLE drafts (id TEXT PRIMARY KEY NOT NULL, content BLOB NOT NULL);
CREATE TABLE outbox (id TEXT PRIMARY KEY NOT NULL, content BLOB NOT NULL, phase TEXT NOT NULL);
CREATE TABLE reminders (id TEXT PRIMARY KEY NOT NULL, message_id INTEGER, due_at INTEGER NOT NULL, state TEXT NOT NULL);
CREATE TABLE pending_operations (id TEXT PRIMARY KEY NOT NULL, account_id TEXT NOT NULL, folder_id INTEGER, uidvalidity INTEGER, uid INTEGER, state TEXT NOT NULL);
CREATE TABLE analysis (message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE, data BLOB NOT NULL);
CREATE VIRTUAL TABLE message_search USING fts5(subject, addresses, body);
";

/// Database and blob root. The caller must serialize access on a blocking owner.
pub struct Store {
    /// Blob directory.
    blobs: PathBuf,
    /// Sole database connection.
    connection: Connection,
}

/// Errors produced by local persistence.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Filesystem access failed.
    Io(io::Error),
    /// Schema migration failed.
    Migration(rusqlite_migration::Error),
    /// SQLite operation failed.
    Sql(rusqlite::Error),
}

#[expect(
    clippy::needless_return,
    reason = "Restriction group also requires explicit return."
)]
impl From<io::Error> for Error {
    #[inline]
    fn from(value: io::Error) -> Self {
        return Self::Io(value);
    }
}
#[expect(
    clippy::needless_return,
    reason = "Restriction group also requires explicit return."
)]
impl From<rusqlite::Error> for Error {
    #[inline]
    fn from(value: rusqlite::Error) -> Self {
        return Self::Sql(value);
    }
}
#[expect(
    clippy::needless_return,
    reason = "Restriction group also requires explicit return."
)]
impl From<rusqlite_migration::Error> for Error {
    #[inline]
    fn from(value: rusqlite_migration::Error) -> Self {
        return Self::Migration(value);
    }
}

#[expect(
    clippy::arbitrary_source_item_ordering,
    reason = "The constructor precedes the operations it enables."
)]
impl Store {
    /// Open or migrate a database under a private data directory.
    ///
    /// # Errors
    /// Returns an error on filesystem, SQLite, or migration failure.
    #[inline]
    #[expect(
        clippy::needless_return,
        reason = "Restriction group also requires explicit return."
    )]
    #[expect(
        clippy::question_mark_used,
        reason = "Propagate concrete I/O and database failures without repetitive matches."
    )]
    pub fn open(root: &Path) -> Result<Self, Error> {
        fs::create_dir_all(root)?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        let blobs = root.join("blobs");
        fs::create_dir_all(&blobs)?;
        fs::set_permissions(&blobs, fs::Permissions::from_mode(0o700))?;
        let database = root.join("mail.sqlite3");
        let _file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&database)?;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600))?;
        let mut connection = Connection::open(database)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Migrations::new(vec![M::up(SCHEMA)]).to_latest(&mut connection)?;
        return Ok(Self { blobs, connection });
    }

    /// Write a blob atomically before committing a database reference.
    ///
    /// # Errors
    /// Returns an error if the filesystem cannot persist the blob.
    #[inline]
    #[expect(
        clippy::needless_return,
        reason = "Restriction group also requires explicit return."
    )]
    #[expect(
        clippy::question_mark_used,
        reason = "Propagate concrete I/O and database failures without repetitive matches."
    )]
    pub fn write_blob(&self, bytes: &[u8]) -> Result<String, Error> {
        use std::io::Write as _;
        let hash = format!("{:x}", Sha256::digest(bytes));
        let destination = self.blobs.join(&hash);
        if destination.exists() {
            return Ok(hash);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.blobs)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&destination)
            .map_err(|error| return error.error)?;
        fs::File::open(&self.blobs)?.sync_all()?;
        return Ok(hash);
    }

    /// Clear the rebuildable search index without evicting message bodies or durable state.
    ///
    /// # Errors
    /// Returns an error if the transaction fails.
    #[inline]
    #[expect(
        clippy::needless_return,
        reason = "Restriction group also requires explicit return."
    )]
    #[expect(
        clippy::question_mark_used,
        reason = "Propagate concrete I/O and database failures without repetitive matches."
    )]
    pub fn clear_rebuildable_indexes(&mut self) -> Result<(), Error> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch("DELETE FROM message_search;")?;
        transaction.commit()?;
        return Ok(());
    }

    /// Remove blobs not referenced by any cached message.
    ///
    /// # Errors
    /// Returns an error if SQLite or filesystem access fails.
    #[inline]
    #[expect(
        clippy::needless_return,
        reason = "Restriction group also requires explicit return."
    )]
    #[expect(
        clippy::question_mark_used,
        reason = "Propagate concrete I/O and database failures without repetitive matches."
    )]
    #[expect(
        clippy::filetype_is_file,
        reason = "Only regular blob files may be deleted; leave other filesystem entries untouched."
    )]
    pub fn collect_orphan_blobs(&self) -> Result<(), Error> {
        for result in fs::read_dir(&self.blobs)? {
            let entry = result?;
            let filename = entry.file_name();
            let Some(name) = filename.to_str() else {
                continue;
            };
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM messages WHERE blob_hash = ?1",
                params![name],
                |row| return row.get(0),
            )?;
            if count == 0 && entry.file_type()?.is_file() {
                fs::remove_file(entry.path())?;
            }
        }
        return Ok(());
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::implicit_return,
    reason = "Test setup uses concise assertions and propagates fixture errors."
)]
mod tests {
    use super::Store;
    use rusqlite::params;

    #[test]
    #[expect(
        clippy::question_mark_used,
        reason = "Propagate SQLite row decoding errors in this regression test."
    )]
    fn index_clear_preserves_durable_state_and_referenced_blob() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let hash = store.write_blob(b"synthetic message").unwrap();
        assert_eq!(hash, store.write_blob(b"synthetic message").unwrap());
        let orphan = store.write_blob(b"unreferenced synthetic blob").unwrap();
        store.connection.execute_batch("INSERT INTO accounts VALUES ('a'); INSERT INTO folders VALUES (7, 'a', 'INBOX', 42, 1); INSERT INTO drafts VALUES ('d', x'01'); INSERT INTO outbox VALUES ('o', x'02', 'pending');").unwrap();
        store.connection.execute("INSERT INTO messages (id, account_id, message_id, subject, preview, blob_hash) VALUES (11, 'a', '<synthetic>', 'subject', 'preview', ?1)", params![hash]).unwrap();
        store.connection.execute_batch("INSERT INTO memberships VALUES ('a', 7, 42, 3, 11, 1); INSERT INTO provider_ids VALUES ('a', 'provider', 11); INSERT INTO reminders VALUES ('r', 11, 10, 'pending'); INSERT INTO analysis VALUES (11, x'726561642d636c65617265643b6469736d69737365643b6f76657272696465'); INSERT INTO message_search (rowid, subject, addresses, body) VALUES (11, 'subject', 'synthetic', 'body');").unwrap();
        store.clear_rebuildable_indexes().unwrap();
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        let search_count: i64 = reopened
            .connection
            .query_row("SELECT count(*) FROM message_search", [], |row| row.get(0))
            .unwrap();
        assert_eq!(search_count, 0);
        let message: (String, String, String, String, String) = reopened.connection.query_row("SELECT account_id, message_id, subject, preview, blob_hash FROM messages WHERE id = 11", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).unwrap();
        assert_eq!(
            message,
            (
                "a".into(),
                "<synthetic>".into(),
                "subject".into(),
                "preview".into(),
                hash.clone()
            )
        );
        let membership: (String, i64, i64, i64, i64, i64) = reopened
            .connection
            .query_row(
                "SELECT account_id, folder_id, uidvalidity, uid, message_id, seen FROM memberships",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(membership, ("a".into(), 7, 42, 3, 11, 1));
        let provider: (String, String, i64) = reopened
            .connection
            .query_row(
                "SELECT account_id, provider_id, message_id FROM provider_ids",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(provider, ("a".into(), "provider".into(), 11));
        let reminder: (String, i64, i64, String) = reopened
            .connection
            .query_row(
                "SELECT id, message_id, due_at, state FROM reminders",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(reminder, ("r".into(), 11, 10, "pending".into()));
        let analysis: (i64, Vec<u8>) = reopened
            .connection
            .query_row("SELECT message_id, data FROM analysis", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(analysis, (11, b"read-cleared;dismissed;override".to_vec()));
        let draft: (String, Vec<u8>) = reopened
            .connection
            .query_row("SELECT id, content FROM drafts", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(draft, ("d".into(), vec![1]));
        let outbox: (String, Vec<u8>, String) = reopened
            .connection
            .query_row("SELECT id, content, phase FROM outbox", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(outbox, ("o".into(), vec![2], "pending".into()));
        reopened.collect_orphan_blobs().unwrap();
        assert!(reopened.blobs.join(&hash).exists());
        assert!(!reopened.blobs.join(&orphan).exists());
    }

    #[test]
    fn migrations_and_foreign_keys_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO folders (account_id, name) VALUES ('missing', 'INBOX')",
                [],
            )
            .unwrap_err();
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        let reopened_version: i64 = reopened
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(reopened_version, 1);
    }
}
