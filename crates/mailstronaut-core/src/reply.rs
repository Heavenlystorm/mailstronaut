//! Local, reviewable reply/reply-all and text-only forward preparation.

use core::{error::Error as StdError, fmt};
use std::collections::BTreeSet;

use mailstronaut_config::Account;
use mailstronaut_mail::compose::{self, Address, Content};

use crate::{
    mime::{Mailbox, Projection},
    store::outbox::Draft,
};

/// Explicit user-selected composition action. Forward never includes attachments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// Address the declared Reply-To, or the original author.
    Reply,
    /// Also retain visible recipients, excluding this account's identities.
    ReplyAll,
    /// Start a new thread with a plain-text copy and no attachments.
    ForwardText,
}

/// Static template errors, never quoting original headers or message text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A safe, complete quotation is unavailable.
    Incomplete,
    /// Routing or subject fields cannot be represented safely.
    Addresses,
    /// Prepared content exceeds the existing composer budgets.
    TooLarge,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Incomplete=>"This message is incomplete or encrypted; automatic quotation is unavailable.",
            Self::Addresses=>"Reply addresses or headers could not be prepared safely. Start a new message instead.",
            Self::TooLarge=>"The quoted message exceeds the composer size limit. Start a new message instead.",
        })
    }
}
impl StdError for Error {}

/// Prepare a new editable local draft for the original receiving account.
/// Only an explicit later Send can freeze/submit it. No Bcc is copied to replies.
///
/// # Errors
/// Rejects incomplete/encrypted input, invalid routing fields and oversized quotes.
pub fn prepare(account: &Account, message: &Projection, kind: Kind) -> Result<Draft, Error> {
    if message.status.encrypted_unsupported
        || message.status.headers_truncated
        || message.status.parts_truncated
        || message.status.body_truncated
    {
        return Err(Error::Incomplete);
    }
    let own = std::iter::once(&account.address)
        .chain(&account.aliases)
        .map(|address| address.parse::<Address>().map_err(|_| Error::Addresses))
        .collect::<Result<Vec<_>, _>>()?;
    let is_self = |address: &Address| {
        own.iter().any(|own| {
            own.user().eq_ignore_ascii_case(address.user())
                && own.domain().eq_ignore_ascii_case(address.domain())
        })
    };
    let from = parse(&message.headers.from)?;
    let sent_by_self = !from.is_empty() && from.iter().all(|(address, _)| is_self(address));
    let receiving = if sent_by_self {
        &message.headers.from
    } else {
        &message.headers.to
    };
    let receiving = receiving
        .iter()
        .chain(&message.headers.cc)
        .chain(&message.headers.bcc)
        .filter_map(|mailbox| mailbox.address.as_deref()?.parse::<Address>().ok())
        .find(is_self);
    // Use the configured spelling, not a case variant from an untrusted header:
    // the native identity selector and frozen-envelope guard use that identity.
    let sender = receiving
        .and_then(|received| {
            own.iter()
                .find(|identity| same_identity(identity, &received))
        })
        .unwrap_or(&own[0]);
    let mut content = Content {
        from: compose::sender(sender.as_ref(), &account.display_name)
            .map_err(|_| Error::Addresses)?,
        ..Content::default()
    };
    content.subject = subject(message.headers.subject.as_deref().unwrap_or_default(), kind)?;
    if kind != Kind::ForwardText {
        recipients(message, kind, sent_by_self, from, &is_self, &mut content)?;
        ancestry(message, &mut content);
    }
    content.body = quoted(message, kind)?;
    let mut draft = Draft::new(account.id.clone(), content.from.clone());
    draft.content = content;
    Ok(draft)
}

fn same_identity(left: &Address, right: &Address) -> bool {
    left.user().eq_ignore_ascii_case(right.user())
        && left.domain().eq_ignore_ascii_case(right.domain())
}
fn subject(subject: &str, kind: Kind) -> Result<String, Error> {
    if subject.chars().any(char::is_control) {
        return Err(Error::Addresses);
    }
    let (prefix, already) = match kind {
        Kind::ForwardText => (
            "Fwd: ",
            subject
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("fwd:")),
        ),
        _ => (
            "Re: ",
            subject
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:")),
        ),
    };
    let subject = if already {
        subject.into()
    } else {
        format!("{prefix}{subject}")
    };
    if subject.len() > 4096 {
        return Err(Error::TooLarge);
    }
    Ok(subject)
}
fn recipients(
    message: &Projection,
    kind: Kind,
    sent_by_self: bool,
    from: Recipients,
    is_self: &impl Fn(&Address) -> bool,
    content: &mut Content,
) -> Result<(), Error> {
    let targets = if !message.headers.reply_to.is_empty() {
        parse(&message.headers.reply_to)?
    } else if sent_by_self {
        parse(&message.headers.to)?
    } else {
        from
    };
    let mut seen = BTreeSet::new();
    let mut to = Vec::new();
    let mut cc = Vec::new();
    add(targets, &mut to, &mut seen, is_self);
    if kind == Kind::ReplyAll {
        add(parse(&message.headers.to)?, &mut to, &mut seen, is_self);
        add(parse(&message.headers.cc)?, &mut cc, &mut seen, is_self);
    }
    if to.is_empty() && cc.is_empty() {
        return Err(Error::Addresses);
    }
    if seen.len() > 256 {
        return Err(Error::TooLarge);
    }
    content.to = to.join(", ");
    content.cc = cc.join(", ");
    if content.to.len() > 4096 || content.cc.len() > 4096 {
        return Err(Error::TooLarge);
    }
    Ok(())
}

type Recipients = Vec<(Address, String)>;
fn parse(mailboxes: &[Mailbox]) -> Result<Recipients, Error> {
    mailboxes
        .iter()
        .map(|mailbox| {
            let address = mailbox.address.as_deref().ok_or(Error::Addresses)?;
            let parsed = address.parse::<Address>().map_err(|_| Error::Addresses)?;
            let display = compose::sender(address, mailbox.name.as_deref().unwrap_or_default())
                .map_err(|_| Error::Addresses)?;
            Ok((parsed, display))
        })
        .collect()
}
fn add(
    recipients: Recipients,
    target: &mut Vec<String>,
    seen: &mut BTreeSet<(String, String)>,
    is_self: &impl Fn(&Address) -> bool,
) {
    for (address, display) in recipients {
        // Domains are case-insensitive; other recipients' local parts retain
        // their spelling, including distinct case-sensitive local mailboxes.
        if !is_self(&address)
            && seen.insert((address.user().into(), address.domain().to_ascii_lowercase()))
        {
            target.push(display);
        }
    }
}
fn ancestry(message: &Projection, content: &mut Content) {
    let parent = message
        .headers
        .message_id
        .as_deref()
        .filter(|id| compose::valid_id(id));
    let ancestors =
        if message.headers.references.is_empty() && message.headers.in_reply_to.len() == 1 {
            &message.headers.in_reply_to
        } else {
            &message.headers.references
        };
    for id in ancestors {
        if compose::valid_id(id) && Some(id.as_str()) != parent && !content.references.contains(id)
        {
            content.references.push(id.clone());
        }
    }
    if let Some(parent) = parent {
        content.in_reply_to = Some(parent.into());
        content.references.push(parent.into());
    }
    // Only complete identifiers fit; oldest history is expendable, never slice
    // an identifier or remove the direct parent to make a bounded header.
    while content.references.len() > 256
        || content.references.iter().map(String::len).sum::<usize>() > 4096
    {
        content.references.remove(0);
    }
}
fn quoted(message: &Projection, kind: Kind) -> Result<String, Error> {
    let text = message.readable_text().ok_or(Error::Incomplete)?;
    let mut body = String::from("\n\n");
    if kind == Kind::ForwardText {
        push(
            &mut body,
            "---------- Forwarded as text (attachments not included) ----------\n",
        )?;
        for (label, value) in [
            (
                "Subject",
                message.headers.subject.clone().unwrap_or_default(),
            ),
            (
                "From",
                parse(&message.headers.from)?
                    .into_iter()
                    .map(|(_, display)| display)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            (
                "To",
                parse(&message.headers.to)?
                    .into_iter()
                    .map(|(_, display)| display)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            (
                "Cc",
                parse(&message.headers.cc)?
                    .into_iter()
                    .map(|(_, display)| display)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ] {
            push(&mut body, &format!("{label}: {value}\n"))?;
        }
        push(&mut body, "\n")?;
        push(&mut body, &text)?;
    } else {
        let sender = parse(&message.headers.from)?
            .into_iter()
            .map(|(_, display)| display)
            .collect::<Vec<_>>()
            .join(", ");
        let date = message
            .headers
            .date
            .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
            .map(|date| date.to_rfc2822());
        push(
            &mut body,
            &date.map_or_else(
                || format!("{sender} wrote:\n"),
                |date| format!("On {date}, {sender} wrote:\n"),
            ),
        )?;
        for line in text.lines() {
            push(&mut body, "> ")?;
            push(&mut body, line)?;
            push(&mut body, "\n")?;
        }
    }
    Ok(body)
}
fn push(body: &mut String, text: &str) -> Result<(), Error> {
    if text.len() > compose::MAX_BODY_BYTES.saturating_sub(body.len()) {
        return Err(Error::TooLarge);
    }
    body.push_str(text);
    Ok(())
}
