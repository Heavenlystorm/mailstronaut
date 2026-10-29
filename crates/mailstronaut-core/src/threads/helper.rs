//! Same-executable resource-limited RFC grouping child, initialized before the GUI.

use std::{
    io::{self, Read as _, Write as _},
    process::ExitCode,
};

use rustix::process::{Resource, Rlimit, setrlimit};

use super::{MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, Message};

/// Run one metadata-only request with no database, account credentials or network.
#[must_use = "Return this status from the dedicated threading child."]
pub fn run() -> ExitCode {
    std::panic::set_hook(Box::new(|_| {}));
    if run_inner().is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
fn run_inner() -> Result<(), Box<dyn core::error::Error>> {
    for (resource, current, maximum) in [
        (Resource::As, 1024 * 1024 * 1024, 1024 * 1024 * 1024),
        (Resource::Cpu, 5, 6),
        (Resource::Core, 0, 0),
    ] {
        setrlimit(
            resource,
            Rlimit {
                current: Some(current),
                maximum: Some(maximum),
            },
        )?;
    }
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take(u64::try_from(MAX_INPUT_BYTES + 1)?)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(super::Error::TooLarge.into());
    }
    let messages: Vec<Message> = serde_json::from_slice(&bytes)?;
    let result = super::project(&messages)?;
    let output = serde_json::to_vec(&result)?;
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(super::Error::TooLarge.into());
    }
    io::stdout().lock().write_all(&output)?;
    Ok(())
}
