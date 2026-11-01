//! A message too large to cache, reduced to what can be read and what can be fetched.
//!
//! The headers and readable text are assembled into an ordinary message, so that the list,
//! reader, search, threads and replies need no special case. What it leaves out is recorded
//! in [`Omitted`], so that nothing pretends the small copy is the whole message.

use mail_parser::MessageParser;
use mailstronaut_mail::imap::Outline;
use serde::{Deserialize, Serialize};

use crate::mime::MAX_INPUT_BYTES;

/// An attachment that stayed on the server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteAttachment {
    /// IMAP section number.
    pub section: String,
    /// Decoded file name, as the sender gave it.
    pub name: Option<String>,
    /// Declared media type.
    pub media_type: Option<String>,
    /// Approximate decoded size, computed from the encoded size on the server.
    pub size: u64,
}

/// What a stored message leaves on the server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Omitted {
    /// The whole message's size on the server.
    pub size: u32,
    /// Everything that can still be fetched.
    pub attachments: Vec<RemoteAttachment>,
}

/// Remove the headers that describe the original body, which this copy no longer has.
fn without_body_headers(header: &[u8]) -> Vec<u8> {
    let mut kept = Vec::with_capacity(header.len());
    let mut keeping = true;
    for line in header.split_inclusive(|byte| *byte == b'\n') {
        let continued = line
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'));
        if !continued {
            let name = line.split(|byte| *byte == b':').next().unwrap_or_default();
            keeping = !name.to_ascii_lowercase().starts_with(b"content-")
                && !name.eq_ignore_ascii_case(b"mime-version");
        }
        let blank = matches!(line, b"\r\n" | b"\n");
        if keeping && !blank {
            kept.extend_from_slice(line);
        }
    }
    kept
}

/// Base64 carries three bytes in every four; other encodings are close enough to size.
fn decoded_estimate(mime: &[u8], octets: u32) -> u64 {
    let base64 = String::from_utf8_lossy(mime)
        .to_ascii_lowercase()
        .contains("base64");
    if base64 {
        u64::from(octets) / 4 * 3
    } else {
        u64::from(octets)
    }
}

fn described(mime: &[u8]) -> (Option<String>, Option<String>) {
    let mut message = mime.to_vec();
    message.extend_from_slice(b"\r\n");
    let Some(parsed) = MessageParser::new().parse(&message) else {
        return (None, None);
    };
    let part = parsed.root_part();
    let name = part.attachment_name().map(str::to_owned);
    let media_type = part.content_type().map(|kind| {
        kind.subtype().map_or_else(
            || kind.ctype().to_owned(),
            |subtype| format!("{}/{subtype}", kind.ctype()),
        )
    });
    (name, media_type)
}

/// The small copy to store, and the record of what it leaves out.
///
/// # Errors
/// Returns `None` if even this reduced copy is over the store's limit.
#[must_use]
pub fn reduce(outline: &Outline) -> Option<(Vec<u8>, Omitted)> {
    let mut original = without_body_headers(&outline.header);
    original.extend_from_slice(b"MIME-Version: 1.0\r\n");
    match &outline.text {
        Some((mime, content)) => {
            original.extend_from_slice(mime);
            if !mime.ends_with(b"\r\n\r\n") {
                original.extend_from_slice(b"\r\n");
            }
            original.extend_from_slice(content);
        }
        None => original.extend_from_slice(b"\r\n"),
    }
    if original.len() > MAX_INPUT_BYTES {
        return None;
    }
    let attachments = outline
        .parts
        .iter()
        .map(|part| {
            let (name, media_type) = described(&part.mime);
            RemoteAttachment {
                section: part.section.clone(),
                name,
                media_type,
                size: decoded_estimate(&part.mime, part.octets),
            }
        })
        .collect();
    Some((
        original,
        Omitted {
            size: outline.metadata.size,
            attachments,
        },
    ))
}

/// Decode a downloaded part with its own MIME headers, as the parser would any message.
/// The bytes are untrusted and are never opened here.
#[must_use]
pub fn decode_part(mime: &[u8], content: &[u8]) -> Option<Vec<u8>> {
    let mut message = Vec::with_capacity(mime.len() + content.len() + 2);
    message.extend_from_slice(mime);
    if !mime.ends_with(b"\r\n\r\n") {
        message.extend_from_slice(b"\r\n");
    }
    message.extend_from_slice(content);
    let parsed = MessageParser::new()
        .max_encoded_nesting(0)
        .parse(&message)?;
    let (bytes, problems) = parsed.root_part().decoded_checked();
    problems.is_empty().then(|| bytes.into_owned())
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU32;

    use mailstronaut_mail::imap::{Metadata, RemotePart};

    use super::*;
    use crate::mime;

    fn outline(text: Option<(&[u8], &[u8])>, parts: Vec<RemotePart>) -> Outline {
        Outline {
            metadata: Metadata {
                uid: NonZeroU32::new(9).expect("UID"),
                seen: false,
                starred: false,
                size: 30_000_000,
                received_at: 1_000,
                gmail_id: None,
            },
            header: b"From: Sender <sender@example.test>\r\nTo: reader@example.test\r\nSubject: =?utf-8?q?R=C3=A9union?=\r\nMessage-ID: <big@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed;\r\n boundary=\"gone\"\r\nX-Other: kept\r\n\r\n".to_vec(),
            text: text.map(|(mime, content)| (mime.to_vec(), content.to_vec())),
            parts,
        }
    }

    #[test]
    fn the_small_copy_is_an_ordinary_message_without_the_original_structure() {
        let (original, omitted) = reduce(&outline(
            Some((
                b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n",
                b"Caf=C3=A9 notes.",
            )),
            vec![RemotePart {
                section: "2".into(),
                mime: b"Content-Type: application/pdf; name=\"=?utf-8?q?R=C3=A9sum=C3=A9=2Epdf?=\"\r\nContent-Disposition: attachment\r\nContent-Transfer-Encoding: base64\r\n\r\n".to_vec(),
                octets: 40_000_000,
            }],
        ))
        .expect("reduced");
        let text = String::from_utf8_lossy(&original);
        assert!(text.contains("X-Other: kept"));
        assert!(!text.contains("multipart/mixed"), "{text}");
        assert!(!text.contains("gone"), "{text}");
        let projection = mime::project(&original).expect("parses");
        assert_eq!(projection.headers.subject.as_deref(), Some("Réunion"));
        assert_eq!(
            projection.headers.message_id.as_deref(),
            Some("big@example.test")
        );
        assert_eq!(projection.readable_text().as_deref(), Some("Café notes."));
        assert_eq!(omitted.size, 30_000_000);
        assert_eq!(
            omitted.attachments,
            [RemoteAttachment {
                section: "2".into(),
                name: Some("Résumé.pdf".into()),
                media_type: Some("application/pdf".into()),
                size: 30_000_000,
            }]
        );
    }

    #[test]
    fn a_message_with_no_readable_text_still_has_its_headers() {
        let (original, omitted) = reduce(&outline(None, Vec::new())).expect("reduced");
        let projection = mime::project(&original).expect("parses");
        assert_eq!(projection.headers.subject.as_deref(), Some("Réunion"));
        assert_eq!(omitted.attachments, []);
    }

    #[test]
    fn nothing_too_large_for_the_store_is_produced() {
        let content = vec![b'x'; MAX_INPUT_BYTES];
        assert!(
            reduce(&outline(
                Some((b"Content-Type: text/plain\r\n\r\n", &content)),
                Vec::new()
            ))
            .is_none()
        );
    }

    #[test]
    fn parts_decode_by_their_own_headers() {
        let base64 = decode_part(
            b"Content-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n",
            b"U3ludGhl\r\ndGlj\r\n",
        );
        assert_eq!(base64.as_deref(), Some(b"Synthetic".as_slice()));
        let plain = decode_part(b"Content-Type: application/octet-stream\r\n\r\n", b"raw");
        assert_eq!(plain.as_deref(), Some(b"raw".as_slice()));
        let quoted = decode_part(
            b"Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n",
            b"a=3Db",
        );
        assert_eq!(quoted.as_deref(), Some(b"a=b".as_slice()));
    }
}
