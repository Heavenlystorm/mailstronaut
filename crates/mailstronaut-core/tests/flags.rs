//! Offline coalescing, exact membership scope and callback fencing.

use core::num::NonZeroU32;

use mailstronaut_core::store::{
    Error, Store,
    mailbox::{Flags, Membership},
};
use mailstronaut_mail::imap::{FlagChange, Metadata};

const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RAW:&[u8]=b"From: sender@example.test\r\nTo: reader@example.test\r\nMessage-ID: <flags@example.test>\r\nSubject: Flags fixture\r\n\r\nOffline synthetic body.\r\n";
const fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero")
}
fn setup() -> (tempfile::TempDir, Store, i64, i64) {
    let root = tempfile::tempdir().expect("profile");
    let mut store = Store::open(root.path()).expect("store");
    store.ensure_account("a").expect("account");
    store.ensure_account("b").expect("other account");
    store
        .bind_mailbox_context("a", CONTEXT)
        .expect("authenticated context");
    let folder = store.ensure_folder("a", "INBOX", true).expect("folder").id;
    store.select_folder("a", folder, nz(42)).expect("namespace");
    let mut message = 0;
    for uid in [11, 12] {
        message = store
            .cache_message(
                "a",
                Membership {
                    folder,
                    uid_validity: nz(42),
                    uid: nz(uid),
                    flags: Flags::default(),
                },
                100,
                None,
                RAW,
            )
            .expect("cache exact copies");
    }
    (root, store, message, folder)
}
fn observation(uid: u32, seen: bool, starred: bool) -> Metadata {
    Metadata {
        uid: nz(uid),
        size: u32::try_from(RAW.len()).expect("size"),
        received_at: 100,
        seen,
        starred,
        gmail_id: None,
    }
}
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "One regression follows the same originals through copy coalescing, exact folder/UID acknowledgement and server namespace replacement."
)]
fn explicit_read_targets_current_copies_and_sent_original_without_marking_siblings() {
    use mailstronaut_core::store::browse::Scope;
    let (_root, mut store, original, _) = setup();
    let archive = store
        .ensure_folder("a", "Archive", false)
        .expect("archive")
        .id;
    let sent = store.ensure_folder("a", "Sent", false).expect("sent").id;
    for folder in [archive, sent] {
        store.select_folder("a", folder, nz(42)).expect("select");
    }
    let copy = store
        .cache_message(
            "a",
            Membership {
                folder: archive,
                uid_validity: nz(42),
                uid: nz(7),
                flags: Flags::default(),
            },
            100,
            None,
            RAW,
        )
        .expect("canonical archive copy");
    assert_eq!(copy, original);
    let sent_raw = b"From: reader@example.test\r\nTo: sender@example.test\r\nMessage-ID: <sent@example.test>\r\nSubject: Sent original\r\n\r\nSynthetic sent original.\r\n";
    let sent_message = store
        .cache_message(
            "a",
            Membership {
                folder: sent,
                uid_validity: nz(42),
                uid: nz(9),
                flags: Flags::default(),
            },
            101,
            None,
            sent_raw,
        )
        .expect("sent original");
    let sibling = store
        .cache_message(
            "a",
            Membership {
                folder: sent,
                uid_validity: nz(42),
                uid: nz(10),
                flags: Flags::default(),
            },
            102,
            None,
            b"Message-ID: <sibling@example.test>\r\n\r\nUnread sibling.",
        )
        .expect("unread sibling");
    store
        .set_flag("a", original, FlagChange::Seen, true, CONTEXT)
        .expect("explicit read");
    store
        .set_flag("a", sent_message, FlagChange::Seen, true, CONTEXT)
        .expect("explicit Sent read");
    let rows = store
        .conversation_page(&["a".into()], &Scope::Account("a".into()), "", None, 100)
        .expect("all cached originals");
    assert!(
        rows.iter()
            .filter(|row| [original, sent_message].contains(&row.latest.id))
            .all(|row| row.latest.seen)
    );
    assert!(
        !rows
            .iter()
            .find(|row| row.latest.id == sibling)
            .expect("sibling")
            .latest
            .seen
    );
    let mut targets = Vec::new();
    while let Some(attempt) = store.begin_flag("a", CONTEXT).expect("claim") {
        targets.push((attempt.folder.clone(), attempt.uid.get()));
        assert_eq!(attempt.validity, nz(42));
        store
            .confirm_flag(&attempt, &observation(attempt.uid.get(), true, false))
            .expect("exact namespace acknowledgement");
    }
    targets.sort();
    assert_eq!(
        targets,
        [
            ("Archive".into(), 7),
            ("INBOX".into(), 11),
            ("INBOX".into(), 12),
            ("Sent".into(), 9)
        ]
    );
    store
        .select_folder("a", sent, nz(43))
        .expect("server replaces UID namespace");
    assert!(matches!(
        store.set_flag("a", sent_message, FlagChange::Seen, false, CONTEXT),
        Err(Error::MissingMessage)
    ));
    assert_eq!(
        store
            .flag_counts(&["b".into()])
            .expect("other account")
            .pending,
        0
    );
}
#[test]
fn offline_flags_overlay_all_inbox_copies_and_survive_restart_without_touching_other_accounts() {
    let (root, mut store, message, _) = setup();
    assert!(matches!(
        store.set_flag("b", message, FlagChange::Seen, true, CONTEXT),
        Err(Error::StaleMailbox)
    ));
    store
        .set_flag("a", message, FlagChange::Seen, true, CONTEXT)
        .expect("local read");
    store
        .set_flag("a", message, FlagChange::Starred, true, CONTEXT)
        .expect("local star");
    let rows = store.inbox(&["a".into()], None, 100).expect("inbox");
    assert!(rows[0].seen && rows[0].starred);
    assert_eq!(store.flag_counts(&["a".into()]).expect("counts").pending, 4);
    assert_eq!(
        store
            .flag_counts(&["b".into()])
            .expect("other counts")
            .pending,
        0
    );
    drop(store);
    let store = Store::open(root.path()).expect("restart");
    assert_eq!(
        store.inbox(&["a".into()], None, 100).expect("offline rows"),
        rows
    );
}
#[test]
fn opposite_edit_while_submitted_survives_old_ack_and_stale_callback_cannot_complete_new_attempt() {
    let (_root, mut store, message, _) = setup();
    store
        .set_flag("a", message, FlagChange::Seen, true, CONTEXT)
        .expect("read");
    let first = store
        .begin_flag("a", CONTEXT)
        .expect("claim")
        .expect("first");
    assert!(matches!(
        store.begin_flag("a", CONTEXT),
        Err(Error::OperationConflict)
    ));
    store
        .set_flag("a", message, FlagChange::Seen, false, CONTEXT)
        .expect("newer unread");
    store
        .confirm_flag(&first, &observation(first.uid.get(), true, true))
        .expect("old ack");
    assert!(!store.inbox(&["a".into()], None, 100).expect("rows")[0].seen);
    let newer = store
        .begin_flag("a", CONTEXT)
        .expect("claim newer")
        .expect("newer");
    assert!(!newer.desired);
    assert!(matches!(
        store.confirm_flag(&first, &observation(first.uid.get(), true, true)),
        Err(Error::OperationConflict)
    ));
    store
        .confirm_flag(&newer, &observation(newer.uid.get(), false, true))
        .expect("new ack");
    let second = store
        .begin_flag("a", CONTEXT)
        .expect("claim remaining copy")
        .expect("second");
    store
        .confirm_flag(&second, &observation(second.uid.get(), false, false))
        .expect("copy ack");
    assert!(store.begin_flag("a", CONTEXT).expect("drained").is_none());
    let row = &store.inbox(&["a".into()], None, 100).expect("rows")[0];
    assert!(!row.seen && row.starred);
}
#[test]
fn failure_requires_explicit_retry_and_unknown_confirmation_is_not_success() {
    let (_root, mut store, message, _) = setup();
    store
        .set_flag("a", message, FlagChange::Seen, true, CONTEXT)
        .expect("read");
    let first = store
        .begin_flag("a", CONTEXT)
        .expect("claim")
        .expect("first");
    assert!(matches!(
        store.confirm_flag(&first, &observation(first.uid.get(), false, false)),
        Err(Error::InvalidInput)
    ));
    store.fail_flag(&first, false).expect("failed");
    assert_eq!(store.flag_counts(&["a".into()]).expect("counts").failed, 1);
    // The other copy may proceed, but the failed one is never silently retried.
    let other = store
        .begin_flag("a", CONTEXT)
        .expect("other claim")
        .expect("other");
    assert_ne!(first.uid, other.uid);
    store
        .confirm_flag(&other, &observation(other.uid.get(), true, false))
        .expect("other ack");
    assert!(
        store
            .begin_flag("a", CONTEXT)
            .expect("no automatic retry")
            .is_none()
    );
    store.retry_flags("a", CONTEXT).expect("explicit retry");
    let retry = store
        .begin_flag("a", CONTEXT)
        .expect("retry claim")
        .expect("retry");
    assert_eq!(retry.uid, first.uid);
    assert!(matches!(
        store.fail_flag(&first, false),
        Err(Error::OperationConflict)
    ));
}
#[test]
fn changed_server_or_uid_namespace_retires_old_work_without_rebinding_identical_uid_numbers() {
    let (_root, mut store, message, folder) = setup();
    store
        .set_flag("a", message, FlagChange::Seen, true, CONTEXT)
        .expect("read");
    let first = store
        .begin_flag("a", CONTEXT)
        .expect("claim")
        .expect("first");
    assert!(
        store
            .bind_mailbox_context("a", OTHER)
            .expect("new authenticated server")
    );
    store
        .select_folder("a", folder, nz(42))
        .expect("same numeric namespace on different server");
    assert!(matches!(
        store.confirm_flag(&first, &observation(first.uid.get(), true, false)),
        Err(Error::OperationConflict)
    ));
    assert!(
        store
            .begin_flag("a", OTHER)
            .expect("no old UID replay")
            .is_none()
    );
    assert!(matches!(
        store.set_flag("a", message, FlagChange::Seen, true, OTHER),
        Err(Error::MissingMessage)
    ));
    assert_eq!(
        store.flag_counts(&["a".into()]).expect("retired").conflicts,
        2
    );
    assert!(
        store.message("a", message).is_ok(),
        "original/user state retained"
    );
}
#[tokio::test]
async fn only_the_exclusive_owner_recovers_interrupted_idempotent_flags() {
    let (root, mut store, message, _) = setup();
    store
        .set_flag("a", message, FlagChange::Starred, true, CONTEXT)
        .expect("star");
    let old = store.begin_flag("a", CONTEXT).expect("claim").expect("old");
    let mut independent = Store::open(root.path()).expect("independent integrity reader");
    assert!(matches!(
        independent.begin_flag("a", CONTEXT),
        Err(Error::OperationConflict)
    ));
    drop(independent);
    drop(store);
    let owner = mailstronaut_core::worker::Worker::open(root.path().to_owned())
        .await
        .expect("exclusive recovery");
    let new = owner
        .handle()
        .begin_flag("a".into(), CONTEXT.into())
        .await
        .expect("recovered claim")
        .expect("new");
    assert_eq!(new.uid, old.uid);
    assert_eq!(new.flag, FlagChange::Starred);
    owner.shutdown().await.expect("shutdown");
}

#[test]
fn undo_is_local_only_before_submission_and_never_revives_priority_read_history() {
    let (root, mut store, message, _) = setup();
    store
        .set_flag("a", message, FlagChange::Seen, true, CONTEXT)
        .expect("read");
    store
        .set_flag("a", message, FlagChange::Starred, true, CONTEXT)
        .expect("star");
    assert!(matches!(
        store.undo_flags("b", message),
        Err(Error::OperationConflict)
    ));
    store.undo_flags("a", message).expect("undo unsent");
    let rows = store.inbox(&["a".into()], None, 100).expect("rows");
    assert!(!rows[0].seen && !rows[0].starred);
    let db = rusqlite::Connection::open(root.path().join("mail.sqlite3")).expect("inspect");
    assert!(
        db.query_row(
            "SELECT read_cleared FROM messages WHERE id=?1",
            [message],
            |row| row.get::<_, bool>(0)
        )
        .expect("sticky read history")
    );
    store
        .set_flag("a", message, FlagChange::Seen, true, CONTEXT)
        .expect("read again");
    let attempt = store
        .begin_flag("a", CONTEXT)
        .expect("claim")
        .expect("attempt");
    assert!(matches!(
        store.undo_flags("a", message),
        Err(Error::OperationConflict)
    ));
    store.fail_flag(&attempt, false).expect("unknown failure");
    store
        .set_flag("a", message, FlagChange::Seen, false, CONTEXT)
        .expect("newer opposite intent");
    assert!(
        matches!(
            store.undo_flags("a", message),
            Err(Error::OperationConflict)
        ),
        "a newer edit cannot erase earlier possible STORE delivery"
    );
}

#[test]
fn changed_server_cannot_reuse_provider_ids_to_replace_originals_or_user_state() {
    use core::num::NonZeroU64;
    use mailstronaut_core::store::mailbox::GmailId;
    let root = tempfile::tempdir().expect("profile");
    let mut store = Store::open(root.path()).expect("store");
    store.ensure_account("a").expect("account");
    store
        .bind_mailbox_context("a", CONTEXT)
        .expect("old server");
    let folder = store.ensure_folder("a", "INBOX", true).expect("folder").id;
    store.select_folder("a", folder, nz(42)).expect("namespace");
    let membership = Membership {
        folder,
        uid_validity: nz(42),
        uid: nz(11),
        flags: Flags::default(),
    };
    let provider = Some(GmailId(NonZeroU64::new(123).expect("provider ID")));
    let old = store
        .cache_message("a", membership, 100, provider, RAW)
        .expect("old original");
    let db = rusqlite::Connection::open(root.path().join("mail.sqlite3")).expect("user state");
    db.execute(
        "INSERT INTO reminders(id,message_id,due_at,state) VALUES('keep',?1,123,'pending')",
        [old],
    )
    .expect("reminder");
    store
        .bind_mailbox_context("a", OTHER)
        .expect("different authenticated server");
    store
        .select_folder("a", folder, nz(42))
        .expect("same numeric generation");
    let different=b"From: other@example.test\r\nSubject: Different account original\r\nMessage-ID: <other@example.test>\r\n\r\nDifferent synthetic content.\r\n";
    let new = store
        .cache_message("a", membership, 101, provider, different)
        .expect("reused numeric provider ID");
    assert_ne!(old, new);
    assert_eq!(
        store
            .message("a", old)
            .expect("retained original")
            .headers
            .subject
            .as_deref(),
        Some("Flags fixture")
    );
    assert_eq!(
        store
            .message("a", new)
            .expect("new original")
            .headers
            .subject
            .as_deref(),
        Some("Different account original")
    );
    assert_eq!(
        db.query_row(
            "SELECT message_id FROM reminders WHERE id='keep'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("retained reminder"),
        old
    );
}
