//! Account authentication and mail transport boundaries.

pub mod credentials;

/// TLS-only, bounded IMAP connection and authentication.
pub mod imap;

mod tls;
