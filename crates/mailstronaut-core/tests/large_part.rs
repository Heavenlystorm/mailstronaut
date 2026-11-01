//! Downloading one attachment from a real server, for the private acceptance harness.

use core::{fmt::Write as _, num::NonZeroU32};
use std::path::PathBuf;

use mailstronaut_config::{Endpoint, Tls};
use mailstronaut_core::outline;
use mailstronaut_mail::imap::{Authentication, Connection};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

fn variable(name: &str) -> String {
    std::env::var(name).expect("explicit harness setting")
}

#[tokio::test]
#[ignore = "explicit private harness only; needs a real server with synthetic mail"]
async fn a_large_part_downloads_intact_from_a_real_server() {
    let cancel = CancellationToken::new();
    let endpoint: Endpoint = serde_json::from_value(serde_json::json!({
        "host": "localhost",
        "port": variable("MAILSTRONAUT_PART_PORT").parse::<u16>().expect("port"),
        "security": "tls",
    }))
    .expect("endpoint");
    let ca =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mailstronaut-mail/tests/certs/ca.pem");
    let trust: Tls = serde_json::from_value(serde_json::json!({"caFiles":[ca]})).expect("trust");
    let password = SecretString::from(variable("MAILSTRONAUT_PART_PASSWORD"));
    let mut connection = Connection::connect_for_download(
        &endpoint,
        &trust,
        &variable("MAILSTRONAUT_PART_USER"),
        Authentication::Password(&password),
        &cancel,
    )
    .await
    .expect("login");
    connection.examine("INBOX", &cancel).await.expect("examine");
    let uid =
        NonZeroU32::new(variable("MAILSTRONAUT_PART_UID").parse().expect("UID")).expect("UID");
    let (mime, content) = connection
        .part(uid, "2", &cancel)
        .await
        .expect("download")
        .expect("present");
    let decoded = outline::decode_part(&mime, &content).expect("decoded");
    assert_eq!(
        hex(&Sha256::digest(&decoded)),
        variable("MAILSTRONAUT_PART_DIGEST"),
        "the saved file must be exactly what was attached"
    );
    println!(
        "OK: the {} MiB attachment downloaded from the server byte for byte",
        decoded.len() / 1024 / 1024
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        _ = write!(text, "{byte:02x}");
        text
    })
}
