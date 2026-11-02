//! Extra synthetic originals for the private long-thread Wayland walkthrough.
use mailstronaut_core::{
    store::mailbox::{Flags, Membership},
    worker::Worker,
};
use std::{fs, num::NonZeroU32, path::PathBuf};

#[tokio::test]
#[ignore = "private synthetic Wayland fixture only"]
async fn seed_long_thread() {
    let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").expect("private data"))
        .canonicalize()
        .expect("data");
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    assert!(data.starts_with(target));
    let owner = Worker::open(data.join("mailstronaut"))
        .await
        .expect("worker");
    let store = owner.handle();
    let account = "conversation-synthetic".to_owned();
    let folder = store
        .ensure_folder(account.clone(), "INBOX".into(), true)
        .await
        .expect("folder")
        .id;
    let validity = NonZeroU32::new(42).expect("validity");
    let mut ids = Vec::new();
    for number in 1..=105 {
        let raw = format!(
            "From: Synthetic <sender@example.test>\r\nTo: reader@example.test\r\nMessage-ID: <long-{number}@example.test>\r\nReferences: <parent@example.test>\r\nSubject: Shared conversation\r\n\r\nSynthetic thread original {number:03}.\r\n"
        );
        ids.push(
            store
                .cache_message(
                    account.clone(),
                    Membership {
                        folder,
                        uid_validity: validity,
                        uid: NonZeroU32::new(1000 + number).expect("uid"),
                        flags: Flags::default(),
                    },
                    1_793_310_000 + i64::from(number),
                    None,
                    raw.into_bytes(),
                )
                .await
                .expect("seed original"),
        );
    }
    owner.shutdown().await.expect("shutdown");
    fs::write(
        data.parent().expect("profile").join("long-thread-ids.json"),
        serde_json::to_vec(&ids).expect("ids"),
    )
    .expect("receipt");
}
