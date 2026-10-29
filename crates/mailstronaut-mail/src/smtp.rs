//! Standard lettre submission. The engine persists intent before calling submit.

use core::{error::Error as StdError, fmt, time::Duration};

use lettre::{
    AsyncSmtpTransport, AsyncTransport as _, Tokio1Executor,
    transport::smtp::{
        authentication::{Credentials, Mechanism},
        client::{Certificate, Tls as TransportTls, TlsParameters},
    },
};
use mailstronaut_config::{Endpoint, Sasl, Security, Tls};
use secrecy::ExposeSecret as _;
use tokio_util::sync::CancellationToken;

use crate::{compose::Frozen, imap::Authentication};

/// Preparation failures happen before SMTP submission starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Invalid endpoint/credential or unsupported authentication mechanism.
    Configuration,
    /// Required CA/TLS configuration could not be constructed.
    Trust,
    /// Preparation timed out; no SMTP operation was started.
    TimedOut,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid or unsupported SMTP configuration",
            Self::Trust => "SMTP trust certificate configuration failed",
            Self::TimedOut => "SMTP preparation timed out; message was not submitted",
        })
    }
}
impl StdError for Error {}

/// Delivery result. Unknown is never an automatic retry signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// Server positively acknowledged DATA; Sent APPEND is a separate operation.
    Accepted,
    /// Server explicitly rejected this transaction; the message was not accepted.
    Rejected {
        /// A 4xx rejection may be retried later with the same frozen message.
        temporary: bool,
    },
    /// Authentication was explicitly refused; stop until foreground intervention.
    AuthenticationRequired,
    /// A required STARTTLS, auth or encoding capability is unavailable. In the
    /// pinned lettre adapter, these client checks precede DATA submission.
    CapabilityUnavailable,
    /// TLS failed before sending credentials or starting a mail transaction.
    TlsFailed,
    /// Cancellation was already requested before submitting anything.
    CancelledBeforeSubmission,
    /// Connection loss, cancellation or timeout after submission was attempted.
    /// The server might have accepted DATA. Persist and reconcile; do not resend.
    DeliveryUnknown,
}

/// Prepared transport, without an open socket. Debug/credentials are private.
pub struct Transport(AsyncSmtpTransport<Tokio1Executor>);
impl Transport {
    /// Construct mandatory TLS and authentication settings with native/explicit CAs.
    /// No server is contacted here and no insecure TLS variant is selected.
    ///
    /// # Errors
    /// Returns only static errors; does not retain certificate paths or secrets.
    pub async fn prepare(
        endpoint: &Endpoint,
        certificates: &Tls,
        user: &str,
        authentication: Authentication<'_>,
    ) -> Result<Self, Error> {
        if user.is_empty()
            || user.len() > 4096
            || user.chars().any(char::is_control)
            || endpoint.port == 0
        {
            return Err(Error::Configuration);
        }
        let (secret, mechanisms) = match authentication {
            Authentication::Password(password) => {
                (password, vec![Mechanism::Plain, Mechanism::Login])
            }
            Authentication::OAuth2 {
                token,
                mechanism: Sasl::Xoauth2,
            } => (token, vec![Mechanism::Xoauth2]),
            Authentication::OAuth2 { .. } => return Err(Error::Configuration),
        };
        if secret.expose_secret().is_empty()
            || secret.expose_secret().len() > crate::credentials::MAX_SECRET_BYTES
            || secret.expose_secret().chars().any(char::is_control)
        {
            return Err(Error::Configuration);
        }
        let credentials = Credentials::new(user.into(), secret.expose_secret().into());
        let endpoint = endpoint.clone();
        let files = certificates.ca_files.clone();
        tokio::time::timeout(
            Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                let host =
                    match url::Host::parse(&endpoint.host).map_err(|_| Error::Configuration)? {
                        url::Host::Domain(host) => host,
                        url::Host::Ipv4(address) => address.to_string(),
                        url::Host::Ipv6(address) => address.to_string(),
                    };
                let mut parameters = TlsParameters::builder(host.clone());
                for certificate in crate::tls::additional(&files).map_err(|_| Error::Trust)? {
                    parameters = parameters.add_root_certificate(
                        Certificate::from_der(certificate.to_vec()).map_err(|_| Error::Trust)?,
                    );
                }
                let parameters = parameters.build_rustls().map_err(|_| Error::Trust)?;
                let (builder, tls) = match endpoint.security {
                    Security::Tls => (
                        AsyncSmtpTransport::<Tokio1Executor>::relay(&host),
                        TransportTls::Wrapper(parameters),
                    ),
                    Security::Starttls => (
                        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host),
                        TransportTls::Required(parameters),
                    ),
                    _ => return Err(Error::Configuration),
                };
                let transport = builder
                    .map_err(|_| Error::Trust)?
                    .port(endpoint.port)
                    .tls(tls)
                    .credentials(credentials)
                    .authentication(mechanisms)
                    .timeout(Some(Duration::from_secs(30)))
                    .build();
                Ok(Self(transport))
            }),
        )
        .await
        .map_err(|_| Error::TimedOut)?
        .map_err(|_| Error::Trust)?
    }

    /// Submit exactly the stored MIME/envelope once. Call only after durable
    /// outbox intent is committed, and persist this result before offering retry.
    pub async fn submit(&self, message: &Frozen, cancel: &CancellationToken) -> Outcome {
        if cancel.is_cancelled() {
            return Outcome::CancelledBeforeSubmission;
        }
        // lettre's pinned DATA implementation unconditionally adds CRLF before
        // the dot terminator. Our frozen MIME includes that final CRLF, so let
        // the standard transport supply it once, preserving recipient/Sent bytes.
        let Some(data) = message.original().strip_suffix(b"\r\n") else {
            return Outcome::CapabilityUnavailable;
        };
        let operation = tokio::time::timeout(
            Duration::from_secs(120),
            self.0.send_raw(message.envelope(), data),
        );
        match cancel.run_until_cancelled(operation).await {
            Some(Ok(Ok(_))) => Outcome::Accepted,
            Some(Ok(Err(error))) if certificate_failure(&error) => Outcome::TlsFailed,
            Some(Ok(Err(error))) if error.is_client() => Outcome::CapabilityUnavailable,
            Some(Ok(Err(error)))
                if error
                    .status()
                    .is_some_and(|code| matches!(code.to_string().as_str(), "534" | "535")) =>
            {
                Outcome::AuthenticationRequired
            }
            Some(Ok(Err(error))) if error.is_permanent() || error.is_transient() => {
                Outcome::Rejected {
                    temporary: error.is_transient(),
                }
            }
            _ => Outcome::DeliveryUnknown,
        }
    }
}

fn certificate_failure(error: &lettre::transport::smtp::Error) -> bool {
    if error.is_tls() {
        return true;
    }
    // Tokio wraps Rustls handshake validation failures in an I/O error, which
    // lettre labels a connection failure. Use typed sources, never error text.
    let mut source: Option<&(dyn StdError + 'static)> = Some(error);
    for _ in 0..16 {
        let Some(error) = source else { return false };
        if error
            .downcast_ref::<rustls::Error>()
            .is_some_and(|error| matches!(error, rustls::Error::InvalidCertificate(_)))
        {
            return true;
        }
        source = error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|error| error as &(dyn StdError + 'static))
            .or_else(|| error.source());
    }
    false
}
