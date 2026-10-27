//! Buildable entry point for the Mailstronaut planning baseline.

/// Report the scaffold's status until the application milestones are implemented.
#[expect(
    clippy::print_stdout,
    reason = "This scaffold only reports its status."
)]
fn main() {
    println!("Mailstronaut: planning scaffold; the email client is not implemented yet.");
}
