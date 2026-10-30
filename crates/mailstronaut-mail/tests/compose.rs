//! Synthetic MIME behavior; no account, keyring or network activity.

use mailstronaut_mail::compose::{Content, Error, Frozen};

fn draft() -> Content {
    Content {
        from: "Expéditeur <sender@example.test>".into(),
        to: "\"Recipient, Synthetic\" <to@example.test>".into(),
        cc: "copy@example.test".into(),
        bcc: "blind@example.test".into(),
        subject: "Café — 日本語".into(),
        body: "Plain body.\nCafé — 日本語\n.line starting with a dot\n".into(),
        in_reply_to: Some("parent@example.test".into()),
        references: vec!["ancestor@example.test".into(), "parent@example.test".into()],
    }
}

#[test]
fn mime_round_trip_preserves_unicode_ancestry_and_hides_bcc() {
    let content = draft();
    let frozen = content.freeze().expect("valid MIME");
    let projection =
        mailstronaut_core::mime::project(frozen.original()).expect("independent MIME decoder");
    assert_eq!(
        projection.headers.subject.as_deref(),
        Some(content.subject.as_str())
    );
    assert_eq!(
        projection.headers.from[0].name.as_deref(),
        Some("Expéditeur")
    );
    assert_eq!(
        projection.headers.to[0].name.as_deref(),
        Some("Recipient, Synthetic")
    );
    assert_eq!(
        projection.headers.message_id.as_deref(),
        Some(frozen.message_id())
    );
    assert_eq!(projection.headers.in_reply_to, ["parent@example.test"]);
    assert_eq!(
        projection.headers.references,
        ["ancestor@example.test", "parent@example.test"]
    );
    assert!(
        projection
            .text
            .as_deref()
            .is_some_and(|body| body.contains("Café — 日本語"))
    );
    assert_eq!(projection.headers.bcc, []);
    assert!(!String::from_utf8_lossy(frozen.original()).contains("blind@example.test"));
    assert_eq!(frozen.envelope().to().len(), 3);
    assert!(
        frozen
            .envelope()
            .to()
            .iter()
            .any(|address| address.to_string() == "blind@example.test")
    );
    let restored = Frozen::restore(
        frozen.message_id().into(),
        frozen.envelope().from().expect("sender").as_ref(),
        frozen
            .envelope()
            .to()
            .iter()
            .map(ToString::to_string)
            .collect(),
        frozen.original().to_vec(),
    )
    .expect("restore frozen outbox record");
    assert_eq!(restored.original(), frozen.original());
    assert_eq!(restored.message_id(), frozen.message_id());
    assert!(!format!("{frozen:?} {content:?}").contains("Plain body"));
}

#[test]
fn malformed_and_injected_headers_are_rejected_and_bcc_only_is_valid() {
    for field in ["sender", "to", "cc", "bcc", "subject"] {
        let mut content = draft();
        let value = "injected\r\nBcc: injected@example.test".into();
        match field {
            "sender" => content.from = value,
            "to" => content.to = value,
            "cc" => content.cc = value,
            "bcc" => content.bcc = value,
            _ => content.subject = value,
        }
        assert!(matches!(content.freeze(), Err(Error::Invalid)));
    }
    let mut content = draft();
    content.references.push("parent\r\nInjected: true".into());
    assert!(matches!(content.freeze(), Err(Error::Invalid)));
    let mut content = draft();
    content.to.clear();
    content.cc.clear();
    let frozen = content.freeze().expect("blind recipients only");
    assert_eq!(frozen.envelope().to().len(), 1);
    content.bcc.clear();
    assert!(matches!(content.freeze(), Err(Error::Invalid)));
}

#[test]
fn typed_multipart_rejects_unsafe_names_and_bounds_encoded_size() {
    use mailstronaut_mail::compose::{AttachmentData, MAX_ATTACHMENT_BYTES, MAX_MESSAGE_BYTES};
    for name in [
        "",
        ".",
        "..",
        "../x",
        "x/y",
        "x\\y",
        "x\r\nInjected: yes",
        "x\0",
    ] {
        assert!(
            draft()
                .freeze_with_attachments(&[AttachmentData {
                    name: name.into(),
                    bytes: vec![0]
                }])
                .is_err()
        );
    }
    let frozen = draft()
        .freeze_with_attachments(&[AttachmentData {
            name: "max.bin".into(),
            bytes: vec![255; MAX_ATTACHMENT_BYTES],
        }])
        .expect("bounded MIME");
    assert!(frozen.original().len() <= MAX_MESSAGE_BYTES);
    assert!(
        !format!(
            "{:?}",
            AttachmentData {
                name: "private filename".into(),
                bytes: vec![0]
            }
        )
        .contains("private filename")
    );
}
