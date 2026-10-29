//! Account authentication and mail transport boundaries.

pub mod credentials;

/// TLS-only, bounded IMAP connection and authentication.
pub mod imap;

mod tls;

/// Typed MIME construction; only explicit submission sends frozen message bytes.
pub mod compose;

/// Required-TLS SMTP submission of immutable MIME, with explicit ambiguity results.
pub mod smtp;
