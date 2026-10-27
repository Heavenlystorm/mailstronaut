//! Typed, secret-free configuration contract for Mailstronaut.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Configuration format supported by this crate.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
pub struct Background {
    /// Keep running after the window closes.
    pub enabled: bool,
    /// Request portal autostart.
    pub autostart: bool,
}
/// Notification settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Notifications {
    /// Enable notifications.
    pub enabled: bool,
    /// Include private message previews.
    pub show_preview: bool,
}
impl Default for Notifications {
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
pub struct Privacy {
    /// Policy for remote content.
    pub remote_content: RemoteContent,
}
/// Remote resources are never fetched on message open.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
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
pub struct Sync {
    /// Initial history in days.
    pub initial_days: u32,
    /// Polling interval in seconds.
    pub poll_seconds: u32,
    /// Maximum body bytes for analysis.
    pub max_body_bytes: u32,
}
impl Default for Sync {
    fn default() -> Self {
        Self {
            initial_days: 30,
            poll_seconds: 300,
            max_body_bytes: 2_097_152,
        }
    }
}
/// Local priority analysis settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
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
pub struct Reminders {
    /// Morning time in HH:MM format.
    pub morning: String,
    /// Evening time in HH:MM format.
    pub evening: String,
}
impl Default for Reminders {
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
/// Supported endpoint presets.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
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
pub enum Security {
    /// Implicit TLS.
    Tls,
    /// Required STARTTLS.
    Starttls,
}
/// Authentication metadata. Secret values are held in a credential service.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
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
/// OAuth provider.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
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
pub enum Sasl {
    /// XOAUTH2 mechanism.
    Xoauth2,
    /// OAUTHBEARER mechanism.
    Oauthbearer,
}
/// Additional trusted certificate files.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Tls {
    /// Absolute paths to CA files.
    pub ca_files: Vec<String>,
}
/// Explicit server mailbox mappings.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
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
pub struct ValidationError {
    /// Field path, without user data.
    pub field: &'static str,
    /// Redacted explanation.
    pub message: &'static str,
}

impl Config {
    /// Validate all configuration values without accessing the network or filesystem.
    #[must_use]
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let mut ids = HashSet::new();
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
            (1..=16_777_216).contains(&self.sync.max_body_bytes),
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
        if let Some(path) = &self.priority.model_directory {
            check(
                valid_path(path),
                "priority.modelDirectory",
                "absolute path required",
            );
        }
        if let Some(revision) = &self.priority.model_revision {
            check(
                valid_text(revision),
                "priority.modelRevision",
                "revision required",
            );
        }
        check(
            self.priority.languages.iter().all(|s| valid_text(s)),
            "priority.languages",
            "invalid language",
        );
        if let Some(zone) = &self.time_zone {
            check(
                zone.parse::<chrono_tz::Tz>().is_ok(),
                "timeZone",
                "unknown time zone",
            );
        }
        if let Some(locale) = &self.locale {
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
        for account in &self.accounts {
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
                    .is_none_or(|s| valid_text(s)),
                "accounts.smtpUserName",
                "invalid username",
            );
            check(
                account.aliases.iter().all(|s| valid_text(s)),
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
            for auth in std::iter::once(&account.auth).chain(account.smtp_auth.iter()) {
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
                            scopes.iter().all(|scope| {
                                valid_text(scope) && !scope.chars().any(char::is_whitespace)
                            }),
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
            check(
                !matches!(account.provider, Provider::Google)
                    || matches!(
                        account.auth,
                        Auth::Oauth2 {
                            provider: OAuthProvider::Google,
                            ..
                        }
                    ),
                "accounts.auth",
                "Google requires Google OAuth",
            );
            check(
                !matches!(account.provider, Provider::Icloud)
                    || matches!(account.auth, Auth::Password { .. }),
                "accounts.auth",
                "iCloud requires app password authentication",
            );
            check(
                account.tls.ca_files.iter().all(|p| valid_path(p)),
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
        errors
    }
}

fn valid_text(s: &str) -> bool {
    !s.trim().is_empty() && !s.chars().any(char::is_control)
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn valid_path(s: &str) -> bool {
    std::path::Path::new(s).is_absolute() && valid_text(s)
}
fn valid_time(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
        && &s[..2] < "24"
        && &s[3..] < "60"
}
fn valid_https(s: &str) -> bool {
    !s.chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        && url::Url::parse(s).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        })
}
fn valid_host(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(char::is_whitespace) && url::Host::parse(s).is_ok()
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
mod tests {
    use super::*;

    #[test]
    fn pure_expression_and_defaults() {
        let result = evaluate_unisolated(
            r#"let n = 1; in { schemaVersion = if n == 1 then n else 2; accounts = []; }"#,
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
    fn tagged_auth_spike_exposes_bridge_failure() {
        let auth: Result<Auth, _> =
            snix_serde::from_str(r#"{ kind = "password"; secretRef = "test"; }"#);
        assert!(
            auth.is_err(),
            "the upstream bridge behavior changed; revisit this regression"
        );
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
    fn rejects_unsupported_version_and_oversize_source() {
        assert!(evaluate_unisolated("{ schemaVersion = 2; }").is_err());
        assert!(evaluate_unisolated(&" ".repeat(MAX_SOURCE_BYTES + 1)).is_err());
    }
}
