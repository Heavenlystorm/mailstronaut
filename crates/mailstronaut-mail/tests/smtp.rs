//! Synthetic localhost SMTP only: no user account, real recipients or relay.

use std::{path::PathBuf, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use mailstronaut_config::{Endpoint, Sasl, Tls};
use mailstronaut_mail::{
    compose::{Content, Frozen},
    imap::Authentication,
    smtp::{Outcome, Transport},
};
use secrecy::SecretString;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tokio_util::sync::CancellationToken;

const USER: &str = "synthetic-smtp-user";
const PASSWORD: &str = "SYNTHETIC-SMTP-SECRET-NOT-REAL";
#[derive(Clone, Copy)]
enum Mode {
    Accept,
    AcceptOAuth,
    DropAfterData,
    RejectData,
    RejectAuth,
    /// Completes TLS, then never greets, so the client's attempt stays in flight.
    Stall,
}
#[derive(Default)]
struct Observed {
    commands: Vec<String>,
    data: Vec<u8>,
}

fn endpoint(host: &str, port: u16, starttls: bool) -> Endpoint {
    serde_json::from_value(
        serde_json::json!({"host":host,"port":port,"security":if starttls{"starttls"}else{"tls"}}),
    )
    .expect("endpoint")
}
fn roots() -> Tls {
    serde_json::from_value(serde_json::json!({"caFiles":[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/certs/ca.pem")]})).expect("CA")
}
fn message() -> Frozen {
    Content {
        from: "Sender <sender@example.test>".into(),
        to: "to@example.test".into(),
        bcc: "blind@example.test".into(),
        subject: "Synthetic SMTP fixture".into(),
        body: "Café — 日本語\n.leading dot\n".into(),
        ..Content::default()
    }
    .freeze_with_attachments(&[mailstronaut_mail::compose::AttachmentData {
        name: "synthetic-binary.bin".into(),
        bytes: vec![0, 255, 1, 13, 10],
    }])
    .expect("MIME")
}
fn acceptor() -> TlsAcceptor {
    let certificates = rustls_pemfile::certs(&mut include_bytes!("certs/localhost.pem").as_slice())
        .collect::<Result<Vec<_>, _>>()
        .expect("certificates");
    let key =
        rustls_pemfile::private_key(&mut include_bytes!("certs/localhost-key.pem").as_slice())
            .expect("key")
            .expect("private fixture key");
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("versions")
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .expect("server TLS");
    TlsAcceptor::from(Arc::new(config))
}
async fn fixture(starttls: bool, advertise_tls: bool, mode: Mode) -> (u16, JoinHandle<Observed>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback");
    let port = listener.local_addr().expect("address").port();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        connection(socket, starttls, advertise_tls, mode).await
    });
    (port, server)
}
/// The application checks the server first and submits on a second connection, so this
/// keeps accepting until a connection carries DATA.
async fn fixture_until_data(port: u16, mode: Mode) -> (u16, JoinHandle<Observed>) {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("loopback");
    let port = listener.local_addr().expect("address").port();
    let server = tokio::spawn(async move {
        let mut observed = Observed::default();
        while !observed.commands.iter().any(|line| line == "DATA\r\n") {
            let (socket, _) = listener.accept().await.expect("accept");
            let next = connection(socket, false, true, mode).await;
            observed.commands.extend(next.commands);
            observed.data.extend(next.data);
        }
        observed
    });
    (port, server)
}
async fn connection(
    socket: TcpStream,
    starttls: bool,
    advertise_tls: bool,
    mode: Mode,
) -> Observed {
    {
        let mut observed = Observed::default();
        let socket = if starttls {
            let mut socket = BufReader::new(socket);
            socket
                .get_mut()
                .write_all(b"220 localhost synthetic SMTP\r\n")
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
                    return observed;
                }
                observed.commands.push(line.clone());
                if line.starts_with("EHLO ") {
                    let capabilities = if advertise_tls {
                        "250-localhost\r\n250-STARTTLS\r\n250 AUTH PLAIN LOGIN\r\n"
                    } else {
                        "250-localhost\r\n250 AUTH PLAIN LOGIN\r\n"
                    };
                    socket
                        .get_mut()
                        .write_all(capabilities.as_bytes())
                        .await
                        .expect("capabilities");
                } else if line == "STARTTLS\r\n" && advertise_tls {
                    socket
                        .get_mut()
                        .write_all(b"220 Ready for TLS\r\n")
                        .await
                        .expect("upgrade");
                    break socket.into_inner();
                } else {
                    panic!("credentials or mail sent before required TLS")
                }
            }
        } else {
            socket
        };
        match acceptor().accept(socket).await {
            Ok(socket) => serve(socket, observed, !starttls, mode).await,
            Err(_) => observed,
        }
    }
}
async fn read_body(socket: &mut BufReader<TlsStream<TcpStream>>, observed: &mut Observed) {
    loop {
        let mut line = Vec::new();
        assert!(
            socket.read_until(b'\n', &mut line).await.expect("body") > 0,
            "client disconnected before DATA terminator"
        );
        if line == b".\r\n" {
            return;
        }
        if line.starts_with(b"..") {
            line.remove(0);
        }
        observed.data.extend(line);
    }
}
async fn serve(
    socket: TlsStream<TcpStream>,
    mut observed: Observed,
    greeting: bool,
    mode: Mode,
) -> Observed {
    let mut socket = BufReader::new(socket);
    if matches!(mode, Mode::Stall) {
        std::future::pending::<()>().await;
    }
    if greeting {
        socket
            .get_mut()
            .write_all(b"220 localhost synthetic SMTP\r\n")
            .await
            .expect("greeting");
    }
    loop {
        let mut line = String::new();
        let count = match socket.read_line(&mut line).await {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof && line.is_empty() => 0,
            Err(error) => panic!("synthetic command read: {:?}", error.kind()),
        };
        if count == 0 {
            return observed;
        }
        observed.commands.push(line.clone());
        let response = if line.starts_with("EHLO ") {
            if matches!(mode, Mode::AcceptOAuth) {
                "250-localhost\r\n250-8BITMIME\r\n250 AUTH XOAUTH2\r\n"
            } else {
                "250-localhost\r\n250-8BITMIME\r\n250 AUTH PLAIN LOGIN\r\n"
            }
        } else if line.starts_with("AUTH PLAIN ") {
            let encoded = line.trim_end().strip_prefix("AUTH PLAIN ").expect("auth");
            assert_eq!(
                STANDARD.decode(encoded).expect("SASL"),
                format!("\0{USER}\0{PASSWORD}").as_bytes()
            );
            if matches!(mode, Mode::RejectAuth) {
                socket
                    .get_mut()
                    .write_all(b"535 5.7.8 authentication refused\r\n")
                    .await
                    .expect("auth refusal");
                return observed;
            }
            "235 2.7.0 authenticated\r\n"
        } else if line.starts_with("AUTH XOAUTH2 ") {
            assert!(matches!(mode, Mode::AcceptOAuth));
            let encoded = line
                .trim_end()
                .strip_prefix("AUTH XOAUTH2 ")
                .expect("XOAUTH2");
            assert_eq!(
                STANDARD.decode(encoded).expect("SASL"),
                format!("user={USER}\x01auth=Bearer SYNTHETIC-OAUTH-ACCESS-NOT-REAL\x01\x01")
                    .as_bytes()
            );
            "235 2.7.0 authenticated\r\n"
        } else if line.starts_with("MAIL FROM:") || line.starts_with("RCPT TO:") {
            "250 2.1.0 accepted\r\n"
        } else if line == "NOOP\r\n" {
            "250 2.0.0 fine\r\n"
        } else if line == "QUIT\r\n" {
            socket
                .get_mut()
                .write_all(b"221 2.0.0 goodbye\r\n")
                .await
                .expect("QUIT");
            return observed;
        } else if line == "DATA\r\n" {
            socket
                .get_mut()
                .write_all(b"354 Send data\r\n")
                .await
                .expect("DATA");
            read_body(&mut socket, &mut observed).await;
            if !matches!(mode, Mode::DropAfterData) {
                let response = if matches!(mode, Mode::RejectData) {
                    b"451 4.3.0 synthetic temporary rejection\r\n".as_slice()
                } else {
                    b"250 2.0.0 synthetic accepted\r\n".as_slice()
                };
                socket
                    .get_mut()
                    .write_all(response)
                    .await
                    .expect("DATA result");
            }
            return observed;
        } else {
            panic!("unexpected synthetic SMTP command")
        };
        socket
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .expect("response");
    }
}

#[tokio::test]
async fn implicit_and_required_upgrade_preserve_mime_and_blind_envelope() {
    for starttls in [false, true] {
        let (port, server) = fixture(starttls, true, Mode::Accept).await;
        let password = SecretString::from(PASSWORD);
        let transport = Transport::prepare(
            &endpoint("localhost", port, starttls),
            &roots(),
            USER,
            Authentication::Password(&password),
        )
        .await
        .expect("prepare");
        let message = message();
        assert_eq!(
            transport.submit(&message, &CancellationToken::new()).await,
            Outcome::Accepted
        );
        let observed = server.await.expect("server");
        assert_eq!(observed.data, message.original());
        assert!(
            observed
                .commands
                .iter()
                .any(|line| line == "RCPT TO:<blind@example.test>\r\n")
        );
        assert!(!String::from_utf8_lossy(&observed.data).contains("blind@example.test"));
        assert_eq!(
            observed
                .commands
                .iter()
                .filter(|line| line.starts_with("AUTH "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn lost_data_acknowledgment_is_ambiguous_and_explicit_rejection_is_distinct() {
    for (mode, expected) in [
        (Mode::DropAfterData, Outcome::DeliveryUnknown),
        (Mode::RejectData, Outcome::Rejected { temporary: true }),
        (Mode::RejectAuth, Outcome::AuthenticationRequired),
    ] {
        let (port, server) = fixture(false, true, mode).await;
        let password = SecretString::from(PASSWORD);
        let transport = Transport::prepare(
            &endpoint("localhost", port, false),
            &roots(),
            USER,
            Authentication::Password(&password),
        )
        .await
        .expect("prepare");
        assert_eq!(
            transport
                .submit(&message(), &CancellationToken::new())
                .await,
            expected
        );
        let observed = server.await.expect("server");
        assert_eq!(observed.data.is_empty(), matches!(mode, Mode::RejectAuth));
        assert_eq!(
            observed
                .commands
                .iter()
                .filter(|line| line.starts_with("AUTH "))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn missing_starttls_and_invalid_certificates_withhold_credentials_and_mail() {
    for (starttls, host, custom_roots) in [
        (true, "localhost", true),
        (false, "127.0.0.1", true),
        (false, "localhost", false),
    ] {
        let (port, server) = fixture(starttls, false, Mode::Accept).await;
        let password = SecretString::from(PASSWORD);
        let transport = Transport::prepare(
            &endpoint(host, port, starttls),
            &if custom_roots {
                roots()
            } else {
                Tls::default()
            },
            USER,
            Authentication::Password(&password),
        )
        .await
        .expect("prepare");
        let expected = if starttls {
            Outcome::CapabilityUnavailable
        } else {
            Outcome::TlsFailed
        };
        assert_eq!(
            transport
                .submit(&message(), &CancellationToken::new())
                .await,
            expected
        );
        let observed = server.await.expect("server");
        assert_eq!(observed.data, [] as [u8; 0]);
        assert!(
            !observed
                .commands
                .iter()
                .any(|line| line.starts_with("AUTH ") || line.starts_with("MAIL FROM:"))
        );
    }
}

#[tokio::test]
async fn smtp_results_persist_through_the_outbox_and_restart_without_automatic_retry() {
    use mailstronaut_core::{
        store::outbox::{Draft, Phase},
        worker::{Error, Worker},
    };
    for (mode, expected) in [
        (Mode::Accept, Phase::Accepted),
        (Mode::DropAfterData, Phase::DeliveryUnknown),
    ] {
        let root = tempfile::tempdir().expect("disposable outbox");
        let worker = Worker::open(root.path().to_owned()).await.expect("owner");
        let store = worker.handle();
        store.ensure_account("a".into()).await.expect("account");
        let mut draft = Draft::new("a".into(), "sender@example.test".into());
        draft.content.to = "to@example.test".into();
        draft.content.body = "Synthetic durable SMTP intent.".into();
        store
            .save_draft(draft.clone())
            .await
            .expect("durable draft");
        let queued = store
            .queue_draft("a".into(), draft.id, draft.revision, false)
            .await
            .expect("freeze intent");
        let (port, server) = fixture(false, true, mode).await;
        let password = SecretString::from(PASSWORD);
        let transport = Transport::prepare(
            &endpoint("localhost", port, false),
            &roots(),
            USER,
            Authentication::Password(&password),
        )
        .await
        .expect("prepare");
        let submission = store
            .begin_submission("a".into(), queued.id.clone())
            .await
            .expect("intent before network");
        let outcome = transport
            .submit(&submission.message, &CancellationToken::new())
            .await;
        store
            .finish_submission(submission, outcome)
            .await
            .expect("durable outcome");
        let observed = server.await.expect("server");
        assert_eq!(
            observed
                .commands
                .iter()
                .filter(|line| *line == "DATA\r\n")
                .count(),
            1
        );
        worker.shutdown().await.expect("shutdown");
        let worker = Worker::open(root.path().to_owned()).await.expect("restart");
        let store = worker.handle();
        assert_eq!(
            store.outbox(vec!["a".into()]).await.expect("outbox state")[0].phase,
            expected
        );
        assert!(matches!(
            store.begin_submission("a".into(), queued.id.clone()).await,
            Err(Error::Conflict)
        ));
        assert!(matches!(
            store.retry_submission("a".into(), queued.id, false).await,
            Err(Error::Conflict)
        ));
        worker.shutdown().await.expect("shutdown");
    }
}

/// Explicit rendezvous for the private Xvfb/keyring harness. Never contacts a relay.
#[tokio::test]
#[ignore = "requires the private GUI fixture harness"]
async fn gui_loopback_server() {
    let output = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_SMTP_PORT_FILE").expect("explicit rendezvous"),
    );
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    let directory = output
        .parent()
        .expect("fixture directory")
        .canonicalize()
        .expect("existing directory");
    assert!(directory.starts_with(workspace));
    let mode = if std::env::var("MAILSTRONAUT_SMTP_OAUTH").as_deref() == Ok("1") {
        Mode::AcceptOAuth
    } else if std::env::var("MAILSTRONAUT_SMTP_DROP_ACK").as_deref() == Ok("1") {
        Mode::DropAfterData
    } else if std::env::var("MAILSTRONAUT_SMTP_REJECT_DATA").as_deref() == Ok("1") {
        Mode::RejectData
    } else if std::env::var("MAILSTRONAUT_SMTP_REJECT_AUTH").as_deref() == Ok("1") {
        Mode::RejectAuth
    } else if std::env::var("MAILSTRONAUT_SMTP_STALL").as_deref() == Ok("1") {
        Mode::Stall
    } else {
        Mode::Accept
    };
    let bind = std::env::var("MAILSTRONAUT_SMTP_BIND_PORT")
        .map_or(0, |port| port.parse().expect("numeric port"));
    let (port, server) = fixture_until_data(bind, mode).await;
    std::fs::write(output, port.to_string()).expect("rendezvous");
    let observed = tokio::time::timeout(std::time::Duration::from_secs(180), server)
        .await
        .expect("GUI deadline")
        .expect("SMTP fixture");
    assert_eq!(
        observed
            .commands
            .iter()
            .filter(|line| *line == "DATA\r\n")
            .count(),
        1
    );
    assert!(
        observed
            .commands
            .iter()
            .any(|line| line.contains("blind@example.test"))
    );
    assert!(!String::from_utf8_lossy(&observed.data).contains("Bcc:"));
    std::fs::write(directory.join("received.eml"), observed.data)
        .expect("synthetic received original");
    println!("One SMTP DATA received; Bcc recipient was envelope-only.");
}

#[tokio::test]
async fn the_check_signs_in_and_starts_no_mail_transaction() {
    let (port, server) = fixture(false, true, Mode::Accept).await;
    let password = SecretString::from(PASSWORD);
    let transport = Transport::prepare(
        &endpoint("localhost", port, false),
        &roots(),
        USER,
        Authentication::Password(&password),
    )
    .await
    .expect("prepare");
    assert_eq!(transport.check(&CancellationToken::new()).await, Ok(()));
    let observed = server.await.expect("server");
    assert!(
        observed
            .commands
            .iter()
            .any(|line| line.starts_with("AUTH "))
    );
    assert!(observed.commands.iter().any(|line| line == "QUIT\r\n"));
    assert!(!observed.commands.iter().any(|line| {
        line.starts_with("MAIL FROM:") || line.starts_with("RCPT TO:") || line == "DATA\r\n"
    }));
    assert_eq!(observed.data, b"");
}

#[tokio::test]
async fn an_unreachable_server_or_refused_sign_in_is_known_to_have_sent_nothing() {
    let closed = {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("loopback");
        listener.local_addr().expect("address").port()
    };
    let password = SecretString::from(PASSWORD);
    let transport = Transport::prepare(
        &endpoint("localhost", closed, false),
        &roots(),
        USER,
        Authentication::Password(&password),
    )
    .await
    .expect("prepare");
    assert_eq!(
        transport.check(&CancellationToken::new()).await,
        Err(Outcome::Unreachable)
    );
    let (port, server) = fixture(false, true, Mode::RejectAuth).await;
    let transport = Transport::prepare(
        &endpoint("localhost", port, false),
        &roots(),
        USER,
        Authentication::Password(&password),
    )
    .await
    .expect("prepare");
    assert_eq!(
        transport.check(&CancellationToken::new()).await,
        Err(Outcome::AuthenticationRequired)
    );
    assert_eq!(server.await.expect("server").data, b"");
}

#[tokio::test]
async fn an_unreachable_retry_cannot_erase_that_an_earlier_attempt_may_have_delivered() {
    use mailstronaut_core::{
        store::outbox::{Draft, Phase},
        worker::Worker,
    };
    let root = tempfile::tempdir().expect("disposable outbox");
    let worker = Worker::open(root.path().to_owned()).await.expect("owner");
    let store = worker.handle();
    store.ensure_account("a".into()).await.expect("account");
    let mut draft = Draft::new("a".into(), "sender@example.test".into());
    draft.content.to = "to@example.test".into();
    draft.content.body = "Synthetic retry.".into();
    store.save_draft(draft.clone()).await.expect("draft");
    let queued = store
        .queue_draft("a".into(), draft.id, draft.revision, false)
        .await
        .expect("freeze");
    let finish = async |outcome| {
        let submission = store
            .begin_submission("a".into(), queued.id.clone())
            .await
            .expect("attempt");
        store
            .finish_submission(submission, outcome)
            .await
            .expect("durable outcome");
        store.outbox(vec!["a".into()]).await.expect("outbox")[0].clone()
    };
    let entry = finish(Outcome::Unreachable).await;
    assert_eq!(entry.phase, Phase::Failed);
    assert!(!entry.uncertain, "nothing was sent, so no warning is due");
    store
        .retry_submission("a".into(), queued.id.clone(), false)
        .await
        .expect("an ordinary retry needs no consent");
    assert_eq!(
        finish(Outcome::DeliveryUnknown).await.phase,
        Phase::DeliveryUnknown
    );
    store
        .retry_submission("a".into(), queued.id.clone(), true)
        .await
        .expect("explicit consent");
    let entry = finish(Outcome::Unreachable).await;
    assert_eq!(entry.phase, Phase::DeliveryUnknown);
    assert!(
        entry.uncertain,
        "the earlier attempt may still have delivered"
    );
    worker.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn oauth_submission_uses_only_the_advertised_xoauth2_mechanism_over_verified_tls() {
    let (port, server) = fixture(false, true, Mode::AcceptOAuth).await;
    let token = SecretString::from("SYNTHETIC-OAUTH-ACCESS-NOT-REAL");
    let transport = Transport::prepare(
        &endpoint("localhost", port, false),
        &roots(),
        USER,
        Authentication::OAuth2 {
            token: &token,
            mechanism: Sasl::Xoauth2,
        },
    )
    .await
    .expect("prepare");
    let message = message();
    assert_eq!(
        transport.submit(&message, &CancellationToken::new()).await,
        Outcome::Accepted
    );
    let observed = server.await.expect("fixture");
    assert_eq!(observed.data, message.original());
    assert_eq!(
        observed
            .commands
            .iter()
            .filter(|line| line.starts_with("AUTH "))
            .count(),
        1
    );
    assert!(
        !observed
            .commands
            .iter()
            .any(|line| line.starts_with("AUTH PLAIN ") || line.starts_with("AUTH LOGIN"))
    );
}
