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
async fn only_one_application_worker_can_own_a_profile_until_shutdown() {
    let root = tempfile::tempdir().expect("fixture");
    let owner = Worker::open(root.path().to_owned())
        .await
        .expect("first owner");
    assert!(matches!(
        Worker::open(root.path().to_owned()).await,
        Err(Error::ProfileInUse)
    ));
    let _independent_store = mailstronaut_core::store::Store::open(root.path())
        .expect("independent integrity/GC handles remain supported");
    owner.shutdown().await.expect("release profile");
    let next = Worker::open(root.path().to_owned())
        .await
        .expect("new owner after shutdown");
    next.shutdown().await.expect("shutdown");
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

#[derive(serde::Deserialize)]
struct GuiMessage {
    account: usize,
    key: String,
    mid: String,
    references: String,
    uid: u32,
    sent: bool,
    seen: bool,
    body: String,
    from: String,
    order: i64,
}

#[tokio::test]
#[ignore = "explicit private native conversation GUI fixture only"]
#[expect(
    clippy::too_many_lines,
    reason = "Keep project/private-path checks, native config validation and the ordered offline seed in this explicit harness-only fixture."
)]
async fn seed_conversation_gui() {
    use mailstronaut_core::store::flags;
    use std::{fs, path::PathBuf};
    use tokio_util::sync::CancellationToken;
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").expect("private XDG root"))
        .canonicalize()
        .expect("data");
    assert!(data.starts_with(workspace));
    let config = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_THREADS_CONFIG").expect("synthetic configuration"),
    );
    assert!(
        config
            .canonicalize()
            .expect("configuration")
            .starts_with(data.parent().expect("private profile"))
    );
    let binary = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_THREADS_GUI_BINARY").expect("prebuilt application"),
    );
    let config = mailstronaut_config::process::evaluate(
        &binary,
        &fs::read_to_string(config).expect("synthetic source"),
        &CancellationToken::new(),
    )
    .await
    .expect("same native Nix schema");
    assert_eq!(config.accounts.len(), 2);
    let owner = Worker::open(data.join("mailstronaut"))
        .await
        .expect("profile owner");
    let store = owner.handle();
    let messages: Vec<GuiMessage> =
        serde_json::from_str(include_str!("fixtures/conversation-gui.json"))
            .expect("synthetic mail fixture");
    let mut receipt = serde_json::Map::new();
    for (index, account) in config.accounts.iter().enumerate() {
        assert!(
            account
                .incoming_endpoint()
                .expect("endpoint")
                .host
                .ends_with(".invalid")
        );
        assert!(
            account
                .address
                .parse::<mailstronaut_mail::compose::Address>()
                .expect("synthetic address")
                .domain()
                .eq_ignore_ascii_case("example.test")
        );
        store
            .ensure_account(account.id.clone())
            .await
            .expect("account");
        store
            .bind_mailbox_context(
                account.id.clone(),
                flags::context(account).expect("context"),
            )
            .await
            .expect("synthetic provenance");
        let inbox = store
            .ensure_folder(account.id.clone(), "INBOX".into(), true)
            .await
            .expect("inbox")
            .id;
        let sent = store
            .ensure_folder(account.id.clone(), "Sent".into(), false)
            .await
            .expect("sent")
            .id;
        for folder in [inbox, sent] {
            store
                .select_folder(account.id.clone(), folder, nonzero(42))
                .await
                .expect("generation");
        }
        for message in &messages {
            if message.account != index {
                continue;
            }
            let GuiMessage {
                key,
                mid,
                references,
                uid,
                sent: in_sent,
                seen,
                body,
                from,
                order: offset,
                ..
            } = message;
            let folder = if *in_sent { sent } else { inbox };
            let raw = format!(
                "From: {from}\r\nTo: alice@example.test, {}\r\nMessage-ID: <{mid}>\r\nReferences: {references}\r\nSubject: Shared conversation\r\n\r\n{body}\r\n",
                account.address
            );
            let id = store
                .cache_message(
                    account.id.clone(),
                    Membership {
                        folder,
                        uid_validity: nonzero(42),
                        uid: nonzero(*uid),
                        flags: Flags {
                            seen: *seen,
                            starred: false,
                        },
                    },
                    1_793_310_000 + *offset,
                    None,
                    raw.into_bytes(),
                )
                .await
                .expect("cache fixture");
            receipt.insert(key.clone(), serde_json::json!(id));
        }
    }
    owner.shutdown().await.expect("release synthetic profile");
    fs::write(
        data.parent()
            .expect("profile")
            .join("conversation-ids.json"),
        serde_json::to_vec(&receipt).expect("receipt"),
    )
    .expect("receipt file");
}
