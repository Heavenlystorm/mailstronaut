//! Actual session-bus ownership/routing; run only inside the private fixture.

use super::*;

fn private_bus() {
    let root = std::path::PathBuf::from(
        std::env::var_os("MAILSTRONAUT_INSTANCE_ROOT").expect("private fixture root"),
    );
    assert!(root.is_absolute() && root.join("synthetic-instance").is_file());
    assert_eq!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private session bus"),
        format!("unix:path={}/runtime/bus", root.display())
    );
}

async fn activate(connection: &Connection, data: HashMap<String, OwnedValue>) -> zbus::Result<()> {
    connection
        .call_method(Some(NAME), PATH, Some(INTERFACE), "Activate", &(data,))
        .await?
        .body()
        .deserialize()
}

struct Unresponsive;

#[zbus::interface(name = "org.freedesktop.Application")]
impl Unresponsive {
    async fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        drop(platform_data);
        std::future::pending::<()>().await;
    }
}

#[tokio::test]
#[ignore = "requires the private session bus from experiments/p08-reminders/instance.sh"]
async fn private_ownership_activation_conflicts_shutdown_and_timeout() {
    private_bus();
    let primary = launch_profile(b"synthetic-profile".to_vec(), Duration::from_secs(2))
        .await
        .expect("first launch")
        .expect("primary ownership");
    let events = primary.activations();
    let mut launches = Vec::new();
    for _ in 0..8 {
        launches.push(tokio::spawn(launch_profile(
            b"synthetic-profile".to_vec(),
            Duration::from_secs(2),
        )));
    }
    for launch in launches {
        assert!(
            launch
                .await
                .expect("secondary task")
                .expect("activation")
                .is_none()
        );
    }
    assert_eq!(events.len(), 1, "activation bursts coalesce");
    events.recv().await.expect("one passive activation");
    assert!(events.is_empty());

    assert!(
        launch_profile_mode(b"synthetic-profile".to_vec(), Duration::from_secs(2), true)
            .await
            .expect("background launch")
            .is_none()
    );
    assert!(
        events.is_empty(),
        "a background relaunch must not open a window"
    );
    assert!(
        launch_profile_mode(b"other-profile".to_vec(), Duration::from_secs(2), true)
            .await
            .is_err(),
        "background relaunch still checks profile identity"
    );

    assert!(
        launch_profile(b"other-profile".to_vec(), Duration::from_secs(2))
            .await
            .is_err()
    );
    assert!(events.is_empty(), "different sources must not activate");
    let client = Connection::session().await.expect("private client");
    activate(&client, HashMap::new())
        .await
        .expect("standard desktop Activate");
    events.recv().await.expect("desktop activation");
    assert!(
        activate(
            &client,
            HashMap::from([(PROFILE_KEY.to_owned(), OwnedValue::from(false))]),
        )
        .await
        .is_err(),
        "malformed profile data must be refused"
    );
    assert!(events.is_empty());

    events.close();
    assert!(activate(&client, HashMap::new()).await.is_err());
    primary.close().await.expect("release ownership");
    let next = launch_profile(b"next-profile".to_vec(), Duration::from_secs(2))
        .await
        .expect("restart")
        .expect("new primary after close");
    next.close().await.expect("release next owner");

    let held = zbus::connection::Builder::session()
        .expect("private bus")
        .serve_at(PATH, Unresponsive)
        .expect("unresponsive interface")
        .build()
        .await
        .expect("unresponsive connection");
    held.request_name_with_flags(NAME, fdo::RequestNameFlags::DoNotQueue.into())
        .await
        .expect("held name");
    let started = std::time::Instant::now();
    assert!(
        launch_profile(b"synthetic-profile".to_vec(), Duration::from_millis(150))
            .await
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    held.close().await.expect("close nonresponsive fixture");
    println!(
        "Single-instance bus: exclusive ownership, concurrent forwarding, coalescing, standard Activate, source conflicts, closed receiver, restart and timeout passed."
    );
}
