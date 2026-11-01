//! Owned, bounded projections of library-parsed IMAP responses.

use core::num::{NonZeroU32, NonZeroU64};
use std::collections::BTreeSet;

use async_imap::types::{Fetch, Flag, NameAttribute};
use chrono::NaiveDate;
use futures_util::TryStreamExt as _;
use tokio_util::sync::CancellationToken;

use super::{Connection, Error, limited, protocol_error};

/// Maximum original accepted by the MIME/store boundary, in bytes.
pub const MAX_ORIGINAL: u32 = 8 * 1024 * 1024;
/// Maximum metadata records in a single FETCH operation.
const MAX_BATCH: usize = 50;

/// Exact server mailbox identity. Keep the wire name opaque; `display_name` is for
/// showing it and must never change the name sent back to SELECT.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Folder {
    /// Original wire name, including modified UTF-7 on `IMAP4rev1` servers.
    pub wire_name: String,
    /// Whether SELECT is permitted for this entry.
    pub selectable: bool,
    /// Server hierarchy delimiter, without filesystem semantics.
    pub delimiter: Option<String>,
    /// Owned server attributes, including SPECIAL-USE extensions.
    pub attributes: Vec<String>,
}

/// Bounded server metadata, keyed exclusively by UID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Metadata {
    /// Nonzero UID in the selected mailbox's UIDVALIDITY namespace.
    pub uid: NonZeroU32,
    /// Original octets advertised by the server, checked again on PEEK.
    pub size: u32,
    /// Server INTERNALDATE as UTC seconds.
    pub received_at: i64,
    /// Observed read flag; obtaining this metadata never sets it.
    pub seen: bool,
    /// Observed star flag.
    pub starred: bool,
    /// Authenticated provider identity, only when X-GM-EXT-1 was advertised.
    pub gmail_id: Option<NonZeroU64>,
}

/// A complete original fetched without changing Seen. No Debug implementation
/// exists because message bodies must not enter diagnostic output.
pub struct MessageData {
    /// Fresh metadata from the same response as the body.
    pub metadata: Metadata,
    /// Untrusted RFC message bytes, ready for the bounded MIME/store boundary.
    pub original: Vec<u8>,
}

impl Connection {
    /// List up to 4096 folders without decoding or normalizing their identities.
    ///
    /// # Errors
    /// Returns a redacted protocol/limit error and retires a failed session.
    pub async fn folders(&mut self, cancel: &CancellationToken) -> Result<Vec<Folder>, Error> {
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let folders = limited(cancel, async {
            let mut names = session
                .list(Some(""), Some("*"))
                .await
                .map_err(protocol_error)?;
            let mut result = Vec::new();
            while let Some(name) = names.try_next().await.map_err(protocol_error)? {
                if result.len() >= 4096 || name.attributes().len() > 64 {
                    return Err(Error::ResponseLimit);
                }
                validate_mailbox(name.name()).map_err(|_| Error::Protocol)?;
                result.push(Folder {
                    wire_name: name.name().into(),
                    selectable: !name.attributes().contains(&NameAttribute::NoSelect),
                    delimiter: name.delimiter().map(str::to_owned),
                    attributes: name.attributes().iter().map(attribute).collect(),
                });
            }
            Ok(result)
        })
        .await?;
        self.session = Some(session);
        Ok(folders)
    }

    /// Search a closed span of at most 1000 UIDs, optionally restricted by the
    /// server's civil INTERNALDATE. Empty gaps never become sequence-number work.
    ///
    /// # Errors
    /// Invalid spans are rejected before I/O; malformed/out-of-range responses
    /// retire the stream. Cancellation and timeouts also retire the stream.
    pub async fn search(
        &mut self,
        first: NonZeroU32,
        last: NonZeroU32,
        since: Option<NaiveDate>,
        cancel: &CancellationToken,
    ) -> Result<Vec<NonZeroU32>, Error> {
        if last < first || last.get() - first.get() >= 1000 {
            return Err(Error::Configuration);
        }
        let query = format!(
            "UID {}:{}{}",
            first,
            last,
            since.map_or_else(String::new, |date| format!(
                " SINCE {}",
                date.format("%d-%b-%Y")
            ))
        );
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let found = limited(cancel, async {
            session.uid_search(query).await.map_err(protocol_error)
        })
        .await?;
        if found.len() > 1000
            || found
                .iter()
                .any(|uid| *uid < first.get() || *uid > last.get())
        {
            return Err(Error::Protocol);
        }
        let mut found = found
            .into_iter()
            .filter_map(NonZeroU32::new)
            .collect::<Vec<_>>();
        found.sort_unstable();
        self.session = Some(session);
        Ok(found)
    }

    /// Fetch at most 50 messages' flags, size, INTERNALDATE and optional Gmail ID.
    /// Missing UIDs are allowed: a message can be expunged after SEARCH.
    ///
    /// # Errors
    /// Rejects duplicates, invalid batches or malformed responses; retires failed streams.
    pub async fn metadata(
        &mut self,
        uids: &[NonZeroU32],
        cancel: &CancellationToken,
    ) -> Result<Vec<Metadata>, Error> {
        let ids: BTreeSet<_> = uids.iter().copied().collect();
        if ids.is_empty() || ids.len() != uids.len() || ids.len() > MAX_BATCH {
            return Err(Error::Configuration);
        }
        let sequence = uids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let gmail = self.supports("X-GM-EXT-1");
        let query = if gmail {
            "(UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID)"
        } else {
            "(UID FLAGS RFC822.SIZE INTERNALDATE)"
        };
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let result = limited(cancel, async {
            let mut stream = session
                .uid_fetch(sequence, query)
                .await
                .map_err(protocol_error)?;
            let mut result = Vec::new();
            let mut seen = BTreeSet::new();
            let mut responses = 0;
            while let Some(fetch) = stream.try_next().await.map_err(protocol_error)? {
                responses += 1;
                if responses > 200 || fetch.body().is_some() {
                    return Err(Error::ResponseLimit);
                }
                let Some(uid) = fetch
                    .uid
                    .and_then(NonZeroU32::new)
                    .filter(|uid| ids.contains(uid))
                else {
                    continue;
                };
                // Servers may emit unrelated FLAGS-only updates; do not replace
                // complete metadata with a partial unsolicited response.
                if fetch.size.is_none() {
                    continue;
                }
                if !seen.insert(uid) {
                    return Err(Error::Protocol);
                }
                result.push(metadata(&fetch, gmail)?);
            }
            result.sort_unstable_by_key(|entry| entry.uid);
            Ok(result)
        })
        .await?;
        self.session = Some(session);
        Ok(result)
    }

    /// Fetch exactly one preflighted original using BODY.PEEK[], never BODY[].
    /// Returns None if it was expunged. Rejects size changes rather than caching
    /// a partial/different original. Attachments are not separately downloaded.
    ///
    /// # Errors
    /// Rejects oversized mail before sending a FETCH; response/UID/size errors
    /// retire the session. The upstream parser's allocation ceiling remains a
    /// separate resource-hardening gate (see P04 evidence).
    pub async fn peek(
        &mut self,
        expected: &Metadata,
        cancel: &CancellationToken,
    ) -> Result<Option<MessageData>, Error> {
        if expected.size == 0 || expected.size > MAX_ORIGINAL {
            return Err(Error::ResponseLimit);
        }
        let gmail = self.supports("X-GM-EXT-1");
        let query = if gmail {
            "(UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID BODY.PEEK[])"
        } else {
            "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[])"
        };
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let result = limited(cancel, async {
            let mut stream = session
                .uid_fetch(expected.uid.to_string(), query)
                .await
                .map_err(protocol_error)?;
            let mut result = None;
            let mut responses = 0;
            while let Some(fetch) = stream.try_next().await.map_err(protocol_error)? {
                responses += 1;
                if responses > 200 {
                    return Err(Error::ResponseLimit);
                }
                let Some(body) = fetch.body() else { continue };
                let observed = metadata(&fetch, gmail)?;
                if result.is_some()
                    || observed.uid != expected.uid
                    || observed.size != expected.size
                    || body.len() != expected.size as usize
                    || observed.gmail_id != expected.gmail_id
                {
                    return Err(Error::Protocol);
                }
                result = Some(MessageData {
                    metadata: observed,
                    original: body.to_vec(),
                });
            }
            Ok(result)
        })
        .await?;
        self.session = Some(session);
        Ok(result)
    }
}

fn metadata(fetch: &Fetch, gmail: bool) -> Result<Metadata, Error> {
    let uid = fetch.uid.and_then(NonZeroU32::new).ok_or(Error::Protocol)?;
    let size = fetch.size.ok_or(Error::Protocol)?;
    let received_at = fetch.internal_date().ok_or(Error::Protocol)?.timestamp();
    let seen = fetch.flags().any(|flag| flag == Flag::Seen);
    let starred = fetch.flags().any(|flag| flag == Flag::Flagged);
    let gmail_id = if gmail {
        Some(
            fetch
                .gmail_msg_id()
                .copied()
                .and_then(NonZeroU64::new)
                .ok_or(Error::Protocol)?,
        )
    } else {
        None
    };
    Ok(Metadata {
        uid,
        size,
        received_at,
        seen,
        starred,
        gmail_id,
    })
}
fn attribute(value: &NameAttribute<'_>) -> String {
    match value {
        NameAttribute::Extension(value) => value.to_string(),
        other => format!("\\{other:?}"),
    }
}
pub(super) fn validate_mailbox(name: &str) -> Result<(), Error> {
    // UTF8=ACCEPT is not enabled by this baseline. Preserve modified UTF-7
    // names exactly and reject unsupported raw UTF-8 rather than misaddressing.
    if name.is_empty()
        || name.len() > 4096
        || !name.is_ascii()
        || name.chars().any(char::is_control)
    {
        Err(Error::Configuration)
    } else {
        Ok(())
    }
}
