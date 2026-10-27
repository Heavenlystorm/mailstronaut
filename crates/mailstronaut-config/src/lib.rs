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
    /// Perform preliminary validation; this is not yet sufficient for applying a configuration.
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
            self.sync.initial_days > 0
                && self.sync.poll_seconds > 0
                && self.sync.max_body_bytes > 0,
            "sync",
            "limits must be positive",
        );
        check(
            self.priority.default_ttl_hours > 0
                && self.priority.default_ttl_hours <= self.priority.max_ttl_hours,
            "priority",
            "TTL must be positive and no greater than the maximum",
        );
        check(
            self.priority.model_directory.is_some() == self.priority.model_revision.is_some(),
            "priority",
            "model directory and revision must be supplied together",
        );
        for account in &self.accounts {
            check(
                !account.id.trim().is_empty() && ids.insert(&account.id),
                "accounts.id",
                "account IDs must be nonempty and unique",
            );
            check(
                !account.address.trim().is_empty() && !account.user_name.trim().is_empty(),
                "accounts",
                "address and username are required",
            );
            if matches!(account.provider, Provider::Manual) {
                check(
                    account.imap.is_some() && account.smtp.is_some(),
                    "accounts",
                    "manual accounts require IMAP and SMTP endpoints",
                );
            }
            for endpoint in [&account.imap, &account.smtp].into_iter().flatten() {
                check(
                    !endpoint.host.trim().is_empty() && endpoint.port != 0,
                    "accounts.endpoint",
                    "host and nonzero port are required",
                );
            }
            for auth in std::iter::once(&account.auth).chain(account.smtp_auth.iter()) {
                match auth {
                    Auth::Password { secret_ref } => check(
                        !secret_ref.trim().is_empty(),
                        "accounts.auth.secretRef",
                        "credential reference is required",
                    ),
                    Auth::Oauth2 {
                        provider,
                        client_id,
                        secret_ref,
                        authorization_url,
                        token_url,
                        scopes,
                        ..
                    } => {
                        check(
                            !client_id.trim().is_empty() && !secret_ref.trim().is_empty(),
                            "accounts.auth",
                            "client ID and credential reference are required",
                        );
                        if matches!(provider, OAuthProvider::Custom) {
                            check(
                                authorization_url
                                    .as_ref()
                                    .is_some_and(|s| s.starts_with("https://") && s.len() > 8)
                                    && token_url
                                        .as_ref()
                                        .is_some_and(|s| s.starts_with("https://") && s.len() > 8)
                                    && !scopes.is_empty(),
                                "accounts.auth",
                                "custom OAuth requires HTTPS endpoints and scopes",
                            );
                        }
                    }
                }
            }
            check(
                account.tls.ca_files.iter().all(|p| p.starts_with('/')),
                "accounts.tls.caFiles",
                "CA paths must be absolute",
            );
        }
        errors
    }
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
pub fn evaluate_in_helper(source: &str) -> Result<Config, &'static str> {
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
        let result = evaluate_in_helper(
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
            let error = evaluate_in_helper(source).err();
            assert_eq!(error, Some("invalid configuration expression or schema"));
        }
    }

    #[test]
    fn rejects_unsupported_version_and_oversize_source() {
        assert!(evaluate_in_helper("{ schemaVersion = 2; }").is_err());
        assert!(evaluate_in_helper(&" ".repeat(MAX_SOURCE_BYTES + 1)).is_err());
    }
}
