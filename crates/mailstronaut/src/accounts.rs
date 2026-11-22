//! Standard GPUI Kit account controls over immutable native configuration.

use std::sync::Arc;

use gpui::component::{
    Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    description_list::DescriptionList,
    input::{Input, InputState},
    setting::{SettingGroup, SettingItem, SettingPage},
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mailstronaut_config::{Account, Auth, Config};
use mailstronaut_core::{
    store::{flags, retention::Evicted},
    sync::{Page, Position},
    worker::Handle,
};
use tokio::{runtime::Runtime, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::account_actions::{self, Access};

pub enum Event {
    CacheChanged,
    CacheEvicted,
    CredentialsChanged,
}

enum Outcome {
    Saved,
    SignedIn,
    Synced(Page),
    Evicted(Evicted),
}

pub struct AccountPane {
    config: Arc<Config>,
    account: Account,
    store: Handle,
    runtime: Arc<Runtime>,
    access: Access,
    lifetime: CancellationToken,
    password: Entity<InputState>,
    smtp_password: Entity<InputState>,
    login: Entity<InputState>,
    smtp_login: Entity<InputState>,
    preferences: Entity<crate::preferences::Preferences>,
    cancel: CancellationToken,
    busy: bool,
    before: Option<Position>,
    status: String,
}

impl EventEmitter<Event> for AccountPane {}

impl Drop for AccountPane {
    fn drop(&mut self) {
        self.lifetime.cancel();
        self.cancel.cancel();
    }
}

impl AccountPane {
    #[expect(
        clippy::too_many_arguments,
        reason = "The account view receives shared mail services and a configuration cancellation token alongside GPUI's window/context pair."
    )]
    pub fn new(
        config: Arc<Config>,
        account: Account,
        store: Handle,
        runtime: Arc<Runtime>,
        access: Access,
        preferences: Entity<crate::preferences::Preferences>,
        lifetime: CancellationToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password or app password")
        });
        let smtp_password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Outgoing password or app password")
        });
        let login =
            cx.new(|cx| InputState::new(window, cx).default_value(account.user_name.clone()));
        let smtp_login = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(account.smtp_user_name.clone().unwrap_or_default())
                .placeholder("Use provider default")
        });
        let status = if config.sync.automatic {
            "Ready · automatic synchronization is enabled."
        } else {
            "Ready · sync starts only when requested."
        }
        .into();
        Self {
            config,
            account,
            store,
            runtime,
            access,
            cancel: lifetime.child_token(),
            lifetime,
            password,
            smtp_password,
            login,
            smtp_login,
            preferences,
            busy: false,
            before: None,
            status,
        }
    }

    fn begin(&mut self, status: &str, cx: &mut Context<Self>) -> Option<CancellationToken> {
        if self.busy {
            return None;
        }
        self.cancel = self.lifetime.child_token();
        self.busy = true;
        self.status = status.into();
        cx.notify();
        Some(self.cancel.clone())
    }

    fn save(&mut self, outgoing: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = if outgoing {
            self.smtp_password.clone()
        } else {
            self.password.clone()
        };
        let value = input.read(cx).value().to_string();
        if value.is_empty() || value.len() > mailstronaut_mail::credentials::MAX_SECRET_BYTES {
            self.status = "Enter a nonempty password within the supported size limit.".into();
            cx.notify();
            return;
        }
        let Some(cancel) = self.begin("Saving in encrypted keyring…", cx) else {
            return;
        };
        input.update(cx, |input, cx| input.set_value("", window, cx));
        let account = self.account.clone();
        let access = self.access.clone();
        let job = self.runtime.spawn(async move {
            access
                .save_password(&account, outgoing, value, &cancel)
                .await
                .map(|()| Outcome::Saved)
        });
        Self::watch(job, false, cx);
    }

    fn sign_in(&mut self, outgoing: bool, cx: &mut Context<Self>) {
        let Some(cancel) = self.begin("Waiting for browser sign-in…", cx) else {
            return;
        };
        let account = self.account.clone();
        let access = self.access.clone();
        let job = self.runtime.spawn(async move {
            access
                .sign_in(&account, outgoing, &cancel)
                .await
                .map(|()| Outcome::SignedIn)
        });
        Self::watch(job, false, cx);
    }

    fn sync(&mut self, older: bool, cx: &mut Context<Self>) {
        let Some(cancel) = self.begin("Syncing a bounded page of mail…", cx) else {
            return;
        };
        let (access, config, account, store, before) = (
            self.access.clone(),
            self.config.clone(),
            self.account.clone(),
            self.store.clone(),
            self.before,
        );
        let job = self.runtime.spawn(async move {
            access
                .sync(&store, &config, &account, before, older, &cancel)
                .await
                .map(Outcome::Synced)
        });
        Self::watch(job, true, cx);
    }

    fn confirm_eviction(&self, days: u32, window: &mut Window, cx: &mut Context<Self>) {
        let pane = cx.entity().downgrade();
        let account = self.account.display_name.clone();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let pane = pane.clone();
            dialog
                .confirm()
                .title("Remove old cached mail?")
                .description(format!(
                    "Remove downloaded mail older than {days} days for {account}. Server copies stay in place. The app keeps drafts, starred mail, reminders and pending changes."
                ))
                .ok_text("Remove local copies")
                .on_ok(move |_, _, cx| {
                    _ = pane.update(cx, Self::evict);
                    true
                })
        });
    }

    fn evict(&mut self, cx: &mut Context<Self>) {
        if self.config.sync.initial_days.is_none() || self.lifetime.is_cancelled() {
            return;
        }
        let Some(cancel) = self.begin("Removing old cached mail…", cx) else {
            return;
        };
        let cutoff = account_actions::history_floor(&self.config, chrono::Utc::now().timestamp());
        let account = self.account.clone();
        let store = self.store.clone();
        let job = self.runtime.spawn(async move {
            let context = flags::context(&account).map_err(|error| error.to_string())?;
            store
                .evict_cached_before(account.id, context, cutoff, cancel)
                .await
                .map(Outcome::Evicted)
                .map_err(|error| error.to_string())
        });
        Self::watch(job, false, cx);
    }

    fn watch(job: JoinHandle<Result<Outcome, String>>, cache_changed: bool, cx: &Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = job
                .await
                .unwrap_or_else(|_| Err("Account operation did not finish.".into()));
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                let credentials_changed = matches!(result, Ok(Outcome::Saved | Outcome::SignedIn));
                this.status = match result {
                    Ok(Outcome::Saved) => "Password saved in the encrypted keyring.".into(),
                    Ok(Outcome::SignedIn) => "Signed in. You can now sync mail.".into(),
                    Ok(Outcome::Synced(page)) => {
                        this.before = page.next_before;
                        format!(
                            "Downloaded {} · refreshed {} · skipped {}. Cache coverage is partial.",
                            page.downloaded, page.refreshed, page.skipped
                        )
                    }
                    Ok(Outcome::Evicted(evicted)) => {
                        if evicted.messages > 0 {
                            this.before = None;
                            cx.emit(Event::CacheEvicted);
                        }
                        let messages = evicted.messages;
                        if evicted.cleanup_pending {
                            format!("Removed {messages} cached messages. Run cleanup again to retry file removal.")
                        } else {
                            format!("Removed {messages} cached messages.")
                        }
                    }
                    Err(error) => {
                        if cache_changed {
                            this.before = None;
                        }
                        error
                    }
                };
                if cache_changed {
                    cx.emit(Event::CacheChanged);
                }
                if credentials_changed {
                    cx.emit(Event::CredentialsChanged);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn authentication(&self, outgoing: bool, cx: &Context<Self>) -> impl IntoElement {
        let password = matches!(
            account_actions::selected_auth(&self.account, outgoing),
            Auth::Password { .. }
        );
        div()
            .flex()
            .items_center()
            .gap_3()
            .when(password, |view| {
                view.child(
                    div().flex_1().child(
                        Input::new(if outgoing {
                            &self.smtp_password
                        } else {
                            &self.password
                        })
                        .mask_toggle()
                        .disabled(self.busy),
                    ),
                )
                .child(
                    Button::new(if outgoing { "save-smtp" } else { "save-imap" })
                        .label("Save password")
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.save(outgoing, window, cx)),
                        ),
                )
            })
            .when(!password, |view| {
                view.child(
                    Button::new(if outgoing { "oauth-smtp" } else { "oauth-imap" })
                        .label("Sign in with browser")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.sign_in(outgoing, cx))),
                )
            })
    }

    pub fn page(view: &Entity<Self>, cx: &App) -> SettingPage {
        let account = &view.read(cx).account;
        let title = account.id.clone();
        let address = account.address.clone();
        let login = account.user_name.clone();
        let smtp_login = account.outgoing_user_name().to_owned();
        let endpoint = account.incoming_endpoint().map_or_else(
            || "Not configured".to_owned(),
            |endpoint| format!("{}:{}", endpoint.host, endpoint.port),
        );
        let controls = view.clone();
        SettingPage::new(title)
            .description(address.clone())
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("Connection")
                    .description(
                        "Edit login names below when this configuration is writable. In managed Nix, set accounts[].userName and optionally accounts[].smtpUserName, activate Home Manager, then reload. INRIA may use a short username rather than an email address.",
                    )
                    .item(
                        SettingItem::render(move |_, _, _| {
                            DescriptionList::new()
                                .columns(1)
                                .bordered(false)
                                .item("Address", address.clone(), 1)
                                .item("Incoming login", login.clone(), 1)
                                .item("Outgoing login", smtp_login.clone(), 1)
                                .item("Incoming server", endpoint.clone(), 1)
                        })
                        .keywords([
                            "address",
                            "login",
                            "server",
                            &account.address,
                        ]),
                    ),
            )
            .group(
                SettingGroup::new()
                    .title("Sign-in and synchronization")
                    .description(
                        "Credentials stay in the encrypted keyring. Automatic sync follows your native configuration.",
                    )
                    .item(
                        SettingItem::render(move |_, _, _| controls.clone()).keywords([
                            "password",
                            "sign in",
                            "OAuth",
                            "sync",
                            "older",
                            "cancel",
                            "cache",
                            "retention",
                            &account.address,
                        ]),
                    ),
            )
    }
}

impl Render for AccountPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_sm().child("Incoming login"))
            .child(Input::new(&self.login).disabled(
                !self.preferences.read(cx).editable() || self.preferences.read(cx).busy(),
            ))
            .child(
                div()
                    .text_sm()
                    .child("Outgoing login (leave empty for provider default)"),
            )
            .child(Input::new(&self.smtp_login).disabled(
                !self.preferences.read(cx).editable() || self.preferences.read(cx).busy(),
            ))
            .child(
                Button::new("save-logins")
                    .label("Save login names")
                    .disabled(
                        !self.preferences.read(cx).editable() || self.preferences.read(cx).busy(),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        let incoming = this.login.read(cx).value().trim().to_owned();
                        let outgoing = this.smtp_login.read(cx).value().trim().to_owned();
                        let id = this.account.id.clone();
                        this.preferences.update(cx, |preferences, cx| {
                            preferences.save_logins(
                                id,
                                incoming,
                                (!outgoing.is_empty()).then_some(outgoing),
                                window,
                                cx,
                            );
                        });
                    })),
            )
            .child(self.authentication(false, cx))
            .when(self.account.smtp_auth.is_some(), |view| {
                view.child(div().text_sm().child("Separate outgoing authentication"))
                    .child(self.authentication(true, cx))
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("sync")
                            .primary()
                            .label("Sync newest mail")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.sync(false, cx))),
                    )
                    .child(
                        Button::new("older")
                            .label("Load older page")
                            .disabled(self.busy || self.before.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.sync(true, cx))),
                    )
                    .when(self.busy, |view| {
                        view.child(Button::new("cancel").ghost().label("Cancel").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.cancel.cancel();
                                cx.notify();
                            }),
                        ))
                    }),
            )
            .when_some(self.config.sync.initial_days, |view, days| {
                view.child(
                    Button::new("evict-older-cache")
                        .label(format!("Remove cached mail older than {days} days"))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_eviction(days, window, cx);
                        })),
                )
            })
            .child(div().text_sm().child(self.status.clone()))
    }
}
