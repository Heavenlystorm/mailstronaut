//! Synthetic durable mailbox behavior, independent of any real server/account.

use core::num::{NonZeroU32, NonZeroU64};

use mailstronaut_core::store::{
    Error, Store,
    mailbox::{Flags, Folder, GmailId, Membership, UidSnapshot},
};

const fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero fixture")
}

#[test]
fn reconciliation_updates_flags_and_removes_only_searched_expunges() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    let elsewhere = folder(&mut store, "a", "Other inbox", true);
    let bytes = original("reconcile", "Retained original.");
    let first = store
        .cache_message("a", membership(&inbox, 10, false), 100, None, &bytes)
        .expect("first");
    store
        .cache_message(
            "a",
            membership(&inbox, 20, false),
            100,
            None,
            &original("other", "Another."),
        )
        .expect("second");
    store
        .cache_message("a", membership(&elsewhere, 10, false), 100, None, &bytes)
        .expect("another membership");
    assert!(
        store
            .observe_membership("a", membership(&inbox, 10, true))
            .expect("observe")
    );
    assert!(
        !store
            .observe_membership("a", membership(&inbox, 11, true))
            .expect("uncached")
    );
    assert!(
        !store
            .inbox(&["a".into()], None, 100)
            .expect("rows")
            .iter()
            .find(|row| row.id == first)
            .expect("first")
            .seen,
        "other unread membership remains unread"
    );
    store
        .reconcile_span(
            "a",
            &UidSnapshot {
                folder: inbox.id,
                uid_validity: nonzero(42),
                first: nonzero(1),
                last: nonzero(15),
                present: vec![],
            },
        )
        .expect("expunge only window");
    assert_eq!(
        store.inbox(&["a".into()], None, 100).expect("rows").len(),
        2,
        "other folder and UID 20 survive"
    );
    store
        .reconcile_span(
            "a",
            &UidSnapshot {
                folder: elsewhere.id,
                uid_validity: nonzero(42),
                first: nonzero(1),
                last: nonzero(15),
                present: vec![],
            },
        )
        .expect("last membership removed");
    assert_eq!(
        store.inbox(&["a".into()], None, 100).expect("rows").len(),
        1
    );
    assert_eq!(
        store
            .message("a", first)
            .expect("original retained")
            .text
            .as_deref(),
        Some("Retained original.")
    );
}

#[test]
fn reconciliation_rejects_stale_or_filtered_uid_snapshots() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    assert!(matches!(
        store.reconcile_span(
            "a",
            &UidSnapshot {
                folder: inbox.id,
                uid_validity: nonzero(43),
                first: nonzero(1),
                last: nonzero(15),
                present: vec![]
            }
        ),
        Err(Error::StaleMailbox)
    ));
    assert!(matches!(
        store.reconcile_span(
            "a",
            &UidSnapshot {
                folder: inbox.id,
                uid_validity: nonzero(42),
                first: nonzero(1),
                last: nonzero(15),
                present: vec![nonzero(20)]
            }
        ),
        Err(Error::InvalidInput)
    ));
}

#[test]
fn a_server_read_observation_survives_later_unread_and_store_restart() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    let id = store
        .cache_message(
            "a",
            membership(&inbox, 10, false),
            100,
            None,
            &original("read-history", "Read elsewhere."),
        )
        .expect("cache");
    assert!(
        store
            .observe_membership("a", membership(&inbox, 10, true))
            .expect("read elsewhere")
    );
    assert!(
        store
            .observe_membership("a", membership(&inbox, 10, false))
            .expect("later unread")
    );
    drop(store);
    let store = Store::open(root.path()).expect("restart");
    assert!(!store.inbox(&["a".into()], None, 100).expect("row")[0].seen);
    let database = rusqlite::Connection::open(root.path().join("mail.sqlite3"))
        .expect("inspect durable history");
    assert!(
        database
            .query_row(
                "SELECT read_cleared FROM messages WHERE id=?1",
                [id],
                |row| row.get::<_, bool>(0)
            )
            .expect("history"),
        "future analysis must not revive an old priority decision after Unseen"
    );
}
const fn gmail(value: u64) -> GmailId {
    GmailId(NonZeroU64::new(value).expect("nonzero fixture"))
}
fn original(id: &str, body: &str) -> Vec<u8> {
    format!("From: Synthetic Sender <sender@example.test>\r\nTo: recipient@example.test\r\nMessage-ID: <{id}@example.test>\r\nSubject: Synthetic subject\r\n\r\n{body}").into_bytes()
}
const fn membership(folder: &Folder, uid: u32, seen: bool) -> Membership {
    Membership {
        folder: folder.id,
        uid_validity: nonzero(42),
        uid: nonzero(uid),
        flags: Flags {
            seen,
            starred: false,
        },
    }
}
fn folder(store: &mut Store, account: &str, name: &str, inbox: bool) -> Folder {
    store.ensure_account(account).expect("account");
    let folder = store.ensure_folder(account, name, inbox).expect("folder");
    store
        .select_folder(account, folder.id, nonzero(42))
        .expect("SELECT generation");
    folder
}

#[test]
fn canonical_copies_remain_account_local_and_distinct_bodies_stay_distinct() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let a = folder(&mut store, "a", "INBOX", true);
    let archive = folder(&mut store, "a", "Archive", false);
    let b = folder(&mut store, "b", "INBOX", true);
    let bytes = original("same", "First body.");
    let first = store
        .cache_message("a", membership(&a, 1, false), 100, None, &bytes)
        .expect("cache");
    let second = store
        .cache_message("a", membership(&archive, 10, true), 100, None, &bytes)
        .expect("exact copy");
    assert_eq!(first, second);
    let other = store
        .cache_message("b", membership(&b, 1, false), 100, None, &bytes)
        .expect("other account");
    assert_ne!(first, other);
    let distinct = store
        .cache_message(
            "a",
            membership(&a, 2, false),
            101,
            None,
            &original("same", "Different body."),
        )
        .expect("duplicate Message-ID distinct body");
    assert_ne!(first, distinct);
    let rows = store
        .inbox(&["a".into(), "b".into()], None, 100)
        .expect("inbox");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].id, distinct);
    assert!(
        !rows.iter().find(|row| row.id == first).expect("first").seen,
        "a read archive copy must not mark its inbox copy Seen"
    );
    assert_eq!(
        store
            .message("a", first)
            .expect("offline body")
            .text
            .as_deref(),
        Some("First body.")
    );
    assert!(matches!(
        store.message("b", first),
        Err(Error::MissingMessage)
    ));
    drop(store);
    let reopened = Store::open(root.path()).expect("reopen");
    assert_eq!(
        reopened
            .inbox(&["a".into()], None, 100)
            .expect("offline inbox")
            .len(),
        2
    );
}

#[test]
fn a_uid_reset_reattaches_the_same_message_once_and_forgets_the_old_namespace() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    let bytes = original("reset", "Survives a UIDVALIDITY reset.");
    let before = store
        .cache_message("a", membership(&inbox, 1, false), 100, None, &bytes)
        .expect("before the reset");
    // The server starts a new UID namespace for the folder. Old UIDs now name nothing.
    store
        .select_folder("a", inbox.id, nonzero(43))
        .expect("new SELECT generation");
    assert!(
        store
            .inbox(&["a".into()], None, 100)
            .expect("after reset")
            .is_empty(),
        "the old namespace's memberships are not shown under the new one"
    );
    assert_eq!(
        store
            .message("a", before)
            .expect("retained body")
            .text
            .as_deref(),
        Some("Survives a UIDVALIDITY reset.")
    );
    let reset = Membership {
        uid_validity: nonzero(43),
        ..membership(&inbox, 1, false)
    };
    let after = store
        .cache_message("a", reset, 100, None, &bytes)
        .expect("the same message under the new namespace");
    assert_eq!(
        before, after,
        "a reset must not duplicate a message it still holds"
    );
    let rows = store.inbox(&["a".into()], None, 100).expect("inbox");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, before);
}

#[test]
fn gmail_ids_win_over_header_duplicates_and_share_label_copies() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    let label = folder(&mut store, "a", "Label", true);
    let b = folder(&mut store, "b", "INBOX", true);
    let bytes = original("same", "Body.");
    let one = store
        .cache_message(
            "a",
            membership(&inbox, 1, false),
            10,
            Some(gmail(7)),
            &bytes,
        )
        .expect("first Gmail ID");
    let copy = store
        .cache_message(
            "a",
            membership(&label, 20, true),
            10,
            Some(gmail(7)),
            &bytes,
        )
        .expect("label copy");
    assert_eq!(one, copy);
    let two = store
        .cache_message(
            "a",
            membership(&inbox, 2, false),
            10,
            Some(gmail(8)),
            &bytes,
        )
        .expect("different Gmail ID with identical raw message");
    assert_ne!(one, two);
    assert!(
        matches!(
            store.cache_message(
                "a",
                membership(&inbox, 1, false),
                10,
                Some(gmail(99)),
                &bytes
            ),
            Err(Error::IdentityConflict)
        ),
        "an existing UID cannot acquire a different Gmail identity"
    );
    let other = store
        .cache_message("b", membership(&b, 1, false), 10, Some(gmail(7)), &bytes)
        .expect("provider ID scoped to account");
    assert_ne!(one, other);
    assert_eq!(
        store
            .inbox(&["a".into()], None, 100)
            .expect("unified labels")
            .len(),
        2
    );
}

#[test]
fn uid_conflicts_and_stale_generations_do_not_retarget_cached_messages() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    let bytes = original("first", "Original.");
    let id = store
        .cache_message("a", membership(&inbox, 1, false), 10, None, &bytes)
        .expect("cache");
    assert!(matches!(
        store.cache_message(
            "a",
            membership(&inbox, 1, true),
            10,
            None,
            &original("second", "Replacement.")
        ),
        Err(Error::IdentityConflict)
    ));
    assert_eq!(
        store.message("a", id).expect("original retained").original,
        bytes
    );
    assert!(
        !store
            .inbox(&["a".into()], None, 100)
            .expect("flags retained")[0]
            .seen
    );
    store
        .select_folder("a", inbox.id, nonzero(43))
        .expect("new generation");
    assert_eq!(
        store
            .inbox(&["a".into()], None, 100)
            .expect("old UID bindings retired"),
        Vec::new()
    );
    assert!(matches!(
        store.cache_message("a", membership(&inbox, 2, false), 10, None, &bytes),
        Err(Error::StaleMailbox)
    ));
    let mut new_generation = membership(&inbox, 9, false);
    new_generation.uid_validity = nonzero(43);
    let reconciled = store
        .cache_message("a", new_generation, 10, None, &bytes)
        .expect("reconcile exact original");
    assert_eq!(id, reconciled);
}

#[test]
fn keyset_pagination_survives_new_arrivals_and_disabled_accounts() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let a = folder(&mut store, "a", "INBOX", true);
    let b = folder(&mut store, "b", "INBOX", true);
    for uid in 1..=6 {
        store
            .cache_message(
                "a",
                membership(&a, uid, false),
                i64::from(uid / 2),
                None,
                &original(&uid.to_string(), "Body."),
            )
            .expect("cache");
    }
    store
        .cache_message(
            "b",
            membership(&b, 1, false),
            100,
            None,
            &original("b", "Body."),
        )
        .expect("other account");
    let first = store.inbox(&["a".into()], None, 2).expect("first page");
    let cursor = first.last().expect("nonempty").cursor();
    store
        .cache_message(
            "a",
            membership(&a, 7, false),
            1000,
            None,
            &original("new", "New body."),
        )
        .expect("arrival");
    let rest = store
        .inbox(&["a".into()], Some(cursor), 100)
        .expect("remaining page");
    assert_eq!(rest.len(), 4);
    assert!(
        rest.iter()
            .all(|row| first.iter().all(|old| old.id != row.id))
    );
    assert!(rest.iter().all(|row| row.account == "a"));
    assert_eq!(
        store.inbox(&[], None, 100).expect("no enabled accounts"),
        Vec::new()
    );
    assert!(matches!(
        store.inbox(&["a".into()], None, 101),
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        store.inbox(&["a".into()], None, 0),
        Err(Error::InvalidInput)
    ));
}

#[test]
fn raw_messages_without_identifiers_do_not_collapse_and_corruption_is_rejected() {
    use std::{fs, os::unix::fs::symlink};
    let root = tempfile::tempdir().expect("fixture");
    let mut store = Store::open(root.path()).expect("open");
    let inbox = folder(&mut store, "a", "INBOX", true);
    let raw = b"Subject: No identifier\r\n\r\nBody.";
    let one = store
        .cache_message("a", membership(&inbox, 1, false), 1, None, raw)
        .expect("first");
    let two = store
        .cache_message("a", membership(&inbox, 2, false), 1, None, raw)
        .expect("second");
    assert_ne!(one, two);
    let blob = fs::read_dir(root.path().join("blobs"))
        .expect("blobs")
        .next()
        .expect("one shared blob")
        .expect("entry")
        .path();
    fs::write(&blob, b"corrupt").expect("corruption fixture");
    assert!(matches!(store.message("a", one), Err(Error::CorruptBlob)));
    fs::remove_file(&blob).expect("remove fixture blob");
    let elsewhere = root.path().join("unrelated");
    fs::write(&elsewhere, raw).expect("unrelated fixture");
    symlink(elsewhere, blob).expect("symlink fixture");
    assert!(matches!(store.message("a", one), Err(Error::CorruptBlob)));
}
