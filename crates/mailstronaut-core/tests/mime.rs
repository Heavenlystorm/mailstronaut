//! Synthetic MIME projection behavior.

#[cfg(test)]
#[expect(
    clippy::needless_return,
    reason = "Synthetic fixtures fail the test on unexpected parsing or missing data; return lints conflict."
)]
mod tests {
    use core::ops::Range;

    use mailstronaut_core::mime::{self, Error};

    #[test]
    fn alternatives_headers_and_encodings() {
        let raw = b"From: =?UTF-8?Q?Ren=C3=A9?= <rene@example.test>\r\nTo: Team: a@example.test, b@example.test;\r\nSubject: =?UTF-8?Q?Caf=C3=A9?=\r\nMessage-ID: <one@example.test>\r\nReferences: <old@example.test> <older@example.test>\r\nIn-Reply-To: <old@example.test>\r\nDate: Tue, 27 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCaf=E9  plain\r\n--inner\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nPGI+SFRNTDwvYj4=\r\n--inner--\r\n--outer--\r\n";
        let result = mime::project(raw).expect("synthetic message");
        assert_eq!(result.text.as_deref(), Some("Caf\u{e9}  plain"));
        assert_eq!(result.html.as_deref(), Some("<b>HTML</b>"));
        assert_eq!(
            result
                .preview
                .as_ref()
                .map(|preview| return preview.text.as_str()),
            Some("Caf\u{e9} plain")
        );
        assert_eq!(result.headers.from[0].name.as_deref(), Some("Ren\u{e9}"));
        assert_eq!(result.headers.to.len(), 2);
        assert_eq!(result.headers.subject.as_deref(), Some("Caf\u{e9}"));
        assert_eq!(result.headers.references.len(), 2);
        assert!(result.headers.date.is_some());
    }

    #[test]
    fn html_preview_attachments_and_duplicate_cid() {
        let raw = b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>world</b></p>\r\n--x\r\nContent-Type: image/png; name=a.png\r\nContent-ID: <same>\r\nContent-Transfer-Encoding: base64\r\n\r\nYWJj\r\n--x\r\nContent-Type: image/png; name=b.png\r\nContent-ID: <same>\r\n\r\nxyz\r\n--x--\r\n";
        let result = mime::project(raw).expect("synthetic message");
        assert_eq!(
            result
                .preview
                .as_ref()
                .map(|preview| return preview.text.as_str()),
            Some("Hello world")
        );
        assert_eq!(result.attachments.len(), 2);
        assert_eq!(
            &result.original[result.attachments[0].raw_body.clone()],
            b"YWJj"
        );
        assert_eq!(result.attachments[0].decoded_size, 3);
        assert!(result.attachment_for_cid("same").is_none());
        assert!(result.attachment_for_cid("unknown").is_none());
    }

    #[test]
    fn bounds_encryption_and_malformed() {
        assert_eq!(
            mime::project(&vec![b'x'; mime::MAX_INPUT_BYTES + 1]),
            Err(Error::InputTooLarge)
        );
        let encrypted = mime::project(b"Content-Type: multipart/encrypted; boundary=x\r\n\r\n--x\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n--x--\r\n").expect("encrypted message");
        assert!(encrypted.status.encrypted_unsupported);
        assert!(encrypted.preview.is_none());
        let raw = format!(
            "Content-Type: text/plain\r\n\r\n{}",
            "a".repeat(mime::MAX_BODY_BYTES + 1)
        );
        let large = mime::project(raw.as_bytes()).expect("large body");
        assert!(large.status.body_truncated);
        assert!(large.text.is_none());
        let malformed = mime::project(b"Content-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n%%%\r\n--x--\r\n").expect("malformed message");
        assert!(malformed.status.degraded);
    }

    #[test]
    fn forwarded_message_is_not_outer_body() {
        let raw = b"Content-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nOuter\r\n--x\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment\r\n\r\nSubject: forwarded\r\n\r\nInner\r\n--x--\r\n";
        let result = mime::project(raw).expect("forwarded message");
        assert_eq!(result.text.as_deref(), Some("Outer"));
        assert_eq!(result.attachments.len(), 1);
        assert_eq!(
            &result.original[result.attachments[0].raw_body.clone()],
            b"Subject: forwarded\r\n\r\nInner"
        );
    }
    #[test]
    fn part_and_attachment_limits_are_reported() {
        let mut raw = String::from("Content-Type: multipart/mixed; boundary=x\r\n\r\n");
        let parts: Range<usize> = 0..300;
        for _ in parts {
            raw.push_str("--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment\r\n\r\na\r\n");
        }
        raw.push_str("--x--\r\n");
        let result = mime::project(raw.as_bytes()).expect("bounded multipart");
        assert!(result.status.parts_truncated);
        assert!(result.status.attachments_truncated);
        assert_eq!(result.attachments.len(), mime::MAX_ATTACHMENTS);
        assert!(result.attachment_for_cid("missing").is_none());
    }

    #[test]
    fn missing_headers_still_produce_a_preview() {
        let result = mime::project(b"\r\nhello\r\nworld").expect("headerless message");
        assert!(result.headers.message_id.is_none());
        assert_eq!(result.headers.from.len(), 0);
        assert_eq!(
            result
                .preview
                .as_ref()
                .map(|preview| return preview.text.as_str()),
            Some("hello world")
        );
    }
    #[test]
    fn unique_cid_is_scoped_to_its_message() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=cid:one>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <one>\r\n\r\nabc\r\n--x--\r\n";
        let result = mime::project(raw).expect("synthetic message");
        assert_eq!(
            result
                .attachment_for_cid("one")
                .map(|part| return part.decoded_size),
            Some(3)
        );
        let other = mime::project(b"\r\nother").expect("other message");
        assert!(other.attachment_for_cid("one").is_none());
    }
    #[test]
    fn inline_encrypted_text_suppresses_all_alternatives() {
        let raw = b"Content-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\nLS0tLS1CRUdJTiBQR1AgTUVTU0FHRS0tLS0t\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>ciphertext</p>\r\n--x--\r\n";
        let result = mime::project(raw).expect("inline encrypted message");
        assert!(result.status.encrypted_unsupported);
        assert!(result.text.is_none());
        assert!(result.html.is_none());
        assert!(result.preview.is_none());
    }

    #[test]
    fn inline_encryption_is_detected_without_headers_or_after_a_long_preamble() {
        let headerless = "\r\n-----BEGIN PGP MESSAGE-----\r\nsynthetic encrypted payload";
        let delayed = format!(
            "Content-Type: text/plain\r\n\r\n{}\r\n-----BEGIN PGP MESSAGE-----\r\nsynthetic encrypted payload",
            "preamble ".repeat(600)
        );
        for source in [headerless, delayed.as_str()] {
            let result = mime::project(source.as_bytes()).expect("inline encrypted message");
            assert!(result.status.encrypted_unsupported);
            assert!(result.text.is_none());
            assert!(result.html.is_none());
            assert!(result.preview.is_none());
        }
    }

    #[test]
    fn preview_records_extracted_source_and_original_revision() {
        let result = mime::project(b"\r\nhello").expect("message");
        let preview = result.preview.expect("preview");
        assert_eq!(preview.source, mime::PreviewSource::Extracted);
        assert_eq!(preview.body_revision, result.revision);
    }

    #[test]
    fn malformed_cid_is_not_resolved() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=cid:bad>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <bad id>\r\n\r\nabc\r\n--x--\r\n";
        let result = mime::project(raw).expect("message");
        assert!(result.attachment_for_cid("bad id").is_none());
        assert!(result.attachment_for_cid("").is_none());
    }
    #[test]
    fn retained_headers_are_bounded_without_inventing_routing_addresses() {
        let subject = "é".repeat(mime::MAX_HEADER_BYTES);
        let address = format!("{}@example.test", "a".repeat(mime::MAX_HEADER_BYTES + 1));
        let references = (0..mime::MAX_HEADER_VALUES + 10)
            .map(|i| format!("<ref-{i}@example.test>"))
            .collect::<Vec<_>>()
            .join(" ");
        let raw = format!(
            "Subject: {subject}\r\nFrom: Oversized <{address}>\r\nReferences: {references}\r\n\r\nReadable body."
        );
        let result = mime::project(raw.as_bytes()).expect("bounded header projection");
        assert!(result.status.headers_truncated);
        assert!(result.headers.subject.as_ref().expect("subject").len() <= mime::MAX_HEADER_BYTES);
        assert!(
            result
                .headers
                .from
                .iter()
                .all(|mailbox| mailbox.address.is_none())
        );
        assert_eq!(result.headers.references.len(), mime::MAX_HEADER_VALUES);
        assert_eq!(result.original, raw.as_bytes());
        assert_eq!(result.text.as_deref(), Some("Readable body."));
    }
}
