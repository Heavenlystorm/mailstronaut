//! Upgrade/backup/rollback and immutable identity behavior.

use std::{fs, os::unix::fs::PermissionsExt as _, path::Path};

use rusqlite::{Connection, params};

use super::{Error, SCHEMA, Store};

fn legacy(root: &Path) -> Connection {
    let connection = Connection::open(root.join("mail.sqlite3")).expect("legacy database");
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA foreign_keys=OFF;",
        )
        .expect("WAL");
    connection.execute_batch(SCHEMA).expect("schema one");
    connection.execute_batch("PRAGMA user_version=1; INSERT INTO accounts VALUES('a'); INSERT INTO folders VALUES(7,'a','INBOX',42,1); INSERT INTO messages(id,account_id,message_id,subject,preview,blob_hash) VALUES(11,'a','old@example.test','old subject','old preview','old-hash'); INSERT INTO memberships VALUES('a',7,42,3,11,1); INSERT INTO provider_ids VALUES('a','provider',11); INSERT INTO analysis VALUES(11,x'01'); INSERT INTO reminders VALUES('reminder',11,123,'pending'); INSERT INTO drafts VALUES('draft',x'02'); INSERT INTO outbox VALUES('outbox',x'03','pending');").expect("legacy data");
    connection
}

fn scalar(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .expect("scalar query")
}

#[test]
fn schema_two_upgrade_retains_legacy_drafts_and_has_a_schema_two_backup() {
    let root = tempfile::tempdir().expect("fixture");
    let connection =
        Connection::open(root.path().join("mail.sqlite3")).expect("schema two database");
    rusqlite_migration::Migrations::new(vec![
        rusqlite_migration::M::up(SCHEMA),
        rusqlite_migration::M::up(super::mailbox::SCHEMA),
    ])
    .to_latest(&mut { connection })
    .expect("old migrations");
    let old = Connection::open(root.path().join("mail.sqlite3")).expect("old data");
    old.execute_batch("INSERT INTO accounts VALUES('a'); INSERT INTO drafts(id,content) VALUES('legacy',x'0102'); INSERT INTO outbox(id,content,phase) VALUES('legacy-outgoing',x'0304','pending');").expect("legacy payloads");
    let store = Store::open(root.path()).expect("upgrade");
    assert_eq!(
        scalar(&store.connection, "PRAGMA user_version"),
        i64::from(super::SCHEMA_VERSION)
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT content FROM drafts WHERE id='legacy'", [], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .expect("original draft"),
        [1, 2]
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT content FROM outbox WHERE id='legacy-outgoing'",
                [],
                |row| row.get::<_, Vec<u8>>(0)
            )
            .expect("original outgoing"),
        [3, 4]
    );
    let backup = fs::read_dir(root.path().join("backups"))
        .expect("backup directory")
        .next()
        .expect("backup")
        .expect("entry")
        .path();
    let backup = Connection::open(backup).expect("backup opens");
    assert_eq!(scalar(&backup, "PRAGMA user_version"), 2);
    assert_eq!(scalar(&backup, "SELECT count(*) FROM drafts"), 1);
}

#[test]
fn upgrades_with_consistent_private_backup_including_committed_wal() {
    let root = tempfile::tempdir().expect("fixture");
    let old = legacy(root.path());
    assert!(
        fs::metadata(root.path().join("mail.sqlite3-wal"))
            .expect("uncheckpointed WAL")
            .len()
            > 0
    );
    let store = Store::open(root.path()).expect("upgrade");
    assert_eq!(
        scalar(&store.connection, "PRAGMA user_version"),
        i64::from(super::SCHEMA_VERSION)
    );
    for table in [
        "messages",
        "memberships",
        "provider_ids",
        "analysis",
        "reminders",
        "drafts",
        "outbox",
    ] {
        assert_eq!(
            scalar(&store.connection, &format!("SELECT count(*) FROM {table}")),
            1
        );
    }
    assert_eq!(scalar(&store.connection, "PRAGMA foreign_keys"), 1);
    let backup = fs::read_dir(root.path().join("backups"))
        .expect("backup directory")
        .next()
        .expect("retained backup")
        .expect("entry")
        .path();
    assert_eq!(
        fs::metadata(&backup)
            .expect("backup metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let snapshot = Connection::open(&backup).expect("backup opens");
    assert_eq!(scalar(&snapshot, "PRAGMA user_version"), 1);
    assert_eq!(scalar(&snapshot, "SELECT id FROM messages"), 11);
    assert_eq!(scalar(&snapshot, "SELECT count(*) FROM drafts"), 1);
    drop(old);
    drop(store);
    let _reopened = Store::open(root.path()).expect("reopen upgraded database");
    assert_eq!(
        fs::read_dir(root.path().join("backups"))
            .expect("backups")
            .count(),
        1,
        "reopening must not make redundant upgrade backups"
    );
}

#[test]
fn failed_upgrade_rolls_back_schema_and_keeps_original_user_state() {
    let root = tempfile::tempdir().expect("fixture");
    let old = legacy(root.path());
    old.execute("INSERT INTO memberships VALUES('a',7,42,4,999,0)", [])
        .expect("legacy foreign-key violation fixture");
    assert!(matches!(Store::open(root.path()), Err(Error::Migration(_))));
    assert_eq!(scalar(&old, "PRAGMA user_version"), 1);
    assert_eq!(scalar(&old, "SELECT count(*) FROM messages"), 1);
    assert_eq!(scalar(&old, "SELECT count(*) FROM memberships"), 2);
    assert_eq!(
        scalar(
            &old,
            "SELECT count(*) FROM pragma_table_info('messages') WHERE name='received_at'"
        ),
        0
    );
    assert_eq!(scalar(&old, "SELECT count(*) FROM drafts"), 1);
    assert_eq!(
        fs::read_dir(root.path().join("backups"))
            .expect("recovery backup")
            .count(),
        1
    );
}

#[test]
fn removed_messages_never_reuse_an_old_reminder_identity_even_after_restart() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    store.ensure_account("a").expect("account");
    let first = store
        .ingest_message("a", b"first synthetic original")
        .expect("first");
    store
        .connection
        .execute(
            "INSERT INTO reminders VALUES('old',?1,123,'pending')",
            params![first],
        )
        .expect("old reminder");
    store
        .connection
        .execute("DELETE FROM messages WHERE id=?1", params![first])
        .expect("remove original fixture");
    let second = store
        .ingest_message("a", b"unrelated synthetic original")
        .expect("second");
    assert!(second > first);
    assert_eq!(
        scalar(
            &store.connection,
            "SELECT message_id FROM reminders WHERE id='old'"
        ),
        first
    );
    drop(store);
    let mut reopened = Store::open(root.path()).expect("reopen");
    let third = reopened
        .ingest_message("a", b"another original")
        .expect("third");
    assert!(third > second);
}

#[test]
fn future_schema_is_rejected_without_downgrading_or_clearing_data() {
    let root = tempfile::tempdir().expect("fixture");
    let old = legacy(root.path());
    old.execute_batch("PRAGMA user_version=999;")
        .expect("future version fixture");
    assert!(matches!(
        Store::open(root.path()),
        Err(Error::UnsupportedSchema)
    ));
    assert_eq!(scalar(&old, "PRAGMA user_version"), 999);
    assert_eq!(scalar(&old, "SELECT count(*) FROM drafts"), 1);
}
