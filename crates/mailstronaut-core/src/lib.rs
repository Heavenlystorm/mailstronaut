//! Local persistence primitives for Mailstronaut.

pub mod mime;
pub mod store;

/// Bounded asynchronous access to one dedicated database owner.
pub mod worker;
