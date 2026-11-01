//! Native activation/export and cooperative writer regressions, in private profiles.

use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
    process::{Command, Output, Stdio},
    time::Duration,
};

use mailstronaut_config::{Config, native, process, source};
use tokio_util::sync::CancellationToken;
use wait_timeout::ChildExt as _;

const BINARY: &str = env!("CARGO_BIN_EXE_mailstronaut");

fn command(args: &[&OsStr]) -> Output {
    let mut child = Command::new(BINARY)
        .args(args)
        .env("PATH", "")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("CLI child");
    if child
        .wait_timeout(Duration::from_secs(10))
        .expect("watchdog")
        .is_none()
    {
        child.kill().expect("kill timed-out child");
        child.wait().expect("reap child");
        panic!("configuration operation exceeded its outer watchdog");
    }
    child.wait_with_output().expect("CLI output")
}

fn reconcile(declared: &Path, path: &Path) -> Output {
    command(&[
        "config".as_ref(),
        "reconcile".as_ref(),
        "--declared".as_ref(),
        declared.as_os_str(),
        "--path".as_ref(),
        path.as_os_str(),
    ])
}

async fn export(path: &Path) -> Config {
    let result = command(&[
        "config".as_ref(),
        "export".as_ref(),
        "--path".as_ref(),
        path.as_os_str(),
    ]);
    assert!(
        result.status.success(),
        "export: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).expect("native UTF-8 output");
    process::evaluate(Path::new(BINARY), &text, &CancellationToken::new())
        .await
        .expect("export is valid native configuration")
}

#[tokio::test]
async fn activation_preserves_omissions_replaces_lists_and_round_trips_native_data() {
    let root = tempfile::tempdir().expect("private HOME");
    let path = root.path().join("config with spaces/config.nix");
    let declared = root.path().join("declared.nix");
    fs::write(&declared, "let interval = 120; in { schemaVersion = 1; appearance = \"dark\"; sync.pollSeconds = interval; accounts = []; } # trailing comment").expect("declaration");
    assert!(reconcile(&declared, &path).status.success());
    assert!(
        fs::read_to_string(&path)
            .expect("source")
            .starts_with(native::MUTABLE_HEADER)
    );
    assert_eq!(
        fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().expect("parent"))
            .expect("directory")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let first = export(&path).await;
    assert!(!first.managed);
    assert_eq!(first.sync.poll_seconds, 120);
    let changed = format!(
        "{}{{ schemaVersion = 1; appearance = \"light\"; sync = {{ pollSeconds = 90; initialDays = 7; }}; notifications.showPreview = true; }}",
        native::MUTABLE_HEADER
    );
    fs::write(&path, changed).expect("synthetic UI edit");
    assert!(reconcile(&declared, &path).status.success());
    let after = export(&path).await;
    assert!(matches!(
        after.appearance,
        mailstronaut_config::Appearance::Dark
    ));
    assert_eq!(after.sync.poll_seconds, 120);
    assert_eq!(after.sync.initial_days, 7);
    assert!(after.notifications.show_preview);
    assert!(after.accounts.is_empty());
    let before = fs::read(&path).expect("valid original");
    for invalid in [
        "[]",
        "null",
        "true",
        "{ schemaVersion = 99; }",
        "{ password = \"do-not-echo\"; }",
    ] {
        fs::write(&declared, invalid).expect("bad declaration");
        let result = reconcile(&declared, &path);
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("do-not-echo"));
        assert_eq!(fs::read(&path).expect("preserved"), before);
    }
}

#[test]
fn user_authored_readonly_and_unexpected_links_are_never_silently_adopted() {
    let root = tempfile::tempdir().expect("private profile");
    let declaration = root.path().join("declared.nix");
    fs::write(
        &declaration,
        "{ schemaVersion = 1; appearance = \"dark\"; }",
    )
    .expect("declaration");
    let authored = root.path().join("user-authored.nix");
    fs::write(&authored, "let v = 1; in { schemaVersion = v; }").expect("authored expression");
    let before = fs::read(&authored).expect("source");
    assert!(!reconcile(&declaration, &authored).status.success());
    assert_eq!(fs::read(&authored).expect("preserved"), before);
    let lookalike = root.path().join("home-manager-files-personal");
    fs::create_dir(&lookalike).expect("lookalike directory");
    let link = lookalike.join("config.nix");
    symlink(&authored, &link).expect("personal dotfile link");
    assert!(!reconcile(&declaration, &link).status.success());
    assert!(link.is_symlink());
    assert_eq!(fs::read(&authored).expect("preserved"), before);
    let readonly = root.path().join("readonly.nix");
    fs::write(
        &readonly,
        format!("{}{{ schemaVersion = 1; }}", native::MUTABLE_HEADER),
    )
    .expect("owned source");
    fs::set_permissions(&readonly, fs::Permissions::from_mode(0o400)).expect("read-only");
    let before = fs::read(&readonly).expect("source");
    assert!(!reconcile(&declaration, &readonly).status.success());
    assert_eq!(fs::read(&readonly).expect("preserved"), before);
}

#[test]
fn writers_exclude_each_other_and_reject_in_place_and_atomic_external_edits() {
    let root = tempfile::tempdir().expect("profile");
    let declared = root.path().join("declared.nix");
    let path = root.path().join("config.nix");
    fs::write(&declared, "{ schemaVersion = 1; }").expect("declaration");
    assert!(reconcile(&declared, &path).status.success());
    let edit = source::Edit::begin(&path).expect("exclusive writer");
    assert!(matches!(
        source::Edit::begin(&path),
        Err(source::Error::Conflict)
    ));
    assert!(
        !reconcile(&declared, &path).status.success(),
        "separate process cannot overwrite a held writer"
    );
    let changed = format!(
        "{}{{ schemaVersion = 1; appearance = \"dark\"; }}",
        native::MUTABLE_HEADER
    );
    fs::write(&path, &changed).expect("external editor");
    assert!(matches!(edit.verify(), Err(source::Error::Conflict)));
    drop(edit);
    let edit = source::Edit::begin(&path).expect("lock released by drop");
    let replacement = root.path().join("replacement.nix");
    fs::write(&replacement, &changed).expect("same bytes, new identity");
    fs::rename(replacement, &path).expect("external atomic save");
    assert!(matches!(edit.verify(), Err(source::Error::Conflict)));
    drop(edit);
    assert!(reconcile(&declared, &path).status.success());
    let linked = root.path().join("linked.nix");
    symlink(&path, root.path().join(".linked.nix.lock")).expect("untrusted lock link");
    assert!(source::Edit::begin(&linked).is_err());
}

#[tokio::test]
async fn raw_helper_preserves_exact_strings_and_rejects_impure_and_recursive_sources() {
    let cancel = CancellationToken::new();
    let result = process::evaluate_raw(Path::new(BINARY), r#"{ "with space" = "\${builtins.readFile /etc/passwd}"; data = [ "Réinscription" true null (-12) ]; } # trailing comment"#, &cancel).await.expect("limited raw helper");
    assert_eq!(
        result,
        serde_json::json!({"with space": "${builtins.readFile /etc/passwd}", "data": ["Réinscription", true, null, -12]})
    );
    for source in ["builtins.readFile /etc/passwd", "let f = x: f x; in f 0"] {
        assert!(
            process::evaluate_raw(Path::new(BINARY), source, &cancel)
                .await
                .is_err()
        );
    }
    cancel.cancel();
    assert!(matches!(
        process::evaluate_raw(Path::new(BINARY), "{}", &cancel).await,
        Err(process::EvaluationError::Cancelled)
    ));
}
