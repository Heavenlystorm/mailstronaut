//! SQLite schema and content-addressed message blobs.

use core::time::Duration;
use std::{
    fs, io,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, TransactionBehavior, params};
use rusqlite_migration::{M, Migrations};
use sha2::{Digest as _, Sha256};

pub mod mailbox;

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
    /// Invalid identity, page size or filesystem encoding.
    InvalidInput,
    /// A server UID cannot be rebound to a different canonical message.
    IdentityConflict,
    /// The requested message is not cached in this account.
    MissingMessage,
    /// SELECT's generation has changed; old work must be discarded.
    StaleMailbox,
    /// Database belongs to a newer application schema.
    UnsupportedSchema,
    /// Original MIME could not be projected within its resource budget.
    Mime(crate::mime::Error),
    /// Cached metadata serialization failed.
    Metadata(serde_json::Error),
    /// A hash-named destination is not the expected regular file.
    CorruptBlob,
    /// Filesystem access failed.
    Io(io::Error),
    /// Schema migration failed.
    Migration(rusqlite_migration::Error),
    /// SQLite operation failed.
    Sql(rusqlite::Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "invalid mailbox identity or request limit",
            Self::IdentityConflict => "server message identity conflicts with cached mail",
            Self::MissingMessage => "message is not cached in this account",
            Self::StaleMailbox => "mailbox generation changed; resynchronization required",
            Self::UnsupportedSchema => "mail database was created by a newer application",
            Self::Mime(_) => "message exceeds supported MIME parsing limits",
            Self::Metadata(_) => "cached message metadata is invalid",
            Self::CorruptBlob => "cached original failed its integrity check",
            Self::Io(_) => "mail storage filesystem operation failed",
            Self::Migration(_) => "mail database migration failed; original data retained",
            Self::Sql(_) => "mail database operation failed",
        })
    }
}
impl core::error::Error for Error {}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Metadata(value)
    }
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
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;")?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 2 {
            return Err(Error::UnsupportedSchema);
        }
        if version > 0 && version < 2 {
            backup_schema(&connection, root, version)?;
        }
        // SQLite's documented table-rebuild procedure: disable enforcement
        // outside the transaction, then validate every FK before committing.
        connection.pragma_update(None, "foreign_keys", "OFF")?;
        let migrated = Migrations::new(vec![
            M::up(SCHEMA).foreign_key_check(),
            M::up(mailbox::SCHEMA).foreign_key_check(),
        ])
        .to_latest(&mut connection);
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrated?;
        return Ok(Self { blobs, connection });
    }

    /// Ensure an account with this stable configuration ID exists.
    ///
    /// # Errors
    /// Returns an error if SQLite cannot save the account.
    #[expect(
        clippy::needless_return,
        reason = "Explicit returns and concise error propagation are both required here."
    )]
    #[inline]
    pub fn ensure_account(&mut self, account_id: &str) -> Result<(), Error> {
        self.connection.execute(
            "INSERT OR IGNORE INTO accounts (id) VALUES (?1)",
            params![account_id],
        )?;
        return Ok(());
    }

    /// Persist an original message and commit its reference, returning its local ID.
    ///
    /// The account must already exist. The immediate transaction coordinates with
    /// collection by other store handles; an unsuccessful insert can leave an orphan.
    /// Each call creates a distinct message, even for identical content. Canonical
    /// account-local deduplication requires message identity metadata and is future work.
    /// This blocking operation must run on the future dedicated store worker.
    ///
    /// # Errors
    /// Returns an error on an invalid existing blob, filesystem, or SQLite failure.
    #[expect(
        clippy::needless_return,
        reason = "Explicit returns and concise error propagation are both required here."
    )]
    #[inline]
    pub fn ingest_message(&mut self, account_id: &str, bytes: &[u8]) -> Result<i64, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = write_blob(&self.blobs, bytes)?;
        transaction.execute(
            "INSERT INTO messages (account_id, blob_hash) VALUES (?1, ?2)",
            params![account_id, hash],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.commit()?;
        return Ok(id);
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
    pub fn collect_orphan_blobs(&mut self) -> Result<(), Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for result in fs::read_dir(&self.blobs)? {
            let entry = result?;
            let filename = entry.file_name();
            let Some(name) = filename.to_str() else {
                continue;
            };
            if name.len() != 64
                || !name
                    .bytes()
                    .all(|byte| return byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                continue;
            }
            let count: i64 = transaction.query_row(
                "SELECT count(*) FROM messages WHERE blob_hash = ?1",
                params![name],
                |row| return row.get(0),
            )?;
            if count == 0 && entry.file_type()?.is_file() {
                fs::remove_file(entry.path())?;
            }
        }
        transaction.commit()?;
        return Ok(());
    }
}

/// SQLite creates a consistent backup that includes committed WAL state. A
/// filesystem copy of the main database file alone would not provide that.
fn backup_schema(connection: &Connection, root: &Path, version: u32) -> Result<(), Error> {
    let directory = root.join("backups");
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let backup = tempfile::Builder::new()
        .prefix(&format!("schema-{version}-"))
        .suffix(".sqlite3")
        .tempfile_in(&directory)?;
    let path = backup.path().to_str().ok_or(Error::InvalidInput)?;
    connection.execute("VACUUM INTO ?1", params![path])?;
    backup.as_file().sync_all()?;
    let _retained = backup.keep().map_err(|error| error.error)?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

/// Persist and validate a content-addressed original while holding the SQLite writer lock.
#[expect(
    clippy::needless_return,
    reason = "Chunk lengths are bounded by the buffer; this helper isolates blob persistence and validation."
)]
fn write_blob(blobs: &Path, bytes: &[u8]) -> Result<String, Error> {
    use std::io::{Read as _, Write as _};
    let hash = format!("{:x}", Sha256::digest(bytes));
    let destination = blobs.join(&hash);
    match fs::symlink_metadata(&destination) {
        Ok(metadata) => {
            if !metadata.file_type().is_file()
                || usize::try_from(metadata.len()).ok() != Some(bytes.len())
            {
                return Err(Error::CorruptBlob);
            }
            let mut file = fs::File::open(&destination)?;
            let mut buffer: [u8; 8192] = [0; 8192];
            for chunk in bytes.chunks(buffer.len()) {
                file.read_exact(&mut buffer[..chunk.len()])?;
                if &buffer[..chunk.len()] != chunk {
                    return Err(Error::CorruptBlob);
                }
            }
            return Ok(hash);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut temporary = tempfile::NamedTempFile::new_in(blobs)?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(&destination)
        .map_err(|error| return error.error)?;
    fs::File::open(blobs)?.sync_all()?;
    return Ok(hash);
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Test setup uses concise assertions and propagates fixture errors."
)]
mod tests {
    use super::Store;
    use core::time::Duration;
    use rusqlite::params;
    use std::fs;

    #[test]
    fn index_clear_preserves_durable_state_and_referenced_blob() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let hash = super::write_blob(&store.blobs, b"synthetic message").unwrap();
        let orphan = super::write_blob(&store.blobs, b"unreferenced synthetic blob").unwrap();
        store.connection.execute_batch("INSERT INTO accounts VALUES ('a'); INSERT INTO folders VALUES (7, 'a', 'INBOX', 42, 1); INSERT INTO drafts VALUES ('d', x'01'); INSERT INTO outbox VALUES ('o', x'02', 'pending');").unwrap();
        store.connection.execute("INSERT INTO messages (id, account_id, message_id, subject, preview, blob_hash) VALUES (11, 'a', '<synthetic>', 'subject', 'preview', ?1)", params![hash]).unwrap();
        store.connection.execute_batch("INSERT INTO memberships (account_id, folder_id, uidvalidity, uid, message_id, seen) VALUES ('a', 7, 42, 3, 11, 1); INSERT INTO provider_ids VALUES ('a', 'provider', 11); INSERT INTO reminders VALUES ('r', 11, 10, 'pending'); INSERT INTO analysis VALUES (11, x'726561642d636c65617265643b6469736d69737365643b6f76657272696465'); INSERT INTO message_search (rowid, subject, addresses, body) VALUES (11, 'subject', 'synthetic', 'body');").unwrap();
        store.clear_rebuildable_indexes().unwrap();
        drop(store);
        let mut reopened = Store::open(dir.path()).unwrap();
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
    fn ingestion_reopens_and_shares_blobs_without_collapsing_messages() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        store.ensure_account("a").unwrap();
        store.ensure_account("b").unwrap();
        let first = store.ingest_message("a", b"synthetic mail").unwrap();
        let second = store.ingest_message("a", b"synthetic mail").unwrap();
        let third = store.ingest_message("b", b"synthetic mail").unwrap();
        assert_ne!(first, second);
        assert_ne!(second, third);
        drop(store);
        let mut reopened = Store::open(dir.path()).unwrap();
        let (rows, hashes): (i64, i64) = reopened
            .connection
            .query_row(
                "SELECT count(*), count(DISTINCT blob_hash) FROM messages",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((rows, hashes), (3, 1));
        reopened.collect_orphan_blobs().unwrap();
        assert_eq!(fs::read_dir(&reopened.blobs).unwrap().count(), 1);
    }

    #[test]
    fn writer_lock_prevents_collection_before_reference_commit() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = Store::open(dir.path()).unwrap();
        writer.ensure_account("a").unwrap();
        let mut collector = Store::open(dir.path()).unwrap();
        collector.connection.busy_timeout(Duration::ZERO).unwrap();
        let tx = writer
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let hash = super::write_blob(&writer.blobs, b"in flight").unwrap();
        collector.collect_orphan_blobs().unwrap_err();
        assert!(writer.blobs.join(&hash).exists());
        tx.execute(
            "INSERT INTO messages (account_id, blob_hash) VALUES ('a', ?1)",
            params![hash],
        )
        .unwrap();
        tx.commit().unwrap();
        collector.collect_orphan_blobs().unwrap();
        assert!(writer.blobs.join(&hash).exists());
        let collector_tx = collector
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        writer.connection.busy_timeout(Duration::ZERO).unwrap();
        writer.ingest_message("a", b"blocked").unwrap_err();
        collector_tx.rollback().unwrap();
    }

    #[test]
    fn failed_insert_leaves_collectible_orphan() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        store.ensure_account("a").unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT, 'test'); END;").unwrap();
        store.ingest_message("a", b"orphan").unwrap_err();
        assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 1);
        let count: i64 = store
            .connection
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        store.collect_orphan_blobs().unwrap();
        assert_eq!(fs::read_dir(&store.blobs).unwrap().count(), 0);
    }

    #[test]
    fn corruption_and_symlinks_are_rejected_and_unknown_files_retained() {
        use sha2::Digest as _;
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        store.ensure_account("a").unwrap();
        let hash = format!("{:x}", sha2::Sha256::digest(b"synthetic"));
        let path = store.blobs.join(&hash);
        fs::write(&path, b"corrupted").unwrap();
        assert!(matches!(
            store.ingest_message("a", b"synthetic"),
            Err(super::Error::CorruptBlob)
        ));
        fs::remove_file(&path).unwrap();
        let unrelated = store.blobs.join("unrelated");
        fs::write(&unrelated, b"synthetic").unwrap();
        symlink(&unrelated, &path).unwrap();
        assert!(matches!(
            store.ingest_message("a", b"synthetic"),
            Err(super::Error::CorruptBlob)
        ));
        store.collect_orphan_blobs().unwrap();
        assert!(path.is_symlink());
        assert!(unrelated.exists());
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
        assert_eq!(version, 2);
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        let reopened_version: i64 = reopened
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(reopened_version, 2);
    }
}

#[cfg(test)]
mod migration_tests;
