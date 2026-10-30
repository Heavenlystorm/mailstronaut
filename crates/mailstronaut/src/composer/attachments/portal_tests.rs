//! Synthetic file grants on a private bus; never use the user's portal.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use zbus::{
    Connection, fdo,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use super::choose;

type Responses = Arc<Mutex<VecDeque<(u32, Vec<String>)>>>;
struct Portal(Responses);
#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl Portal {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "D-Bus property getters use an instance receiver."
    )]
    fn version(&self) -> u32 {
        4
    }

    #[zbus(name = "OpenFile")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus supplies owned options and a message header."
    )]
    fn open(
        &self,
        parent: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        assert_eq!(parent, "");
        assert_eq!(title, "Attach files");
        assert_eq!(
            options
                .get("multiple")
                .and_then(|v| v.downcast_ref::<bool>().ok()),
            Some(true)
        );
        assert_eq!(
            options
                .get("modal")
                .and_then(|v| v.downcast_ref::<bool>().ok()),
            Some(true)
        );
        let token = options
            .get("handle_token")
            .and_then(|v| <&str>::try_from(v).ok())
            .ok_or_else(|| fdo::Error::InvalidArgs("handle".into()))?;
        let sender = header
            .sender()
            .expect("sender")
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .expect("request path");
        let response = self
            .0
            .lock()
            .expect("responses")
            .pop_front()
            .expect("expected chooser call");
        let (connection, response_path) = (connection.clone(), path.clone());
        tokio::spawn(async move {
            let mut values = HashMap::<String, OwnedValue>::new();
            values.insert(
                "uris".into(),
                OwnedValue::try_from(Value::from(response.1)).expect("URI array"),
            );
            connection
                .emit_signal(
                    None::<&str>,
                    response_path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(response.0, values),
                )
                .await
                .expect("response");
        });
        Ok(path)
    }
}

#[tokio::test]
#[ignore = "requires experiments/p07-compose/attachments-portal.sh private D-Bus fixture"]
async fn private_portal_grants_cancel_invalid_uris_and_failed_copies() {
    let root = std::path::PathBuf::from(
        std::env::var_os("MAILSTRONAUT_ATTACHMENT_TEST_ROOT").expect("explicit fixture"),
    );
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
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
    let source = root.join("granted-file.bin");
    std::fs::write(&source, [0, 255, 1]).expect("source");
    let uri = format!("file://{}", source.display());
    let responses = Arc::new(Mutex::new(VecDeque::from([
        (0, vec![uri.clone()]),
        (1, vec![]),
        (0, vec!["https://example.invalid/must-not-fetch".into()]),
        (0, vec![uri.clone(), uri.clone()]),
        (0, vec![format!("file://{}/missing", root.display())]),
        (0, vec![uri.clone()]),
    ])));
    let _portal = zbus::connection::Builder::session()
        .expect("bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("portal name")
        .serve_at("/org/freedesktop/portal/desktop", Portal(responses.clone()))
        .expect("interface")
        .build()
        .await
        .expect("portal service");
    let files = tokio::time::timeout(std::time::Duration::from_secs(5), choose(3, 16))
        .await
        .expect("bounded chooser")
        .expect("grant")
        .expect("selected");
    assert_eq!(files[0].name, "granted-file.bin");
    assert_eq!(files[0].bytes, [0, 255, 1]);
    assert!(choose(3, 16).await.expect("cancel").is_none());
    assert!(choose(3, 16).await.is_err(), "remote URI is not fetched");
    assert!(choose(5, 16).await.is_err(), "aggregate budget is enforced");
    assert!(
        choose(3, 16).await.is_err(),
        "access loss gives a recoverable error"
    );
    assert!(choose(3, 0).await.is_err(), "count checked before copying");
    if let Some(binary) = std::env::var_os("MAILSTRONAUT_ATTACHMENT_GUI_BINARY") {
        responses
            .lock()
            .expect("responses")
            .extend([(0, vec![uri.clone()]), (0, vec![uri])]);
        let status = tokio::process::Command::new("bash")
            .arg("experiments/p07-compose/attachments-gui.sh")
            .arg(binary)
            .arg(&root)
            .status()
            .await
            .expect("native GUI fixture");
        assert!(status.success(), "native attachment controls");
    }
    assert!(responses.lock().expect("responses").is_empty());
    println!(
        "Private file portal: copied granted binary; cancellation, remote URI, aggregate size, missing source and count failures preserved drafts."
    );
}
