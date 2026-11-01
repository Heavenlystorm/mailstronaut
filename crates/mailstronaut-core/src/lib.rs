//! Local persistence primitives for Mailstronaut.

pub mod attachments;
pub mod mime;
pub mod outline;
pub mod store;

/// Bounded asynchronous access to one dedicated database owner.
pub mod worker;

/// Bounded read-only synchronization between secured IMAP and local persistence.
pub mod sync;

/// Bounded local reply and text-forward draft preparation.
pub mod reply;

/// Bounded foreground application of durable read/star intentions.
pub mod actions;

/// Bounded account-local RFC conversation projection.
pub mod threads;

/// Durable reminder time policy.
pub mod reminders;
