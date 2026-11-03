//! Synthetic private-bus acceptance; never contacts the user's notification server.
use super::*;
use ashpd::zvariant::OwnedValue;
use mailstronaut_core::{
    store::Store,
    worker::{self, Worker},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
struct Calls {
    adds: Mutex<Vec<(String, String)>>,
    removes: Mutex<Vec<String>>,
    deny: AtomicBool,
}
struct Portal(Arc<Calls>);
#[zbus::interface(name = "org.freedesktop.portal.Notification")]
impl Portal {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "D-Bus property getters use an instance receiver."
    )]
    fn version(&self) -> u32 {
        1
    }
    #[zbus(name = "AddNotification")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus deserializes an owned notification dictionary for this interface method."
    )]
    fn add(&self, id: &str, notification: HashMap<String, OwnedValue>) -> zbus::fdo::Result<()> {
        let body = notification
            .get("body")
            .and_then(|value| <&str>::try_from(value).ok())
            .unwrap_or("");
        self.0
            .adds
            .lock()
            .expect("receipts")
            .push((id.into(), body.into()));
        assert_eq!(
            notification
                .get("default-action")
                .and_then(|value| <&str>::try_from(value).ok()),
            Some("open")
        );
        assert!(
            notification
                .get("default-action-target")
                .is_some_and(|value| value.downcast_ref::<i64>().is_ok())
        );
        if self.0.deny.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::AccessDenied("Synthetic denial".into()));
        }
        Ok(())
    }
    #[zbus(name = "RemoveNotification")]
    fn remove(&self, id: &str) {
        self.0.removes.lock().expect("receipts").push(id.into());
    }
}
fn config() -> Config {
    let config:Config=serde_json::from_value(serde_json::json!({"schemaVersion":1,"timeZone":"Europe/Paris","accounts":[{"id":"a","provider":"manual","address":"reader@example.test","displayName":"Synthetic reader","userName":"reader","imap":{"host":"imap.example.invalid","port":993,"security":"tls"},"smtp":{"host":"smtp.example.invalid","port":587,"security":"starttls"},"auth":{"kind":"password","secretRef":"synthetic-only"}}]})).expect("synthetic config");
    assert_eq!(config.validate(), Vec::new());
    config
}
fn root() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(
        std::env::var_os("MAILSTRONAUT_REMINDER_TEST_ROOT").expect("explicit private fixture"),
    );
    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    assert!(root.canonicalize().expect("root").starts_with(workspace));
    assert_eq!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
        format!("unix:path={}/runtime/bus", root.display())
    );
    root
}
async fn emit(portal: &zbus::Connection, notification: &str, name: &str, generation: i64) {
    portal
        .emit_signal(
            None::<&str>,
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Notification",
            "ActionInvoked",
            &(notification, name, vec![OwnedValue::from(generation)]),
        )
        .await
        .expect("action");
}
struct Snapshot {
    records: Vec<Reminder>,
    status: String,
}
async fn snapshot_for(
    handle: &worker::Handle,
    config: &Config,
    client: Option<Arc<desktop::Client>>,
) -> Snapshot {
    let mut client = client;
    let (records, status, _) = step(handle, config, &mut client, &CancellationToken::new()).await;
    Snapshot { records, status }
}
#[tokio::test]
#[ignore = "requires experiments/p08-reminders/portal.sh private D-Bus fixture"]
#[expect(
    clippy::too_many_lines,
    reason = "One serial desktop session tests delivery, restart and action ownership without competing portal services."
)]
async fn private_portal_reminders_replace_after_crash_and_honor_actions_privacy_and_mute() {
    let root = root();
    let profile = root.join("profile");
    let mut store = Store::open(&profile).expect("profile");
    store.ensure_account("a").expect("account");
    let message = store
        .ingest_message(
            "a",
            b"Subject: SYNTHETIC-PRIVATE-SUBJECT\r\n\r\nSYNTHETIC-PRIVATE-BODY",
        )
        .expect("original");
    drop(store);
    let owner = Worker::open(profile.clone()).await.expect("owner");
    let handle = owner.handle();
    let first = handle
        .schedule_reminder(Schedule {
            account: "a".into(),
            message,
            due_at: 200,
            time_zone: "Europe/Paris".into(),
            now: 100,
            expected: None,
        })
        .await
        .expect("schedule");
    let calls = Arc::new(Calls::default());
    let portal = zbus::connection::Builder::session()
        .expect("private bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("name")
        .serve_at("/org/freedesktop/portal/desktop", Portal(calls.clone()))
        .expect("service")
        .build()
        .await
        .expect("portal");
    let client = desktop::bounded(desktop::Client::connect())
        .await
        .expect("connect and listen");
    // Crash window before calling the desktop: exclusive restart reclaims it.
    let unsubmitted = handle
        .claim_reminder(vec!["a".into()], 200)
        .await
        .expect("claim")
        .expect("due");
    owner
        .shutdown()
        .await
        .expect("simulate exit before handoff");
    let owner = Worker::open(profile.clone()).await.expect("recover");
    let handle = owner.handle();
    let submitted = handle
        .claim_reminder(vec!["a".into()], 200)
        .await
        .expect("claim")
        .expect("due");
    assert_eq!(
        unsubmitted.reminder.notification,
        submitted.reminder.notification
    );
    // Crash window after desktop acceptance, before recording its acknowledgment.
    desktop::bounded(client.deliver(&submitted.reminder, &handle, false))
        .await
        .expect("accepted");
    owner.shutdown().await.expect("simulate exit after handoff");
    let owner = Worker::open(profile)
        .await
        .expect("recover acknowledgment gap");
    let handle = owner.handle();
    let mut config = config();
    let snapshot = snapshot_for(&handle, &config, Some(client.clone())).await;
    assert_eq!(snapshot.records[0].state, State::Delivered);
    let adds = calls.adds.lock().expect("receipts").clone();
    assert_eq!(adds.len(), 2);
    assert_eq!(adds[0].0, adds[1].0);
    assert_eq!(adds[0].0, first.notification);
    assert!(
        !adds
            .iter()
            .any(|(_, body)| body.contains("SYNTHETIC-PRIVATE"))
    );
    let again = snapshot_for(&handle, &config, Some(client.clone())).await;
    assert_eq!(again.records[0].state, State::Delivered);
    assert_eq!(calls.adds.lock().expect("receipts").len(), 2);
    for (name, kind) in [
        ("open", desktop::Kind::Open),
        ("again", desktop::Kind::Again),
        ("dismiss", desktop::Kind::Dismiss),
    ] {
        emit(&portal, &first.notification, name, first.generation).await;
        let action = tokio::time::timeout(core::time::Duration::from_secs(2), client.events.recv())
            .await
            .expect("received")
            .expect("action");
        assert_eq!(
            core::mem::discriminant(&kind),
            core::mem::discriminant(&action.kind)
        );
        let resolved = handle
            .reminder_action(vec!["a".into()], action.notification, action.generation)
            .await
            .expect("resolve")
            .expect("current");
        assert_eq!(
            (resolved.account.as_str(), resolved.message),
            ("a", message)
        );
    }
    emit(&portal, &first.notification, "open", 0).await;
    emit(&portal, &first.notification, "unknown", 1).await;
    assert!(
        tokio::time::timeout(core::time::Duration::from_millis(100), client.events.recv())
            .await
            .is_err()
    );
    let replacement = handle
        .schedule_reminder(Schedule {
            account: "a".into(),
            message,
            due_at: 300,
            time_zone: "Europe/Paris".into(),
            now: 100,
            expected: Some(1),
        })
        .await
        .expect("reschedule");
    assert!(
        handle
            .reminder_action(vec!["a".into()], first.notification.clone(), 1)
            .await
            .expect("stale")
            .is_none()
    );
    config.notifications.enabled = false;
    let muted = snapshot_for(&handle, &config, Some(client.clone())).await;
    assert_eq!(muted.records[0].state, State::Pending);
    assert_eq!(calls.adds.lock().expect("receipts").len(), 2);
    assert_eq!(
        calls.removes.lock().expect("receipts").as_slice(),
        std::slice::from_ref(&first.notification)
    );
    config.notifications.enabled = true;
    calls.deny.store(true, Ordering::SeqCst);
    let denied = snapshot_for(&handle, &config, Some(client.clone())).await;
    assert_eq!(denied.records[0].state, State::Failed);
    assert_ne!(denied.status, "");
    let count = calls.adds.lock().expect("receipts").len();
    let _unchanged = snapshot_for(&handle, &config, Some(client.clone())).await;
    assert_eq!(calls.adds.lock().expect("receipts").len(), count);
    calls.deny.store(false, Ordering::SeqCst);
    handle
        .edit_reminder(
            "a".into(),
            replacement.id.clone(),
            replacement.generation,
            true,
        )
        .await
        .expect("explicit retry");
    let retried = snapshot_for(&handle, &config, Some(client.clone())).await;
    assert_eq!(retried.records[0].state, State::Delivered);
    let record = &retried.records[0];
    desktop::bounded(client.deliver(record, &handle, true))
        .await
        .expect("explicit private preview");
    assert!(
        calls
            .adds
            .lock()
            .expect("receipts")
            .last()
            .expect("preview receipt")
            .1
            .contains("SYNTHETIC-PRIVATE-SUBJECT")
    );
    handle
        .edit_reminder("a".into(), record.id.clone(), record.generation, false)
        .await
        .expect("cancel");
    config.notifications.enabled = false;
    assert_eq!(
        snapshot_for(&handle, &config, Some(client.clone()))
            .await
            .records,
        Vec::new()
    );
    assert_eq!(calls.removes.lock().expect("receipts").len(), 2);
    assert!(
        handle
            .reminder_action(
                vec!["a".into()],
                record.notification.clone(),
                record.generation
            )
            .await
            .expect("cancelled action")
            .is_none()
    );
    // Apply an action received from the private portal without creating a window.
    let current = handle
        .reminder_for("a".into(), message)
        .await
        .expect("current")
        .expect("reminder");
    let next = handle
        .schedule_reminder(Schedule {
            account: "a".into(),
            message,
            due_at: 300,
            time_zone: "Europe/Paris".into(),
            now: 100,
            expected: Some(current.generation),
        })
        .await
        .expect("reschedule fixture");
    emit(&portal, &next.notification, "again", next.generation).await;
    let action = tokio::time::timeout(Duration::from_secs(2), client.events.recv())
        .await
        .expect("portal action deadline")
        .expect("action");
    assert!(
        handle_action(&handle, &config, action)
            .await
            .expect("apply without window")
            .is_none()
    );
    let updated = handle
        .reminder_for("a".into(), message)
        .await
        .expect("read")
        .expect("reminder");
    assert_eq!(updated.generation, next.generation + 1);
    assert!((3500..=3600).contains(&(updated.due_at - now())));
    drop(client);
    // The scheduler's service remains active without a GPUI view or subscriber.
    config.notifications.enabled = true;
    let current_time = now();
    let due = handle
        .schedule_reminder(Schedule {
            account: "a".into(),
            message,
            due_at: current_time + 1,
            time_zone: "Europe/Paris".into(),
            now: current_time,
            expected: Some(updated.generation),
        })
        .await
        .expect("due reminder");
    let service = Service::start(
        &tokio::runtime::Handle::current(),
        handle.clone(),
        Arc::new(config),
        Arc::new(Notify::new()),
        &CancellationToken::new(),
    );
    let mut state = service.state.clone();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let delivered = state.borrow_and_update().as_ref().is_some_and(|snapshot| {
                snapshot
                    .0
                    .iter()
                    .any(|record| record.id == due.id && record.state == State::Delivered)
            });
            if delivered {
                break;
            }
            state.changed().await.expect("service remains active");
        }
    })
    .await
    .expect("desktop delivery without a window");
    drop(state);
    emit(&portal, &due.notification, "open", due.generation).await;
    let event = tokio::time::timeout(Duration::from_secs(2), service.events.recv())
        .await
        .expect("open action deadline")
        .expect("service event");
    assert!(matches!(event, Update::Open(record) if record.message == message));
    drop(service);
    drop(snapshot);
    drop(again);
    drop(muted);
    drop(denied);
    drop(retried);
    owner.shutdown().await.expect("shutdown");
    println!(
        "Private reminder portal: stable replacement across both crash windows, scoped actions, generic previews, mute, denial, explicit retry and withdrawal passed."
    );
}

#[tokio::test]
async fn in_app_open_uses_the_original_even_without_a_desktop_receipt() {
    let root = tempfile::tempdir().expect("profile");
    let mut store = Store::open(root.path()).expect("store");
    store.ensure_account("a").expect("account");
    let message = store
        .ingest_message("a", b"Subject: Synthetic reminder\r\n\r\nLocal original.")
        .expect("message");
    drop(store);
    let owner = Worker::open(root.path().into()).await.expect("owner");
    let handle = owner.handle();
    let mut record = handle
        .schedule_reminder(Schedule {
            account: "a".into(),
            message,
            due_at: 200,
            time_zone: "UTC".into(),
            now: 100,
            expected: None,
        })
        .await
        .expect("schedule");
    record.notification.clear();
    assert_eq!(
        resolve_original(handle.clone(), vec!["a".into()], record.clone())
            .await
            .expect("resolve")
            .expect("original")
            .message,
        message
    );
    assert!(
        resolve_original(handle.clone(), vec!["b".into()], record.clone())
            .await
            .expect("foreign scope")
            .is_none()
    );
    let current_generation = record.generation;
    record.generation += 1;
    assert!(
        resolve_original(handle.clone(), vec!["a".into()], record.clone())
            .await
            .expect("stale generation")
            .is_none()
    );
    record.generation = current_generation;
    record.id = "another-reminder".into();
    assert!(
        resolve_original(handle.clone(), vec!["a".into()], record)
            .await
            .expect("foreign ID")
            .is_none()
    );
    assert_eq!(
        handle
            .reminder_for("a".into(), message)
            .await
            .expect("unchanged")
            .expect("record")
            .state,
        State::Pending
    );
    owner.shutdown().await.expect("shutdown");
}
