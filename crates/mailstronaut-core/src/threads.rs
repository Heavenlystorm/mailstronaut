//! RFC 5256/JWZ grouping through the pinned library, with explicit resource limits.

use core::{error::Error as StdError, fmt};
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub mod helper;
pub mod process;

/// Largest account snapshot in the initial bounded projection.
pub const MAX_MESSAGES: usize = 50_000;
/// Largest distinct message/phantom identifier set accepted by the helper.
pub const MAX_NODES: usize = 100_000;
/// Encoded metadata request ceiling; no bodies, addresses or subjects are needed.
pub const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
/// Encoded local-ID grouping ceiling.
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// One account-local metadata item. Debug omits RFC identifiers.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// Immutable local database identity, never an IMAP sequence number.
    pub id: i64,
    /// Bounded parser-projected RFC identifier, without angle brackets.
    pub message_id: Option<String>,
    /// Unambiguous single parent when References is absent.
    pub parent: Option<String>,
    /// Ordered bounded RFC ancestry.
    pub references: Vec<String>,
    /// Server receipt time in UTC seconds, used only for stable library ordering.
    pub received_at: i64,
}
impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadMessage(<private metadata>)")
    }
}

/// Complete grouping of the supplied account snapshot. Debug exposes no IDs.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Groups {
    /// Each local message occurs exactly once in a nonempty group.
    pub groups: Vec<Vec<i64>>,
}
impl fmt::Debug for Groups {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadGroups(<private metadata>)")
    }
}

/// Static projection failures, never containing RFC metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Metadata exceeds the supported count, bytes or identifier budget.
    TooLarge,
    /// Invalid identities, malformed IPC or incomplete/duplicate group coverage.
    Invalid,
    /// The bounded subprocess was cancelled and reaped.
    Cancelled,
    /// Process start, I/O, resource limit or exit prevented grouping.
    Unavailable,
    /// The child exceeded its elapsed-time budget and was reaped.
    TimedOut,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "conversation metadata exceeds the current grouping budget",
            Self::Invalid => "conversation projection data is invalid",
            Self::Cancelled => "conversation grouping cancelled",
            Self::Unavailable => {
                "bounded conversation helper unavailable; messages remain accessible"
            }
            Self::TimedOut => {
                "conversation grouping exceeded its time budget; messages remain accessible"
            }
        })
    }
}
impl StdError for Error {}

/// Validate before serialization or invoking the recursive upstream algorithm.
///
/// This bounds data; the child supplies actual CPU/address-space enforcement.
///
/// # Errors
/// Rejects duplicate/nonpositive local IDs and excessive metadata.
pub fn validate(messages: &[Message]) -> Result<(), Error> {
    if messages.len() > MAX_MESSAGES {
        return Err(Error::TooLarge);
    }
    let mut ids = BTreeSet::new();
    let mut nodes = BTreeSet::new();
    // A conservative JSON bound: field names/numbers/punctuation per item and
    // at most two encoded bytes per ASCII identifier byte after escaping.
    let mut encoded = 1024_usize;
    for message in messages {
        if message.id <= 0 || !ids.insert(message.id) {
            return Err(Error::Invalid);
        }
        if message.references.len() > 256 {
            return Err(Error::TooLarge);
        }
        encoded = encoded.checked_add(192).ok_or(Error::TooLarge)?;
        for id in message
            .message_id
            .iter()
            .chain(&message.parent)
            .chain(&message.references)
        {
            if !mailstronaut_mail::compose::valid_id(id) {
                return Err(Error::Invalid);
            }
            nodes.insert(id.as_str());
            encoded = encoded
                .checked_add(id.len() * 2 + 4)
                .ok_or(Error::TooLarge)?;
            if encoded > MAX_INPUT_BYTES || nodes.len() + messages.len() > MAX_NODES {
                return Err(Error::TooLarge);
            }
        }
    }
    if encoded > MAX_INPUT_BYTES || nodes.len() + messages.len() > MAX_NODES {
        return Err(Error::TooLarge);
    }
    Ok(())
}

impl Groups {
    /// Verify exact coverage before any database projection can be replaced.
    ///
    /// # Errors
    /// Rejects missing, extra, duplicate or nonpositive local identities.
    pub fn validate(&self, messages: &[Message]) -> Result<(), Error> {
        validate(messages)?;
        let expected = messages
            .iter()
            .map(|message| message.id)
            .collect::<BTreeSet<_>>();
        let mut actual = BTreeSet::new();
        if self.groups.len() > messages.len() {
            return Err(Error::Invalid);
        }
        for group in &self.groups {
            if group.is_empty() || group.len() > messages.len() {
                return Err(Error::Invalid);
            }
            for id in group {
                if !expected.contains(id) || !actual.insert(*id) {
                    return Err(Error::Invalid);
                }
            }
        }
        if actual != expected {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

/// Run only inside the bounded helper or focused synthetic tests.
///
/// # Errors
/// Rejects invalid/oversized input or unexpected upstream output. The upstream
/// traversal is recursive, so a desktop caller must use `process::project`.
pub(crate) fn project(messages: &[Message]) -> Result<Groups, Error> {
    validate(messages)?;
    let input = messages
        .iter()
        .map(|message| mail_threading::Message {
            id: message.id.to_string(),
            message_id: message.message_id.clone(),
            in_reply_to: message.parent.clone(),
            references: message.references.clone(),
            date: chrono::DateTime::from_timestamp(message.received_at, 0).unwrap_or_default(),
            subject: String::new(),
        })
        .collect::<Vec<_>>();
    let options = mail_threading::ThreadingOptions {
        subject_merge: false,
        prune_phantoms: false,
        ..Default::default()
    };
    let threads = mail_threading::thread_messages_with(&input, &options);
    let groups = threads
        .into_iter()
        .map(|thread| {
            thread
                .messages
                .into_iter()
                .map(|id| id.parse::<i64>().map_err(|_| Error::Invalid))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = Groups { groups };
    result.validate(messages)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: i64, mid: Option<&str>, references: &[&str]) -> Message {
        Message {
            id,
            message_id: mid.map(str::to_owned),
            parent: None,
            references: references.iter().map(|id| (*id).into()).collect(),
            received_at: id,
        }
    }
    fn normalized(messages: &[Message]) -> Vec<Vec<i64>> {
        let mut groups = project(messages).expect("synthetic grouping").groups;
        for group in &mut groups {
            group.sort_unstable();
        }
        groups.sort_unstable();
        groups
    }
    #[test]
    fn missing_ancestor_siblings_and_a_late_parent_remain_one_conversation() {
        let mut messages = vec![
            message(1, Some("a@example.test"), &["parent@example.test"]),
            message(2, Some("b@example.test"), &["parent@example.test"]),
        ];
        assert_eq!(normalized(&messages), vec![vec![1, 2]]);
        messages.push(message(3, Some("parent@example.test"), &[]));
        assert_eq!(normalized(&messages), vec![vec![1, 2, 3]]);
    }
    #[test]
    fn duplicate_header_ids_preserve_distinct_messages_and_bind_references_to_the_first() {
        let messages = [
            message(1, Some("duplicate@example.test"), &[]),
            message(2, Some("duplicate@example.test"), &[]),
            message(3, Some("child@example.test"), &["duplicate@example.test"]),
        ];
        assert_eq!(normalized(&messages), vec![vec![1, 3], vec![2]]);
    }
    #[test]
    fn cycles_self_references_and_headerless_mail_preserve_exact_coverage() {
        let messages = [
            message(1, Some("a@example.test"), &["b@example.test"]),
            message(2, Some("b@example.test"), &["a@example.test"]),
            message(3, Some("self@example.test"), &["self@example.test"]),
            message(4, None, &[]),
            message(5, None, &[]),
        ];
        assert_eq!(
            normalized(&messages),
            vec![vec![1, 2], vec![3], vec![4], vec![5]]
        );
    }
    #[test]
    fn single_parent_fallback_is_used_only_when_references_are_absent() {
        let parent = message(1, Some("parent@example.test"), &[]);
        let mut reply = message(2, Some("reply@example.test"), &[]);
        reply.parent = Some("parent@example.test".into());
        assert_eq!(
            normalized(&[parent.clone(), reply.clone()]),
            vec![vec![1, 2]]
        );
        reply.references = vec!["other@example.test".into()];
        assert_eq!(normalized(&[parent, reply]), vec![vec![1], vec![2]]);
    }
    #[test]
    fn budgets_and_exact_output_coverage_reject_invalid_metadata_without_exposing_it() {
        let input = vec![
            message(1, Some("private@example.test"), &[]),
            message(2, None, &[]),
        ];
        for groups in [
            vec![vec![1]],
            vec![vec![1, 1, 2]],
            vec![vec![1, 2, 3]],
            vec![vec![], vec![1, 2]],
        ] {
            assert_eq!(Groups { groups }.validate(&input), Err(Error::Invalid));
        }
        assert_eq!(
            validate(&[input[0].clone(), input[0].clone()]),
            Err(Error::Invalid)
        );
        assert_eq!(
            validate(&vec![input[0].clone(); MAX_MESSAGES + 1]),
            Err(Error::TooLarge)
        );
        assert!(!format!("{:?}", input[0]).contains("private@example.test"));
    }
}
