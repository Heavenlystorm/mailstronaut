//! Mailbox queries at the scale the plan targets, over synthetic rows inserted directly:
//! parsing and fsyncing 100,000 messages one by one would measure something else.

use core::num::NonZeroU32;
use std::{path::Path, time::Instant};

use rusqlite::params;

use super::{Store, browse::Scope, mailbox::Cursor};
use crate::threads;

/// Messages per conversation, so that grouping has real work to do.
const THREAD: i64 = 3;

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("nonzero")
}

/// Insert messages into one account; returns its inbox and archive folder IDs.
pub(super) fn seed(root: &Path, account: &str, first: i64, count: i64) -> (i64, i64) {
    let mut store = Store::open(root).expect("open");
    store.ensure_account(account).expect("account");
    let inbox = store
        .ensure_folder(account, "INBOX", true)
        .expect("inbox")
        .id;
    let archive = store
        .ensure_folder(account, "Archive", false)
        .expect("archive")
        .id;
    for folder in [inbox, archive] {
        store
            .select_folder(account, folder, nonzero(42))
            .expect("generation");
    }
    let tx = store.connection.transaction().expect("transaction");
    {
        let mut message = tx.prepare("INSERT INTO messages(id,account_id,message_id,subject,preview,blob_hash,received_at,sender,read_cleared,thread_references,thread_parent,thread_routing_valid) VALUES (?1,?2,?3,?4,?5,'synthetic',?6,?7,?8,?9,?10,1)").expect("statement");
        let mut membership = tx.prepare("INSERT INTO memberships(account_id,folder_id,uidvalidity,uid,message_id,seen,starred) VALUES (?1,?2,42,?3,?4,?5,0)").expect("statement");
        let mut text = tx
            .prepare(
                "INSERT INTO message_search(rowid,subject,addresses,body) VALUES (?1,?2,?3,?4)",
            )
            .expect("statement");
        for id in first..first + count {
            let root_id = id - (id - first) % THREAD;
            let is_root = id == root_id;
            let references = if is_root {
                "[]".to_owned()
            } else {
                format!("[\"{root_id}@example.test\"]")
            };
            let parent = (!is_root).then(|| format!("{root_id}@example.test"));
            // One in ten is unread, as in a mailbox that is kept up with.
            let seen = id % 10 != 0;
            let subject = format!("Synthetic subject number {root_id}");
            message
                .execute(params![
                    id,
                    account,
                    format!("{id}@example.test"),
                    subject,
                    "A short synthetic preview of the message.",
                    id,
                    format!("Person {}", id % 500),
                    seen,
                    references,
                    parent,
                ])
                .expect("message");
            // Most mail is in the inbox, and the rest has been archived.
            let folder = if id % 5 == 0 { archive } else { inbox };
            membership
                .execute(params![account, folder, id, id, seen])
                .expect("membership");
            text.execute(params![
                id,
                subject,
                format!("person{}@example.test reader@example.test", id % 500),
                format!(
                    "Synthetic body of message {id}, about budget number {}.",
                    id % 100
                ),
            ])
            .expect("search text");
        }
    }
    tx.commit().expect("commit");
    (inbox, archive)
}

fn median_ms(mut measure: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..5)
        .map(|_| {
            let began = Instant::now();
            measure();
            began.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[2]
}

#[test]
fn an_account_too_large_to_group_still_lists_and_searches_its_mail() {
    let root = tempfile::tempdir().expect("fixture");
    let total = i64::try_from(threads::MAX_MESSAGES).expect("limit") + 1;
    let (inbox, _) = seed(root.path(), "a", 1, total);
    let store = Store::open(root.path()).expect("reopen");
    assert!(
        store
            .conversation_snapshot("a")
            .expect("not an error")
            .is_none(),
        "there is nothing to project, and that must not fail the list"
    );
    let accounts = ["a".to_owned()];
    let rows = store
        .conversation_page(&accounts, &Scope::Inbox, "", None, 100)
        .expect("page");
    assert_eq!(rows.len(), 100);
    assert_eq!(rows[0].latest.id, total - i64::from(total % 5 == 0));
    let scoped = Scope::Folder {
        account: "a".into(),
        folder: inbox,
    };
    assert_eq!(
        store
            .conversation_page(&accounts, &scoped, "budget", None, 100)
            .expect("search")
            .len(),
        100
    );
}

fn project_with_timing(store: &mut Store, account: &str) {
    let began = Instant::now();
    let snapshot = store
        .conversation_snapshot(account)
        .expect("snapshot")
        .expect("stale");
    let read = began.elapsed();
    let began = Instant::now();
    let groups = threads::project(&snapshot.messages).expect("project");
    let projected = began.elapsed();
    let began = Instant::now();
    store
        .commit_conversations(&snapshot, &groups)
        .expect("commit");
    println!(
        "account {account}: snapshot {read:?}, grouping {projected:?}, commit {:?}",
        began.elapsed()
    );
}

/// Times for the record. Not part of the normal run, and only meaningful in a release build:
/// `cargo test --release -p mailstronaut-core --lib scale -- --ignored --nocapture`
#[test]
#[ignore = "explicit release-mode measurement"]
fn scale_measurement() {
    let root = tempfile::tempdir().expect("fixture");
    let each = i64::try_from(threads::MAX_MESSAGES).expect("limit");
    let began = Instant::now();
    let (inbox, _) = seed(root.path(), "a", 1, each);
    seed(root.path(), "b", each + 1, each);
    println!("seeded {} messages in {:?}", each * 2, began.elapsed());
    let mut store = Store::open(root.path()).expect("reopen");
    let accounts = ["a".to_owned(), "b".to_owned()];
    project_with_timing(&mut store, "a");
    project_with_timing(&mut store, "b");
    let hits: Vec<i64> = (1..=1000).map(|index| index * 90).collect();
    let deep = Cursor {
        received_at: each,
        message_id: each,
    };
    let folder = Scope::Folder {
        account: "a".into(),
        folder: inbox,
    };
    let report: [(&str, f64); 9] = [
        (
            "first page, unified inbox",
            median_ms(|| {
                store
                    .conversation_page(&accounts, &Scope::Inbox, "", None, 100)
                    .expect("page");
            }),
        ),
        (
            "deep page, unified inbox",
            median_ms(|| {
                store
                    .conversation_page(&accounts, &Scope::Inbox, "", Some(deep), 100)
                    .expect("page");
            }),
        ),
        (
            "first page, one folder",
            median_ms(|| {
                store
                    .conversation_page(&accounts, &folder, "", None, 100)
                    .expect("page");
            }),
        ),
        (
            "literal search",
            median_ms(|| {
                store
                    .conversation_page(&accounts, &Scope::Inbox, "budget", None, 100)
                    .expect("page");
            }),
        ),
        (
            "ranked page of 1000 hits",
            median_ms(|| {
                store
                    .ranked_conversation_page(&accounts, &Scope::Inbox, &hits, None, 100)
                    .expect("page");
            }),
        ),
        (
            "unread counts",
            median_ms(|| {
                store.unread_counts(&accounts).expect("counts");
            }),
        ),
        (
            "folders",
            median_ms(|| {
                store.folders(&accounts).expect("folders");
            }),
        ),
        (
            "priority group",
            median_ms(|| {
                store.priority_items(&accounts, 0, 20).expect("items");
            }),
        ),
        (
            "analysis queue",
            median_ms(|| {
                store
                    .analysis_pending(&accounts, 1, 0, 64)
                    .expect("pending");
            }),
        ),
    ];
    for (name, milliseconds) in report {
        println!("{name}: {milliseconds:.1} ms");
    }
}
