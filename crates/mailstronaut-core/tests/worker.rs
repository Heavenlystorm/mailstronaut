//! The async facade keeps a locked SQLite writer off the async/UI thread.

use core::{num::NonZeroU32, time::Duration};
use std::time::Instant;

use mailstronaut_core::{
    store::mailbox::{Flags, Membership},
    worker::{Error, Worker},
};

const fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero fixture")
}

#[tokio::test]
async fn durable_round_trip_restarts_and_shutdown_closes_cloned_handles() {
    let root = tempfile::tempdir().expect("fixture");
    let owner = Worker::open(root.path().to_owned()).await.expect("owner");
    let handle = owner.handle();
    handle.ensure_account("a".into()).await.expect("account");
    let folder = handle
        .ensure_folder("a".into(), "INBOX".into(), true)
        .await
        .expect("folder");
    handle
        .select_folder("a".into(), folder.id, nonzero(42))
        .await
        .expect("SELECT");
    let membership = Membership {
        folder: folder.id,
        uid_validity: nonzero(42),
        uid: nonzero(1),
        flags: Flags::default(),
    };
    let raw = b"Message-ID: <worker@example.test>\r\nSubject: Worker fixture\r\n\r\nOffline body."
        .to_vec();
    let id = handle
        .cache_message("a".into(), membership, 123, None, raw)
        .await
        .expect("cache");
    let page = handle
        .inbox(vec!["a".into()], None, 100)
        .await
        .expect("page");
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, id);
    assert_eq!(
        handle
            .message("a".into(), id)
            .await
            .expect("body")
            .text
            .as_deref(),
        Some("Offline body.")
    );
    assert!(matches!(
        handle.inbox(vec!["a".into()], None, 101).await,
        Err(Error::InvalidRequest)
    ));
    owner.shutdown().await.expect("joined shutdown");
    assert!(matches!(
        handle.inbox(vec!["a".into()], None, 100).await,
        Err(Error::Closed)
    ));
    let reopened = Worker::open(root.path().to_owned()).await.expect("reopen");
    assert_eq!(
        reopened
            .handle()
            .inbox(vec!["a".into()], None, 100)
            .await
            .expect("persisted")[0]
            .id,
        id
    );
    reopened.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn sqlite_writer_contention_does_not_block_the_async_timer() {
    let root = tempfile::tempdir().expect("fixture");
    let owner = Worker::open(root.path().to_owned()).await.expect("owner");
    let handle = owner.handle();
    handle.ensure_account("a".into()).await.expect("account");
    let folder = handle
        .ensure_folder("a".into(), "INBOX".into(), true)
        .await
        .expect("folder");
    handle
        .select_folder("a".into(), folder.id, nonzero(42))
        .await
        .expect("SELECT");
    let connection =
        rusqlite::Connection::open(root.path().join("mail.sqlite3")).expect("lock fixture");
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .expect("exclusive writer reservation");
    let membership = Membership {
        folder: folder.id,
        uid_validity: nonzero(42),
        uid: nonzero(1),
        flags: Flags::default(),
    };
    let pending = tokio::spawn(async move {
        handle
            .cache_message(
                "a".into(),
                membership,
                123,
                None,
                b"Subject: Contention fixture\r\n\r\nBody.".to_vec(),
            )
            .await
    });
    tokio::task::yield_now().await;
    let started = Instant::now();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "SQLite contention stalled the async/UI thread"
    );
    assert!(
        !pending.is_finished(),
        "writer unexpectedly passed the held IMMEDIATE transaction"
    );
    connection
        .execute_batch("ROLLBACK")
        .expect("release writer");
    pending
        .await
        .expect("request task")
        .expect("cache after release");
    owner.shutdown().await.expect("shutdown");
}
