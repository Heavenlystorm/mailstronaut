//! Local plain-text composition using lettre's address, header and MIME builders.

use core::{error::Error as StdError, fmt};

use lettre::{
    Message,
    address::Envelope,
    message::{Mailbox, Mailboxes, header::ContentType},
};
use serde::{Deserialize, Serialize};

/// Maximum decoded editor body in the initial composer.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// Maximum complete frozen MIME original, matching local message storage.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Editable local content. Debug intentionally omits all headers and body text.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    /// Single explicit sender address/identity, parsed with lettre.
    pub from: String,
    /// Comma-separated RFC mailboxes, including quoted display names.
    pub to: String,
    /// Visible copy recipients.
    pub cc: String,
    /// Envelope-only blind recipients; never present in frozen visible headers.
    pub bcc: String,
    /// Plain decoded subject, without control characters.
    pub subject: String,
    /// Plain UTF-8 body, encoded by lettre.
    pub body: String,
    /// Parent identifier without angle brackets, if replying.
    pub in_reply_to: Option<String>,
    /// Ancestors without angle brackets, in original order.
    pub references: Vec<String>,
}
impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Content(<private mail>)")
    }
}

/// Immutable message and its separate SMTP envelope. Persist this once; retries
/// must reuse its ID and bytes rather than rebuilding Content.
#[derive(Clone)]
pub struct Frozen {
    id: String,
    envelope: Envelope,
    original: Vec<u8>,
}
impl fmt::Debug for Frozen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Frozen(<private mail>)")
    }
}

/// Redacted composition failures; invalid input is never embedded in diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Invalid/oversized header, address, identifier or body.
    Invalid,
    /// Complete MIME exceeds the bounded original size.
    TooLarge,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid or oversized message fields",
            Self::TooLarge => "message exceeds the supported submission size",
        })
    }
}
impl StdError for Error {}

/// Format an explicitly configured address/display name using the MIME library.
///
/// # Errors
/// Rejects invalid addresses or control characters in a display name.
pub fn sender(address: &str, name: &str) -> Result<String, Error> {
    if address.len() > 4096 || name.len() > 4096 || name.chars().any(char::is_control) {
        return Err(Error::Invalid);
    }
    Ok(Mailbox::new(
        (!name.is_empty()).then(|| name.into()),
        address.parse().map_err(|_| Error::Invalid)?,
    )
    .to_string())
}

impl Content {
    /// Validate editable fields without generating a Message-ID or contacting a server.
    ///
    /// # Errors
    /// Rejects malformed/oversized fields and missing recipients.
    pub fn validate(&self) -> Result<(), Error> {
        for header in [&self.from, &self.to, &self.cc, &self.bcc, &self.subject] {
            if header.len() > 4096 || header.chars().any(char::is_control) {
                return Err(Error::Invalid);
            }
        }
        if self.body.len() > MAX_BODY_BYTES || self.references.len() > 256 {
            return Err(Error::TooLarge);
        }
        self.from.parse::<Mailbox>().map_err(|_| Error::Invalid)?;
        let count = recipients(&self.to)?.len()
            + recipients(&self.cc)?.len()
            + recipients(&self.bcc)?.len();
        if count == 0 || count > 256 {
            return Err(Error::Invalid);
        }
        if self
            .in_reply_to
            .iter()
            .chain(&self.references)
            .any(|id| !valid_id(id))
            || self.references.iter().map(String::len).sum::<usize>() > 4096
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }

    /// Freeze a typed, UTF-8 plain message once, with a UUID-based Message-ID.
    /// The library creates the SMTP envelope before stripping Bcc headers.
    ///
    /// # Errors
    /// Rejects invalid content or a message exceeding the original byte budget.
    pub fn freeze(&self) -> Result<Frozen, Error> {
        self.validate()?;
        let id = format!("{}@mailstronaut.invalid", uuid::Uuid::new_v4());
        let mut builder = Message::builder()
            .from(self.from.parse().map_err(|_| Error::Invalid)?)
            .subject(&self.subject)
            .message_id(Some(format!("<{id}>")))
            .header(ContentType::TEXT_PLAIN);
        for recipient in recipients(&self.to)? {
            builder = builder.to(recipient);
        }
        for recipient in recipients(&self.cc)? {
            builder = builder.cc(recipient);
        }
        for recipient in recipients(&self.bcc)? {
            builder = builder.bcc(recipient);
        }
        if let Some(parent) = &self.in_reply_to {
            builder = builder.in_reply_to(format!("<{parent}>"));
        }
        if !self.references.is_empty() {
            builder = builder.references(
                self.references
                    .iter()
                    .map(|id| format!("<{id}>"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        let message = builder
            .body(self.body.clone())
            .map_err(|_| Error::Invalid)?;
        let mut original = message.formatted();
        // SMTP must terminate the final data line with CRLF. Freeze that byte
        // sequence now so submission and a later Sent copy remain identical.
        if !original.ends_with(b"\r\n") {
            original.extend_from_slice(b"\r\n");
        }
        if original.len() > MAX_MESSAGE_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(Frozen {
            id,
            envelope: message.envelope().clone(),
            original,
        })
    }
}

impl Frozen {
    /// Compare an enabled sender identity using the library's address type.
    #[must_use]
    pub fn sender_is(&self, address: &str) -> bool {
        address
            .parse::<lettre::Address>()
            .is_ok_and(|address| self.envelope.from() == Some(&address))
    }
    /// Original ID, without angle brackets, retained across retries/reconciliation.
    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.id
    }
    /// Separate envelope, including blind recipients.
    #[must_use]
    pub const fn envelope(&self) -> &Envelope {
        &self.envelope
    }
    /// Exactly the immutable bytes submitted to SMTP and later copied to Sent.
    #[must_use]
    pub fn original(&self) -> &[u8] {
        &self.original
    }

    /// Reconstruct a previously frozen private outbox record, without rebuilding
    /// MIME or allocating a new ID. The store separately verifies the blob hash.
    ///
    /// # Errors
    /// Rejects invalid envelope/ID or oversized/empty stored bytes.
    pub fn restore(
        id: String,
        sender: &str,
        recipients: Vec<String>,
        original: Vec<u8>,
    ) -> Result<Self, Error> {
        if !valid_id(&id)
            || recipients.is_empty()
            || recipients.len() > 256
            || original.is_empty()
            || !original.ends_with(b"\r\n")
            || original.len() > MAX_MESSAGE_BYTES
        {
            return Err(Error::Invalid);
        }
        let sender = sender.parse().map_err(|_| Error::Invalid)?;
        let recipients = recipients
            .into_iter()
            .map(|value| value.parse().map_err(|_| Error::Invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let envelope = Envelope::new(Some(sender), recipients).map_err(|_| Error::Invalid)?;
        Ok(Self {
            id,
            envelope,
            original,
        })
    }
}

fn recipients(value: &str) -> Result<Vec<Mailbox>, Error> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    value
        .parse::<Mailboxes>()
        .map(Into::into)
        .map_err(|_| Error::Invalid)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 998
        && id.is_ascii()
        && !id.chars().any(|character| {
            character.is_ascii_whitespace()
                || character.is_ascii_control()
                || matches!(character, '<' | '>')
        })
}
