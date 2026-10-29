//! IMAP over validated TLS, with cancellation, operation limits and redacted errors.

use core::{error::Error as StdError, fmt, future::Future, time::Duration};
use std::collections::BTreeSet;

use async_imap::{
    Client, Session,
    imap_proto::{Capability, Response, Status},
};
use mailstronaut_config::{Endpoint, Sasl, Security, Tls};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

mod read;
pub use read::{Folder, MessageData, Metadata};

/// Total received bytes for one connection, including protocol framing. A worker
/// reconnects/reconciles when this budget is exhausted rather than reading forever.
pub const MAX_SESSION_BYTES: u64 = 32 * 1024 * 1024;
/// Wall-time budget for connection/authentication and each finite operation.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport failures contain no server text, credential, URL or message contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Authentication was refused; stop automatic credential retries.
    Authentication,
    /// The requested authentication mechanism is not advertised.
    AuthenticationUnavailable,
    /// Caller cancelled the operation.
    Cancelled,
    /// Invalid endpoint, credentials or unsupported settings.
    Configuration,
    /// Session ended or is no longer usable.
    Disconnected,
    /// Network operation failed.
    Network,
    /// Malformed or unexpected protocol response.
    Protocol,
    /// A response exceeded the local budget.
    ResponseLimit,
    /// Mandatory STARTTLS was unavailable or refused.
    StarttlsRequired,
    /// Operation exceeded its wall-time budget.
    TimedOut,
    /// Certificate, hostname or TLS handshake validation failed.
    Tls,
    /// Required trust roots could not be loaded.
    TrustStore,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Authentication => "mail authentication refused; reconnect in the foreground",
            Self::AuthenticationUnavailable => "required authentication mechanism unavailable",
            Self::Cancelled => "mail operation cancelled",
            Self::Configuration => "invalid mail connection configuration or credentials",
            Self::Disconnected => "mail connection closed; reconnect required",
            Self::Network => "mail network operation failed",
            Self::Protocol => "unexpected mail server response",
            Self::ResponseLimit => "mail server response exceeds the supported limit",
            Self::StarttlsRequired => "required STARTTLS unavailable; credentials were not sent",
            Self::TimedOut => "mail operation timed out; reconnect required",
            Self::Tls => "mail TLS certificate, hostname or handshake validation failed",
            Self::TrustStore => "unable to load required mail trust certificates",
        })
    }
}
impl StdError for Error {}

trait Io: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send> Io for T {}
type Wire = Box<dyn Io>;

/// Credential material is borrowed for authentication and not kept by a session.
#[derive(Clone, Copy)]
pub enum Authentication<'a> {
    /// Password or app password from the system credential store.
    Password(&'a SecretString),
    /// A refreshed access token; refresh tokens never enter a SASL exchange.
    OAuth2 {
        /// Redacted, in-memory access token.
        token: &'a SecretString,
        /// Explicit configured mechanism, checked against server capabilities.
        mechanism: Sasl,
    },
}

/// An authenticated connection. Its raw session/stream are deliberately private.
/// Any failed/cancelled operation retires its session to avoid protocol desynchronization.
pub struct Connection {
    session: Option<Session<Wire>>,
    capabilities: BTreeSet<String>,
}
impl Connection {
    /// Connect and authenticate only after validating implicit/required-upgrade TLS.
    /// Native roots and explicit CA files use Rustls' standard certificate/hostname
    /// verifier. No accept-invalid-certificate or plaintext fallback exists.
    ///
    /// # Errors
    /// Returns a redacted configuration, TLS, network, cancellation or auth error.
    pub async fn connect(
        endpoint: &Endpoint,
        certificates: &Tls,
        username: &str,
        authentication: Authentication<'_>,
        cancellation: &CancellationToken,
    ) -> Result<Self, Error> {
        limited(cancellation, async {
            if username.is_empty()
                || username.len() > 4096
                || username.chars().any(char::is_control)
            {
                return Err(Error::Configuration);
            }
            let (client, capabilities) = secure_client(endpoint, certificates).await?;
            let session = match authentication {
                Authentication::Password(password) => {
                    validate_secret(password)?;
                    if capabilities.contains("AUTH=PLAIN") {
                        client
                            .authenticate("PLAIN", SaslReply::new(username, password, Kind::Plain))
                            .await
                            .map_err(|(error, _)| auth_error(error))?
                    } else if !capabilities.contains("LOGINDISABLED") {
                        client
                            .login(username, password.expose_secret())
                            .await
                            .map_err(|(error, _)| auth_error(error))?
                    } else {
                        return Err(Error::AuthenticationUnavailable);
                    }
                }
                Authentication::OAuth2 { token, mechanism } => {
                    validate_secret(token)?;
                    let kind = match mechanism {
                        Sasl::Xoauth2 => Kind::Xoauth2,
                        Sasl::Oauthbearer => Kind::Bearer,
                        _ => return Err(Error::Configuration),
                    };
                    if !capabilities.contains(&format!("AUTH={}", kind.name())) {
                        return Err(Error::AuthenticationUnavailable);
                    }
                    client
                        .authenticate(kind.name(), SaslReply::new(username, token, kind))
                        .await
                        .map_err(|(error, _)| auth_error(error))?
                }
            };
            let mut connection = Self {
                session: Some(session),
                capabilities,
            };
            // TLS and authentication may both change capabilities. The post-auth
            // set drives folder discovery, IDLE and optional provider extensions.
            connection.capabilities = connection
                .session
                .as_mut()
                .ok_or(Error::Disconnected)?
                .capabilities()
                .await
                .map_err(protocol_error)?
                .iter()
                .map(capability_name)
                .collect();
            Ok(connection)
        })
        .await
    }

    /// Whether this authenticated session advertised a concrete extension.
    #[must_use]
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.contains(&capability.to_ascii_uppercase())
    }

    /// Select one mailbox, returning the library's typed UIDVALIDITY/UIDNEXT data.
    /// The result's sequence-number fields must never be used for message identity.
    ///
    /// # Errors
    /// Returns a redacted error and retires the stream on any failed operation.
    pub async fn select(
        &mut self,
        mailbox: &str,
        cancellation: &CancellationToken,
    ) -> Result<async_imap::types::Mailbox, Error> {
        read::validate_mailbox(mailbox)?;
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let selected = limited(cancellation, async {
            session.select(mailbox).await.map_err(protocol_error)
        })
        .await?;
        if selected.uid_validity.is_none_or(|value| value == 0) {
            return Err(Error::Protocol);
        }
        self.session = Some(session);
        Ok(selected)
    }
}

async fn secure_client(
    endpoint: &Endpoint,
    certificates: &Tls,
) -> Result<(Client<Wire>, BTreeSet<String>), Error> {
    let host = match url::Host::parse(&endpoint.host).map_err(|_| Error::Configuration)? {
        url::Host::Domain(host) => host,
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    };
    if endpoint.port == 0 {
        return Err(Error::Configuration);
    }
    let configuration = crate::tls::config(certificates.ca_files.clone()).await?;
    let tcp = TcpStream::connect((host.as_str(), endpoint.port))
        .await
        .map_err(|_| Error::Network)?;
    tcp.set_nodelay(true).map_err(|_| Error::Network)?;
    let (reader, writer) = tokio::io::split(tcp);
    let mut initial = tokio::io::join(reader.take(64 * 1024), writer);
    let (wire, greeting_read) = match endpoint.security {
        Security::Tls => {
            initial.reader_mut().set_limit(MAX_SESSION_BYTES);
            (initial, false)
        }
        Security::Starttls => {
            let mut plain = Client::new(initial);
            greeting(&mut plain).await?;
            let capabilities = capabilities(&mut plain).await?;
            if !capabilities.contains("STARTTLS") {
                return Err(Error::StarttlsRequired);
            }
            plain
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|_| Error::StarttlsRequired)?;
            let mut wire = plain.into_inner();
            wire.reader_mut().set_limit(MAX_SESSION_BYTES);
            (wire, true)
        }
        _ => return Err(Error::Configuration),
    };
    let server = rustls::pki_types::ServerName::try_from(host).map_err(|_| Error::Configuration)?;
    let tls = TlsConnector::from(configuration)
        .connect(server, wire)
        .await
        .map_err(|_| Error::Tls)?;
    // Standard Tokio adapters bound reads while retaining normal writes.
    let (reader, writer) = tokio::io::split(tls);
    let wire: Wire = Box::new(tokio::io::join(reader.take(MAX_SESSION_BYTES), writer));
    let mut client = Client::new(wire);
    if !greeting_read {
        greeting(&mut client).await?;
    }
    let capabilities = capabilities(&mut client).await?;
    Ok((client, capabilities))
}

async fn greeting<T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send>(
    client: &mut Client<T>,
) -> Result<(), Error> {
    let response = client
        .read_response()
        .await
        .map_err(|_| Error::Protocol)?
        .ok_or(Error::Disconnected)?;
    if matches!(
        response.parsed(),
        Response::Data {
            status: Status::Ok,
            ..
        }
    ) {
        Ok(())
    } else {
        Err(Error::Protocol)
    }
}
async fn capabilities<T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send>(
    client: &mut Client<T>,
) -> Result<BTreeSet<String>, Error> {
    let (sender, receiver) = async_channel::bounded(64);
    client
        .run_command_and_check_ok("CAPABILITY", Some(sender))
        .await
        .map_err(protocol_error)?;
    let mut capabilities = BTreeSet::new();
    while let Ok(response) = receiver.try_recv() {
        if let async_imap::types::UnsolicitedResponse::Other(response) = response
            && let Response::Capabilities(values) = response.parsed()
        {
            for value in values {
                capabilities.insert(protocol_capability_name(value));
            }
        }
    }
    if capabilities.is_empty() || capabilities.len() > 512 {
        return Err(Error::Protocol);
    }
    Ok(capabilities)
}
fn protocol_capability_name(value: &Capability<'_>) -> String {
    match value {
        Capability::Imap4rev1 => "IMAP4REV1".into(),
        Capability::Auth(name) => format!("AUTH={}", name.to_ascii_uppercase()),
        Capability::Atom(name) => name.to_ascii_uppercase(),
    }
}
fn capability_name(value: &async_imap::types::Capability) -> String {
    match value {
        async_imap::types::Capability::Imap4rev1 => "IMAP4REV1".into(),
        async_imap::types::Capability::Auth(name) => format!("AUTH={}", name.to_ascii_uppercase()),
        async_imap::types::Capability::Atom(name) => name.to_ascii_uppercase(),
    }
}
#[expect(
    clippy::needless_pass_by_value,
    reason = "The map_err boundary consumes and discards potentially sensitive server diagnostics."
)]
fn protocol_error(error: async_imap::error::Error) -> Error {
    match error {
        async_imap::error::Error::ConnectionLost => Error::Disconnected,
        async_imap::error::Error::Io(_) => Error::Network,
        _ => Error::Protocol,
    }
}
fn auth_error(error: async_imap::error::Error) -> Error {
    match error {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => Error::Authentication,
        other => protocol_error(other),
    }
}
fn validate_secret(value: &SecretString) -> Result<(), Error> {
    let secret = value.expose_secret();
    if secret.is_empty()
        || secret.len() > crate::credentials::MAX_SECRET_BYTES
        || secret.chars().any(char::is_control)
    {
        Err(Error::Configuration)
    } else {
        Ok(())
    }
}
async fn limited<T>(
    cancellation: &CancellationToken,
    operation: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    cancellation
        .run_until_cancelled(tokio::time::timeout(OPERATION_TIMEOUT, operation))
        .await
        .ok_or(Error::Cancelled)?
        .map_err(|_| Error::TimedOut)?
}

#[derive(Clone, Copy)]
enum Kind {
    Plain,
    Xoauth2,
    Bearer,
}
impl Kind {
    const fn name(self) -> &'static str {
        match self {
            Self::Plain => "PLAIN",
            Self::Xoauth2 => "XOAUTH2",
            Self::Bearer => "OAUTHBEARER",
        }
    }
}
struct SecretReply(SecretString);
impl AsRef<[u8]> for SecretReply {
    fn as_ref(&self) -> &[u8] {
        self.0.expose_secret().as_bytes()
    }
}
struct SaslReply<'a> {
    user: &'a str,
    secret: &'a SecretString,
    kind: Kind,
    sent: bool,
}
impl<'a> SaslReply<'a> {
    const fn new(user: &'a str, secret: &'a SecretString, kind: Kind) -> Self {
        Self {
            user,
            secret,
            kind,
            sent: false,
        }
    }
}
impl async_imap::Authenticator for SaslReply<'_> {
    type Response = SecretReply;
    fn process(&mut self, challenge: &[u8]) -> Self::Response {
        if core::mem::replace(&mut self.sent, true) || !challenge.is_empty() {
            return SecretReply(SecretString::default());
        }
        let secret = self.secret.expose_secret();
        let response = match self.kind {
            Kind::Plain => format!("\0{}\0{secret}", self.user),
            Kind::Xoauth2 => format!("user={}\x01auth=Bearer {secret}\x01\x01", self.user),
            Kind::Bearer => format!(
                "n,a={},\x01auth=Bearer {secret}\x01\x01",
                self.user.replace('=', "=3D").replace(',', "=2C")
            ),
        };
        SecretReply(SecretString::from(response))
    }
}
