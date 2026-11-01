//! Private-bus web-link transport fixture. Never launches a browser or fetches URLs.

use std::{collections::HashMap, fs::OpenOptions, io::Write as _, path::PathBuf, time::Duration};

use zbus::{
    Connection, fdo,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue},
};

use super::*;

const TARGET: &str = "https://links.example.test/report?SYNTHETIC-SECRET";

struct Portal(PathBuf);

#[zbus::interface(name = "org.freedesktop.portal.OpenURI")]
impl Portal {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "zbus requires a non-const instance property getter."
    )]
    fn version(&self) -> u32 {
        5
    }

    #[zbus(name = "OpenURI")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "D-Bus supplies owned decoded options and the message header."
    )]
    fn open_uri(
        &self,
        parent: &str,
        uri: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        let local = std::fs::read_to_string(self.0.join("http-port"))
            .ok()
            .map(|port| format!("http://127.0.0.1:{}/activate", port.trim()));
        if uri != TARGET && local.as_deref() != Some(uri) {
            return Err(fdo::Error::AccessDenied(
                "Only the private synthetic link is accepted.".into(),
            ));
        }
        let mode =
            std::fs::read_to_string(self.0.join("portal-response")).unwrap_or_else(|_| "0".into());
        let receipt = serde_json::json!({"uri":uri,"parent":parent,"response":mode.trim()});
        writeln!(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.0.join("portal-requests.jsonl"))
                .map_err(|_| fdo::Error::Failed("fixture receipt".into()))?,
            "{receipt}"
        )
        .map_err(|_| fdo::Error::Failed("fixture receipt".into()))?;
        if mode.trim() == "refuse" {
            return Err(fdo::Error::Failed(
                "SYNTHETIC-SECRET portal diagnostic".into(),
            ));
        }
        let token = options
            .get("handle_token")
            .and_then(|v| <&str>::try_from(v).ok())
            .ok_or_else(|| fdo::Error::InvalidArgs("missing request handle".into()))?;
        let sender = header
            .sender()
            .ok_or_else(|| fdo::Error::Failed("missing sender".into()))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .map_err(|_| fdo::Error::InvalidArgs("request path".into()))?;
        if mode.trim() != "hold" {
            let response = mode.trim().parse::<u32>().unwrap_or(2);
            let connection = connection.clone();
            let response_path = path.clone();
            tokio::spawn(async move {
                connection
                    .emit_signal(
                        None::<&str>,
                        response_path,
                        "org.freedesktop.portal.Request",
                        "Response",
                        &(response, HashMap::<String, OwnedValue>::new()),
                    )
                    .await
                    .expect("synthetic response");
            });
        }
        Ok(path)
    }
}

fn root() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_LINK_PORTAL_ROOT").expect("private fixture root"),
    );
    assert!(root.is_absolute() && root.join("synthetic-link-portal").is_file());
    assert_eq!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
        format!("unix:path={}/runtime/bus", root.display())
    );
    root
}

async fn server(root: PathBuf) -> Connection {
    zbus::connection::Builder::session()
        .expect("private bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("synthetic portal name")
        .serve_at("/org/freedesktop/portal/desktop", Portal(root))
        .expect("portal interface")
        .build()
        .await
        .expect("private portal")
}

fn target() -> Link {
    let mut document =
        mailstronaut_render::Document::new(&format!("<a href='{TARGET}'>Synthetic link</a>"), &[])
            .expect("synthetic HTML");
    document
        .paint(mailstronaut_render::View {
            width: 100,
            height: 80,
            scale: 1.0,
            dark: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
        })
        .expect("painted document")
        .links[0]
        .clone()
}

fn count(root: &std::path::Path) -> usize {
    std::fs::read_to_string(root.join("portal-requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test]
#[ignore = "requires the isolated activation-free bus from experiments/p01-blitz/portal.sh"]
async fn handoff_is_explicit_bounded_cancellable_and_redacted() {
    let root = root();
    let portal = server(root.clone()).await;
    let cancel = CancellationToken::new();
    assert_eq!(open(target(), &cancel).await, Ok(()));
    assert_eq!(count(&root), 1);
    std::fs::write(root.join("portal-response"), "1").expect("cancel response");
    assert_eq!(
        open(target(), &cancel).await,
        Err("Opening the link was cancelled or refused.")
    );
    std::fs::write(root.join("portal-response"), "refuse").expect("refusal");
    assert_eq!(
        open(target(), &cancel).await,
        Err("Browser portal is unavailable or refused the request.")
    );
    std::fs::write(root.join("portal-response"), "hold").expect("withheld response");
    assert_eq!(
        super::open_with_timeout(target(), &cancel, Duration::from_millis(100)).await,
        Err("Browser portal did not respond. Try again.")
    );
    assert_eq!(
        count(&root),
        4,
        "deadline observer saw the real method call"
    );
    let already_cancelled = CancellationToken::new();
    already_cancelled.cancel();
    assert_eq!(
        open(target(), &already_cancelled).await,
        Err("Opening the link was cancelled.")
    );
    assert_eq!(
        count(&root),
        4,
        "cancelled intent cannot contact the portal"
    );
    let active = CancellationToken::new();
    let work_cancel = active.clone();
    let work = tokio::spawn(async move { open(target(), &work_cancel).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while count(&root) < 5 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("portal sees the waiting request");
    active.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), work)
            .await
            .expect("bounded cancellation")
            .expect("task"),
        Err("Opening the link was cancelled.")
    );
    drop(portal);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let unavailable = open(target(), &cancel)
        .await
        .expect_err("no portal service");
    assert_eq!(
        unavailable,
        "Browser portal is unavailable or refused the request."
    );
    assert!(!unavailable.contains("SYNTHETIC-SECRET"));
}

#[tokio::test]
#[ignore = "private native GUI fixture; never a host desktop portal"]
async fn serve_native_portal() {
    let root = root();
    let _portal = server(root.clone()).await;
    std::fs::write(root.join("portal-ready"), b"ready").expect("fixture ready");
    tokio::time::timeout(Duration::from_secs(150), async {
        while !root.join("portal-stop").exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("fixture owner stops the service");
}
