use super::*;
fn fixture() -> (tempfile::TempDir, Store, i64, i64) {
    let root = tempfile::tempdir().expect("profile");
    let mut store = Store::open(root.path()).expect("store");
    store.ensure_account("a").expect("a");
    store.ensure_account("b").expect("b");
    let a = store.ingest_message("a", b"synthetic a").expect("original");
    let b = store.ingest_message("b", b"synthetic b").expect("original");
    (root, store, a, b)
}
fn request(message: i64, expected: Option<i64>) -> Schedule {
    Schedule {
        account: "a".into(),
        message,
        due_at: 200,
        time_zone: "Europe/Paris".into(),
        now: 100,
        expected,
    }
}
#[test]
fn generations_scope_restart_and_late_delivery_cannot_change_another_reminder() {
    let (root, mut store, message, foreign) = fixture();
    assert!(matches!(
        store.schedule_reminder(&request(foreign, None)),
        Err(Error::MissingMessage)
    ));
    let first = store
        .schedule_reminder(&request(message, None))
        .expect("schedule");
    assert!(matches!(
        store.schedule_reminder(&request(message, None)),
        Err(Error::OperationConflict)
    ));
    assert!(
        store
            .claim_reminder(&["b".into()], 300)
            .expect("disabled")
            .is_none()
    );
    assert!(
        store
            .claim_reminder(&["a".into()], 199)
            .expect("not due")
            .is_none()
    );
    let attempt = store
        .claim_reminder(&["a".into()], 200)
        .expect("claim")
        .expect("due");
    let later = store
        .schedule_reminder(&Schedule {
            due_at: 400,
            ..request(message, Some(first.generation))
        })
        .expect("reschedule");
    assert_eq!(first.notification, later.notification);
    assert_eq!(later.generation, 2);
    assert!(matches!(
        store.finish_reminder(&attempt, true, 201),
        Err(Error::OperationConflict)
    ));
    assert!(
        store
            .claim_reminder(&["a".into()], 500)
            .expect("withdraw first")
            .is_none()
    );
    assert_eq!(
        store
            .reminder_withdrawals(&["a".into()])
            .expect("withdrawals"),
        vec![later.clone()]
    );
    store.withdrew_reminder(&later).expect("desktop removal");
    let next = store
        .claim_reminder(&["a".into()], 500)
        .expect("claim")
        .expect("due");
    store.finish_reminder(&next, true, 500).expect("ack");
    store
        .edit_reminder("a", &first.id, 2, false)
        .expect("cancel");
    assert!(matches!(
        store.finish_reminder(&next, true, 501),
        Err(Error::OperationConflict)
    ));
    drop(store);
    let store = Store::open(root.path()).expect("restart");
    assert_eq!(
        store
            .reminder_for("a", message)
            .expect("record")
            .expect("exists")
            .state,
        State::Cancelled
    );
    assert_eq!(store.reminders(&["a".into()]).expect("visible"), Vec::new());
    assert_eq!(
        store
            .reminder_withdrawals(&["a".into()])
            .expect("cancel withdrawal")
            .len(),
        1
    );
}
#[tokio::test]
async fn only_the_exclusive_owner_recovers_interrupted_delivery_with_the_same_notification_id() {
    let (root, mut store, message, _) = fixture();
    store
        .schedule_reminder(&request(message, None))
        .expect("schedule");
    let old = store
        .claim_reminder(&["a".into()], 200)
        .expect("claim")
        .expect("due");
    drop(store);
    let store = Store::open(root.path()).expect("ordinary handle");
    assert_eq!(
        store
            .reminder_for("a", message)
            .expect("record")
            .expect("exists")
            .state,
        State::Delivering
    );
    drop(store);
    let owner = crate::worker::Worker::open(root.path().into())
        .await
        .expect("exclusive owner");
    let handle = owner.handle();
    let fresh = handle
        .claim_reminder(vec!["a".into()], 201)
        .await
        .expect("recover")
        .expect("due");
    assert_eq!(old.reminder.notification, fresh.reminder.notification);
    assert!(handle.finish_reminder(old, true, 201).await.is_err());
    handle
        .finish_reminder(fresh, true, 202)
        .await
        .expect("new ack");
    owner.shutdown().await.expect("shutdown");
}
#[test]
fn failures_missing_targets_and_read_history_remain_independent() {
    let (_root, mut store, message, _) = fixture();
    let record = store
        .schedule_reminder(&request(message, None))
        .expect("schedule");
    let attempt = store
        .claim_reminder(&["a".into()], 200)
        .expect("claim")
        .expect("due");
    store
        .finish_reminder(&attempt, false, 200)
        .expect("failure");
    assert!(
        store
            .claim_reminder(&["a".into()], 201)
            .expect("no automatic failure retry")
            .is_none()
    );
    store
        .edit_reminder("a", &record.id, 1, true)
        .expect("explicit retry");
    store
        .connection
        .execute("UPDATE messages SET read_cleared=1 WHERE id=?1", [message])
        .expect("read history");
    assert_eq!(
        store.next_reminder(&["a".into()]).expect("still scheduled"),
        Some(200)
    );
    store
        .connection
        .execute("DELETE FROM messages WHERE id=?1", [message])
        .expect("remove original");
    assert!(
        store
            .claim_reminder(&["a".into()], 202)
            .expect("missing target")
            .is_none()
    );
    let record = store
        .reminder_for("a", message)
        .expect("record")
        .expect("retained");
    assert!(!record.available);
    assert_eq!(record.state, State::Unavailable);
    store
        .edit_reminder("a", &record.id, record.generation, false)
        .expect("can cancel unavailable target");
    assert!(!format!("{record:?}").contains("Europe/Paris"));
}

#[test]
fn invalid_times_stale_desktop_actions_and_foreign_edits_do_not_change_state() {
    let (_root, mut store, message, _) = fixture();
    for (due, zone) in [
        (100, "UTC"),
        (0, "UTC"),
        (253_402_300_800, "UTC"),
        (200, "invalid/zone"),
    ] {
        assert!(matches!(
            store.schedule_reminder(&Schedule {
                due_at: due,
                time_zone: zone.into(),
                ..request(message, None)
            }),
            Err(Error::InvalidInput)
        ));
    }
    let first = store
        .schedule_reminder(&request(message, None))
        .expect("schedule");
    assert!(
        store
            .reminder_action(&["b".into()], &first.notification, first.generation)
            .expect("foreign action")
            .is_none()
    );
    assert!(matches!(
        store.edit_reminder("b", &first.id, first.generation, false),
        Err(Error::OperationConflict)
    ));
    let next = store
        .schedule_reminder(&request(message, Some(first.generation)))
        .expect("reschedule");
    assert!(
        store
            .reminder_action(&["a".into()], &first.notification, first.generation)
            .expect("stale action")
            .is_none()
    );
    assert_eq!(
        store
            .reminder_action(&["a".into()], &next.notification, next.generation)
            .expect("current action"),
        Some(next.clone())
    );
    assert!(matches!(
        store.withdrew_reminder(&first),
        Err(Error::OperationConflict)
    ));
    store
        .edit_reminder("a", &next.id, next.generation, false)
        .expect("cancel");
    assert!(
        store
            .reminder_action(&["a".into()], &next.notification, next.generation)
            .expect("cancelled action")
            .is_none()
    );
}
