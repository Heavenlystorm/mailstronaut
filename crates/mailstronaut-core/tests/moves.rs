//! Archive/trash/move intentions: immediate local effect and exact UID scope.

use core::num::NonZeroU32;

use mailstronaut_core::store::{
    Error, Store,
    analysis::AnalysisResult,
    mailbox::{Flags, Membership, UidSnapshot},
    reminders::Schedule,
};

const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero")
}

struct Fixture {
    root: tempfile::TempDir,
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
        root,
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

/// Opening a message sets Seen, the server reports that over IDLE, and a sync pass starts at
/// once. That pass must not take the move the user can still undo.
#[test]
fn no_sync_pass_can_claim_a_move_during_its_undo_window() {
    let mut f = fixture();
    let inbox = f.inbox;
    let message = add(&mut f, inbox, 42, 1, "held@example.test");
    let later = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs(),
    )
    .expect("clock")
        + 3600;
    f.store
        .queue_move_after("a", message, None, "Archive", CONTEXT, later)
        .expect("queue");
    assert!(
        f.store.begin_move("a", CONTEXT).expect("claim").is_none(),
        "still in its Undo window"
    );
    f.store.undo_move("a", message).expect("Undo still works");
    // Queued again with its window over, the same message can now be claimed.
    f.store
        .queue_move_after("a", message, None, "Archive", CONTEXT, 0)
        .expect("queue");
    assert!(f.store.begin_move("a", CONTEXT).expect("claim").is_some());
}

#[test]
fn a_move_given_up_on_shows_the_message_and_is_never_tried_again() {
    let mut f = fixture();
    let inbox = f.inbox;
    let message = add(&mut f, inbox, 42, 1, "refused@example.test");
    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queue");
    assert_eq!(inbox_ids(&f.store), Vec::<i64>::new());
    let attempt = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("attempt");
    f.store.abandon_move(&attempt).expect("abandon");
    assert_eq!(inbox_ids(&f.store), [message], "back where it was");
    f.store.retry_moves("a").expect("retry");
    assert!(f.store.begin_move("a", CONTEXT).expect("claim").is_none());
    assert!(
        f.store.abandon_move(&attempt).is_err(),
        "a stale attempt changes nothing"
    );
    drop(f.store);
    let mut reopened = Store::open(f.root.path()).expect("reopen");
    let accounts = ["a".to_owned()];
    let expected = [("a".to_owned(), CONTEXT.to_owned())];
    assert_eq!(
        reopened.move_warnings(&accounts).expect("warning"),
        expected
    );
    reopened
        .dismiss_move_warning("a", OTHER)
        .expect("stale dismissal");
    assert_eq!(
        reopened
            .move_warnings(&accounts)
            .expect("still belongs to original"),
        expected
    );
    reopened
        .dismiss_move_warning("a", CONTEXT)
        .expect("dismiss");
    drop(reopened);
    let store = Store::open(f.root.path()).expect("reopen after dismissal");
    assert_eq!(
        store.move_warnings(&accounts).expect("acknowledged"),
        Vec::<(String, String)>::new()
    );
}

#[test]
fn a_transient_move_failure_leaves_no_permanent_warning_and_rebind_retires_it() {
    let mut f = fixture();
    let inbox = f.inbox;
    let message = add(&mut f, inbox, 42, 1, "temporary@example.test");
    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queue");
    let attempt = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("attempt");
    f.store.finish_move(&attempt, false).expect("transient");
    let accounts = ["a".to_owned()];
    assert_eq!(
        f.store.move_warnings(&accounts).expect("no warning"),
        Vec::<(String, String)>::new()
    );
    f.store.retry_moves("a").expect("retry");
    let retry = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("retry");
    f.store.abandon_move(&retry).expect("unsupported");
    f.store
        .bind_mailbox_context("a", OTHER)
        .expect("new authenticated context");
    assert_eq!(
        f.store.move_warnings(&accounts).expect("retired"),
        Vec::<(String, String)>::new()
    );
}

#[test]
fn a_reminder_a_dismissal_and_the_analysis_follow_the_message_through_a_move() {
    let mut f = fixture();
    let inbox = f.inbox;
    let message = add(&mut f, inbox, 42, 1, "tracked");
    f.store
        .store_analysis(&[AnalysisResult {
            id: message,
            policy: 1,
            expires_at: Some(4_102_444_800),
            due: Some("2100-01-01".into()),
            data: "{\"rules\":1}".into(),
        }])
        .expect("analysis");
    f.store
        .schedule_reminder(&Schedule {
            account: "a".into(),
            message,
            due_at: 3_600,
            time_zone: "UTC".into(),
            now: 100,
            expected: None,
        })
        .expect("reminder");
    f.store.dismiss_priority(message).expect("dismissed");
    f.store
        .queue_move("a", message, None, "Archive", CONTEXT)
        .expect("queued");
    let attempt = f
        .store
        .begin_move("a", CONTEXT)
        .expect("claim")
        .expect("attempt");
    f.store.finish_move(&attempt, true).expect("moved");
    // The server now has the copy in Archive, which the next sync caches. Its identity matches.
    let archived = f
        .store
        .cache_message(
            "a",
            Membership {
                folder: f.archive,
                uid_validity: nz(43),
                uid: nz(7),
                flags: Flags::default(),
            },
            100,
            None,
            "From: sender@example.test\r\nTo: reader@example.test\r\nMessage-ID: <tracked>\r\nSubject: Move fixture\r\n\r\nSynthetic body tracked.\r\n".as_bytes(),
        )
        .expect("archived copy");
    assert_eq!(archived, message, "the moved copy is the same message");
    // The source folder then reports that UID 1 is gone.
    f.store
        .reconcile_span(
            "a",
            &UidSnapshot {
                folder: f.inbox,
                uid_validity: nz(42),
                first: nz(1),
                last: nz(15),
                present: vec![],
            },
        )
        .expect("source reconciled");
    assert!(
        f.store
            .reminder_for("a", message)
            .expect("reminder query")
            .is_some(),
        "the reminder survives the move and reconciliation"
    );
    assert_eq!(
        f.store.analysis_data(message).expect("analysis").as_deref(),
        Some("{\"rules\":1}"),
        "the analysis survives the move"
    );
    let needing_attention = f
        .store
        .priority_items(&["a".to_owned()], 100, 10)
        .expect("priority");
    assert!(
        needing_attention.iter().all(|item| item.id != message),
        "a dismissal is not undone by moving the message"
    );
}
