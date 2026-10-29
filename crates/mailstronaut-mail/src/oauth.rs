//! Public-client OAuth using the standard OAuth2 library.
//!
//! Authorization codes/URLs/tokens never become diagnostic strings or config data.

mod callback;

use core::{error::Error as StdError, fmt, time::Duration};

use mailstronaut_config::{Auth, OAuthProvider, Sasl, Tls};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, RequestTokenError, Scope,
    TokenResponse as _, TokenUrl,
    basic::{BasicClient, BasicErrorResponseType, BasicTokenResponse, BasicTokenType},
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Retained maximum token endpoint body, excluding bounded transport buffers.
const RESPONSE_LIMIT: usize = 64 * 1024;
type Client = BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// Static OAuth errors. Provider descriptions, HTTP errors, URLs and tokens are discarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Unsupported or malformed public-client metadata.
    Configuration,
    /// TLS roots/client preparation failed.
    Trust,
    /// Token endpoint refused a grant, or no persistent refresh credential exists.
    Reauthenticate,
    /// User declined consent.
    Denied,
    /// Loopback callback was malformed or the request budget was exhausted.
    Callback,
    /// No callback arrived within the authorization time budget.
    TimedOut,
    /// The caller cancelled before completion.
    Cancelled,
    /// Network/protocol/size failure. No diagnostic response body is retained.
    Network,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid or unsupported OAuth configuration",
            Self::Trust => "OAuth TLS certificate configuration failed",
            Self::Reauthenticate => "OAuth authorization required; connect again in Accounts",
            Self::Denied => "OAuth consent was declined",
            Self::Callback => "OAuth callback was rejected",
            Self::TimedOut => "OAuth authorization timed out",
            Self::Cancelled => "OAuth authorization cancelled",
            Self::Network => "OAuth connection or token response failed",
        })
    }
}
impl StdError for Error {}

/// Private token result. Persist a rotated refresh token before publishing access.
pub struct Grant {
    access: SecretString,
    refresh: Option<SecretString>,
    expires_in: Option<Duration>,
}
impl fmt::Debug for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Grant(<private tokens>)")
    }
}
impl Grant {
    /// Borrow the bearer token for the configured SASL mechanism only.
    #[must_use]
    pub const fn access(&self) -> &SecretString {
        &self.access
    }
    /// Refresh token, if issued/rotated. Store through the scoped encrypted backend.
    #[must_use]
    pub const fn refresh(&self) -> Option<&SecretString> {
        self.refresh.as_ref()
    }
    /// Server lifetime; absence means this access token must not enter a timed cache.
    #[must_use]
    pub const fn expires_in(&self) -> Option<Duration> {
        self.expires_in
    }
}

/// Validated public-client metadata and an HTTPS-only client.
///
/// It opens no socket until the explicit authorization exchange or refresh operation.
#[derive(Clone)]
pub struct Provider {
    client: Client,
    http: reqwest::Client,
    scopes: Vec<String>,
    google: bool,
    mechanism: Sasl,
}
struct Metadata {
    client: Client,
    scopes: Vec<String>,
    google: bool,
    mechanism: Sasl,
}
impl Provider {
    /// Prepare an OAuth public client. Preset URLs cannot be overridden.
    /// No credentials, provider form, discovery, or remote code is involved.
    ///
    /// # Errors
    /// Rejects invalid metadata, unsupported mechanisms and TLS configuration.
    pub async fn prepare(auth: &Auth, certificates: &Tls) -> Result<Self, Error> {
        let Metadata {
            client,
            scopes,
            google,
            mechanism,
        } = Self::metadata(auth)?;
        let files = certificates.ca_files.clone();
        let http = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                let mut builder = reqwest::Client::builder()
                    .https_only(true)
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(10))
                    .timeout(Duration::from_secs(30));
                for certificate in crate::tls::additional(&files).map_err(|_| Error::Trust)? {
                    builder = builder.add_root_certificate(
                        reqwest::Certificate::from_der(&certificate).map_err(|_| Error::Trust)?,
                    );
                }
                builder.build().map_err(|_| Error::Trust)
            }),
        )
        .await
        .map_err(|_| Error::TimedOut)?
        .map_err(|_| Error::Trust)??;
        Ok(Self {
            client,
            http,
            scopes,
            google,
            mechanism,
        })
    }
    fn metadata(auth: &Auth) -> Result<Metadata, Error> {
        let Auth::Oauth2 {
            provider,
            client_id,
            authorization_url,
            token_url,
            scopes,
            sasl,
            ..
        } = auth
        else {
            return Err(Error::Configuration);
        };
        if client_id.trim().is_empty()
            || client_id.len() > 4096
            || client_id.chars().any(char::is_control)
            || scopes.len() > 64
        {
            return Err(Error::Configuration);
        }
        let (authorization, token, defaults) = match provider {
            OAuthProvider::Google => (
                "https://accounts.google.com/o/oauth2/v2/auth",
                "https://oauth2.googleapis.com/token",
                vec!["https://mail.google.com/"],
            ),
            OAuthProvider::Microsoft => (
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                vec![
                    "https://outlook.office.com/IMAP.AccessAsUser.All",
                    "https://outlook.office.com/SMTP.Send",
                    "offline_access",
                ],
            ),
            OAuthProvider::Custom => (
                authorization_url.as_deref().ok_or(Error::Configuration)?,
                token_url.as_deref().ok_or(Error::Configuration)?,
                Vec::new(),
            ),
            _ => return Err(Error::Configuration),
        };
        if !matches!(provider, OAuthProvider::Custom)
            && (authorization_url.is_some() || token_url.is_some())
        {
            return Err(Error::Configuration);
        }
        for value in [authorization, token] {
            let parsed = Url::parse(value).map_err(|_| Error::Configuration)?;
            if value.len() > 4096
                || parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err(Error::Configuration);
            }
        }
        let mut required = defaults.into_iter().map(str::to_owned).collect::<Vec<_>>();
        for scope in scopes {
            if scope.is_empty()
                || scope.len() > 1024
                || scope.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(Error::Configuration);
            }
            if !required.contains(scope) {
                required.push(scope.clone());
            }
        }
        if required.is_empty() {
            return Err(Error::Configuration);
        }
        let mechanism = sasl.unwrap_or(Sasl::Xoauth2);
        if matches!(provider, OAuthProvider::Custom) && sasl.is_none() {
            return Err(Error::Configuration);
        }
        let client = BasicClient::new(ClientId::new(client_id.clone()))
            .set_auth_uri(AuthUrl::new(authorization.into()).map_err(|_| Error::Configuration)?)
            .set_token_uri(TokenUrl::new(token.into()).map_err(|_| Error::Configuration)?)
            .set_auth_type(AuthType::RequestBody);
        Ok(Metadata {
            client,
            scopes: required,
            google: matches!(provider, OAuthProvider::Google),
            mechanism,
        })
    }
    /// SASL mechanism from validated metadata, subject to server capability checks.
    #[must_use]
    pub const fn mechanism(&self) -> Sasl {
        self.mechanism
    }
    /// Bind the loopback callback before providing the external-browser URL.
    ///
    /// # Errors
    /// Returns a local listener/configuration failure without authorizing anything.
    pub async fn begin(&self, address: &str) -> Result<Flow, Error> {
        if address.len() > 4096 || address.chars().any(char::is_control) {
            return Err(Error::Configuration);
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| Error::Callback)?;
        let authority = listener
            .local_addr()
            .map_err(|_| Error::Callback)?
            .to_string();
        let redirect = RedirectUrl::new(format!("http://{authority}/oauth2/callback"))
            .map_err(|_| Error::Configuration)?;
        let client = self.client.clone().set_redirect_uri(redirect);
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(challenge)
            .add_extra_param("login_hint", address);
        for scope in &self.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }
        if self.google {
            request = request
                .add_extra_param("access_type", "offline")
                .add_extra_param("prompt", "consent");
        }
        let (url, state) = request.url();
        Ok(Flow {
            provider: self.clone(),
            client,
            listener,
            authority,
            url,
            state,
            verifier,
        })
    }
    /// Refresh a scoped encrypted token with serialized callers supplied by the
    /// account credential owner. No interactive prompt or automatic retry occurs.
    ///
    /// # Errors
    /// Invalid/revoked grants require foreground authorization; other failures are static.
    pub async fn refresh(
        &self,
        refresh: &SecretString,
        cancel: &CancellationToken,
    ) -> Result<Grant, Error> {
        if !valid_secret(refresh.expose_secret()) {
            return Err(Error::Reauthenticate);
        }
        let refresh = RefreshToken::new(refresh.expose_secret().into());
        let result = cancel
            .run_until_cancelled(
                self.client
                    .exchange_refresh_token(&refresh)
                    .request_async(self),
            )
            .await
            .ok_or(Error::Cancelled)?;
        self.grant(&result.map_err(token_error)?)
    }
    async fn request(&self, request: oauth2::HttpRequest) -> Result<oauth2::HttpResponse, Error> {
        let (parts, body) = request.into_parts();
        let mut response = self
            .http
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if response
            .content_length()
            .is_some_and(|size| size > RESPONSE_LIMIT as u64)
        {
            return Err(Error::Network);
        }
        let mut output = oauth2::http::Response::builder().status(response.status());
        *output.headers_mut().ok_or(Error::Network)? = response.headers().clone();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(body.len()) {
                return Err(Error::Network);
            }
            body.extend_from_slice(&chunk);
        }
        output.body(body).map_err(|_| Error::Network)
    }
    fn grant(&self, response: &BasicTokenResponse) -> Result<Grant, Error> {
        if *response.token_type() != BasicTokenType::Bearer
            || !valid_secret(response.access_token().secret())
        {
            return Err(Error::Network);
        }
        if let Some(granted) = response.scopes() {
            for required in &self.scopes {
                if required != "offline_access"
                    && !granted.iter().any(|scope| scope.as_str() == required)
                {
                    return Err(Error::Reauthenticate);
                }
            }
        }
        let refresh = response.refresh_token().map(RefreshToken::secret);
        if refresh.is_some_and(|token| !valid_secret(token))
            || response.expires_in() == Some(Duration::ZERO)
        {
            return Err(Error::Reauthenticate);
        }
        Ok(Grant {
            access: SecretString::from(response.access_token().secret().clone()),
            refresh: refresh.cloned().map(SecretString::from),
            expires_in: response.expires_in(),
        })
    }
}

impl<'a> oauth2::AsyncHttpClient<'a> for Provider {
    type Error = Error;
    type Future = futures_util::future::BoxFuture<'a, Result<oauth2::HttpResponse, Error>>;

    fn call(&'a self, request: oauth2::HttpRequest) -> Self::Future {
        Box::pin(self.request(request))
    }
}

/// Single-use browser authorization. Dropping or completing it closes its listener.
pub struct Flow {
    provider: Provider,
    client: Client,
    listener: TcpListener,
    authority: String,
    url: Url,
    state: CsrfToken,
    verifier: PkceCodeVerifier,
}
impl Flow {
    /// Borrow only for an external browser portal; never log this URL.
    #[must_use]
    pub const fn authorization_url(&self) -> &Url {
        &self.url
    }
    /// Accept a bounded, state-checked callback and exchange its one-use code.
    ///
    /// # Errors
    /// Returns cancellation, denial, timeout or static callback/token errors.
    pub async fn finish(self, cancel: &CancellationToken) -> Result<Grant, Error> {
        let callback = callback::receive(&self.listener, &self.authority, &self.state);
        let code = cancel
            .run_until_cancelled(tokio::time::timeout(Duration::from_secs(300), callback))
            .await
            .ok_or(Error::Cancelled)?
            .map_err(|_| Error::TimedOut)??;
        // Close the listener before token exchange; a duplicate callback cannot reuse it.
        drop(self.listener);
        let result = cancel
            .run_until_cancelled(
                self.client
                    .exchange_code(AuthorizationCode::new(code))
                    .set_pkce_verifier(self.verifier)
                    .request_async(&self.provider),
            )
            .await
            .ok_or(Error::Cancelled)?;
        self.provider.grant(&result.map_err(token_error)?)
    }
}
fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= crate::credentials::MAX_SECRET_BYTES
        && !value.chars().any(char::is_control)
}
fn token_error(error: RequestTokenError<Error, oauth2::basic::BasicErrorResponse>) -> Error {
    match error {
        RequestTokenError::ServerResponse(response)
            if *response.error() == BasicErrorResponseType::InvalidGrant =>
        {
            Error::Reauthenticate
        }
        RequestTokenError::Request(error) => error,
        _ => Error::Network,
    }
}
