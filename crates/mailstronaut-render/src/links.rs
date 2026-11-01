//! Web-only link values validated with the renderer's existing URL library.

use core::fmt;

/// Maximum distinct external links retained for one bounded email document.
pub const MAX_LINKS: usize = 128;
/// Maximum encoded length of an external web address.
pub const MAX_LINK_BYTES: usize = 4096;

/// A canonical HTTP(S) target from a sanitized email; not navigation authority.
///
/// Constructed only by the renderer. A host must still require an explicit user
/// action on the current document before giving this value to a browser portal.
#[derive(Clone, Eq, PartialEq)]
pub struct Link(String);

impl Link {
    pub(crate) fn parse(raw: &str, document: &blitz_dom::BaseDocument) -> Option<Self> {
        if raw.len() > MAX_LINK_BYTES
            || raw.chars().any(|c| {
                c.is_control()
                    || c == '\\'
                    || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        {
            return None;
        }
        // Use the public URL type already supplied by Blitz, not a URL parser
        // or scheme/authority splitter implemented in the application.
        let url = document.url().join(raw).ok()?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.as_str().len() > MAX_LINK_BYTES
        {
            return None;
        }
        Some(Self(url.into()))
    }

    /// Canonical address, including the ASCII host, for native display/portal use.
    /// Never include it in application errors, logs or generic Debug output.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}
