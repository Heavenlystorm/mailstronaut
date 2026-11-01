//! Archive/trash/move intentions: immediate local effect and exact UID scope.

use core::num::NonZeroU32;

use mailstronaut_core::store::{
    Error, Store,
    mailbox::{Flags, Membership},
};

const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero")
}

struct Fixture {
    _root: tempfile::TempDir,
    store: Store,
    inbox: i64,
    archive: i64,
    sent: i64,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().expect("profile");
    let mut store = Store::open(root.path()).expect("store");
    store.ensure_account("a").expect("account");
    store.bind_mailbox_context("a", CONTEXT).expect("context");
    let mut folder = |name: &str, inbox: bool, validity: u32| {
        let id = store.ensure_folder("a", name, inbox).expect("folder").id;
        store.select_folder("a", id, nz(validity)).expect("select");
        id
    };
    let (inbox, archive, sent) = (
        folder("INBOX", true, 42),
        folder("Archive", false, 43),
        folder("Sent", false, 44),
    );
    Fixture {
        _root: root,
        store,
        inbox,
        archive,
        sent,
    }
}

fn add(fixture: &mut Fixture, folder: i64, validity: u32, uid: u32, id: &str) -> i64 {
    fixture
        .store
        .cache_message(
            "a",
            Membership {
                folder,
                uid_validity: nz(validity),
                uid: nz(uid),
                flags: Flags::default(),
            },
            100,
            None,
            format!(
                "From: sender@example.test\r\nTo: reader@example.test\r\nMessage-ID: <{id}>\r\nSubject: Move fixture\r\n\r\nSynthetic body {id}.\r\n"
            )
            .as_bytes(),
        )
        .expect("cache")
}

fn inbox_ids(store: &Store) -> Vec<i64> {
    store
        .inbox(&["a".to_owned()], None, 100)
        .expect("inbox")
        .iter()
        .map(|row| row.id)
        .collect()
}

#[test]
fn a_queued_move_hides_at_once_and_a_failure_brings_the_message_back() {
    let mut f = fixture();
    let (inbox, archive) = (f.inbox, f.archive);
    let first = add(&mut f, inbox, 42, 1, "first@example.test");
    let second = add(&mut f, inbox, 42, 2, "second@example.test");
    assert_eq!(
        f.store
            .queue_move("a", first, None, "Archive", CONTEXT)
            .expect("queue"),
        1
    );
    assert_eq!(inbox_ids(&f.store), [second], "hidden before any network");
    let attempt = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("queued move");
    assert_eq!(attempt.folder, "INBOX");
    assert_eq!(attempt.uid, nz(1));
    assert_eq!(attempt.validity, nz(42));
    assert_eq!(attempt.destination, "Archive");
    assert!(
        matches!(
            f.store.begin_move("a", CONTEXT),
            Err(Error::OperationConflict)
        ),
        "one move per account at a time"
    );
    f.store.finish_move(&attempt, false).expect("failure");
    let mut visible = inbox_ids(&f.store);
    visible.sort_unstable();
    assert_eq!(visible, [first, second], "a failed move shows it again");
    assert!(f.store.begin_move("a", CONTEXT).expect("claim").is_none());
    f.store.retry_moves("a").expect("explicit retry");
    let retried = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("requeued");
    f.store.finish_move(&retried, true).expect("moved");
    assert_eq!(inbox_ids(&f.store), [second]);
    assert!(
        matches!(
            f.store.finish_move(&attempt, true),
            Err(Error::OperationConflict)
        ),
        "a stale callback cannot commit"
    );
    assert_eq!(
        f.store
            .folders(&["a".to_owned()])
            .expect("folders")
            .iter()
            .find(|row| row.id == archive)
            .map(|row| row.name.clone()),
        Some("Archive".to_owned())
    );
}

#[test]
fn undo_works_only_before_the_server_could_have_seen_it() {
    let mut f = fixture();
    let inbox = f.inbox;
    let message = add(&mut f, inbox, 42, 1, "undo@example.test");
    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queue");
    f.store.undo_move("a", message).expect("undo unsent move");
    assert_eq!(inbox_ids(&f.store), [message]);
    assert!(f.store.begin_move("a", CONTEXT).expect("claim").is_none());
    assert!(matches!(
        f.store.undo_move("a", message),
        Err(Error::OperationConflict)
    ));

    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queue again");
    let attempt = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("queued");
    assert!(matches!(
        f.store.undo_move("a", message),
        Err(Error::OperationConflict)
    ));
    f.store.finish_move(&attempt, false).expect("failure");
    assert!(
        matches!(
            f.store.undo_move("a", message),
            Err(Error::OperationConflict)
        ),
        "an attempted move may be at the destination"
    );
}

#[test]
fn moving_leaves_copies_in_other_folders_and_rejects_bad_requests() {
    let mut f = fixture();
    let (inbox, sent) = (f.inbox, f.sent);
    let message = add(&mut f, inbox, 42, 1, "copy@example.test");
    assert_eq!(
        add(&mut f, sent, 44, 9, "copy@example.test"),
        message,
        "same original in two folders"
    );
    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queue");
    let attempt = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("queued");
    assert_eq!((attempt.folder.as_str(), attempt.uid), ("INBOX", nz(1)));
    assert!(f.store.begin_move("a", CONTEXT).is_err());
    f.store.finish_move(&attempt, true).expect("moved");
    assert!(
        f.store.begin_move("a", CONTEXT).expect("claim").is_none(),
        "only the inbox copy"
    );
    assert_eq!(
        f.store
            .message("a", message)
            .expect("kept")
            .headers
            .subject
            .as_deref(),
        Some("Move fixture")
    );

    assert!(matches!(
        f.store.queue_move("a", message, None, "Missing", CONTEXT),
        Err(Error::InvalidInput)
    ));
    assert!(matches!(
        f.store.queue_move("a", message, None, "Archive", OTHER),
        Err(Error::StaleMailbox)
    ));
    assert!(matches!(
        f.store
            .queue_move("a", message, Some(inbox), "Archive", CONTEXT),
        Err(Error::MissingMessage)
    ));
}

#[test]
fn a_changed_configuration_drops_queued_moves_instead_of_moving_other_mail() {
    let mut f = fixture();
    let inbox = f.inbox;
    let message = add(&mut f, inbox, 42, 1, "context@example.test");
    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queue");
    assert!(f.store.begin_move("a", OTHER).expect("claim").is_none());
    assert_eq!(inbox_ids(&f.store), [message], "visible again");
}
