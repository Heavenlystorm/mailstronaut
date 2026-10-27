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

    /// Remove rebuildable mail projections while retaining user-owned state.
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
    pub fn clear_cache(&mut self) -> Result<(), Error> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch("DELETE FROM message_search; DELETE FROM memberships; DELETE FROM provider_ids; DELETE FROM analysis; DELETE FROM messages;")?;
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
    fn cache_clear_preserves_owned_state_and_collects_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let hash = store.write_blob(b"synthetic message").unwrap();
        assert_eq!(hash, store.write_blob(b"synthetic message").unwrap());
        store.connection.execute_batch("INSERT INTO accounts VALUES ('a'); INSERT INTO drafts VALUES ('d', x'01'); INSERT INTO outbox VALUES ('o', x'02', 'pending'); INSERT INTO reminders VALUES ('r', NULL, 10, 'pending');").unwrap();
        store
            .connection
            .execute(
                "INSERT INTO messages (account_id, blob_hash) VALUES ('a', ?1)",
                params![hash],
            )
            .unwrap();
        store.collect_orphan_blobs().unwrap();
        assert!(store.blobs.join(&hash).exists());
        store.clear_cache().unwrap();
        store.collect_orphan_blobs().unwrap();
        assert!(!store.blobs.join(&hash).exists());
        for table in ["drafts", "outbox", "reminders"] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1);
        }
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
