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
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    mime: Option<String>,
    from: String,
    order: i64,
}

const GUI_RECEIVED_AT: i64 = 1_793_310_000;

/// One long HTML original followed by 100 short ones. Each HTML original starts with a block
/// of its own colour, so a native capture identifies exactly which original is painted.
fn performance_messages() -> Vec<GuiMessage> {
    use std::fmt::Write as _;
    (0..=100_u32)
        .map(|index| {
            let (key, paragraphs, mut text) = if index == 0 {
                (
                    "perf-long".to_owned(),
                    1_500,
                    "Synthetic long performance original.".to_owned(),
                )
            } else {
                (
                    format!("perf-{index:03}"),
                    3,
                    format!("Synthetic performance original {index:03}."),
                )
            };
            if index == 1 {
                text.push_str("\r\nRéinscription au doctorat.\r\nScadenza di domani.");
            }
            let red = 16 + (index % 16) * 12;
            let green = 16 + ((index / 16) % 16) * 12;
            let blue = 16 + (index / 256) * 100;
            let mut html =
                format!("<div style=\"height:120px;background:rgb({red},{green},{blue})\"></div>");
            for paragraph in 0..paragraphs {
                let _ = write!(
                    html,
                    "<p>Synthetic paragraph {paragraph:04} with ordinary words for layout measurement.</p>"
                );
            }
            GuiMessage {
                account: 0,
                key,
                mid: format!("perf-{index}@example.test"),
                references: String::new(),
                uid: 300 + index,
                sent: false,
                seen: false,
                body: text.clone(),
                subject: Some(format!("Performance {index:03}")),
                mime: Some(format!(
                    "MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=perf\r\n\r\n--perf\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n--perf\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--perf--\r\n"
                )),
                from: "Synthetic <perf@example.test>".into(),
                order: -i64::from(index),
            }
        })
        .collect()
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
    let html = std::env::var("MAILSTRONAUT_HTML_GUI").is_ok_and(|value| value == "1");
    let mut messages: Vec<GuiMessage> = serde_json::from_str(if html {
        include_str!("fixtures/html-gui.json")
    } else {
        include_str!("fixtures/conversation-gui.json")
    })
    .expect("synthetic mail fixture");
    if std::env::var("MAILSTRONAUT_MAILBOX_NAVIGATION").is_ok_and(|value| value == "1") {
        assert!(!html, "separate synthetic navigation fixture");
        messages.extend((1..=105).map(|number| GuiMessage {
            account: 0,
            key: format!("page-{number:03}"),
            mid: format!("page-{number}@example.test"),
            references: String::new(),
            uid: 100 + number,
            sent: false,
            seen: false,
            body: format!("Synthetic cached letter {number:03}."),
            subject: Some(format!("Cached letter {number:03}")),
            mime: None,
            from: "Correspondent <writer@example.test>".into(),
            order: -i64::from(number),
        }));
    }
    if std::env::var("MAILSTRONAUT_PERFORMANCE_MAILBOX").is_ok_and(|value| value == "1") {
        assert!(!html, "separate synthetic performance fixture");
        messages = performance_messages();
        if std::env::var_os("MAILSTRONAUT_ACCESSIBLE_ROWS").is_some() {
            let first = &mut messages[0];
            first.from = "Zoë <zoe@example.test>".into();
            first.subject = Some("Performance 000 · Réinscription et suivi de la demande de remboursement avant le prochain conseil".into());
            first.body = "Could you send me your expense report by tomorrow?".into();
            first.mime = None;
            first.order = chrono::Utc::now().timestamp() - GUI_RECEIVED_AT;
        }
    }
    if html {
        let port: u16 = std::env::var("MAILSTRONAUT_HTML_HTTP_PORT")
            .expect("loopback observer port")
            .parse()
            .expect("port");
        assert_ne!(port, 0);
        let file = data.parent().expect("private root").join("blocked.png");
        assert!(file.is_file());
        for message in &mut messages {
            if let Some(mime) = &mut message.mime {
                *mime = mime
                    .replace("__HTTP_ENDPOINT__", &format!("http://127.0.0.1:{port}"))
                    .replace("__FILE_ENDPOINT__", &format!("file://{}", file.display()));
            }
        }
    }
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
            let subject = message.subject.as_deref().unwrap_or("Shared conversation");
            let content = message
                .mime
                .clone()
                .unwrap_or_else(|| format!("\r\n{body}\r\n"));
            let raw = format!(
                "From: {from}\r\nTo: alice@example.test, {}\r\nMessage-ID: <{mid}>\r\nReferences: {references}\r\nSubject: {subject}\r\n{content}",
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
                    GUI_RECEIVED_AT + *offset,
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
