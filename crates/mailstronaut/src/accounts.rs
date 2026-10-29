//! Native configuration and explicit foreground credential setup.

use std::{env, path::PathBuf};

use cosmic::{Action, Element, app::Task, iced::widget::column, widget};
use mailstronaut_config::{Appearance, Auth, Config, source};
use mailstronaut_mail::{
    auth::Session,
    credentials::{self, Key, Purpose, Store},
    oauth,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::sync::CancellationToken;

/// Account page events. `SecretString` redacts typed input in derived Debug output.
#[derive(Clone, Debug)]
pub enum Message {
    Reload,
    Loaded(u64, Box<Result<Option<Config>, source::Error>>),
    Select(String, Slot),
    Password(SecretString),
    Save,
    Unlock,
    Authorize,
    CancelAuthorization,
    Authorized(u64, Result<(), String>),
    Connected(u64, Result<Store, credentials::Error>),
    Stored(u64, Result<(), credentials::Error>),
}

/// Incoming and optional independently configured outgoing credentials.
#[derive(Clone, Copy, Debug)]
pub enum Slot {
    Incoming,
    Outgoing,
}

/// Whole-declaration snapshots and transient, zeroizing setup input.
#[derive(Default)]
pub struct Accounts {
    config: source::Runtime,
    path: Option<PathBuf>,
    loading: bool,
    generation: u64,
    cancellation: CancellationToken,
    selected: Option<(String, Slot)>,
    password: SecretString,
    keyring: Option<Store>,
    authentication: Option<Session>,
    authorization: CancellationToken,
    authenticating: bool,
    busy: bool,
    status: Option<String>,
}
impl Accounts {
    pub const fn loading(&self) -> bool {
        self.loading
    }
    pub fn status(&mut self, status: &str) {
        self.status = Some(status.into());
    }
    /// Validated configuration snapshot for native mailbox queries/workers.
    pub fn active(&self) -> Option<std::sync::Arc<Config>> {
        self.config.active().cloned()
    }
    pub const fn revision(&self) -> u64 {
        self.config.revision()
    }
    /// Existing foreground-unlocked backend; this never opens a prompt itself.
    pub fn keyring(&self) -> Option<Store> {
        self.keyring.clone()
    }
    pub fn authentication(&self) -> Option<Session> {
        self.authentication.clone()
    }
    pub const fn authenticating(&self) -> bool {
        self.authenticating
    }
    pub fn cancel_authorization(&self) {
        self.authorization.cancel();
    }
    pub fn new(path: Option<PathBuf>) -> (Self, Task<super::Message>) {
        let mut state = Self {
            path,
            ..Self::default()
        };
        let task = state.reload();
        (state, task)
    }

    fn reload(&mut self) -> Task<super::Message> {
        if self.busy {
            return Task::none();
        }
        self.cancellation.cancel();
        self.cancellation = CancellationToken::new();
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let path = self.path.clone();
        let cancellation = self.cancellation.clone();
        self.loading = true;
        self.password = SecretString::default();
        self.status = None;
        Task::perform(
            async move {
                let path = path.ok_or(source::Error::Read)?;
                let executable = env::current_exe().map_err(|_| source::Error::Read)?;
                source::load(path, &executable, &cancellation).await
            },
            move |result| {
                Action::App(super::Message::Accounts(Message::Loaded(
                    generation,
                    Box::new(result),
                )))
            },
        )
    }

    pub fn update(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::Reload => return self.reload(),
            Message::Loaded(generation, result) if generation == self.generation => {
                self.loading = false;
                if self.config.apply(*result) {
                    self.selected = None;
                    self.password = SecretString::default();
                    let theme = match self.config.active().map(|config| config.appearance) {
                        Some(Appearance::Dark) => cosmic::Theme::dark(),
                        Some(Appearance::Light) => cosmic::Theme::light(),
                        _ => cosmic::theme::system_preference(),
                    };
                    let dark = theme.cosmic().is_dark;
                    return cosmic::command::set_theme(theme)
                        .chain(Task::done(Action::App(super::Message::Appearance(dark))));
                }
            }
            Message::Select(id, slot) if !self.busy && !self.loading => {
                self.selected = Some((id, slot));
                self.password = SecretString::default();
                self.status = None;
            }
            Message::Password(value) if !self.busy && !self.loading => {
                if value.expose_secret().len() <= credentials::MAX_SECRET_BYTES {
                    self.password = value;
                }
            }
            Message::Unlock if !self.busy && !self.loading => return self.unlock(),
            Message::Connected(generation, result) if generation == self.generation => {
                self.busy = false;
                match result {
                    Ok(store) => {
                        if self.authentication.is_none() {
                            self.authentication = Some(Session::new(store.clone()));
                        }
                        self.keyring = Some(store);
                        self.status = Some("Encrypted keyring ready.".into());
                    }
                    Err(error) => {
                        self.keyring = None;
                        self.authentication = None;
                        self.status = Some(error.to_string());
                    }
                }
            }
            Message::Authorize if !self.busy && !self.loading => return self.authorize(),
            Message::CancelAuthorization => self.authorization.cancel(),
            Message::Authorized(generation, result) if generation == self.generation => {
                self.busy = false;
                self.authenticating = false;
                self.status = Some(match result {
                    Ok(()) => {
                        "Authorization saved in encrypted keyring. You can sync or send.".into()
                    }
                    Err(error) => error,
                });
            }
            Message::Save if !self.busy && !self.loading => return self.save(),
            Message::Stored(generation, result) if generation == self.generation => {
                self.busy = false;
                self.status = Some(match result {
                    Ok(()) => "Credential saved in encrypted keyring.".into(),
                    Err(error) => error.to_string(),
                });
            }
            _ => {}
        }
        Task::none()
    }

    /// Clear transient password text whenever the account page is left.
    pub fn clear_input(&mut self) {
        self.password = SecretString::default();
    }

    /// Explicit configuration wins over system-mode callbacks.
    pub fn effective_dark(&self, system: bool) -> bool {
        match self.config.active().map(|config| config.appearance) {
            Some(Appearance::Dark) => true,
            Some(Appearance::Light) => false,
            _ => system,
        }
    }

    fn unlock(&mut self) -> Task<super::Message> {
        self.busy = true;
        self.status = Some("Waiting for encrypted keyring access…".into());
        let generation = self.generation;
        let previous = self.keyring.clone();
        Task::perform(
            async move {
                let store = match previous {
                    Some(store) => store,
                    None => Store::connect_foreground().await?,
                };
                store.unlock_foreground().await?;
                Ok(store)
            },
            move |result| {
                Action::App(super::Message::Accounts(Message::Connected(
                    generation, result,
                )))
            },
        )
    }

    fn save(&mut self) -> Task<super::Message> {
        let (Some(keyring), Some(key)) = (self.keyring.clone(), self.selected_key()) else {
            return Task::none();
        };
        if self.password.expose_secret().is_empty() {
            return Task::none();
        }
        self.busy = true;
        let password = core::mem::take(&mut self.password);
        self.status = Some("Saving credential in encrypted keyring…".into());
        let generation = self.generation;
        Task::perform(
            async move { keyring.put(&key, &password).await },
            move |result| {
                Action::App(super::Message::Accounts(Message::Stored(
                    generation, result,
                )))
            },
        )
    }

    fn authorize(&mut self) -> Task<super::Message> {
        let (Some(session), Some((id, slot)), Some(config)) = (
            self.authentication.clone(),
            self.selected.clone(),
            self.active(),
        ) else {
            self.status = Some("Unlock the encrypted keyring before connecting.".into());
            return Task::none();
        };
        let Some(account) = config
            .accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
        else {
            return Task::none();
        };
        self.busy = true;
        self.authenticating = true;
        self.authorization = CancellationToken::new();
        let (cancel, generation) = (self.authorization.clone(), self.generation);
        self.status = Some("Complete authorization in your external browser.".into());
        Task::perform(
            authorize(session, account, matches!(slot, Slot::Outgoing), cancel),
            move |result| {
                Action::App(super::Message::Accounts(Message::Authorized(
                    generation, result,
                )))
            },
        )
    }

    fn selected_key(&self) -> Option<Key> {
        let (id, slot) = self.selected.as_ref()?;
        let account = self
            .config
            .active()?
            .accounts
            .iter()
            .find(|account| account.id == *id)?;
        let auth = match slot {
            Slot::Incoming => &account.auth,
            Slot::Outgoing => account.smtp_auth.as_ref().unwrap_or(&account.auth),
        };
        match auth {
            Auth::Password { secret_ref } => {
                Key::new(&account.id, secret_ref, Purpose::Password).ok()
            }
            _ => None,
        }
    }

    pub fn view(&self) -> Element<'_, super::Message> {
        let mut content = column![
            widget::text::title1("Accounts"),
            widget::text(
                "Configuration is native Nix. Home Manager can install the same file directly."
            )
        ]
        .spacing(12)
        .padding(20);
        if let Some(path) = &self.path {
            content = content.push(widget::text(format!("Source: {}", path.display())));
        }
        if self.loading {
            content = content.push(widget::text("Loading configuration…"));
        }
        if let Some(error) = self.config.error() {
            let state = if self.config.active().is_some() {
                "The last valid configuration remains active."
            } else {
                "No accounts have been started."
            };
            content = content.push(widget::text(format!("{error}. {state}")));
        }
        if let Some(config) = self.config.active() {
            content = content.push(widget::text(format!(
                "Configuration revision {} · {} accounts · managed source",
                self.config.revision(),
                config.accounts.len()
            )));
            for account in &config.accounts {
                content = content.push(
                    widget::button::text(format!("{} — {}", account.display_name, account.address))
                        .on_press_maybe((!self.busy && !self.loading).then(|| {
                            super::Message::Accounts(Message::Select(
                                account.id.clone(),
                                Slot::Incoming,
                            ))
                        })),
                );
                if account.smtp_auth.is_some() {
                    content = content.push(
                        widget::button::text(format!(
                            "Outgoing credentials — {}",
                            account.display_name
                        ))
                        .on_press_maybe(
                            (!self.busy && !self.loading).then(|| {
                                super::Message::Accounts(Message::Select(
                                    account.id.clone(),
                                    Slot::Outgoing,
                                ))
                            }),
                        ),
                    );
                }
            }
            if config.accounts.is_empty() {
                content = content.push(widget::text(
                    "No accounts configured. Add accounts to the native Nix source and reload.",
                ));
            }
        } else if !self.loading && self.config.error().is_none() {
            content = content.push(widget::text("Welcome to Mailstronaut. Create your native configuration with: mailstronaut config init"));
            content = content.push(widget::text("Then add your accounts using examples/config.nix and reload. No mail connections start without valid configuration."));
        }
        content = content.push(
            widget::button::text("Reload configuration")
                .on_press_maybe((!self.busy).then_some(super::Message::Accounts(Message::Reload))),
        );
        content = content.push(self.credentials_view());
        widget::scrollable(content).into()
    }
    fn credentials_view(&self) -> Element<'_, super::Message> {
        let mut content = column![].spacing(12);
        if self.selected_key().is_some() {
            content = content.push(widget::text("Use an app password for iCloud. Credentials are stored separately from Nix configuration."));
            content = content.push(widget::button::text("Unlock keyring").on_press_maybe(
                (!self.busy && !self.loading).then_some(super::Message::Accounts(Message::Unlock)),
            ));
            content = content.push(
                widget::text_input("Password / app password", self.password.expose_secret())
                    .password()
                    .on_input(|value| {
                        super::Message::Accounts(Message::Password(SecretString::from(value)))
                    }),
            );
            content = content.push(
                widget::button::text("Save password").on_press_maybe(
                    (!self.busy
                        && !self.loading
                        && self.keyring.is_some()
                        && !self.password.expose_secret().is_empty())
                    .then_some(super::Message::Accounts(Message::Save)),
                ),
            );
        } else if self.selected.is_some() {
            content = content.push(widget::text(
                "Sign in using your browser. Credentials stay in your encrypted keyring.",
            ));
            content = content.push(widget::button::text("Unlock keyring").on_press_maybe(
                (!self.busy && !self.loading).then_some(super::Message::Accounts(Message::Unlock)),
            ));
            content = content.push(
                widget::button::text("Connect in browser").on_press_maybe(
                    (!self.busy && !self.loading && self.authentication.is_some())
                        .then_some(super::Message::Accounts(Message::Authorize)),
                ),
            );
            if self.authenticating {
                content = content.push(
                    widget::button::text("Cancel authorization")
                        .on_press(super::Message::Accounts(Message::CancelAuthorization)),
                );
            }
        }
        if let Some(status) = &self.status {
            content = content.push(widget::text(status));
        }
        content.into()
    }
}

async fn authorize(
    session: Session,
    account: mailstronaut_config::Account,
    outgoing: bool,
    cancel: CancellationToken,
) -> Result<(), String> {
    let auth = if outgoing {
        account.smtp_auth.as_ref().unwrap_or(&account.auth)
    } else {
        &account.auth
    };
    let provider = oauth::Provider::prepare(auth, &account.tls)
        .await
        .map_err(|error| error.to_string())?;
    let flow = provider
        .begin(&account.address)
        .await
        .map_err(|error| error.to_string())?;
    let opened = cancel
        .run_until_cancelled(tokio::time::timeout(
            std::time::Duration::from_secs(30),
            ashpd::desktop::open_uri::OpenFileRequest::default()
                .ask(false)
                .send_uri(flow.authorization_url()),
        ))
        .await
        .ok_or("Authorization cancelled.")?
        .map_err(|_| "External browser portal timed out.")?
        .map_err(|_| "External browser portal unavailable or access refused.")?;
    opened
        .response()
        .map_err(|_| "External browser request was declined.")?;
    let grant = flow
        .finish(&cancel)
        .await
        .map_err(|error| error.to_string())?;
    // Finish this bounded encrypted write even if cancellation arrives now.
    session
        .accept(&account, outgoing, grant)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn password_events_do_not_reveal_typed_values() {
        let event = crate::Message::Accounts(super::Message::Password(
            secrecy::SecretString::from("SYNTHETIC-UI-PASSWORD-MARKER"),
        ));
        assert!(!format!("{event:?}").contains("SYNTHETIC-UI-PASSWORD-MARKER"));
    }
}
