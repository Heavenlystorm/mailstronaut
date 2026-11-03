//! Private-bus `OpenURI` fixture for synthetic localhost authorization only.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use secrecy::ExposeSecret as _;
use zbus::{
    Connection, fdo,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue},
};

use super::{Reply, browser, callback, fixture_with_timeout, query};

struct Portal {
    port: u16,
    opens: Arc<AtomicUsize>,
    runtime: tokio::runtime::Handle,
}
#[zbus::interface(name = "org.freedesktop.portal.OpenURI")]
impl Portal {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        clippy::missing_const_for_fn,
        reason = "zbus prescribes a non-const instance getter for this D-Bus property."
    )]
    fn version(&self) -> u32 {
        5
    }

    #[zbus(name = "OpenURI")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "The D-Bus interface receives decoded owned options and an injected message header."
    )]
    fn open_uri(
        &self,
        parent: &str,
        uri: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        let _ = parent;
        let url = url::Url::parse(uri)
            .map_err(|_| fdo::Error::InvalidArgs("invalid synthetic URI".into()))?;
        if url.scheme() != "https"
            || url.host_str() != Some("localhost")
            || url.port() != Some(self.port)
            || url.path() != "/authorize"
        {
            return Err(fdo::Error::AccessDenied(
                "fixture accepts only its local IdP".into(),
            ));
        }
        let fields = query(&url);
        if fields.get("client_id").map(String::as_str) != Some("synthetic-public-client")
            || fields.get("code_challenge_method").map(String::as_str) != Some("S256")
        {
            return Err(fdo::Error::InvalidArgs(
                "expected public client and PKCE".into(),
            ));
        }
        let token = options
            .get("handle_token")
            .and_then(|value| <&str>::try_from(value).ok())
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
        let target = callback(&url, "SYNTHETIC-PORTAL-CODE");
        if target.host_str() != Some("127.0.0.1") || target.path() != "/oauth2/callback" {
            return Err(fdo::Error::AccessDenied("non-loopback callback".into()));
        }
        self.opens.fetch_add(1, Ordering::SeqCst);
        let connection = connection.clone();
        let response_path = path.clone();
        self.runtime.spawn(async move {
            connection
                .emit_signal(
                    None::<&str>,
                    response_path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(0_u32, HashMap::<String, OwnedValue>::new()),
                )
                .await
                .expect("private portal response");
            let response = browser()
                .get(target)
                .send()
                .await
                .expect("private authorization callback");
            assert_eq!(response.status(), 200);
        });
        Ok(path)
    }
}

pub async fn run() {
    let output = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_GUI_OAUTH_PORT_FILE").expect("explicit rendezvous"),
    );
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_SECRET_TEST_ROOT").expect("private fixture"));
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    assert!(root.canonicalize().expect("root").starts_with(workspace));
    assert!(
        output
            .parent()
            .expect("directory")
            .canonicalize()
            .expect("directory")
            .starts_with(root.canonicalize().expect("root"))
    );
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .expect("private bus")
            .starts_with(&format!("unix:path={}/runtime/bus", root.display()))
    );
    let (port, mut requests, server) = fixture_with_timeout(
        vec![
            Reply::grant(
                "SYNTHETIC-OAUTH-ACCESS-NOT-REAL",
                "SYNTHETIC-PORTAL-REFRESH-ONE",
            ),
            Reply::grant(
                "SYNTHETIC-OAUTH-ACCESS-NOT-REAL",
                "SYNTHETIC-PORTAL-REFRESH-TWO",
            ),
        ],
        Duration::from_secs(120),
    )
    .await;
    let opens = Arc::new(AtomicUsize::new(0));
    let _portal = zbus::connection::Builder::session()
        .expect("bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("portal name")
        .serve_at(
            "/org/freedesktop/portal/desktop",
            Portal {
                port,
                opens: opens.clone(),
                runtime: tokio::runtime::Handle::current(),
            },
        )
        .expect("OpenURI interface")
        .build()
        .await
        .expect("private portal");
    std::fs::write(output, port.to_string()).expect("rendezvous");
    let initial = tokio::time::timeout(Duration::from_secs(120), requests.recv())
        .await
        .expect("native authorization deadline")
        .expect("native token exchange");
    assert_eq!(initial["grant_type"], "authorization_code");
    assert_eq!(initial["code"], "SYNTHETIC-PORTAL-CODE");
    let refresh = tokio::time::timeout(Duration::from_secs(120), requests.recv())
        .await
        .expect("restart refresh deadline")
        .expect("refresh");
    assert_eq!(refresh["grant_type"], "refresh_token");
    assert_eq!(refresh["refresh_token"], "SYNTHETIC-PORTAL-REFRESH-ONE");
    server.await.expect("IdP fixture");
    assert_eq!(
        opens.load(Ordering::SeqCst),
        1,
        "restart used refresh without another browser request"
    );
    println!(
        "Native OpenURI used PKCE; restart refreshed the encrypted grant without reopening the browser."
    );
}

pub async fn fixture_round_trip() {
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_SECRET_TEST_ROOT").expect("fixture root"));
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    let root = root.canonicalize().expect("fixture root");
    assert!(root.starts_with(target));
    assert_eq!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
        format!("unix:path={}/runtime/bus", root.display())
    );
    let (port, mut requests, server) = fixture_with_timeout(
        vec![Reply::grant("SYNTHETIC-ACCESS", "SYNTHETIC-REFRESH")],
        Duration::from_secs(10),
    )
    .await;
    let opens = Arc::new(AtomicUsize::new(0));
    let portal = zbus::connection::Builder::session()
        .expect("private bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("portal name")
        .serve_at(
            "/org/freedesktop/portal/desktop",
            Portal {
                port,
                opens: opens.clone(),
                runtime: tokio::runtime::Handle::current(),
            },
        )
        .expect("OpenURI fixture")
        .build()
        .await
        .expect("portal");
    let provider = super::Provider::prepare(&super::auth(port), &super::roots())
        .await
        .expect("provider");
    let flow = provider
        .begin("reader@example.test")
        .await
        .expect("OAuth flow");
    let url = flow.authorization_url().to_string();
    let finish = tokio::spawn(async move {
        flow.finish(&tokio_util::sync::CancellationToken::new())
            .await
    });
    let client = Connection::session().await.expect("client connection");
    let proxy = zbus::Proxy::new(
        &client,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.OpenURI",
    )
    .await
    .expect("proxy");
    let token = zbus::zvariant::Value::from("synthetic_oauth")
        .try_to_owned()
        .expect("token");
    let options = HashMap::from([("handle_token", token)]);
    let _: OwnedObjectPath = proxy
        .call("OpenURI", &("", url.as_str(), options))
        .await
        .expect("portal request");
    let grant = tokio::time::timeout(Duration::from_secs(10), finish)
        .await
        .expect("callback deadline")
        .expect("flow task")
        .expect("grant");
    assert_eq!(grant.access().expose_secret(), "SYNTHETIC-ACCESS");
    let request = requests.recv().await.expect("token exchange");
    assert_eq!(request["code"], "SYNTHETIC-PORTAL-CODE");
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    server.await.expect("server");
    portal.close().await.expect("close portal");
}
