//! Account authentication and mail transport boundaries.

pub mod credentials;

/// TLS-only, bounded IMAP connection and authentication.
pub mod imap;

mod tls;

/// Typed MIME construction; only explicit submission sends frozen message bytes.
pub mod compose;

/// Required-TLS SMTP submission of immutable MIME, with explicit ambiguity results.
pub mod smtp;

/// External-browser public-client OAuth and bounded token exchange.
pub mod oauth;

/// Shared password access and serialized encrypted OAuth refresh lifecycle.
pub mod auth;
