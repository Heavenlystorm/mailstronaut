//! Synthetic localhost OAuth only. No browser, provider, real user or keyring.

use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::{Response, body::Bytes, service::service_fn};
use hyper_util::rt::TokioIo;
use mailstronaut_config::{Auth, Tls};
use mailstronaut_mail::oauth::{Error, Provider};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use url::Url;

#[path = "oauth/portal.rs"]
mod portal;

fn roots() -> Tls {
    serde_json::from_value(serde_json::json!({"caFiles":[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/certs/ca.pem")]})).expect("roots")
}
fn auth(port: u16) -> Auth {
    serde_json::from_value(serde_json::json!({"kind":"oauth2","provider":"custom","clientId":"synthetic-public-client","secretRef":"fixture","authorizationUrl":format!("https://localhost:{port}/authorize"),"tokenUrl":format!("https://localhost:{port}/token"),"scopes":["mail.read","mail.send"],"sasl":"xoauth2"})).expect("public metadata")
}
fn acceptor() -> TlsAcceptor {
    let certificates = rustls_pemfile::certs(&mut include_bytes!("certs/localhost.pem").as_slice())
        .collect::<Result<Vec<_>, _>>()
        .expect("certificates");
    let key =
        rustls_pemfile::private_key(&mut include_bytes!("certs/localhost-key.pem").as_slice())
            .expect("key")
            .expect("fixture key");
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("versions")
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .expect("server config");
    TlsAcceptor::from(Arc::new(config))
}
struct Reply {
    status: u16,
    body: String,
    location: Option<String>,
}
impl Reply {
    fn grant(access: &str, refresh: &str) -> Self {
        Self {status:200,body:serde_json::json!({"token_type":"Bearer","access_token":access,"refresh_token":refresh,"expires_in":3600,"scope":"mail.read mail.send"}).to_string(),location:None}
    }
}
async fn fixture(
    replies: Vec<Reply>,
) -> (
    u16,
    mpsc::Receiver<HashMap<String, String>>,
    tokio::task::JoinHandle<()>,
) {
    fixture_with_timeout(replies, Duration::from_secs(10)).await
}
async fn fixture_with_timeout(
    replies: Vec<Reply>,
    deadline: Duration,
) -> (
    u16,
    mpsc::Receiver<HashMap<String, String>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback");
    let port = listener.local_addr().expect("address").port();
    let (sent, requests) = mpsc::channel(4);
    let server = tokio::spawn(async move {
        for reply in replies {
            let (socket, _) = tokio::time::timeout(deadline, listener.accept())
                .await
                .expect("server deadline")
                .expect("accept");
            let socket = acceptor().accept(socket).await.expect("TLS");
            let reply = Arc::new(reply);
            let sent = sent.clone();
            let connection = hyper::server::conn::http1::Builder::new()
                .keep_alive(false)
                .serve_connection(
                    TokioIo::new(socket),
                    service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                        let reply = reply.clone();
                        let sent = sent.clone();
                        async move {
                            assert_eq!(request.method(), hyper::Method::POST);
                            assert_eq!(request.uri().path(), "/token");
                            assert!(!request.headers().contains_key("authorization"));
                            let body = Limited::new(request.into_body(), 8192)
                                .collect()
                                .await
                                .expect("bounded request")
                                .to_bytes();
                            let fields = url::form_urlencoded::parse(&body)
                                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                                .collect::<HashMap<_, _>>();
                            sent.send(fields)
                                .await
                                .expect("capture public fixture request");
                            let mut response = Response::new(Full::new(Bytes::copy_from_slice(
                                reply.body.as_bytes(),
                            )));
                            *response.status_mut() =
                                hyper::StatusCode::from_u16(reply.status).expect("status");
                            response.headers_mut().insert(
                                "content-type",
                                hyper::header::HeaderValue::from_static("application/json"),
                            );
                            if let Some(location) = &reply.location {
                                response
                                    .headers_mut()
                                    .insert("location", location.parse().expect("location"));
                            }
                            Ok::<_, Infallible>(response)
                        }
                    }),
                );
            connection.await.expect("HTTP connection");
        }
    });
    (port, requests, server)
}
fn query(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}
fn callback(authorize: &Url, code: &str) -> Url {
    let fields = query(authorize);
    let mut callback =
        Url::parse(fields.get("redirect_uri").expect("redirect")).expect("loopback URI");
    callback
        .query_pairs_mut()
        .append_pair("state", fields.get("state").expect("state"))
        .append_pair("code", code);
    callback
}
fn browser() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .expect("synthetic browser")
}

async fn reject_invalid_callbacks(browser: &reqwest::Client, target: &Url) {
    let mut wrong = target.clone();
    wrong.set_query(Some("state=wrong&code=wrong"));
    assert_eq!(
        browser
            .get(wrong)
            .send()
            .await
            .expect("wrong state")
            .status(),
        400
    );
    let mut duplicate = target.clone();
    duplicate
        .query_pairs_mut()
        .append_pair("state", &query(target)["state"]);
    assert_eq!(
        browser
            .get(duplicate)
            .send()
            .await
            .expect("duplicate")
            .status(),
        400
    );
    let mut wrong_path = target.clone();
    wrong_path.set_path("/wrong");
    assert_eq!(
        browser.get(wrong_path).send().await.expect("path").status(),
        400
    );
    assert_eq!(
        browser
            .get(target.clone())
            .header("host", "attacker.invalid")
            .send()
            .await
            .expect("host")
            .status(),
        400
    );
    assert_eq!(
        browser
            .post(target.clone())
            .send()
            .await
            .expect("method")
            .status(),
        400
    );
}

#[tokio::test]
async fn pkce_callback_exchange_and_refresh_use_standard_grants_without_exposing_tokens() {
    let (port, mut requests, server) = fixture(vec![
        Reply::grant("SYNTHETIC-ACCESS-ONE", "SYNTHETIC-REFRESH-ONE"),
        Reply::grant("SYNTHETIC-ACCESS-TWO", "SYNTHETIC-REFRESH-TWO"),
    ])
    .await;
    let provider = Provider::prepare(&auth(port), &roots())
        .await
        .expect("prepare");
    let flow = provider
        .begin("reader@example.test")
        .await
        .expect("bind before browser");
    let url = flow.authorization_url().clone();
    let fields = query(&url);
    assert_eq!(fields.get("code_challenge_method").expect("S256"), "S256");
    let target = callback(&url, "SYNTHETIC-CODE");
    let finish = tokio::spawn(async move { flow.finish(&CancellationToken::new()).await });
    let browser = browser();
    reject_invalid_callbacks(&browser, &target).await;
    assert!(requests.try_recv().is_err());
    assert_eq!(
        browser
            .get(target.clone())
            .send()
            .await
            .expect("valid callback")
            .status(),
        200
    );
    let grant = finish.await.expect("flow task").expect("grant");
    let request = requests.recv().await.expect("code exchange");
    assert_eq!(request["grant_type"], "authorization_code");
    assert_eq!(request["code"], "SYNTHETIC-CODE");
    assert_eq!(request["client_id"], "synthetic-public-client");
    assert_eq!(request["redirect_uri"], fields["redirect_uri"]);
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(request["code_verifier"].as_bytes())),
        fields["code_challenge"]
    );
    assert!(!request.contains_key("client_secret"));
    assert_eq!(grant.access().expose_secret(), "SYNTHETIC-ACCESS-ONE");
    assert_eq!(
        grant.refresh().expect("refresh").expose_secret(),
        "SYNTHETIC-REFRESH-ONE"
    );
    assert_eq!(grant.expires_in(), Some(Duration::from_secs(3600)));
    assert!(!format!("{grant:?}").contains("SYNTHETIC"));
    assert!(
        browser.get(target).send().await.is_err(),
        "completed listener closed"
    );
    let rotated = provider
        .refresh(grant.refresh().expect("refresh"), &CancellationToken::new())
        .await
        .expect("refresh grant");
    assert_eq!(
        rotated.refresh().expect("rotation").expose_secret(),
        "SYNTHETIC-REFRESH-TWO"
    );
    let request = requests.recv().await.expect("refresh request");
    assert_eq!(request["grant_type"], "refresh_token");
    assert_eq!(request["refresh_token"], "SYNTHETIC-REFRESH-ONE");
    server.await.expect("fixture");
}

#[tokio::test]
async fn denial_cancellation_and_duplicate_callback_budget_never_exchange_a_code() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("uncontacted token endpoint");
    let provider = Provider::prepare(
        &auth(listener.local_addr().expect("address").port()),
        &roots(),
    )
    .await
    .expect("prepare");
    let flow = provider.begin("reader@example.test").await.expect("begin");
    let mut target = callback(flow.authorization_url(), "unused");
    let state = query(flow.authorization_url())["state"].clone();
    target.set_query(None);
    target
        .query_pairs_mut()
        .append_pair("state", &state)
        .append_pair("error", "access_denied");
    let finish = tokio::spawn(async move { flow.finish(&CancellationToken::new()).await });
    assert_eq!(
        browser()
            .get(target)
            .send()
            .await
            .expect("denial callback")
            .status(),
        200
    );
    assert_eq!(
        finish.await.expect("task").expect_err("declined"),
        Error::Denied
    );
    let flow = provider.begin("reader@example.test").await.expect("begin");
    let target = callback(flow.authorization_url(), "unused");
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        flow.finish(&cancel).await.expect_err("cancelled"),
        Error::Cancelled
    );
    assert!(browser().get(target).send().await.is_err());
    let flow = provider.begin("reader@example.test").await.expect("begin");
    let mut target = callback(flow.authorization_url(), "unused");
    target.query_pairs_mut().append_pair("code", "duplicate");
    let finish = tokio::spawn(async move { flow.finish(&CancellationToken::new()).await });
    for _ in 0..16 {
        assert_eq!(
            browser()
                .get(target.clone())
                .send()
                .await
                .expect("invalid duplicate")
                .status(),
            400
        );
    }
    assert_eq!(
        finish.await.expect("task").expect_err("budget"),
        Error::Callback
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn token_redirect_oversize_and_revoked_grant_are_redacted_and_never_followed() {
    let destination = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("redirect observer");
    for (reply,expected) in [
        (Reply{status:302,body:String::new(),location:Some(format!("https://localhost:{}/stolen",destination.local_addr().expect("address").port()))},Error::Network),
        (Reply{status:200,body:"X".repeat(64*1024+1),location:None},Error::Network),
        (Reply{status:400,body:r#"{"error":"invalid_grant","error_description":"SYNTHETIC-PRIVATE-DESCRIPTION"}"#.into(),location:None},Error::Reauthenticate),
    ] {
        let (port,mut requests,server)=fixture(vec![reply]).await;
        let provider=Provider::prepare(&auth(port),&roots()).await.expect("prepare");
        let error=provider.refresh(&SecretString::from("SYNTHETIC-REFRESH"),&CancellationToken::new()).await.expect_err("refused response");
        assert_eq!(error,expected);assert!(!format!("{error:?} {error}").contains("SYNTHETIC"));
        assert!(requests.recv().await.is_some());
        server.await.expect("fixture");
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), destination.accept())
            .await
            .is_err()
    );
}

fn private_account(port: u16) -> mailstronaut_config::Account {
    serde_json::from_value(serde_json::json!({"id":"oauth-synthetic","provider":"manual","address":"reader@example.test","displayName":"Synthetic OAuth","userName":"reader@example.test","auth":serde_json::to_value(auth(port)).expect("metadata"),"tls":serde_json::to_value(roots()).expect("CA")})).expect("account")
}
async fn private_store() -> mailstronaut_mail::credentials::Store {
    let root = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_SECRET_TEST_ROOT")
            .expect("explicit private keyring fixture"),
    );
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("project target");
    assert!(
        root.canonicalize()
            .expect("fixture root")
            .starts_with(workspace)
    );
    assert_eq!(
        std::env::var("XDG_DATA_HOME").expect("private XDG"),
        root.join("data").to_string_lossy()
    );
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .expect("private bus")
            .starts_with(&format!("unix:path={}/runtime/bus", root.display()))
    );
    let store = mailstronaut_mail::credentials::Store::connect_foreground()
        .await
        .expect("private backend");
    store.unlock_foreground().await.expect("private unlock");
    store
}
fn assert_bearer(credential: &mailstronaut_mail::auth::Credential, expected: &str) {
    let mailstronaut_mail::imap::Authentication::OAuth2 { token, .. } = credential.authentication()
    else {
        panic!("expected OAuth SASL");
    };
    assert_eq!(token.expose_secret(), expected);
    assert!(!format!("{credential:?}").contains(expected));
}
async fn authorize_private(provider: &Provider) -> mailstronaut_mail::oauth::Grant {
    let flow = provider.begin("reader@example.test").await.expect("bind");
    let target = callback(flow.authorization_url(), "SYNTHETIC-PRIVATE-CODE");
    let finish = tokio::spawn(async move { flow.finish(&CancellationToken::new()).await });
    assert_eq!(
        browser()
            .get(target)
            .send()
            .await
            .expect("callback")
            .status(),
        200
    );
    finish.await.expect("task").expect("grant")
}

#[tokio::test]
#[ignore = "requires the explicit project-local Secret Service harness"]
#[expect(
    clippy::too_many_lines,
    reason = "One ordered private-keyring lifecycle proves rotation, concurrent reuse, restart, lock and revocation without resetting its state."
)]
async fn private_secret_service_oauth_rotation_is_serialized_and_revocation_stops_retries() {
    use mailstronaut_mail::{
        auth::{Error as AuthError, Session},
        credentials::{Error as CredentialError, Key, Purpose},
    };
    let store = private_store().await;
    let mut initial = Reply::grant("SYNTHETIC-OAUTH-INITIAL", "SYNTHETIC-OAUTH-REFRESH-ONE");
    let mut value: serde_json::Value = serde_json::from_str(&initial.body).expect("fixture");
    value["expires_in"] = 1.into();
    initial.body = value.to_string();
    let restart=Reply{status:200,body:serde_json::json!({"access_token":"SYNTHETIC-OAUTH-AFTER-RESTART","token_type":"Bearer","expires_in":3600,"scope":"mail.read mail.send"}).to_string(),location:None};
    let revoked = Reply {
        status: 400,
        body: r#"{"error":"invalid_grant","error_description":"SYNTHETIC-PRIVATE-REVOCATION"}"#
            .into(),
        location: None,
    };
    let (port, mut requests, server) = fixture(vec![
        initial,
        Reply::grant("SYNTHETIC-OAUTH-ROTATED", "SYNTHETIC-OAUTH-REFRESH-TWO"),
        restart,
        revoked,
        Reply::grant(
            "SYNTHETIC-OAUTH-RECONNECTED",
            "SYNTHETIC-OAUTH-REFRESH-THREE",
        ),
    ])
    .await;
    let account = private_account(port);
    let provider = Provider::prepare(&account.auth, &account.tls)
        .await
        .expect("provider");
    let session = Session::new(store.clone());
    session
        .accept(&account, false, authorize_private(&provider).await)
        .await
        .expect("encrypted offline grant");
    assert_eq!(
        requests.recv().await.expect("authorization")["grant_type"],
        "authorization_code"
    );
    let key = Key::new(&account.id, "fixture", Purpose::RefreshToken).expect("scope");
    assert_eq!(
        store
            .get(&key)
            .await
            .expect("stored")
            .expect("refresh")
            .expose_secret(),
        "SYNTHETIC-OAUTH-REFRESH-ONE"
    );
    let mut changed = account.clone();
    if let Auth::Oauth2 { client_id, .. } = &mut changed.auth {
        *client_id = "another-public-client".into();
    }
    let other = Session::new(store.clone());
    assert_eq!(
        other
            .resolve(&changed, false, &CancellationToken::new())
            .await
            .expect_err("client binding"),
        AuthError::OAuth(Error::Reauthenticate)
    );
    assert!(
        requests.try_recv().is_err(),
        "a changed public client must not receive the old refresh token"
    );
    drop(other);
    let cancel = CancellationToken::new();
    let (incoming, outgoing) = tokio::join!(
        session.resolve_automatic(&account, false, &cancel),
        session.resolve_automatic(&account, true, &cancel)
    );
    assert_bearer(&incoming.expect("incoming"), "SYNTHETIC-OAUTH-ROTATED");
    assert_bearer(&outgoing.expect("outgoing"), "SYNTHETIC-OAUTH-ROTATED");
    assert_eq!(
        requests.recv().await.expect("single refresh")["refresh_token"],
        "SYNTHETIC-OAUTH-REFRESH-ONE"
    );
    assert!(
        requests.try_recv().is_err(),
        "shared incoming/outgoing reference refreshes once"
    );
    assert_eq!(
        store
            .get(&key)
            .await
            .expect("durable rotation")
            .expect("refresh")
            .expose_secret(),
        "SYNTHETIC-OAUTH-REFRESH-TWO"
    );
    drop(session);
    let session = Session::new(store.clone());
    assert_bearer(
        &session
            .resolve_automatic(&account, false, &cancel)
            .await
            .expect("restart refresh"),
        "SYNTHETIC-OAUTH-AFTER-RESTART",
    );
    assert_eq!(
        requests.recv().await.expect("restart used rotated token")["refresh_token"],
        "SYNTHETIC-OAUTH-REFRESH-TWO"
    );
    assert_eq!(
        store
            .get(&key)
            .await
            .expect("no replacement")
            .expect("refresh retained")
            .expose_secret(),
        "SYNTHETIC-OAUTH-REFRESH-TWO"
    );
    session.invalidate(&account, false).await;
    assert_eq!(
        session
            .resolve(&account, false, &cancel)
            .await
            .expect_err("revoked"),
        AuthError::OAuth(Error::Reauthenticate)
    );
    assert_eq!(
        requests.recv().await.expect("revocation request")["refresh_token"],
        "SYNTHETIC-OAUTH-REFRESH-TWO"
    );
    assert!(
        store
            .get(&key)
            .await
            .expect("revoked item removed")
            .is_none()
    );
    assert_eq!(
        session
            .resolve(&account, true, &cancel)
            .await
            .expect_err("no retry"),
        AuthError::OAuth(Error::Reauthenticate)
    );
    session
        .accept(&account, false, authorize_private(&provider).await)
        .await
        .expect("foreground reconnection clears revoked state");
    assert_eq!(
        requests.recv().await.expect("new authorization")["grant_type"],
        "authorization_code"
    );
    assert_bearer(
        &session
            .resolve(&account, false, &cancel)
            .await
            .expect("new cached grant"),
        "SYNTHETIC-OAUTH-RECONNECTED",
    );
    let backend = oo7::Keyring::new().await.expect("private lock controller");
    backend.lock().await.expect("lock private keyring");
    assert_eq!(
        session
            .resolve(&account, false, &cancel)
            .await
            .expect_err("cache cannot bypass lock"),
        AuthError::Credential(CredentialError::Locked)
    );
    server.await.expect("fixture");
    println!(
        "Private OAuth: refresh rotation persisted before shared use, restart recovery, locked-cache denial and revoked-credential removal passed."
    );
}

#[tokio::test]
#[ignore = "requires the private GUI/keyring harness"]
async fn gui_loopback_server() {
    portal::run().await;
}

#[tokio::test]
#[ignore = "requires the private bus from experiments/p02-accounts/credentials.sh"]
async fn private_oauth_portal_round_trip() {
    portal::fixture_round_trip().await;
}
