//! Synthetic durable submission states. No SMTP server or user mail is contacted.

use mailstronaut_core::{
    store::{
        Error, Store,
        outbox::{Draft, Phase},
    },
    worker::Worker,
};
use mailstronaut_mail::{compose::Content, smtp::Outcome};

fn draft() -> Draft {
    let mut draft = Draft::new("a".into(), "Sender <sender@example.test>".into());
    draft.content = Content {
        from: draft.content.from,
        to: "to@example.test".into(),
        bcc: "blind@example.test".into(),
        subject: "Synthetic draft".into(),
        body: "Persistent body.".into(),
        ..Content::default()
    };
    draft
}
fn store(root: &std::path::Path) -> Store {
    let mut store = Store::open(root).expect("store");
    store.ensure_account("a").expect("account");
    store
}

#[test]
fn autosave_rejects_older_revisions_and_closed_drafts_do_not_reappear() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = store(root.path());
    let old = draft();
    let mut latest = old.clone();
    latest.revision += 1;
    latest.content.body = "Newer edit.".into();
    store.save_draft(&latest).expect("autosave newer");
    assert!(matches!(
        store.save_draft(&old),
        Err(Error::OperationConflict)
    ));
    assert_eq!(
        store.draft("a", &old.id).expect("draft").content.body,
        "Newer edit."
    );
    assert!(matches!(
        store.draft("another-account", &old.id),
        Err(Error::MissingMessage)
    ));
    assert!(matches!(
        store.queue_draft("a", &old.id, old.revision, false),
        Err(Error::OperationConflict)
    ));
    let queued = store
        .queue_draft("a", &latest.id, latest.revision, false)
        .expect("freeze current");
    assert_eq!(queued.phase, Phase::Pending);
    assert!(matches!(
        store.save_draft(&latest),
        Err(Error::OperationConflict)
    ));
    assert_eq!(store.drafts(&["a".into()]).expect("draft list").len(), 0);
    drop(store);
    let mut reopened = Store::open(root.path()).expect("restart");
    assert!(
        matches!(reopened.save_draft(&latest), Err(Error::OperationConflict)),
        "late autosave cannot recreate an already queued draft"
    );
    reopened
        .cancel_submission("a", &queued.id)
        .expect("cancel before submission");
    assert_eq!(
        reopened
            .draft("a", &latest.id)
            .expect("recovered editable draft")
            .content
            .body,
        "Newer edit."
    );
    assert!(matches!(
        reopened.begin_submission("a", &queued.id),
        Err(Error::OperationConflict)
    ));
}

#[test]
fn frozen_outbox_blobs_survive_index_clear_collection_and_exact_retry() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = store(root.path());
    let draft = draft();
    store.save_draft(&draft).expect("save");
    let queued = store
        .queue_draft("a", &draft.id, draft.revision, false)
        .expect("queue");
    store
        .clear_rebuildable_indexes()
        .expect("clear cache indexes");
    store
        .collect_orphan_blobs()
        .expect("outbox is a durable blob owner");
    let first = store
        .begin_submission("a", &queued.id)
        .expect("claim verified original");
    assert!(!String::from_utf8_lossy(first.message.original()).contains("blind@example.test"));
    assert!(matches!(
        store.begin_submission("a", &queued.id),
        Err(Error::OperationConflict)
    ));
    assert!(matches!(
        store.cancel_submission("a", &queued.id),
        Err(Error::OperationConflict)
    ));
    store
        .finish_submission(&first, Outcome::Rejected { temporary: true })
        .expect("definite refusal");
    store
        .retry_submission("a", &queued.id, false)
        .expect("explicit retry");
    let retry = store
        .begin_submission("a", &queued.id)
        .expect("new attempt");
    assert_ne!(retry.attempt, first.attempt);
    assert_eq!(retry.message.message_id(), first.message.message_id());
    assert_eq!(retry.message.original(), first.message.original());
    assert!(matches!(
        store.finish_submission(&first, Outcome::Accepted),
        Err(Error::OperationConflict)
    ));
    store
        .finish_submission(&retry, Outcome::Accepted)
        .expect("DATA acknowledged");
    assert!(
        matches!(
            store.retry_submission("a", &queued.id, true),
            Err(Error::OperationConflict)
        ),
        "Sent-copy failure must never make an accepted message sendable"
    );
    let summary = store.outbox(&["a".into()]).expect("outbox");
    assert_eq!(summary[0].phase, Phase::Accepted);
    assert!(summary[0].sent_copy_pending);
    assert_eq!(
        store
            .outbox(&["disabled-account".into()])
            .expect("scope")
            .len(),
        0
    );
}

#[tokio::test]
async fn only_exclusive_startup_recovers_interrupted_delivery_and_requires_conscious_retry() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = store(root.path());
    let draft = draft();
    store.save_draft(&draft).expect("save");
    let queued = store
        .queue_draft("a", &draft.id, draft.revision, true)
        .expect("queue");
    let interrupted = store
        .begin_submission("a", &queued.id)
        .expect("intent committed before network");
    let second = Store::open(root.path()).expect("independent integrity handle");
    assert_eq!(
        second.outbox(&["a".into()]).expect("live state")[0].phase,
        Phase::Submitting,
        "Store::open alone must not declare a live attempt interrupted"
    );
    drop(second);
    drop(store);
    let worker = Worker::open(root.path().to_owned())
        .await
        .expect("exclusive startup");
    let handle = worker.handle();
    let outgoing = handle
        .outbox(vec!["a".into()])
        .await
        .expect("recovered state");
    assert_eq!(outgoing[0].phase, Phase::DeliveryUnknown);
    assert!(
        !outgoing[0].sent_copy_pending,
        "provider auto-save recorded separately"
    );
    assert!(matches!(
        handle.begin_submission("a".into(), queued.id.clone()).await,
        Err(mailstronaut_core::worker::Error::Conflict)
    ));
    assert!(matches!(
        handle
            .retry_submission("a".into(), queued.id.clone(), false)
            .await,
        Err(mailstronaut_core::worker::Error::Conflict)
    ));
    handle
        .retry_submission("a".into(), queued.id.clone(), true)
        .await
        .expect("explicit possible-duplicate decision");
    let retry = handle
        .begin_submission("a".into(), queued.id.clone())
        .await
        .expect("new attempt");
    assert_eq!(retry.message.original(), interrupted.message.original());
    assert!(matches!(
        handle
            .finish_submission(interrupted, Outcome::Accepted)
            .await,
        Err(mailstronaut_core::worker::Error::Conflict)
    ));
    handle
        .finish_submission(retry, Outcome::Accepted)
        .await
        .expect("accepted current attempt");
    worker.shutdown().await.expect("shutdown");
}

#[test]
fn cancelling_or_rejecting_a_retry_never_erases_earlier_delivery_uncertainty() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = store(root.path());
    let draft = draft();
    store.save_draft(&draft).expect("save");
    let queued = store
        .queue_draft("a", &draft.id, draft.revision, false)
        .expect("freeze");
    let first = store.begin_submission("a", &queued.id).expect("claim");
    store
        .finish_submission(&first, Outcome::DeliveryUnknown)
        .expect("lost DATA acknowledgment");
    store
        .retry_submission("a", &queued.id, true)
        .expect("explicit retry consent");
    store
        .cancel_submission("a", &queued.id)
        .expect("cancel only current retry");
    let outgoing = store.outbox(&["a".into()]).expect("outbox");
    assert_eq!(outgoing[0].phase, Phase::DeliveryUnknown);
    assert!(outgoing[0].uncertain);
    assert!(matches!(
        store.recover_draft("a", &queued.id),
        Err(Error::OperationConflict)
    ));
    store
        .retry_submission("a", &queued.id, true)
        .expect("another explicit decision");
    let retry = store.begin_submission("a", &queued.id).expect("retry");
    store
        .finish_submission(&retry, Outcome::Rejected { temporary: false })
        .expect("current attempt refused");
    assert_eq!(
        store.outbox(&["a".into()]).expect("state")[0].phase,
        Phase::DeliveryUnknown
    );
    assert!(matches!(
        store.retry_submission("a", &queued.id, false),
        Err(Error::OperationConflict)
    ));
    assert!(
        matches!(store.draft("a", &draft.id), Err(Error::MissingMessage)),
        "uncertain delivery must not silently reopen as an unsent draft"
    );
}

#[test]
fn editing_a_definite_rejection_creates_a_saved_new_draft_and_disables_the_old_intent() {
    let root = tempfile::tempdir().expect("fixture");
    let mut store = store(root.path());
    let draft = draft();
    store.save_draft(&draft).expect("save");
    let queued = store
        .queue_draft("a", &draft.id, draft.revision, false)
        .expect("freeze");
    let first = store.begin_submission("a", &queued.id).expect("claim");
    store
        .finish_submission(&first, Outcome::Rejected { temporary: false })
        .expect("refused");
    let recovered = store
        .recover_draft("a", &queued.id)
        .expect("edit as new draft");
    assert_ne!(recovered.id, draft.id);
    assert_eq!(recovered.content.body, draft.content.body);
    assert_eq!(
        store
            .draft("a", &recovered.id)
            .expect("already durable")
            .content
            .to,
        draft.content.to
    );
    assert!(matches!(
        store.retry_submission("a", &queued.id, false),
        Err(Error::OperationConflict)
    ));
}
