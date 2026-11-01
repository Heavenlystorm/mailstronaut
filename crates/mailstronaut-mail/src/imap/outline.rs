//! Reading a message too large to cache: its headers, its text and the list of what else
//! it holds, with the rest fetched part by part only when asked for.

use core::{num::NonZeroU32, time::Duration};
use std::collections::BTreeMap;

use async_imap::imap_proto::{BodyStructure, MessageSection, SectionPath};
use futures_util::TryStreamExt as _;
use tokio_util::sync::CancellationToken;

use super::{Connection, Error, Metadata, limited, protocol_error, read};

/// Beyond this, a text part is not something a reader would read as mail.
const MAX_TEXT_OCTETS: u32 = 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_PARTS: usize = 64;
/// The largest single part this client will download, as encoded on the wire.
pub const MAX_PART_OCTETS: u32 = 64 * 1024 * 1024;
/// A session's receive budget when it exists to download one such part.
pub(super) const DOWNLOAD_SESSION_BYTES: u64 = MAX_PART_OCTETS as u64 + 1024 * 1024;
/// Generous for a slow link, since the user is waiting for this one explicit request.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(15);

/// One part of a message on the server, by its IMAP section number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemotePart {
    /// Section number such as `2` or `1.3`, which is all a later fetch needs.
    pub section: String,
    /// The part's own MIME headers, unparsed.
    pub mime: Vec<u8>,
    /// Encoded size on the wire.
    pub octets: u32,
}

/// What can be shown of a message without downloading it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outline {
    /// The same metadata the size decision was made from.
    pub metadata: Metadata,
    /// The whole message's header block.
    pub header: Vec<u8>,
    /// The readable part, with its own MIME headers, if one fits.
    pub text: Option<(Vec<u8>, Vec<u8>)>,
    /// Everything else a reader may want.
    pub parts: Vec<RemotePart>,
}

/// A readable part or a listed one, found by walking the structure.
#[derive(Default)]
struct Walk {
    plain: Option<(Vec<u32>, u32)>,
    html: Option<(Vec<u32>, u32)>,
    parts: Vec<(Vec<u32>, u32)>,
}

fn section_name(path: &[u32]) -> String {
    path.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn walk(structure: &BodyStructure<'_>, path: &[u32], found: &mut Walk) {
    let part_path = || {
        // RFC 3501: the single body of a non-multipart message is section 1.
        if path.is_empty() {
            vec![1]
        } else {
            path.to_vec()
        }
    };
    let attached = |common: &async_imap::imap_proto::BodyContentCommon<'_>| {
        common
            .disposition
            .as_ref()
            .is_some_and(|value| value.ty.eq_ignore_ascii_case("attachment"))
    };
    match structure {
        BodyStructure::Multipart { bodies, .. } => {
            for (index, body) in bodies.iter().enumerate() {
                let mut child = path.to_vec();
                child.push(u32::try_from(index + 1).unwrap_or(u32::MAX));
                walk(body, &child, found);
            }
        }
        BodyStructure::Text { common, other, .. } if !attached(common) => {
            let slot = if common.ty.subtype.eq_ignore_ascii_case("plain") {
                &mut found.plain
            } else if common.ty.subtype.eq_ignore_ascii_case("html") {
                &mut found.html
            } else {
                found.parts.push((part_path(), other.octets));
                return;
            };
            if slot.is_none() {
                *slot = Some((part_path(), other.octets));
            } else {
                found.parts.push((part_path(), other.octets));
            }
        }
        BodyStructure::Text { other, .. }
        | BodyStructure::Basic { other, .. }
        | BodyStructure::Message { other, .. } => {
            found.parts.push((part_path(), other.octets));
        }
    }
}

impl Connection {
    /// Read what can be shown of an oversized message using BODY.PEEK, never BODY, so
    /// that this cannot mark it read. Returns None if it has since been expunged.
    ///
    /// # Errors
    /// A malformed or excessive response retires the session.
    pub async fn outline(
        &mut self,
        expected: &Metadata,
        cancel: &CancellationToken,
    ) -> Result<Option<Outline>, Error> {
        let gmail = self.supports("X-GM-EXT-1");
        let query = if gmail {
            "(UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID BODYSTRUCTURE BODY.PEEK[HEADER])"
        } else {
            "(UID FLAGS RFC822.SIZE INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER])"
        };
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let first = limited(cancel, async {
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
                let (Some(structure), Some(header)) = (fetch.bodystructure(), fetch.header())
                else {
                    continue;
                };
                let observed = read::metadata(&fetch, gmail)?;
                if result.is_some() || observed.uid != expected.uid {
                    return Err(Error::Protocol);
                }
                if header.len() > MAX_HEADER_BYTES {
                    return Err(Error::ResponseLimit);
                }
                let mut found = Walk::default();
                walk(structure, &[], &mut found);
                result = Some((observed, header.to_vec(), found));
            }
            Ok(result)
        })
        .await?;
        self.session = Some(session);
        let Some((metadata, header, found)) = first else {
            return Ok(None);
        };
        // Prefer plain text. Whichever is not read stays listed, as it would otherwise
        // be unreachable: the rest of this message is not on this machine.
        let (text, other) = match (found.plain, found.html) {
            (Some(plain), html) => (Some(plain), html),
            (None, html) => (html, None),
        };
        let text = text.filter(|(_, octets)| *octets <= MAX_TEXT_OCTETS);
        let mut parts = found.parts;
        parts.extend(other);
        if parts.len() > MAX_PARTS {
            return Err(Error::ResponseLimit);
        }
        let fetched = self
            .fetch_sections(expected.uid, text.as_ref(), &parts, cancel)
            .await?;
        let mime_of = |path: &[u32]| fetched.get(&section_name(path)).cloned();
        let text = text.and_then(|(path, _)| {
            let content = fetched
                .get(&format!("{}.content", section_name(&path)))?
                .clone();
            Some((mime_of(&path)?, content))
        });
        let parts = parts
            .into_iter()
            .map(|(path, octets)| RemotePart {
                section: section_name(&path),
                mime: mime_of(&path).unwrap_or_default(),
                octets,
            })
            .collect();
        Ok(Some(Outline {
            metadata,
            header,
            text,
            parts,
        }))
    }

    async fn fetch_sections(
        &mut self,
        uid: NonZeroU32,
        text: Option<&(Vec<u32>, u32)>,
        parts: &[(Vec<u32>, u32)],
        cancel: &CancellationToken,
    ) -> Result<BTreeMap<String, Vec<u8>>, Error> {
        let mut wanted: Vec<String> = parts
            .iter()
            .map(|(path, _)| format!("BODY.PEEK[{}.MIME]", section_name(path)))
            .collect();
        if let Some((path, _)) = text {
            wanted.push(format!("BODY.PEEK[{}.MIME]", section_name(path)));
            wanted.push(format!("BODY.PEEK[{}]", section_name(path)));
        }
        if wanted.is_empty() {
            return Ok(BTreeMap::new());
        }
        let query = format!("(UID {})", wanted.join(" "));
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let fetched = limited(cancel, async {
            let mut stream = session
                .uid_fetch(uid.to_string(), query)
                .await
                .map_err(protocol_error)?;
            let mut fetched = BTreeMap::new();
            let mut responses = 0;
            while let Some(fetch) = stream.try_next().await.map_err(protocol_error)? {
                responses += 1;
                if responses > 200 {
                    return Err(Error::ResponseLimit);
                }
                if fetch.uid.and_then(NonZeroU32::new) != Some(uid) {
                    continue;
                }
                for (path, _) in parts.iter().chain(text) {
                    let mime = SectionPath::Part(path.clone(), Some(MessageSection::Mime));
                    if let Some(bytes) = fetch.section(&mime) {
                        fetched.insert(section_name(path), bytes.to_vec());
                    }
                }
                if let Some((path, _)) = text
                    && let Some(bytes) = fetch.section(&SectionPath::Part(path.clone(), None))
                {
                    fetched.insert(format!("{}.content", section_name(path)), bytes.to_vec());
                }
            }
            Ok(fetched)
        })
        .await?;
        self.session = Some(session);
        Ok(fetched)
    }

    /// Download one part of a message by its section number, for an explicit request.
    /// Returns its MIME headers and its still-encoded content, or None if it is gone.
    ///
    /// # Errors
    /// Rejects a malformed section number and a part over [`MAX_PART_OCTETS`]; a failed
    /// transfer retires the session.
    pub async fn part(
        &mut self,
        uid: NonZeroU32,
        section: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, Error> {
        let path = section
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::Configuration)?;
        if path.is_empty() || path.len() > 16 || path.contains(&0) {
            return Err(Error::Configuration);
        }
        let name = section_name(&path);
        let query = format!("(UID BODY.PEEK[{name}.MIME] BODY.PEEK[{name}])");
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let operation = async {
            let mut stream = session
                .uid_fetch(uid.to_string(), query)
                .await
                .map_err(protocol_error)?;
            let mut result = None;
            while let Some(fetch) = stream.try_next().await.map_err(protocol_error)? {
                if fetch.uid.and_then(NonZeroU32::new) != Some(uid) {
                    continue;
                }
                let mime =
                    fetch.section(&SectionPath::Part(path.clone(), Some(MessageSection::Mime)));
                let content = fetch.section(&SectionPath::Part(path.clone(), None));
                if let (Some(mime), Some(content)) = (mime, content) {
                    if content.len() > MAX_PART_OCTETS as usize {
                        return Err(Error::ResponseLimit);
                    }
                    result = Some((mime.to_vec(), content.to_vec()));
                }
            }
            Ok(result)
        };
        let result = cancel
            .run_until_cancelled(tokio::time::timeout(DOWNLOAD_TIMEOUT, operation))
            .await
            .ok_or(Error::Cancelled)?
            .map_err(|_| Error::TimedOut)??;
        self.session = Some(session);
        Ok(result)
    }
}
