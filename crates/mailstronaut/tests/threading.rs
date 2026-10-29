//! Real same-executable conversation helper; all metadata is synthetic.

use std::{path::Path, process::Stdio};

use mailstronaut_core::threads::{self, Error, Message, process};
use tokio::{io::AsyncWriteExt as _, process::Command};
use tokio_util::sync::CancellationToken;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_mailstronaut");
fn message(id: i64, mid: &str, references: &[&str]) -> Message {
    Message {
        id,
        message_id: Some(mid.into()),
        parent: None,
        references: references.iter().map(|value| (*value).into()).collect(),
        received_at: id,
    }
}
#[tokio::test]
async fn actual_helper_groups_metadata_without_a_desktop_and_rejects_oversized_input() {
    let messages = [
        message(1, "a@example.test", &["missing@example.test"]),
        message(2, "b@example.test", &["missing@example.test"]),
    ];
    let mut child = Command::new(EXECUTABLE)
        .arg("--internal-project-threads")
        .env("PATH", "")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("XDG_RUNTIME_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("actual helper");
    let mut input = child.stdin.take().expect("stdin");
    input
        .write_all(&serde_json::to_vec(&messages).expect("metadata"))
        .await
        .expect("request");
    drop(input);
    let output = tokio::time::timeout(process::TIMEOUT, child.wait_with_output())
        .await
        .expect("independent watchdog")
        .expect("output");
    assert!(output.status.success());
    let groups: threads::Groups = serde_json::from_slice(&output.stdout).expect("typed groups");
    groups.validate(&messages).expect("exact coverage");
    assert_eq!(groups.groups, vec![vec![1, 2]]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("example.test"));
    let cancel = CancellationToken::new();
    let groups = process::project(Path::new(EXECUTABLE), &messages, &cancel)
        .await
        .expect("production parent");
    groups.validate(&messages).expect("coverage");
    cancel.cancel();
    assert!(matches!(
        process::project(Path::new(EXECUTABLE), &messages, &cancel).await,
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        process::project(Path::new("relative"), &messages, &CancellationToken::new()).await,
        Err(Error::Invalid)
    ));
    assert!(matches!(
        process::project(
            Path::new(EXECUTABLE),
            &vec![messages[0].clone(); threads::MAX_MESSAGES + 1],
            &CancellationToken::new()
        )
        .await,
        Err(Error::TooLarge)
    ));
}

#[tokio::test]
async fn pathological_ancestry_cannot_kill_the_parent_and_a_fresh_helper_still_works() {
    let messages = (1..=20_000)
        .map(|id| Message {
            id,
            message_id: Some(format!("{id}@deep.test")),
            parent: (id > 1).then(|| format!("{}@deep.test", id - 1)),
            references: Vec::new(),
            received_at: id,
        })
        .collect::<Vec<_>>();
    let result =
        process::project(Path::new(EXECUTABLE), &messages, &CancellationToken::new()).await;
    // Hardware/compiler speed can finish a valid chain; otherwise resource/stack
    // enforcement must contain the failure to this child. Parent remains alive.
    if let Ok(groups) = result {
        groups.validate(&messages).expect("complete result");
    }
    let small = [message(1, "after@example.test", &[])];
    process::project(Path::new(EXECUTABLE), &small, &CancellationToken::new())
        .await
        .expect("fresh helper after hostile work")
        .validate(&small)
        .expect("coverage");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_kills_and_reaps_an_observed_live_grouping_child() {
    let messages = (1..=50_000)
        .map(|id| Message {
            id,
            message_id: Some(format!("{id}@cancel.test")),
            parent: (id > 1).then(|| format!("{}@cancel.test", id - 1)),
            references: Vec::new(),
            received_at: id,
        })
        .collect::<Vec<_>>();
    let cancel = CancellationToken::new();
    let child_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        process::project(Path::new(EXECUTABLE), &messages, &child_cancel).await
    });
    let child = tokio::time::timeout(core::time::Duration::from_secs(2), async {
        loop {
            if let Some(path) = limited_thread_child() {
                break path;
            }
            tokio::time::sleep(core::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a live limited helper must be observed before cancellation");
    cancel.cancel();
    let result = tokio::time::timeout(core::time::Duration::from_secs(2), task)
        .await
        .expect("prompt cancellation")
        .expect("parent task");
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(!child.exists(), "the observed child must have been reaped");
}

#[cfg(target_os = "linux")]
fn limited_thread_child() -> Option<std::path::PathBuf> {
    // Tokio's default test runtime stays on this test's thread. Inspect only its
    // children, so concurrent integration tests cannot satisfy this observation.
    let children = std::fs::read_to_string("/proc/thread-self/children").ok()?;
    for pid in children.split_whitespace() {
        let path = Path::new("/proc").join(pid);
        if std::fs::read(path.join("cmdline")).is_ok_and(|command| {
            command
                .split(|&byte| byte == 0)
                .any(|arg| arg == b"--internal-project-threads")
        }) && std::fs::read_to_string(path.join("limits")).is_ok_and(|limits| {
            limits.lines().any(|line| {
                line.starts_with("Max cpu time")
                    && line.split_whitespace().skip(3).take(2).eq(["5", "6"])
            })
        }) {
            return Some(path);
        }
    }
    None
}
