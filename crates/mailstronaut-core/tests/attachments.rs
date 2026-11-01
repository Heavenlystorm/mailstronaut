//! Synthetic attachment snapshots, revision fencing and frozen MIME. No live mail.

use mailstronaut_core::store::{Error, Store, outbox::Draft};
use mailstronaut_mail::{
    compose::{AttachmentData, MAX_ATTACHMENT_BYTES},
    smtp::Outcome,
};

fn setup(root: &std::path::Path) -> (Store, Draft) {
    let mut store = Store::open(root).expect("store");
    store.ensure_account("a").expect("account");
    let mut draft = Draft::new("a".into(), "from@example.test".into());
    draft.content.to = "to@example.test".into();
    draft.content.bcc = "blind@example.test".into();
    draft.content.body = "Synthetic attached body".into();
    (store, draft)
}
fn file() -> AttachmentData {
    AttachmentData {
        name: "résumé-日本語.bin".into(),
        bytes: vec![0, 255, 1, 13, 10],
    }
}
fn assert_mime(bytes: &[u8]) {
    let parsed = mail_parser::MessageParser::default()
        .parse(bytes)
        .expect("MIME");
    assert!(
        parsed
            .body_text(0)
            .expect("text")
            .contains("Synthetic attached body")
    );
    assert_eq!(parsed.attachments().count(), 1);
    let attachment = parsed.attachments().next().expect("file");
    assert_eq!(attachment.decoded(), file().bytes);
    assert_eq!(attachment.attachment_name(), Some(file().name.as_str()));
    assert!(!String::from_utf8_lossy(bytes).contains("blind@example.test"));
}

#[test]
fn snapshots_survive_restart_gc_and_frozen_retry_without_original_source() {
    let root = tempfile::tempdir().expect("fixture");
    let (mut store, draft) = setup(root.path());
    let source = root.path().join("source");
    std::fs::write(&source, &file().bytes).expect("source");
    let attached = store
        .add_draft_attachments(
            &draft,
            vec![AttachmentData {
                name: file().name,
                bytes: std::fs::read(&source).expect("read"),
            }],
        )
        .expect("attach");
    std::fs::remove_file(source).expect("source gone");
    assert_eq!(attached.revision, draft.revision + 1);
    store.collect_orphan_blobs().expect("GC");
    assert!(
        std::fs::read_dir(root.path().join("blobs"))
            .expect("blobs")
            .next()
            .is_some()
    );
    drop(store);
    let mut store = Store::open(root.path()).expect("restart");
    let restored = store.draft("a", &draft.id).expect("restored");
    assert_eq!(restored.attachments, attached.attachments);
    let queued = store
        .queue_draft("a", &draft.id, restored.revision, false)
        .expect("freeze");
    let first = store.begin_submission("a", &queued.id).expect("claim");
    assert_mime(first.message.original());
    assert_eq!(first.message.envelope().to().len(), 2);
    store
        .finish_submission(&first, Outcome::Rejected { temporary: false })
        .expect("reject");
    store
        .retry_submission("a", &queued.id, false)
        .expect("retry");
    let second = store.begin_submission("a", &queued.id).expect("reclaim");
    assert_eq!(first.message.original(), second.message.original());
    assert_eq!(first.message.message_id(), second.message.message_id());
    store
        .finish_submission(&second, Outcome::Rejected { temporary: false })
        .expect("reject again");
    let recovered = store
        .recover_draft("a", &queued.id)
        .expect("recover editable");
    assert_ne!(recovered.id, draft.id);
    assert_eq!(recovered.attachments, attached.attachments);
    store.collect_orphan_blobs().expect("GC");
    assert!(
        std::fs::read_dir(root.path().join("blobs"))
            .expect("blobs")
            .next()
            .is_some()
    );
    let requeued = store
        .queue_draft("a", &recovered.id, recovered.revision, false)
        .expect("new intent");
    assert_mime(
        store
            .begin_submission("a", &requeued.id)
            .expect("new claim")
            .message
            .original(),
    );
}

#[test]
fn attachment_edits_reject_stale_and_foreign_drafts_and_copy_between_accounts() {
    let root = tempfile::tempdir().expect("fixture");
    let (mut store, draft) = setup(root.path());
    let attached = store
        .add_draft_attachments(&draft, vec![file()])
        .expect("attach");
    let id = attached.attachments[0].id.clone();
    let mut stale = draft.clone();
    stale.revision = attached.revision + 1;
    assert!(
        matches!(store.save_draft(&stale), Err(Error::OperationConflict)),
        "newer body cannot erase attachment revision"
    );
    assert!(store.remove_draft_attachment(&draft, &id).is_err());
    let mut foreign = attached.clone();
    foreign.account = "b".into();
    assert!(store.remove_draft_attachment(&foreign, &id).is_err());
    store.ensure_account("b").expect("second account");
    let copied = store
        .copy_draft(&attached, "b", "other@example.test".into())
        .expect("switch sender");
    assert_eq!(copied.attachments, attached.attachments);
    assert_eq!(copied.content.from, "other@example.test");
    store.collect_orphan_blobs().expect("GC");
    assert!(
        std::fs::read_dir(root.path().join("blobs"))
            .expect("blobs")
            .next()
            .is_some()
    );
    let removed = store
        .remove_draft_attachment(&attached, &id)
        .expect("remove from a");
    assert_eq!(removed.attachments, []);
    store.collect_orphan_blobs().expect("shared blob protected");
    assert_eq!(
        store
            .draft("b", &copied.id)
            .expect("independent copy")
            .attachments
            .len(),
        1
    );
    store
        .remove_draft_attachment(&copied, &id)
        .expect("remove final reference");
    store
        .collect_orphan_blobs()
        .expect("unreferenced snapshot collected");
    assert!(
        std::fs::read_dir(root.path().join("blobs"))
            .expect("blobs")
            .next()
            .is_none()
    );
}

#[test]
fn missing_or_corrupt_owned_file_keeps_draft_editable_and_does_not_queue_partial_mail() {
    for corrupt in [false, true] {
        let root = tempfile::tempdir().expect("fixture");
        let (mut store, draft) = setup(root.path());
        let attached = store
            .add_draft_attachments(&draft, vec![file()])
            .expect("attach");
        let blob = std::fs::read_dir(root.path().join("blobs"))
            .expect("blob dir")
            .next()
            .expect("blob")
            .expect("entry")
            .path();
        if corrupt {
            std::fs::write(blob, b"damaged").expect("damage");
        } else {
            std::fs::remove_file(blob).expect("remove");
        }
        assert!(
            store
                .queue_draft("a", &draft.id, attached.revision, false)
                .is_err()
        );
        assert_eq!(
            store
                .draft("a", &draft.id)
                .expect("still editable")
                .attachments
                .len(),
            1
        );
        assert!(store.outbox(&["a".into()]).expect("outbox").is_empty());
        let repaired = store
            .remove_draft_attachment(&attached, &attached.attachments[0].id)
            .expect("recover by removal");
        store
            .queue_draft("a", &draft.id, repaired.revision, false)
            .expect("explicitly removed missing file");
    }
}

#[test]
fn import_limits_are_atomic_and_closed_drafts_cannot_gain_files() {
    let root = tempfile::tempdir().expect("fixture");
    let (mut store, draft) = setup(root.path());
    assert!(
        store
            .add_draft_attachments(
                &draft,
                vec![
                    file(),
                    AttachmentData {
                        name: "../escape".into(),
                        bytes: vec![1]
                    }
                ]
            )
            .is_err()
    );
    assert!(
        store
            .add_draft_attachments(&draft, vec![file(); 17])
            .is_err()
    );
    assert!(
        store
            .add_draft_attachments(
                &draft,
                vec![AttachmentData {
                    name: "large".into(),
                    bytes: vec![0; MAX_ATTACHMENT_BYTES + 1]
                }]
            )
            .is_err()
    );
    let attached = store
        .add_draft_attachments(
            &draft,
            vec![AttachmentData {
                name: "max".into(),
                bytes: vec![0; MAX_ATTACHMENT_BYTES],
            }],
        )
        .expect("bounded maximum");
    assert!(
        store
            .add_draft_attachments(&attached, vec![file()])
            .is_err()
    );
    assert_eq!(
        store
            .draft("a", &draft.id)
            .expect("unchanged")
            .attachments
            .len(),
        1
    );
    store
        .queue_draft("a", &draft.id, attached.revision, false)
        .expect("encoded max fits");
    assert!(
        store
            .add_draft_attachments(&attached, vec![file()])
            .is_err()
    );
}

#[test]
fn upgrading_schema_six_preserves_editable_drafts_and_creates_backup() {
    let root = tempfile::tempdir().expect("fixture");
    let (mut store, draft) = setup(root.path());
    store.save_draft(&draft).expect("existing draft");
    drop(store);
    let connection =
        rusqlite::Connection::open(root.path().join("mail.sqlite3")).expect("database");
    connection
        .execute_batch(
            "DROP TRIGGER search_message_removed;
             DROP TABLE search_pending;
             DROP TABLE draft_attachments;
             DROP VIEW effective_memberships;
             DROP TABLE pending_moves;
             CREATE VIEW effective_memberships AS SELECT 1 AS placeholder;
             DROP INDEX one_folder_per_role;
             ALTER TABLE folders DROP COLUMN role;
             PRAGMA user_version=6;",
        )
        .expect("schema six fixture");
    drop(connection);
    let upgraded = Store::open(root.path()).expect("upgrade");
    let restored = upgraded.draft("a", &draft.id).expect("retained draft");
    assert_eq!(restored.content.body, draft.content.body);
    assert_eq!(restored.revision, draft.revision);
    assert_eq!(restored.attachments, []);
    assert!(
        std::fs::read_dir(root.path().join("backups"))
            .expect("backup")
            .any(|entry| entry
                .expect("backup file")
                .file_name()
                .to_string_lossy()
                .starts_with("schema-6-"))
    );
}
