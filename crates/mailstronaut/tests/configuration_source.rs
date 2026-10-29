//! Real bounded-helper startup/reload and managed-source behavior.

use std::{fs, os::unix::fs::symlink, path::Path};

use mailstronaut_config::{
    MAX_SOURCE_BYTES,
    process::EvaluationError,
    source::{self, Error, Runtime},
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn last_valid_reload_survives_invalid_and_removed_managed_source() {
    let root = tempfile::tempdir().expect("fixture root");
    let original = root.path().join("managed.nix");
    let link = root.path().join("configuration.nix");
    let expression =
        "let interval = 123; in { schemaVersion = 1; sync.pollSeconds = interval; accounts = []; }";
    fs::write(&original, expression).expect("fixture");
    symlink(&original, &link).expect("managed link");
    let executable = Path::new(env!("CARGO_BIN_EXE_mailstronaut"));
    let cancellation = CancellationToken::new();
    let mut state = Runtime::default();
    assert!(state.apply(source::load(link.clone(), executable, &cancellation).await));
    assert_eq!(
        state
            .active()
            .expect("active declaration")
            .sync
            .poll_seconds,
        123
    );
    assert_eq!(state.revision(), 1);
    assert_eq!(
        fs::read_to_string(&original).expect("unchanged source"),
        expression
    );
    assert!(link.is_symlink());
    fs::write(
        &original,
        "{ schemaVersion = 999; password = \"SYNTHETIC-SHOULD-NOT-LEAK\"; }",
    )
    .expect("bad replacement");
    assert!(!state.apply(source::load(link.clone(), executable, &cancellation).await));
    assert_eq!(state.revision(), 1);
    assert_eq!(
        state
            .active()
            .expect("retained declaration")
            .sync
            .poll_seconds,
        123
    );
    let error = state.error().expect("diagnostic").to_string();
    assert!(!error.contains("SYNTHETIC-SHOULD-NOT-LEAK"));
    fs::remove_file(&original).expect("break managed link");
    assert!(!state.apply(source::load(link.clone(), executable, &cancellation).await));
    assert_eq!(state.error(), Some(Error::Read));
    fs::write(&original, "{ schemaVersion = 1; sync.pollSeconds = 456; }").expect("repair source");
    assert!(state.apply(source::load(link, executable, &cancellation).await));
    assert_eq!(
        state.active().expect("new declaration").sync.poll_seconds,
        456
    );
    assert_eq!(state.revision(), 2);
    assert_eq!(state.error(), None);
}

#[tokio::test]
async fn missing_startup_oversized_nonfile_and_cancellation_are_distinct() {
    let root = tempfile::tempdir().expect("fixture root");
    let path = root.path().join("source.nix");
    let executable = Path::new(env!("CARGO_BIN_EXE_mailstronaut"));
    let cancellation = CancellationToken::new();
    assert!(
        source::load(path.clone(), executable, &cancellation)
            .await
            .expect("new setup")
            .is_none()
    );
    assert_eq!(
        source::read(root.path()).expect_err("not a regular file"),
        Error::Read
    );
    fs::write(&path, vec![b' '; MAX_SOURCE_BYTES + 1]).expect("oversized fixture");
    assert_eq!(
        source::read(&path).expect_err("bounded read"),
        Error::TooLarge
    );
    fs::write(&path, [0xff]).expect("invalid UTF-8");
    assert_eq!(
        source::read(&path).expect_err("invalid encoding"),
        Error::Read
    );
    fs::write(&path, "{ schemaVersion = 1; }").expect("valid source");
    cancellation.cancel();
    assert!(matches!(
        source::load(path, executable, &cancellation).await,
        Err(Error::Evaluation(EvaluationError::Cancelled))
    ));
}
