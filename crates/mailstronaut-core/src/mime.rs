//! Bounded, untrusted MIME projections. HTML requires sanitization before rendering.

use core::ops::Range;

use mail_parser::{AddressList, MessageParser, MessagePart, PartFlags, PartKind};
use sha2::{Digest as _, Sha256};

/// Maximum accepted original message size.
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum parsed parts, including nested messages.
pub const MAX_PARTS: usize = 256;
/// Maximum retained attachment descriptors.
pub const MAX_ATTACHMENTS: usize = 64;
/// Maximum decoded bytes retained for each body alternative.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// A decoded mailbox; group names are not retained.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Mailbox {
    /// Address, if parsed.
    pub address: Option<String>,
    /// Display name.
    pub name: Option<String>,
}

/// Decoded root-message headers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Headers {
    /// Blind-copy recipients if present in the source.
    pub bcc: Vec<Mailbox>,
    /// Copy recipients.
    pub cc: Vec<Mailbox>,
    /// Parsed UTC timestamp in seconds.
    pub date: Option<i64>,
    /// Sender mailboxes.
    pub from: Vec<Mailbox>,
    /// Reply identifiers in order.
    pub in_reply_to: Vec<String>,
    /// Message identifier without angle brackets.
    pub message_id: Option<String>,
    /// Reference identifiers in order.
    pub references: Vec<String>,
    /// Decoded subject.
    pub subject: Option<String>,
    /// Primary recipients.
    pub to: Vec<Mailbox>,
}

/// A descriptor into this projection's original raw message, never decoded attachment data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attachment {
    /// Content identifier, scoped to this message.
    pub content_id: Option<String>,
    /// Transfer-decoded size.
    pub decoded_size: usize,
    /// Inline disposition.
    pub inline: bool,
    /// Content type, if present.
    pub media_type: Option<String>,
    /// Filename, if present.
    pub name: Option<String>,
    /// Raw encoded body range in `Projection::original`.
    pub raw_body: Range<usize>,
}

/// Deterministic source extraction, distinct from any future generated summary.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Preview {
    /// Hash of the complete source message.
    pub body_revision: [u8; 32],
    /// Whitespace-normalized, at most 320 Unicode characters.
    pub text: String,
}

/// Degradation and resource-limit indicators.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent parse conditions may occur together."
)]
pub struct Status {
    /// Attachment descriptors exceeded their limit.
    pub attachments_truncated: bool,
    /// A body alternative exceeded its limit.
    pub body_truncated: bool,
    /// Malformed structure or decoded body.
    pub degraded: bool,
    /// An encrypted MIME part was detected; body extraction is disabled.
    pub encrypted_unsupported: bool,
    /// Parser part or depth limit was reached.
    pub parts_truncated: bool,
}

/// An owned projection of one message. Neither HTML nor attachment contents are trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Projection {
    /// Root-message attachments, in source order.
    pub attachments: Vec<Attachment>,
    /// Root headers.
    pub headers: Headers,
    /// First HTML alternative, if any; unsanitized.
    pub html: Option<String>,
    /// Original bytes, bounded by `MAX_INPUT_BYTES`.
    pub original: Vec<u8>,
    /// Extracted preview, absent for encrypted mail.
    pub preview: Option<Preview>,
    /// SHA-256 of the original, also used as body revision.
    pub revision: [u8; 32],
    /// Parsing and truncation state.
    pub status: Status,
    /// First plain-text alternative, if any.
    pub text: Option<String>,
}

#[expect(
    clippy::question_mark_used,
    clippy::implicit_return,
    reason = "Question mark expresses an absent CID lookup clearly."
)]
impl Projection {
    /// Resolve a CID only when exactly one root-message attachment has that ID.
    #[must_use]
    #[inline]
    pub fn attachment_for_cid(&self, cid: &str) -> Option<&Attachment> {
        let mut matches = self
            .attachments
            .iter()
            .filter(|part| part.content_id.as_deref() == Some(cid));
        let first = matches.next()?;
        if matches.next().is_some()
            || self.status.attachments_truncated
            || self.status.parts_truncated
        {
            return None;
        }
        Some(first)
    }
}

/// Input cannot be projected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Original exceeds the hard input limit.
    InputTooLarge,
    /// No message could be parsed.
    Unparseable,
}

/// Flatten parsed address groups.
#[expect(
    clippy::implicit_return,
    reason = "Explicit closure returns conflict with idiomatic expression closures."
)]
fn addresses(list: Option<AddressList<'_>>) -> Vec<Mailbox> {
    list.into_iter()
        .flat_map(|items| items.mailboxes())
        .map(|mailbox| Mailbox {
            name: mailbox.name().map(str::to_owned),
            address: mailbox.address().map(str::to_owned),
        })
        .collect()
}

/// Record parser flags and encrypted containers recursively.
fn inspect(part: MessagePart<'_>, status: &mut Status) {
    status.degraded |= part.has_problems();
    status.parts_truncated |= part.flags().contains(PartFlags::LIMIT_REACHED);
    status.encrypted_unsupported |= part.is_content_type("multipart", "encrypted")
        || part.is_content_type("application", "pkcs7-mime")
        || part.is_content_type("application", "x-pkcs7-mime")
        || part.is_content_type("application", "pgp-encrypted");
    for child in part.children() {
        inspect(child, status);
    }
    if let Some(nested) = part.nested() {
        inspect(nested.root_part(), status);
    }
}

/// Decode one bounded text alternative.
#[expect(
    clippy::question_mark_used,
    clippy::implicit_return,
    clippy::arithmetic_side_effects,
    clippy::string_slice,
    reason = "Checked character boundaries and early absence are explicit here; return lints conflict."
)]
fn body(candidate: Option<MessagePart<'_>>, status: &mut Status) -> Option<String> {
    let part = candidate?;
    // Limit work before decoding. Charset conversion can expand the result.
    if part.raw_body().len() > MAX_BODY_BYTES {
        status.body_truncated = true;
        return None;
    }
    let (text, problems) = part.text_checked()?;
    status.degraded |= !problems.is_empty();
    let mut end = text.len().min(MAX_BODY_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    status.body_truncated |= end < text.len();
    Some(text[..end].to_owned())
}

/// Parse a bounded RFC 5322 message without fetching resources or executing HTML.
///
/// # Errors
/// Returns an error if the input exceeds the hard limit or cannot be parsed.
#[inline]
#[expect(
    clippy::question_mark_used,
    clippy::implicit_return,
    reason = "Early parse errors and expression closures are clearer; return lints conflict."
)]
pub fn project(raw: &[u8]) -> Result<Projection, Error> {
    if raw.len() > MAX_INPUT_BYTES {
        return Err(Error::InputTooLarge);
    }
    let message = MessageParser::new()
        .max_parts(MAX_PARTS)
        .max_depth(32)
        .max_encoded_nesting(0)
        .parse(raw)
        .ok_or(Error::Unparseable)?;
    let revision: [u8; 32] = Sha256::digest(raw).into();
    let headers = Headers {
        message_id: message.message_id().map(str::to_owned),
        references: message.references().map_or_else(Vec::new, |values| {
            values.iter().map(str::to_owned).collect()
        }),
        in_reply_to: message.in_reply_to().map_or_else(Vec::new, |values| {
            values.iter().map(str::to_owned).collect()
        }),
        from: addresses(message.from()),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        bcc: addresses(message.bcc()),
        subject: message.subject().map(str::to_owned),
        date: message.date().map(|date| date.to_timestamp()),
    };
    let mut status = Status::default();
    inspect(message.root_part(), &mut status);
    let mut attachments = Vec::new();
    for part in message.attachments() {
        if attachments.len() == MAX_ATTACHMENTS {
            status.attachments_truncated = true;
            break;
        }
        attachments.push(Attachment {
            name: part.attachment_name().map(str::to_owned),
            media_type: part
                .content_type()
                .map(|kind| format!("{}/{}", kind.ctype(), kind.subtype().unwrap_or(""))),
            content_id: part.content_id().map(str::to_owned),
            raw_body: usize::try_from(part.offset_body()).unwrap_or_default()
                ..usize::try_from(part.offset_end()).unwrap_or_default(),
            decoded_size: part.decoded_len(),
            inline: part.is_inline(),
        });
    }
    let (text, html, preview) = if status.encrypted_unsupported {
        (None, None, None)
    } else {
        let text = body(
            message
                .text_body()
                .find(|part| matches!(part.kind(), PartKind::Text)),
            &mut status,
        );
        let html = body(
            message
                .html_body()
                .find(|part| matches!(part.kind(), PartKind::Html)),
            &mut status,
        );
        let source = text
            .clone()
            .or_else(|| html.as_deref().map(mail_parser::html_to_text));
        let preview = source.map(|content| Preview {
            text: content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(320)
                .collect(),
            body_revision: revision,
        });
        (text, html, preview)
    };
    Ok(Projection {
        original: raw.to_vec(),
        revision,
        headers,
        text,
        html,
        preview,
        attachments,
        status,
    })
}
