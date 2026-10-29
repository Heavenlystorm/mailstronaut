//! Typed, secret-free configuration contract for Mailstronaut.
/// Internal evaluator child protocol.
pub mod helper;

/// Parent lifecycle for the short-lived configuration evaluator.
pub mod process;

/// Native configuration files and last-valid runtime snapshots.
pub mod source;

use core::iter::once;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

/// Configuration format supported by this crate.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Config {
    /// Format version, currently 1.
    pub schema_version: u32,
    /// Whether the source is externally managed.
    #[serde(default)]
    pub managed: bool,
    /// User interface locale.
    pub locale: Option<String>,
    /// Time zone identifier.
    pub time_zone: Option<String>,
    /// Native theme selection.
    #[serde(default)]
    pub appearance: Appearance,
    /// Background operation settings.
    #[serde(default)]
    pub background: Background,
    /// Notification settings.
    #[serde(default)]
    pub notifications: Notifications,
    /// Remote-content policy.
    #[serde(default)]
    pub privacy: Privacy,
    /// Synchronization limits.
    #[serde(default)]
    pub sync: Sync,
    /// Local priority configuration.
    #[serde(default)]
    pub priority: Priority,
    /// Default reminder times.
    #[serde(default)]
    pub reminders: Reminders,
    /// Declarative account list.
    #[serde(default)]
    pub accounts: Vec<Account>,
}

/// Native theme selection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Appearance {
    #[default]
    /// Follow the system theme.
    System,
    /// Use the light theme.
    Light,
    /// Use the dark theme.
    Dark,
}
/// Background operation settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Background {
    /// Keep running after the window closes.
    pub enabled: bool,
    /// Request portal autostart.
    pub autostart: bool,
}
/// Notification settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Notifications {
    /// Enable notifications.
    pub enabled: bool,
    /// Include private message previews.
    pub show_preview: bool,
}
impl Default for Notifications {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: true,
            show_preview: false,
        }
    }
}
/// Remote-content policy.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Privacy {
    /// Policy for remote content.
    pub remote_content: RemoteContent,
}
/// Remote resources are never fetched on message open.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum RemoteContent {
    #[default]
    /// Ask before loading remote content.
    Ask,
    /// Never load remote content.
    Never,
}
/// Synchronization limits.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Sync {
    /// Initial history in days.
    pub initial_days: u32,
    /// Polling interval in seconds.
    pub poll_seconds: u32,
    /// Maximum body bytes for analysis.
    pub max_body_bytes: u32,
}
impl Default for Sync {
    #[inline]
    fn default() -> Self {
        Self {
            initial_days: 30,
            poll_seconds: 300,
            max_body_bytes: 0x20_0000,
        }
    }
}
/// Local priority analysis settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Priority {
    /// Whether analysis is enabled.
    pub enabled: bool,
    /// Local model directory, if supplied.
    pub model_directory: Option<String>,
    /// Identity of the local model.
    pub model_revision: Option<String>,
    /// Enabled languages.
    pub languages: Vec<String>,
    /// Default lifetime in hours.
    pub default_ttl_hours: u32,
    /// Maximum lifetime in hours.
    pub max_ttl_hours: u32,
}
impl Default for Priority {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: false,
            model_directory: None,
            model_revision: None,
            languages: vec!["en".into(), "fr".into(), "it".into()],
            default_ttl_hours: 72,
            max_ttl_hours: 168,
        }
    }
}
/// Default reminder times in local time.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Reminders {
    /// Morning time in HH:MM format.
    pub morning: String,
    /// Evening time in HH:MM format.
    pub evening: String,
}
impl Default for Reminders {
    #[inline]
    fn default() -> Self {
        Self {
            morning: "09:00".into(),
            evening: "18:00".into(),
        }
    }
}
/// A configured mailbox; credentials are referenced, never embedded.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Account {
    /// Stable account identifier.
    pub id: String,
    /// Provider preset.
    pub provider: Provider,
    /// Address presented to recipients.
    pub address: String,
    /// Display name.
    pub display_name: String,
    /// Incoming login.
    pub user_name: String,
    /// Optional outgoing login (otherwise incoming login).
    pub smtp_user_name: Option<String>,
    /// Additional sender identities.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Incoming endpoint (required for manual accounts).
    pub imap: Option<Endpoint>,
    /// Outgoing endpoint (required for manual accounts).
    pub smtp: Option<Endpoint>,
    /// Incoming authentication.
    pub auth: Auth,
    /// Optional distinct outgoing authentication.
    pub smtp_auth: Option<Auth>,
    /// Certificate authority files.
    #[serde(default)]
    pub tls: Tls,
    /// Optional mailbox overrides.
    #[serde(default)]
    pub folders: Folders,
}
impl Account {
    /// Effective submission endpoint, always requiring implicit TLS or STARTTLS.
    #[must_use]
    pub fn outgoing_endpoint(&self) -> Option<Endpoint> {
        self.smtp.clone().or_else(|| {
            let host = match self.provider {
                Provider::Google => "smtp.gmail.com",
                Provider::Icloud => "smtp.mail.me.com",
                Provider::Manual => return None,
            };
            Some(Endpoint {
                host: host.into(),
                port: 587,
                security: Security::Starttls,
            })
        })
    }
    /// Explicit outgoing login wins; iCloud otherwise requires the full address.
    #[must_use]
    pub fn outgoing_user_name(&self) -> &str {
        self.smtp_user_name.as_deref().unwrap_or({
            if matches!(self.provider, Provider::Icloud) {
                self.address.as_str()
            } else {
                self.user_name.as_str()
            }
        })
    }
    /// Effective incoming endpoint: explicit configuration overrides the provider
    /// preset. Institutional/manual accounts always require an explicit endpoint.
    #[must_use]
    pub fn incoming_endpoint(&self) -> Option<Endpoint> {
        self.imap.clone().or_else(|| {
            let host = match self.provider {
                Provider::Google => "imap.gmail.com",
                Provider::Icloud => "imap.mail.me.com",
                Provider::Manual => return None,
            };
            Some(Endpoint {
                host: host.into(),
                port: 993,
                security: Security::Tls,
            })
        })
    }
}
/// Supported endpoint presets.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Provider {
    /// Google preset.
    Google,
    /// iCloud preset.
    Icloud,
    /// Manually configured servers.
    Manual,
}
/// Mail server endpoint.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Endpoint {
    /// Server hostname.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Required transport encryption.
    pub security: Security,
}
/// Required transport encryption.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Security {
    /// Implicit TLS.
    Tls,
    /// Required STARTTLS.
    Starttls,
}
/// Authentication metadata. Secret values are held in a credential service.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    try_from = "RawAuth",
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum Auth {
    /// Password or app password stored by reference.
    Password {
        /// Credential identifier.
        secret_ref: String,
    },
    /// Public-client OAuth metadata.
    Oauth2 {
        /// OAuth provider.
        provider: OAuthProvider,
        /// Public client identifier.
        client_id: String,
        /// Credential identifier.
        secret_ref: String,
        /// Custom authorization endpoint.
        authorization_url: Option<String>,
        /// Custom token endpoint.
        token_url: Option<String>,
        /// OAuth scopes.
        #[serde(default)]
        scopes: Vec<String>,
        /// SASL mechanism.
        sasl: Option<Sasl>,
    },
}
/// A typed map avoids Snix's `deserialize_any` string display conversion.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAuth {
    /// Corresponding authentication field.
    kind: AuthKind,
    /// Corresponding authentication field.
    secret_ref: String,
    /// Corresponding authentication field.
    provider: Option<OAuthProvider>,
    /// Corresponding authentication field.
    client_id: Option<String>,
    /// Corresponding authentication field.
    authorization_url: Option<String>,
    /// Corresponding authentication field.
    token_url: Option<String>,
    /// Corresponding authentication field.
    scopes: Option<Vec<String>>,
    /// Corresponding authentication field.
    sasl: Option<Sasl>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
/// Typed authentication discriminator.
enum AuthKind {
    /// Password authentication.
    Password,
    /// OAuth authentication.
    Oauth2,
}

impl TryFrom<RawAuth> for Auth {
    type Error = &'static str;

    #[inline]
    fn try_from(raw: RawAuth) -> Result<Self, Self::Error> {
        match raw.kind {
            AuthKind::Password => {
                if raw.provider.is_some()
                    || raw.client_id.is_some()
                    || raw.authorization_url.is_some()
                    || raw.token_url.is_some()
                    || raw.scopes.is_some()
                    || raw.sasl.is_some()
                {
                    return Err("password authentication has OAuth fields");
                }
                Ok(Self::Password {
                    secret_ref: raw.secret_ref,
                })
            }
            AuthKind::Oauth2 => Ok(Self::Oauth2 {
                provider: raw.provider.ok_or("OAuth provider required")?,
                client_id: raw.client_id.ok_or("OAuth client ID required")?,
                secret_ref: raw.secret_ref,
                authorization_url: raw.authorization_url,
                token_url: raw.token_url,
                scopes: raw.scopes.unwrap_or_default(),
                sasl: raw.sasl,
            }),
        }
    }
}
/// OAuth provider.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum OAuthProvider {
    /// Google preset.
    Google,
    /// Microsoft preset.
    Microsoft,
    /// Custom provider.
    Custom,
}
/// OAuth SASL mechanism.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Sasl {
    /// XOAUTH2 mechanism.
    Xoauth2,
    /// OAUTHBEARER mechanism.
    Oauthbearer,
}
/// Additional trusted certificate files.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Tls {
    /// Absolute paths to CA files.
    pub ca_files: Vec<String>,
}
/// Explicit server mailbox mappings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
#[non_exhaustive]
pub struct Folders {
    /// Inbox override.
    pub inbox: Option<String>,
    /// Sent override.
    pub sent: Option<String>,
    /// Drafts override.
    pub drafts: Option<String>,
    /// Trash override.
    pub trash: Option<String>,
    /// Archive override.
    pub archive: Option<String>,
}

/// A diagnostic containing only a field name and a static message.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ValidationError {
    /// Field path, without user data.
    pub field: &'static str,
    /// Redacted explanation.
    pub message: &'static str,
}

impl Config {
    /// Validate all configuration values without accessing the network or filesystem.
    #[must_use = "Reject invalid configuration before applying it."]
    #[inline]
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let mut check = |ok: bool, field: &'static str, message: &'static str| {
            if !ok {
                errors.push(ValidationError { field, message });
            }
        };
        check(
            self.schema_version == 1,
            "schemaVersion",
            "only version 1 is supported",
        );
        check(
            (1..=3650).contains(&self.sync.initial_days),
            "sync.initialDays",
            "outside supported range",
        );
        check(
            (30..=86400).contains(&self.sync.poll_seconds),
            "sync.pollSeconds",
            "outside supported range",
        );
        check(
            (1..=0x100_0000).contains(&self.sync.max_body_bytes),
            "sync.maxBodyBytes",
            "outside supported range",
        );
        check(
            (1..=8760).contains(&self.priority.max_ttl_hours),
            "priority.maxTtlHours",
            "outside supported range",
        );
        check(
            self.priority.default_ttl_hours > 0
                && self.priority.default_ttl_hours <= self.priority.max_ttl_hours,
            "priority.defaultTtlHours",
            "outside supported range",
        );
        check(
            self.priority.model_directory.is_some() == self.priority.model_revision.is_some(),
            "priority",
            "model directory and revision must be supplied together",
        );
        if let Some(path) = self.priority.model_directory.as_ref() {
            check(
                valid_path(path),
                "priority.modelDirectory",
                "absolute path required",
            );
        }
        if let Some(revision) = self.priority.model_revision.as_ref() {
            check(
                valid_text(revision),
                "priority.modelRevision",
                "revision required",
            );
        }
        check(
            self.priority
                .languages
                .iter()
                .all(|value| valid_text(value)),
            "priority.languages",
            "invalid language",
        );
        if let Some(zone) = self.time_zone.as_ref() {
            check(
                zone.parse::<chrono_tz::Tz>().is_ok(),
                "timeZone",
                "unknown time zone",
            );
        }
        if let Some(locale) = self.locale.as_ref() {
            check(valid_text(locale), "locale", "invalid locale");
        }
        check(
            valid_time(&self.reminders.morning),
            "reminders.morning",
            "expected HH:MM",
        );
        check(
            valid_time(&self.reminders.evening),
            "reminders.evening",
            "expected HH:MM",
        );
        validate_accounts(&self.accounts, &mut check);
        errors
    }
}

/// Validate account-level settings and authentication metadata.
fn validate_accounts(
    accounts: &[Account],
    check: &mut impl FnMut(bool, &'static str, &'static str),
) {
    let mut ids = HashSet::new();
    for account in accounts {
        check(
            valid_id(&account.id) && ids.insert(&account.id),
            "accounts.id",
            "invalid or duplicate account ID",
        );
        check(
            valid_text(&account.address),
            "accounts.address",
            "address required",
        );
        check(
            valid_text(&account.display_name),
            "accounts.displayName",
            "display name required",
        );
        check(
            valid_text(&account.user_name),
            "accounts.userName",
            "username required",
        );
        check(
            account
                .smtp_user_name
                .as_ref()
                .is_none_or(|value| valid_text(value)),
            "accounts.smtpUserName",
            "invalid username",
        );
        check(
            account.aliases.iter().all(|value| valid_text(value)),
            "accounts.aliases",
            "invalid alias",
        );
        if matches!(account.provider, Provider::Manual) {
            check(
                account.imap.is_some() && account.smtp.is_some(),
                "accounts",
                "manual accounts require endpoints",
            );
        }
        for endpoint in [&account.imap, &account.smtp].into_iter().flatten() {
            check(
                valid_host(&endpoint.host) && endpoint.port != 0,
                "accounts.endpoint",
                "invalid host or port",
            );
        }
        validate_auth(account, check);
        check(
            !matches!(account.provider, Provider::Google)
                || matches!(
                    account.auth,
                    Auth::Oauth2 {
                        provider: OAuthProvider::Google,
                        client_id: _,
                        secret_ref: _,
                        authorization_url: _,
                        token_url: _,
                        scopes: _,
                        sasl: _
                    }
                ),
            "accounts.auth",
            "Google requires Google OAuth",
        );
        check(
            !matches!(account.provider, Provider::Icloud)
                || matches!(account.auth, Auth::Password { secret_ref: _ }),
            "accounts.auth",
            "iCloud requires app password authentication",
        );
        check(
            account.tls.ca_files.iter().all(|value| valid_path(value)),
            "accounts.tls.caFiles",
            "absolute paths required",
        );
        for folder in [
            &account.folders.inbox,
            &account.folders.sent,
            &account.folders.drafts,
            &account.folders.trash,
            &account.folders.archive,
        ]
        .into_iter()
        .flatten()
        {
            check(
                valid_text(folder),
                "accounts.folders",
                "invalid mailbox name",
            );
        }
    }
}

/// Validate both incoming and optional outgoing authentication.
fn validate_auth(account: &Account, check: &mut impl FnMut(bool, &'static str, &'static str)) {
    for auth in once(&account.auth).chain(account.smtp_auth.iter()) {
        match auth {
            Auth::Password { secret_ref } => check(
                valid_id(secret_ref),
                "accounts.auth.secretRef",
                "invalid credential reference",
            ),
            Auth::Oauth2 {
                provider,
                client_id,
                secret_ref,
                authorization_url,
                token_url,
                scopes,
                sasl,
            } => {
                check(
                    valid_text(client_id),
                    "accounts.auth.clientId",
                    "invalid client ID",
                );
                check(
                    valid_id(secret_ref),
                    "accounts.auth.secretRef",
                    "invalid credential reference",
                );
                check(
                    scopes
                        .iter()
                        .all(|scope| valid_text(scope) && !scope.chars().any(char::is_whitespace)),
                    "accounts.auth.scopes",
                    "invalid scopes",
                );
                if matches!(provider, OAuthProvider::Custom) {
                    check(
                        authorization_url.as_deref().is_some_and(valid_https),
                        "accounts.auth.authorizationUrl",
                        "invalid HTTPS endpoint",
                    );
                    check(
                        token_url.as_deref().is_some_and(valid_https),
                        "accounts.auth.tokenUrl",
                        "invalid HTTPS endpoint",
                    );
                    check(
                        !scopes.is_empty(),
                        "accounts.auth.scopes",
                        "scopes required",
                    );
                    check(
                        sasl.is_some(),
                        "accounts.auth.sasl",
                        "SASL mechanism required",
                    );
                } else {
                    check(
                        authorization_url.is_none() && token_url.is_none(),
                        "accounts.auth",
                        "preset endpoints cannot be overridden",
                    );
                }
            }
        }
    }
}

/// Check text for empty or control characters.
fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}
/// Check a stable identifier.
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
/// Check an absolute path.
fn valid_path(value: &str) -> bool {
    Path::new(value).is_absolute() && valid_text(value)
}
/// Check a local time.
#[expect(
    clippy::missing_const_for_fn,
    reason = "Digits are checked before bounded arithmetic on the byte pattern."
)]
fn valid_time(value: &str) -> bool {
    match value.as_bytes() {
        [hour_tens, hour_ones, b':', minute_tens, minute_ones] => {
            hour_tens.is_ascii_digit()
                && hour_ones.is_ascii_digit()
                && minute_tens.is_ascii_digit()
                && minute_ones.is_ascii_digit()
                && (*hour_tens - b'0') * 10 + (*hour_ones - b'0') < 24
                && (*minute_tens - b'0') * 10 + (*minute_ones - b'0') < 60
        }
        _ => false,
    }
}
/// Check an HTTPS endpoint.
fn valid_https(value: &str) -> bool {
    !value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control() || character == '\\')
        && url::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        })
}
/// Check a server host.
fn valid_host(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_whitespace) && url::Host::parse(value).is_ok()
}

/// Maximum configuration source size in bytes.
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;

/// Evaluate pure Nix and perform preliminary validation.
///
/// This experimental function is not ready for production configuration loading.
///
/// This function must run inside a separately limited helper process. It does not
/// impose a CPU timeout or address-space limit itself.
///
/// # Errors
/// Returns a redacted error when the source is too large, cannot be evaluated,
/// or does not pass validation. Evaluator diagnostics may contain user input and
/// are deliberately not returned.
#[cfg(test)]
fn evaluate_unisolated(source: &str) -> Result<Config, &'static str> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("configuration exceeds the source size limit");
    }
    let config: Config =
        snix_serde::from_str(source).map_err(|_| "invalid configuration expression or schema")?;
    if !config.validate().is_empty() {
        return Err("configuration validation failed");
    }
    Ok(config)
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "Synthetic tests use direct assertions and fixture extraction."
)]
mod tests {
    use super::*;

    #[test]
    fn pure_expression_and_defaults() {
        let result = evaluate_unisolated(
            "let n = 1; in { schemaVersion = if n == 1 then n else 2; accounts = []; }",
        );
        let Ok(config) = result else {
            panic!("valid expression rejected")
        };
        assert_eq!(config.schema_version, 1);
        assert!(!config.priority.enabled);
        assert!(config.notifications.enabled);
        assert_eq!(config.sync.poll_seconds, 300);
    }

    #[test]
    fn rejects_unknown_and_impure_inputs_without_echoing_them() {
        for source in [
            "{ schemaVersion = 1; password = \"synthetic-secret\"; }",
            "{ schemaVersion = builtins.readFile /etc/passwd; }",
        ] {
            let error = evaluate_unisolated(source).err();
            assert_eq!(error, Some("invalid configuration expression or schema"));
        }
    }

    #[test]
    fn native_accounts_and_auth_fields() {
        let example = include_str!("../../../examples/config.nix");
        assert!(evaluate_unisolated(example).is_ok());
        let documented = include_str!("../../../docs/CONFIGURATION.md");
        let example = documented
            .split("```nix\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        assert!(evaluate_unisolated(example).is_ok());
        let custom = r#"{ kind = "oauth2"; provider = "custom"; clientId = "client";
            secretRef = "oauth"; authorizationUrl = "https://identity.example.edu/authorize";
            tokenUrl = "https://identity.example.edu/token"; scopes = [ "imap" ]; sasl = "xoauth2"; }"#;
        assert!(snix_serde::from_str::<Auth>(custom).is_ok());
        for source in [
            r#"{ kind = "password"; secretRef = "test"; clientId = "hybrid"; }"#,
            r#"{ kind = "oauth2"; secretRef = "test"; provider = "google"; }"#,
            r#"{ kind = "password"; secretRef = "test"; unknown = true; }"#,
        ] {
            assert!(snix_serde::from_str::<Auth>(source).is_err());
        }
    }

    #[test]
    fn url_and_time_validation_rejects_malformed_values() {
        assert!(valid_https("https://identity.example.test/token"));
        for url in [
            "http://example.test",
            "https://user:synthetic-secret@example.test",
            "https://example.test/#fragment",
            "https://",
            "https://example.test\\path",
        ] {
            assert!(!valid_https(url));
        }
        assert!(valid_time("09:00"));
        assert!(!valid_time("24:60"));
    }

    #[test]
    fn incoming_provider_presets_preserve_explicit_institutional_endpoints() {
        for (provider, host) in [
            ("google", Some("imap.gmail.com")),
            ("icloud", Some("imap.mail.me.com")),
            ("manual", None),
        ] {
            let mut account: Account = serde_json::from_value(serde_json::json!({
                "id":"synthetic", "provider":provider,"address":"reader@example.test",
                "displayName":"Synthetic", "userName":"synthetic",
                "auth":{"kind":"password","secretRef":"fixture"}
            }))
            .expect("account fixture");
            assert_eq!(
                account
                    .incoming_endpoint()
                    .as_ref()
                    .map(|endpoint| endpoint.host.as_str()),
                host
            );
            account.imap = Some(Endpoint {
                host: "imap.example.test".into(),
                port: 143,
                security: Security::Starttls,
            });
            let endpoint = account.incoming_endpoint().expect("explicit endpoint");
            assert_eq!(endpoint.host, "imap.example.test");
            assert_eq!(endpoint.port, 143);
            assert!(matches!(endpoint.security, Security::Starttls));
        }
    }

    #[test]
    fn rejects_unsupported_version_and_oversize_source() {
        assert!(evaluate_unisolated("{ schemaVersion = 2; }").is_err());
        assert!(evaluate_unisolated(&" ".repeat(MAX_SOURCE_BYTES + 1)).is_err());
    }
}
