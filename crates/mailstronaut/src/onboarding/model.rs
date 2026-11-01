//! Convert native form values into the existing typed account schema.

use mailstronaut_config::Account;
use serde_json::{Value, json};

#[derive(Default)]
pub struct Draft {
    pub provider: &'static str,
    pub auth: &'static str,
    pub id: String,
    pub incoming_reference: String,
    pub outgoing_reference: String,
    pub name: String,
    pub address: String,
    pub login: String,
    pub smtp_login: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: &'static str,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: &'static str,
    pub client_id: String,
    pub authorization_url: String,
    pub token_url: String,
    pub scopes: String,
    pub sasl: &'static str,
    pub separate_password: bool,
    pub aliases: String,
    pub ca_files: Vec<String>,
    pub sent: String,
    pub archive: String,
    pub trash: String,
}

impl Draft {
    pub fn account(&self) -> Result<Account, &'static str> {
        let id = self.id.trim();
        let name = self.name.trim();
        let address = self.address.trim();
        mailstronaut_mail::compose::sender(address, name)
            .map_err(|_| "Email address: enter a valid sender address and name.")?;
        let aliases: Vec<_> = self
            .aliases
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for address in &aliases {
            mailstronaut_mail::compose::sender(address, name)
                .map_err(|_| "Aliases: enter one valid email address per line.")?;
        }
        let login = if self.login.trim().is_empty() {
            address
        } else {
            self.login.trim()
        };
        let auth = match self.provider {
            "google" => self.oauth("google", &self.incoming_reference),
            "icloud" => json!({"kind": "password", "secretRef": self.incoming_reference}),
            "manual" if self.auth == "password" => {
                json!({"kind": "password", "secretRef": self.incoming_reference})
            }
            "manual" if matches!(self.auth, "microsoft" | "custom") => {
                self.oauth(self.auth, &self.incoming_reference)
            }
            _ => return Err("Choose a supported provider and authentication method."),
        };
        let mut account = json!({
            "id": id, "provider": self.provider, "displayName": name,
            "address": address, "userName": login, "aliases": aliases,
            "smtpUserName": nonempty(&self.smtp_login), "auth": auth,
            "tls": {"caFiles": self.ca_files},
            "folders": {"sent": nonempty(&self.sent), "archive": nonempty(&self.archive), "trash": nonempty(&self.trash)}
        });
        if self.provider == "manual" {
            account["imap"] = endpoint(&self.imap_host, &self.imap_port, self.imap_security)
                .ok_or("Incoming server: enter a host and whole port between 1 and 65535.")?;
            account["smtp"] = endpoint(&self.smtp_host, &self.smtp_port, self.smtp_security)
                .ok_or("Outgoing server: enter a host and whole port between 1 and 65535.")?;
            if self.separate_password && self.auth == "password" {
                account["smtpAuth"] =
                    json!({"kind": "password", "secretRef": self.outgoing_reference});
            }
        }
        serde_json::from_value(account).map_err(|_| "An account field has an unsupported value.")
    }

    fn oauth(&self, provider: &str, reference: &str) -> Value {
        let mut auth = json!({"kind": "oauth2", "provider": provider,
            "clientId": self.client_id.trim(), "secretRef": reference});
        if provider == "custom" {
            auth["authorizationUrl"] = json!(self.authorization_url.trim());
            auth["tokenUrl"] = json!(self.token_url.trim());
            auth["scopes"] = json!(self.scopes.split_whitespace().collect::<Vec<_>>());
            auth["sasl"] = json!(self.sasl);
        }
        auth
    }
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn endpoint(host: &str, port: &str, security: &str) -> Option<Value> {
    let port = port.trim().parse::<u16>().ok().filter(|port| *port != 0)?;
    Some(json!({"host": host.trim(), "port": port, "security": security}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailstronaut_config::{Auth, OAuthProvider, Provider, Security};

    fn example() -> Draft {
        Draft {
            provider: "manual",
            auth: "password",
            id: "university".into(),
            incoming_reference: "synthetic-incoming".into(),
            outgoing_reference: "synthetic-outgoing".into(),
            name: "Éduard Example".into(),
            address: "reader@example.test".into(),
            imap_host: "imap.example.test".into(),
            imap_port: "993".into(),
            imap_security: "tls",
            smtp_host: "smtp.example.test".into(),
            smtp_port: "587".into(),
            smtp_security: "starttls",
            ..Draft::default()
        }
    }

    #[test]
    fn provider_presets_use_the_existing_secure_endpoints_and_authentication() {
        let mut form = example();
        form.provider = "google";
        form.client_id = "registered-public-client".into();
        let google = form.account().expect("Google declaration");
        assert!(matches!(
            google.auth,
            Auth::Oauth2 {
                provider: OAuthProvider::Google,
                ..
            }
        ));
        assert_eq!(
            google.incoming_endpoint().expect("IMAP").host,
            "imap.gmail.com"
        );
        assert!(matches!(
            google.outgoing_endpoint().expect("SMTP").security,
            Security::Starttls
        ));
        form.provider = "icloud";
        form.login = "reader".into();
        let cloud = form.account().expect("iCloud declaration");
        assert!(matches!(cloud.provider, Provider::Icloud));
        assert!(matches!(cloud.auth, Auth::Password { .. }));
        assert_eq!(cloud.user_name, "reader");
        assert_eq!(cloud.outgoing_user_name(), "reader@example.test");
        assert_eq!(cloud.incoming_endpoint().expect("IMAP").port, 993);
    }

    #[test]
    fn manual_connections_keep_distinct_logins_references_and_native_advanced_data() {
        let mut form = example();
        form.login = "incoming-login".into();
        form.smtp_login = "outgoing-login".into();
        form.separate_password = true;
        form.aliases = "alias@example.test\nother@example.test".into();
        form.ca_files = vec!["/certificates/institution root.pem".into()];
        form.sent = "Sent items".into();
        let account = form.account().expect("manual account");
        assert_eq!(account.user_name, "incoming-login");
        assert_eq!(account.outgoing_user_name(), "outgoing-login");
        assert!(
            matches!(account.auth, Auth::Password { secret_ref } if secret_ref == "synthetic-incoming")
        );
        assert!(
            matches!(account.smtp_auth, Some(Auth::Password { secret_ref }) if secret_ref == "synthetic-outgoing")
        );
        assert_eq!(account.aliases.len(), 2);
        assert_eq!(account.tls.ca_files, form.ca_files);
        assert_eq!(account.folders.sent.as_deref(), Some("Sent items"));
    }

    #[test]
    fn malformed_sender_and_ports_are_refused_before_a_source_write() {
        let mut form = example();
        form.address = "not an email".into();
        assert!(form.account().is_err());
        form.address = "reader@example.test".into();
        for port in ["0", "65536", "-1", "993.5", ""] {
            form.imap_port = port.into();
            assert!(form.account().is_err());
        }
    }
}
