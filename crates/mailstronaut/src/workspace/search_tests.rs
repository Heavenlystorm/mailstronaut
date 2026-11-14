//! Regression coverage for ranked list refresh across word-match tiers.
use super::conversation_prefix;
use crate::mail_search::MailSearch;
use core::num::NonZeroU32;
use mailstronaut_core::{
    store::{
        browse::{Scope, SearchFilter},
        conversations::Summary,
        mailbox::{Cursor, Flags, Membership},
    },
    worker::{Handle, Worker},
};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

async fn cache(store: &Handle, folder: i64, uid: u32, subject: &str) -> i64 {
    store
        .cache_message(
            "a".into(),
            Membership {
                folder,
                uid_validity: NonZeroU32::new(42).expect("namespace"),
                uid: NonZeroU32::new(uid).expect("UID"),
                flags: Flags::default(),
            },
            i64::from(uid),
            None,
            format!(
                "Subject: {subject}\r\nMessage-ID: <rank-{uid}@example.test>\r\n\r\nSynthetic correspondence."
            )
            .into_bytes(),
        )
        .await
        .expect("cache")
}

async fn await_index(index: &MailSearch, expected: usize) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if index
                .filtered_hits(
                    "budget".into(),
                    vec!["a".into()],
                    mailstronaut_search::Filter::default(),
                )
                .await
                .is_some_and(|hits| hits.len() == expected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("all synthetic mail indexed");
}

async fn prefix(
    store: &Handle,
    index: &MailSearch,
    boundary: Option<Cursor>,
    cancel: &CancellationToken,
) -> Vec<Summary> {
    conversation_prefix(
        store,
        Some(index),
        None,
        vec!["a".into()],
        Scope::Inbox,
        "budget".into(),
        boundary,
        cancel,
        SearchFilter::default(),
    )
    .await
    .expect("ranked refresh")
}

#[test]
fn ranked_refresh_retains_the_loaded_boundary_across_nonchronological_tiers() {
    let runtime = Runtime::new().expect("runtime");
    runtime.block_on(async {
        let root = tempfile::tempdir().expect("private profile");
        let owner = Worker::open(root.path().to_owned()).await.expect("worker");
        let store = owner.handle();
        store.ensure_account("a".into()).await.expect("account");
        let folder = store
            .ensure_folder("a".into(), "INBOX".into(), true)
            .await
            .expect("folder")
            .id;
        store
            .select_folder("a".into(), folder, NonZeroU32::new(42).expect("namespace"))
            .await
            .expect("select");
        let mut boundary = None;
        for uid in 1..=240 {
            let subject = if uid <= 120 { "Budget" } else { "Budgetary" };
            let id = cache(&store, folder, uid, subject).await;
            if uid == 161 {
                boundary = Some(Cursor {
                    received_at: i64::from(uid),
                    message_id: id,
                });
            }
        }
        let cancel = CancellationToken::new();
        let index = MailSearch::start(
            &runtime,
            root.path().join("search"),
            store.clone(),
            cancel.child_token(),
        );
        await_index(&index, 240).await;
        let initial = prefix(&store, &index, None, &cancel).await;
        assert_eq!(initial.len(), 100);
        assert_eq!(
            initial.last().expect("last exact hit").latest.received_at,
            21
        );
        let boundary = boundary.expect("second loaded batch boundary");
        let refreshed = prefix(&store, &index, Some(boundary), &cancel).await;
        assert_eq!(refreshed.len(), 200);
        assert_eq!(
            refreshed.last().expect("boundary hit").latest.id,
            boundary.message_id
        );
        // Exact hits precede newer prefix hits: their dates do not encode rank.
        assert!(refreshed[119].latest.received_at < refreshed[120].latest.received_at);
        cache(&store, folder, 241, "Budget").await;
        index.wake();
        await_index(&index, 241).await;
        let refreshed = prefix(&store, &index, Some(boundary), &cancel).await;
        assert_eq!(refreshed.len(), 241);
        assert!(
            refreshed
                .iter()
                .any(|row| row.latest.id == boundary.message_id)
        );
        assert!(refreshed.iter().all(|row| !row.latest.seen));
        cancel.cancel();
        owner.shutdown().await.expect("shutdown");
    });
}
