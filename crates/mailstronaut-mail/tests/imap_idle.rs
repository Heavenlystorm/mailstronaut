//! IDLE against a scripted loopback server: what the client sends, and when it stops.
//! Mail is never read here; the sync cycle owns that.

use std::{path::PathBuf, sync::Arc, time::Duration};

use mailstronaut_config::{Endpoint, Tls};
use mailstronaut_mail::imap::{Authentication, Connection, Error, OPERATION_TIMEOUT, Wake};
use secrecy::SecretString;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tokio_util::sync::CancellationToken;

const SECRET: &str = "SYNTHETIC-AUTH-SECRET-NOT-REAL";
/// Far longer than any step needs, so that passing it means the client failed to act.
const BOUND: Duration = Duration::from_secs(10);

/// What the server does with each IDLE, in order.
#[derive(Clone, Copy)]
enum Script {
    /// Report new mail, then wait for DONE.
    Exists,
    /// Keep sending keepalives until DONE: they must not postpone the renewal.
    Keepalive,
    /// Close the connection while the client waits.
    Drop,
    /// Stay silent, so that only the client can end this.
    Silent,
    /// Read DONE and never answer, like a peer that vanished without closing.
    Vanish,
}

fn acceptor() -> TlsAcceptor {
    let certs = rustls_pemfile::certs(&mut include_bytes!("certs/localhost.pem").as_slice())
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

async fn send(socket: &mut BufReader<TlsStream<TcpStream>>, text: &str) -> bool {
    socket.get_mut().write_all(text.as_bytes()).await.is_ok()
}

async fn read_line(socket: &mut BufReader<TlsStream<TcpStream>>) -> Option<String> {
    let mut line = String::new();
    match socket.read_line(&mut line).await {
        Ok(read) if read > 0 => Some(line.trim_end().to_owned()),
        _ => None,
    }
}

/// Answer one IDLE from `+ idling` to its tagged completion; false once disconnected.
async fn idle(socket: &mut BufReader<TlsStream<TcpStream>>, tag: &str, script: Script) -> bool {
    if !send(socket, "+ idling\r\n").await {
        return false;
    }
    let done = match script {
        Script::Exists => {
            send(socket, "* 5 EXISTS\r\n").await;
            read_line(socket).await
        }
        Script::Keepalive => loop {
            send(socket, "* OK Still here\r\n").await;
            tokio::select! {
                line = read_line(socket) => break line,
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        },
        Script::Drop => return false,
        Script::Silent => read_line(socket).await,
        Script::Vanish => {
            read_line(socket).await;
            core::future::pending::<()>().await;
            None
        }
    };
    match done {
        Some(line) if line == "DONE" => send(socket, &format!("{tag} OK Idle completed\r\n")).await,
        _ => false,
    }
}

async fn server(capability: &'static str, scripts: Vec<Script>) -> (u16, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("loopback listener");
    let port = listener.local_addr().expect("address").port();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        let socket = acceptor().accept(socket).await.expect("fixture TLS");
        let mut socket = BufReader::new(socket);
        let mut commands = Vec::new();
        let mut scripts = scripts.into_iter();
        send(&mut socket, "* OK Synthetic fixture\r\n").await;
        while let Some(line) = read_line(&mut socket).await {
            let (tag, command) = line.split_once(' ').expect("tagged command");
            let (tag, command) = (tag.to_owned(), command.to_owned());
            commands.push(command.clone());
            let reply = if command == "CAPABILITY" {
                format!(
                    "* CAPABILITY IMAP4rev1 {capability} AUTH=PLAIN\r\n{tag} OK capabilities\r\n"
                )
            } else if command.starts_with("AUTHENTICATE ") {
                send(&mut socket, "+ \r\n").await;
                read_line(&mut socket).await.expect("SASL response");
                format!("{tag} OK authenticated\r\n")
            } else if command.starts_with("EXAMINE ") {
                format!(
                    "* 4 EXISTS\r\n* OK [UIDVALIDITY 42] generation\r\n* OK [UIDNEXT 1005] next\r\n{tag} OK [READ-ONLY] examined\r\n"
                )
            } else if command == "IDLE" {
                let script = scripts.next().expect("scripted IDLE");
                if idle(&mut socket, &tag, script).await {
                    commands.push("DONE".into());
                    continue;
                }
                return commands;
            } else {
                format!("{tag} BAD unexpected\r\n")
            };
            send(&mut socket, &reply).await;
        }
        commands
    });
    (port, task)
}

async fn connect(port: u16, cancellation: &CancellationToken) -> Connection {
    let endpoint: Endpoint = serde_json::from_value(
        serde_json::json!({"host":"localhost","port":port,"security":"tls"}),
    )
    .expect("endpoint");
    let ca = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/certs/ca.pem");
    let trust: Tls = serde_json::from_value(serde_json::json!({"caFiles":[ca]})).expect("trust");
    let password = SecretString::from(SECRET);
    Connection::connect(
        &endpoint,
        &trust,
        "synthetic-user",
        Authentication::Password(&password),
        cancellation,
    )
    .await
    .expect("validated login")
}

#[tokio::test]
async fn new_mail_ends_idle_with_done_and_the_watch_never_touches_a_message() {
    let (port, server) = server("IDLE", vec![Script::Exists]).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(port, &cancellation).await;
    assert!(connection.supports("IDLE"));
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("examine");
    let wake = tokio::time::timeout(
        BOUND,
        connection.idle(Duration::from_secs(600), &cancellation),
    )
    .await
    .expect("a reported change ends the wait at once")
    .expect("idle");
    assert_eq!(wake, Wake::Changed);
    drop(connection);
    let commands = server.await.expect("server");
    assert_eq!(
        commands[commands.len() - 3..],
        ["EXAMINE \"INBOX\"", "IDLE", "DONE"]
    );
    for command in &commands {
        for writes in ["SELECT", "FETCH", "STORE", "EXPUNGE", "DELETE"] {
            assert!(!command.contains(writes), "{command}");
        }
    }
}

#[tokio::test]
async fn keepalives_do_not_postpone_renewal_and_idle_can_be_reissued() {
    let (port, server) = server("IDLE", vec![Script::Keepalive, Script::Exists]).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(port, &cancellation).await;
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("examine");
    let renew = Duration::from_millis(200);
    let wake = tokio::time::timeout(BOUND, connection.idle(renew, &cancellation))
        .await
        .expect("keepalives every 20ms must not keep a 200ms period open")
        .expect("idle");
    assert_eq!(wake, Wake::Renew, "keepalives are not changes");
    // The same session carries on after DONE.
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("reselect");
    assert_eq!(
        connection
            .idle(Duration::from_secs(600), &cancellation)
            .await
            .expect("second idle"),
        Wake::Changed
    );
    drop(connection);
    let commands = server.await.expect("server");
    assert_eq!(
        commands.iter().filter(|command| *command == "DONE").count(),
        2
    );
}

#[tokio::test]
async fn a_silent_server_still_reaches_renewal_with_done() {
    let (port, server) = server("IDLE", vec![Script::Silent]).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(port, &cancellation).await;
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("examine");
    let wake = tokio::time::timeout(
        BOUND,
        connection.idle(Duration::from_millis(100), &cancellation),
    )
    .await
    .expect("bounded")
    .expect("idle");
    assert_eq!(wake, Wake::Renew);
    drop(connection);
    assert!(server.await.expect("server").contains(&"DONE".to_owned()));
}

#[tokio::test]
async fn a_dropped_connection_is_an_error_and_retires_the_session() {
    let (port, server) = server("IDLE", vec![Script::Drop]).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(port, &cancellation).await;
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("examine");
    let error = tokio::time::timeout(
        BOUND,
        connection.idle(Duration::from_secs(600), &cancellation),
    )
    .await
    .expect("a closed connection is noticed at once")
    .expect_err("not a change");
    assert_ne!(error, Error::Cancelled);
    assert_eq!(
        connection.examine("INBOX", &cancellation).await,
        Err(Error::Disconnected),
        "a failed session is never reused"
    );
    server.await.expect("server");
}

/// What a suspended laptop or a dropped NAT mapping looks like: the socket stays open
/// and nothing ever arrives, so only renewal can discover it. Takes one operation budget.
#[tokio::test]
async fn a_peer_that_vanished_is_found_at_renewal_and_the_session_is_retired() {
    let (port, server) = server("IDLE", vec![Script::Vanish]).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(port, &cancellation).await;
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("examine");
    let error = tokio::time::timeout(
        OPERATION_TIMEOUT + BOUND,
        connection.idle(Duration::from_millis(50), &cancellation),
    )
    .await
    .expect("DONE has the same budget as any other operation")
    .expect_err("no completion ever arrives");
    assert_eq!(error, Error::TimedOut);
    assert_eq!(
        connection.examine("INBOX", &cancellation).await,
        Err(Error::Disconnected)
    );
    server.abort();
}

#[tokio::test]
async fn cancelling_ends_idle_promptly_and_retires_the_session() {
    let (port, server) = server("IDLE", vec![Script::Silent]).await;
    let cancellation = CancellationToken::new();
    let mut connection = connect(port, &cancellation).await;
    connection
        .examine("INBOX", &cancellation)
        .await
        .expect("examine");
    let stop = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
    });
    let error = tokio::time::timeout(
        BOUND,
        connection.idle(Duration::from_secs(600), &cancellation),
    )
    .await
    .expect("cancellation does not wait for the server")
    .expect_err("cancelled");
    assert_eq!(error, Error::Cancelled);
    assert_eq!(
        connection.examine("INBOX", &CancellationToken::new()).await,
        Err(Error::Disconnected)
    );
    drop(connection);
    server.await.expect("server");
}

#[tokio::test]
async fn a_server_without_idle_is_visible_before_anything_is_sent() {
    let (port, server) = server("UIDPLUS", Vec::new()).await;
    let cancellation = CancellationToken::new();
    let connection = connect(port, &cancellation).await;
    assert!(!connection.supports("IDLE"));
    drop(connection);
    let commands = server.await.expect("server");
    assert!(!commands.contains(&"IDLE".to_owned()));
}
