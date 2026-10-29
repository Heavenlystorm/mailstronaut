//! Synthetic loopback IMAP fixtures. No user account, server or keyring is contacted.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use mailstronaut_config::{Endpoint, Sasl, Tls};
use mailstronaut_mail::imap::{Authentication, Connection, Error};
use secrecy::SecretString;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

const SECRET: &str = "SYNTHETIC-AUTH-SECRET-NOT-REAL";
const USER: &str = "synthetic-user";
const ORIGINAL: &[u8] = b"From: Synthetic Sender <sender@example.test>\r\nTo: reader@example.test\r\nMessage-ID: <fixture@example.test>\r\nSubject: Offline fixture\r\n\r\nSynthetic body for offline reading.\r\n";

#[tokio::test]
#[ignore = "explicit private-bus GUI harness only; waits for its synthetic client"]
async fn gui_loopback_server() {
    let output = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_GUI_PORT_FILE").expect("explicit GUI fixture rendezvous"),
    );
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    assert!(
        output
            .parent()
            .expect("fixture root")
            .canonicalize()
            .expect("existing project fixture")
            .starts_with(workspace)
    );
    let (port, server) = fixture(false, true, false).await;
    std::fs::write(output, port.to_string()).expect("fixture port");
    let commands = tokio::time::timeout(std::time::Duration::from_secs(120), server)
        .await
        .expect("GUI fixture timeout")
        .expect("server");
    assert!(
        commands
            .iter()
            .any(|command| command.contains("BODY.PEEK[]"))
    );
    assert!(
        !commands
            .iter()
            .any(|command| command.contains("STORE") || command.contains("EXPUNGE"))
    );
}

fn endpoint(host: &str, port: u16, starttls: bool) -> Endpoint {
    serde_json::from_value(serde_json::json!({"host":host,"port":port,"security":if starttls {"starttls"} else {"tls"}})).expect("endpoint")
}
fn trust(custom: bool) -> Tls {
    let ca: Vec<PathBuf> = if custom {
        vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/certs/ca.pem")]
    } else {
        Vec::new()
    };
    serde_json::from_value(serde_json::json!({"caFiles":ca})).expect("trust roots")
}
#[derive(Clone, Copy)]
enum Certificate {
    Current,
    Expired,
}

fn acceptor(certificate: Certificate) -> TlsAcceptor {
    let mut certificate: &[u8] = if matches!(certificate, Certificate::Expired) {
        include_bytes!("certs/expired.pem")
    } else {
        include_bytes!("certs/localhost.pem")
    };
    let certs = rustls_pemfile::certs(&mut certificate)
        .collect::<Result<Vec<_>, _>>()
        .expect("fixture certificate");
    let key =
        rustls_pemfile::private_key(&mut include_bytes!("certs/localhost-key.pem").as_slice())
            .expect("fixture key")
            .expect("present key");
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("TLS versions")
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .expect("test server config");
    TlsAcceptor::from(Arc::new(config))
}

async fn fixture(
    starttls: bool,
    advertise_upgrade: bool,
    oauth: bool,
) -> (u16, JoinHandle<Vec<String>>) {
    fixture_certificate(starttls, advertise_upgrade, oauth, Certificate::Current).await
}

async fn fixture_certificate(
    starttls: bool,
    advertise_upgrade: bool,
    oauth: bool,
    certificate: Certificate,
) -> (u16, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("loopback listener");
    let port = listener.local_addr().expect("address").port();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        let mut commands = Vec::new();
        let socket = if starttls {
            let mut socket = BufReader::new(socket);
            socket
                .get_mut()
                .write_all(b"* OK Synthetic fixture\r\n")
                .await
                .expect("greeting");
            loop {
                let mut line = String::new();
                if socket
                    .read_line(&mut line)
                    .await
                    .expect("plaintext command")
                    == 0
                {
                    return commands;
                }
                commands.push(line.clone());
                let (tag, command) = line.trim_end().split_once(' ').expect("tagged command");
                if command == "CAPABILITY" {
                    let upgrade = if advertise_upgrade { " STARTTLS" } else { "" };
                    socket
                        .get_mut()
                        .write_all(
                            format!("* CAPABILITY IMAP4rev1{upgrade}\r\n{tag} OK capabilities\r\n")
                                .as_bytes(),
                        )
                        .await
                        .expect("capabilities");
                } else if command == "STARTTLS" && advertise_upgrade {
                    socket
                        .get_mut()
                        .write_all(format!("{tag} OK begin TLS\r\n").as_bytes())
                        .await
                        .expect("upgrade");
                    break socket.into_inner();
                } else {
                    panic!("unexpected plaintext command; authentication must await TLS");
                }
            }
        } else {
            socket
        };
        match acceptor(certificate).accept(socket).await {
            Ok(socket) => serve(socket, &mut commands, !starttls, oauth).await,
            Err(_) => commands,
        }
    });
    (port, task)
}

async fn serve(
    socket: tokio_rustls::server::TlsStream<TcpStream>,
    commands: &mut Vec<String>,
    greeting: bool,
    oauth: bool,
) -> Vec<String> {
    let mut socket = BufReader::new(socket);
    if greeting {
        socket
            .get_mut()
            .write_all(b"* OK Synthetic fixture\r\n")
            .await
            .expect("greeting");
    }
    loop {
        let mut line = String::new();
        let read = match socket.read_line(&mut line).await {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof && line.is_empty() => 0,
            Err(error) => panic!("synthetic TLS command read failed: {:?}", error.kind()),
        };
        if read == 0 {
            return core::mem::take(commands);
        }
        commands.push(line.clone());
        let (tag, command) = line.trim_end().split_once(' ').expect("tagged command");
        if command == "CAPABILITY" {
            let auth = if oauth {
                "AUTH=XOAUTH2 AUTH=OAUTHBEARER"
            } else {
                "AUTH=PLAIN"
            };
            socket
                .get_mut()
                .write_all(
                    format!(
                        "* CAPABILITY IMAP4rev1 IDLE UIDPLUS {auth}\r\n{tag} OK capabilities\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .expect("capabilities");
        } else if command.starts_with("AUTHENTICATE ") {
            socket
                .get_mut()
                .write_all(b"+ \r\n")
                .await
                .expect("challenge");
            let mut response = String::new();
            socket
                .read_line(&mut response)
                .await
                .expect("SASL response");
            let decoded = STANDARD.decode(response.trim_end()).expect("SASL base64");
            let expected = if command.ends_with("XOAUTH2") {
                format!("user={USER}\x01auth=Bearer {SECRET}\x01\x01")
            } else if command.ends_with("OAUTHBEARER") {
                format!("n,a={USER},\x01auth=Bearer {SECRET}\x01\x01")
            } else {
                format!("\0{USER}\0{SECRET}")
            };
            assert_eq!(
                decoded,
                expected.as_bytes(),
                "SASL exchange fixture mismatch"
            );
            socket
                .get_mut()
                .write_all(format!("{tag} OK authenticated\r\n").as_bytes())
                .await
                .expect("authenticated");
        } else if command.starts_with("SELECT ") {
            socket.get_mut().write_all(format!("* 2 EXISTS\r\n* OK [UIDVALIDITY 42] generation\r\n* OK [UIDNEXT 1004] next\r\n{tag} OK [READ-WRITE] selected\r\n").as_bytes()).await.expect("SELECT");
        } else if command.starts_with("LIST ") {
            socket.get_mut().write_all(format!("* LIST (\\HasNoChildren) \"/\" INBOX\r\n* LIST (\\Archive) \"/\" \"&AMk-tudes\"\r\n{tag} OK folders\r\n").as_bytes()).await.expect("LIST");
        } else if command.starts_with("UID SEARCH ") {
            socket
                .get_mut()
                .write_all(format!("* SEARCH 1003 1001\r\n{tag} OK search\r\n").as_bytes())
                .await
                .expect("SEARCH");
        } else if command.starts_with("UID FETCH ") {
            fetch_response(socket.get_mut(), tag, command).await;
        } else {
            panic!("unexpected synthetic IMAP command");
        }
    }
}

async fn fetch_response(
    socket: &mut tokio_rustls::server::TlsStream<TcpStream>,
    tag: &str,
    command: &str,
) {
    let requested = command.split_whitespace().nth(2).expect("UID set");
    let peek = command.contains("BODY.PEEK[]");
    assert!(!command.contains("BODY[]"), "a preview must never set Seen");
    for (sequence, uid, flags) in [(1, "1001", ""), (2, "1003", "\\Seen \\Flagged")] {
        if !requested.split(',').any(|id| id == uid) {
            continue;
        }
        let prefix = format!(
            "* {sequence} FETCH (UID {uid} FLAGS ({flags}) RFC822.SIZE {} INTERNALDATE \"29-Oct-2026 10:00:00 +0000\"",
            ORIGINAL.len()
        );
        let response = if peek {
            format!(
                "{prefix} BODY[] {{{}}}\r\n{} )\r\n",
                ORIGINAL.len(),
                std::str::from_utf8(ORIGINAL).expect("ASCII fixture")
            )
        } else {
            format!("{prefix})\r\n")
        };
        socket
            .write_all(response.as_bytes())
            .await
            .expect("FETCH data");
    }
    socket
        .write_all(format!("{tag} OK fetch\r\n").as_bytes())
        .await
        .expect("FETCH done");
}

#[tokio::test]
async fn uid_search_and_peek_preserve_wire_names_gaps_and_unread_flags() {
    use core::num::NonZeroU32;
    let (port, server) = fixture(false, true, false).await;
    let cancellation = CancellationToken::new();
    let password = SecretString::from(SECRET);
    let mut connection = Connection::connect(
        &endpoint("localhost", port, false),
        &trust(true),
        USER,
        Authentication::Password(&password),
        &cancellation,
    )
    .await
    .expect("login");
    let folders = connection.folders(&cancellation).await.expect("LIST");
    assert_eq!(folders[1].wire_name, "&AMk-tudes");
    assert!(
        folders[1]
            .attributes
            .iter()
            .any(|attribute| attribute == "\\Archive")
    );
    connection
        .select("INBOX", &cancellation)
        .await
        .expect("select");
    let first = NonZeroU32::new(1000).expect("first");
    let last = NonZeroU32::new(1003).expect("last");
    let uids = connection
        .search(first, last, None, &cancellation)
        .await
        .expect("UID SEARCH");
    assert_eq!(
        uids.iter().map(|uid| uid.get()).collect::<Vec<_>>(),
        [1001, 1003]
    );
    let metadata = connection
        .metadata(&uids, &cancellation)
        .await
        .expect("metadata");
    assert!(!metadata[0].seen);
    assert!(metadata[1].seen && metadata[1].starred);
    let body = connection
        .peek(&metadata[0], &cancellation)
        .await
        .expect("PEEK")
        .expect("present");
    assert_eq!(body.original, ORIGINAL);
    assert_eq!(
        body.metadata.uid.get(),
        1001,
        "sequence 1 is not the identity"
    );
    assert!(!body.metadata.seen);
    let mut oversized = metadata[0].clone();
    oversized.size = 8 * 1024 * 1024 + 1;
    assert!(matches!(
        connection.peek(&oversized, &cancellation).await,
        Err(Error::ResponseLimit)
    ));
    assert_eq!(
        connection
            .metadata(&uids, &cancellation)
            .await
            .expect("no I/O on size rejection"),
        metadata
    );
    cancellation.cancel();
    assert_eq!(
        connection
            .select("INBOX", &cancellation)
            .await
            .expect_err("cancel"),
        Error::Cancelled
    );
    assert_eq!(
        connection
            .select("INBOX", &CancellationToken::new())
            .await
            .expect_err("retired session"),
        Error::Disconnected
    );
    drop(connection);
    let commands = server.await.expect("fixture");
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.contains("BODY.PEEK[]"))
            .count(),
        1
    );
    assert!(
        !commands
            .iter()
            .any(|command| command.contains("STORE") || command.contains("EXPUNGE"))
    );
}

#[tokio::test]
async fn a_live_tls_handshake_can_be_cancelled_without_sending_credentials() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback");
    let port = listener.local_addr().expect("address").port();
    let cancellation = CancellationToken::new();
    let password = SecretString::from(SECRET);
    let endpoint = endpoint("localhost", port, false);
    let trust = trust(true);
    let connecting = Connection::connect(
        &endpoint,
        &trust,
        USER,
        Authentication::Password(&password),
        &cancellation,
    );
    let accepted = async {
        let (socket, _) = listener.accept().await.expect("accepted live handshake");
        cancellation.cancel();
        socket
    };
    let (result, _socket) = tokio::join!(connecting, accepted);
    assert!(matches!(result, Err(Error::Cancelled)));
}

#[tokio::test]
async fn sync_pages_commit_once_and_remain_readable_after_network_and_worker_restart() {
    use mailstronaut_core::{sync, worker::Worker};
    let root = tempfile::tempdir().expect("disposable mailbox");
    let worker = Worker::open(root.path().to_owned())
        .await
        .expect("store worker");
    let store = worker.handle();
    let (port, server) = fixture(false, true, false).await;
    let cancel = CancellationToken::new();
    let password = SecretString::from(SECRET);
    let mut connection = Connection::connect(
        &endpoint("localhost", port, false),
        &trust(true),
        USER,
        Authentication::Password(&password),
        &cancel,
    )
    .await
    .expect("TLS login");
    let request = sync::Request {
        account: "synthetic",
        folder: "INBOX",
        inbox: true,
        before: None,
        earliest: 0,
        max_original: 2 * 1024 * 1024,
    };
    let first = sync::page(&store, &mut connection, &request, &cancel)
        .await
        .expect("first sync page");
    assert_eq!(first.downloaded, 2);
    assert_eq!(
        first.next_before.map(|position| position.before.get()),
        Some(876)
    );
    let again = sync::page(&store, &mut connection, &request, &cancel)
        .await
        .expect("same page after reconnect-equivalent restart");
    assert_eq!((again.downloaded, again.refreshed), (0, 2));
    let rows = store
        .inbox(vec!["synthetic".into()], None, 100)
        .await
        .expect("inbox");
    assert_eq!(
        rows.len(),
        1,
        "exact Message-ID/raw copies share canonical identity"
    );
    assert!(!rows[0].seen, "the unread UID copy remains unread");
    let id = rows[0].id;
    drop(connection);
    let commands = server.await.expect("server");
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.contains("BODY.PEEK[]"))
            .count(),
        2,
        "cached messages must not be fetched again"
    );
    worker.shutdown().await.expect("shutdown");
    let worker = Worker::open(root.path().to_owned())
        .await
        .expect("restart offline");
    let body = worker
        .handle()
        .message("synthetic".into(), id)
        .await
        .expect("verified original offline");
    assert!(
        body.text
            .as_deref()
            .is_some_and(|body| body.contains("Synthetic body for offline reading."))
    );
    worker.shutdown().await.expect("shutdown restart");
}

#[tokio::test]
async fn paging_cursors_from_an_old_uid_generation_restart_at_the_newest_mail() {
    use mailstronaut_core::{sync, worker::Worker};
    let root = tempfile::tempdir().expect("disposable mailbox");
    let worker = Worker::open(root.path().to_owned())
        .await
        .expect("store worker");
    let (port, server) = fixture(false, true, false).await;
    let cancel = CancellationToken::new();
    let password = SecretString::from(SECRET);
    let mut connection = Connection::connect(
        &endpoint("localhost", port, false),
        &trust(true),
        USER,
        Authentication::Password(&password),
        &cancel,
    )
    .await
    .expect("TLS login");
    let request = sync::Request {
        account: "synthetic",
        folder: "INBOX",
        inbox: true,
        before: None,
        earliest: 0,
        max_original: 2 * 1024 * 1024,
    };
    let first = sync::page(&worker.handle(), &mut connection, &request, &cancel)
        .await
        .expect("first page");
    let mut stale = first.next_before.expect("more pages");
    stale.uid_validity = core::num::NonZeroU32::new(1).expect("old generation");
    stale.before = core::num::NonZeroU32::new(2).expect("old cursor would omit all newer mail");
    let request = sync::Request {
        before: Some(stale),
        ..request
    };
    let recovered = sync::page(&worker.handle(), &mut connection, &request, &cancel)
        .await
        .expect("restart after generation change");
    assert_eq!(recovered.refreshed, 2);
    assert_eq!(recovered.next_before, first.next_before);
    drop(connection);
    let commands = server.await.expect("server");
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.contains("UID SEARCH UID 876:1003"))
            .count(),
        2,
        "stale UID cursor must not constrain the new namespace"
    );
    worker.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn implicit_and_starttls_authenticate_and_select_only_after_tls_validation() {
    for starttls in [false, true] {
        let (port, server) = fixture(starttls, true, false).await;
        let cancellation = CancellationToken::new();
        let password = SecretString::from(SECRET);
        let mut connection = Connection::connect(
            &endpoint("localhost", port, starttls),
            &trust(true),
            USER,
            Authentication::Password(&password),
            &cancellation,
        )
        .await
        .expect("validated TLS login");
        assert!(connection.supports("IDLE"));
        assert_eq!(
            connection
                .select("INBOX", &cancellation)
                .await
                .expect("SELECT")
                .uid_validity,
            Some(42)
        );
        drop(connection);
        let commands = server.await.expect("fixture");
        assert_eq!(
            commands
                .iter()
                .filter(|line| line.contains("AUTHENTICATE"))
                .count(),
            1
        );
        assert_eq!(
            commands
                .iter()
                .filter(|line| line.contains("STARTTLS"))
                .count(),
            usize::from(starttls)
        );
    }
}

#[tokio::test]
async fn untrusted_expired_and_wrong_hostname_certificates_never_receive_credentials() {
    for (host, trusted, expired) in [
        ("localhost", false, false),
        ("127.0.0.1", true, false),
        ("localhost", true, true),
    ] {
        let (port, server) = fixture_certificate(
            false,
            true,
            false,
            if expired {
                Certificate::Expired
            } else {
                Certificate::Current
            },
        )
        .await;
        let password = SecretString::from(SECRET);
        let result = Connection::connect(
            &endpoint(host, port, false),
            &trust(trusted),
            USER,
            Authentication::Password(&password),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, Err(Error::Tls)));
        assert_eq!(server.await.expect("fixture"), Vec::<String>::new());
    }
}

#[tokio::test]
async fn missing_starttls_and_unadvertised_oauth_fail_before_authentication() {
    let (port, server) = fixture(true, false, false).await;
    let password = SecretString::from(SECRET);
    let result = Connection::connect(
        &endpoint("localhost", port, true),
        &trust(true),
        USER,
        Authentication::Password(&password),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(Error::StarttlsRequired)));
    assert!(
        server
            .await
            .expect("fixture")
            .iter()
            .all(|line| line.contains("CAPABILITY"))
    );
    let (port, server) = fixture(false, true, false).await;
    let result = Connection::connect(
        &endpoint("localhost", port, false),
        &trust(true),
        USER,
        Authentication::OAuth2 {
            token: &password,
            mechanism: Sasl::Xoauth2,
        },
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(Error::AuthenticationUnavailable)));
    assert!(
        server
            .await
            .expect("fixture")
            .iter()
            .all(|line| !line.contains("AUTHENTICATE"))
    );
}

#[tokio::test]
async fn advertised_oauth_mechanisms_use_the_library_sasl_exchange() {
    for mechanism in [Sasl::Xoauth2, Sasl::Oauthbearer] {
        let (port, server) = fixture(false, true, true).await;
        let token = SecretString::from(SECRET);
        let connection = Connection::connect(
            &endpoint("localhost", port, false),
            &trust(true),
            USER,
            Authentication::OAuth2 {
                token: &token,
                mechanism,
            },
            &CancellationToken::new(),
        )
        .await
        .expect("SASL fixture");
        drop(connection);
        server.await.expect("fixture");
    }
}

struct Observe;
static OBSERVER: Observe = Observe;
static OBSERVED: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_RECORDS: AtomicUsize = AtomicUsize::new(0);
impl log::Log for Observe {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record.target().starts_with("async_imap") {
            PROTOCOL_RECORDS.fetch_add(1, Ordering::Relaxed);
        }
        let text = record.args().to_string();
        assert!(
            !text.contains(SECRET),
            "wire logger exposed the synthetic secret"
        );
        if record.target() == "mailstronaut-observer-control" {
            OBSERVED.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn flush(&self) {}
}
#[tokio::test]
async fn protocol_wire_traces_cannot_be_enabled_by_a_verbose_logger() {
    log::set_logger(&OBSERVER).expect("single observation test");
    log::set_max_level(log::LevelFilter::Trace);
    log::info!(target:"mailstronaut-observer-control","positive logging observer control");
    let (port, server) = fixture(false, true, false).await;
    let password = SecretString::from(SECRET);
    let connection = Connection::connect(
        &endpoint("localhost", port, false),
        &trust(true),
        USER,
        Authentication::Password(&password),
        &CancellationToken::new(),
    )
    .await
    .expect("login");
    drop(connection);
    server.await.expect("fixture");
    assert_eq!(OBSERVED.load(Ordering::Relaxed), 1);
    assert_eq!(PROTOCOL_RECORDS.load(Ordering::Relaxed), 0);
}
