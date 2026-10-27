//! Same-binary configuration evaluator child and its internal wire response.
use core::{error::Error, str::from_utf8};
use std::fs::OpenOptions;
use std::io::{self, Read as _, Write};
use std::panic::set_hook;
use std::process::ExitCode;

use rustix::process::{Resource, Rlimit, setrlimit};
use rustix::stdio::dup2_stderr;
use serde::{Deserialize, Serialize};

use super::{Config, MAX_SOURCE_BYTES};

/// Maximum encoded response size in bytes.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Typed response on the private child protocol.
#[expect(
    clippy::large_enum_variant,
    reason = "The agreed wire contract carries Config directly."
)]
#[derive(Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Response {
    /// A successfully evaluated and validated configuration.
    Success(Config),
    /// The source or expanded configuration was rejected.
    Rejected,
}

/// Run only in the dedicated evaluator child before GUI initialization.
///
/// Failure to impose limits, read input, serialize, or write the response exits
/// unsuccessfully. Source and evaluator diagnostics are never emitted.
#[must_use = "Return this status from the evaluator child."]
#[inline]
#[expect(
    clippy::implicit_return,
    reason = "Exit status is selected as an expression."
)]
pub fn run() -> ExitCode {
    set_hook(Box::new(|_| {}));
    if run_inner().is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Impose limits and process one request.
#[expect(
    clippy::question_mark_used,
    clippy::implicit_return,
    clippy::single_call_fn,
    reason = "Keep the I/O sequence and failure propagation clear."
)]
fn run_inner() -> Result<(), Box<dyn Error>> {
    // Snix builtins.trace writes untrusted values to stderr. Discard them.
    let sink = OpenOptions::new().write(true).open("/dev/null")?;
    dup2_stderr(&sink)?;
    setrlimit(
        Resource::As,
        Rlimit {
            current: Some(1024 * 1024 * 1024),
            maximum: Some(1024 * 1024 * 1024),
        },
    )?;
    setrlimit(
        Resource::Cpu,
        Rlimit {
            current: Some(2),
            maximum: Some(3),
        },
    )?;
    setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )?;
    let mut source = Vec::new();
    io::stdin()
        .lock()
        .take(u64::try_from(MAX_SOURCE_BYTES + 1)?)
        .read_to_end(&mut source)?;
    let response = evaluate(&source);
    let encoded = encode(&response)?;
    io::stdout().lock().write_all(&encoded)?;
    Ok(())
}

/// Evaluate only bounded UTF-8 input and redact errors.
#[expect(
    clippy::implicit_return,
    reason = "Separate evaluation makes the child behavior testable."
)]
#[cfg_attr(
    not(test),
    expect(
        clippy::single_call_fn,
        reason = "Evaluation is separated for testing."
    )
)]
fn evaluate(source: &[u8]) -> Response {
    if source.len() > MAX_SOURCE_BYTES {
        return Response::Rejected;
    }
    let Ok(text) = from_utf8(source) else {
        return Response::Rejected;
    };
    let Ok(config) = snix_serde::from_str::<Config>(text) else {
        return Response::Rejected;
    };
    if !config.validate().is_empty() {
        return Response::Rejected;
    }
    Response::Success(config)
}

/// A writer which refuses an entire write once the byte limit would be exceeded.
struct BoundedWriter {
    /// Buffered wire bytes.
    bytes: Vec<u8>,
    /// Whether a write crossed the limit.
    exceeded: bool,
}

#[expect(
    clippy::missing_trait_methods,
    clippy::implicit_return,
    reason = "Default Write methods delegate to the bounded write implementation."
)]
impl Write for BoundedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > MAX_RESPONSE_BYTES.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("response size limit"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Encode within the cap or substitute a rejection.
#[expect(
    clippy::question_mark_used,
    clippy::implicit_return,
    reason = "Separate encoding makes the bounded behavior testable."
)]
#[cfg_attr(
    not(test),
    expect(clippy::single_call_fn, reason = "Encoding is separated for testing.")
)]
fn encode(response: &Response) -> Result<Vec<u8>, serde_json::Error> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, response);
    if writer.exceeded {
        return serde_json::to_vec(&Response::Rejected);
    }
    result?;
    Ok(writer.bytes)
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::shadow_unrelated,
    reason = "Synthetic tests assert directly on encoded fixtures."
)]
mod tests {
    use super::*;

    #[test]
    fn typed_responses_and_rejection() {
        let response = evaluate(b"{ schemaVersion = 1; }");
        let bytes = encode(&response).unwrap();
        assert!(matches!(
            serde_json::from_slice::<Response>(&bytes).unwrap(),
            Response::Success(_)
        ));
        for source in [
            b"{ schemaVersion = 2; }".as_slice(),
            b"\xff",
            b"{ schemaVersion = builtins.readFile /etc/passwd; }",
        ] {
            let bytes = encode(&evaluate(source)).unwrap();
            assert!(matches!(
                serde_json::from_slice::<Response>(&bytes).unwrap(),
                Response::Rejected
            ));
        }
        assert!(matches!(
            evaluate(&vec![b' '; MAX_SOURCE_BYTES + 1]),
            Response::Rejected
        ));
    }

    #[test]
    fn oversized_expansion_is_rejected() {
        let response = evaluate(b"let s = builtins.concatStringsSep \"\" (builtins.genList (_: \"0123456789\") 110000); in { schemaVersion = 1; locale = s; }");
        assert!(matches!(response, Response::Success(_)));
        let bytes = encode(&response).unwrap();
        assert!(bytes.len() <= MAX_RESPONSE_BYTES);
        assert!(matches!(
            serde_json::from_slice::<Response>(&bytes).unwrap(),
            Response::Rejected
        ));
    }
}
