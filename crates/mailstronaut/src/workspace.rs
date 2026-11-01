//! GPUI Kit mailbox workspace. Mail and storage remain toolkit-independent.

use std::{path::PathBuf, sync::Arc};

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    description_list::DescriptionList,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    list::{List, ListEvent, ListState},
    menu::{DropdownMenu as _, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    tab::{Tab, TabBar},
    toolbar::{Toolbar, ToolbarGroup},
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use mailstronaut_config::{Appearance, Config, source};
use mailstronaut_core::{
    store::browse::Scope,
    worker::{Handle, Worker},
};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::mailbox::{Row, Rows};

gpui::actions!(
    mailstronaut,
    [
        #[derive(Eq)]
        /// Focus the native mailbox search field.
        FocusSearch,
        #[derive(Eq)]
        /// Refresh the local mailbox cache without changing Seen state.
        RefreshCache,
        #[derive(Eq)]
        /// Toggle the standard GPUI Kit light/dark appearance.
        ToggleAppearance,
        #[derive(Eq)]
        /// Focus the native read-only message text without opening a message.
        FocusReader,
        #[derive(Eq)]
        /// Focus the current message's native web-link picker.
        FocusLinks,
        #[derive(Eq)]
        /// Focus the message list without opening its selection.
        FocusMessages,
        #[derive(Eq)]
        /// Display configured accounts without starting network work.
        ShowAccounts,
        #[derive(Eq)]
        /// Return to the inbox without opening any message.
        ShowInbox,
        #[derive(Eq)]
        /// Explicitly reload the canonical native Nix source.
        ReloadConfiguration
    ]
);

#[derive(Eq, PartialEq)]
enum View {
    Inbox,
    Accounts,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ReaderMode {
    Text,
    Html,
}

pub struct Workspace {
    focus: FocusHandle,
    link_focus: FocusHandle,
    runtime: Arc<Runtime>,
    owner: Option<Arc<Worker>>,
    accounts: Vec<String>,
    declaration: source::Runtime,
    config_path: Option<PathBuf>,
    loading_config: bool,
    view: View,
    account_access: crate::account_actions::Access,
    account_views: Vec<Entity<crate::accounts::AccountPane>>,
    account_events: Vec<Subscription>,
    account_lifetime: CancellationToken,
    lifetime: CancellationToken,
    query_cancel: CancellationToken,
    read_gate: crate::read_gate::Gate,
    list: Entity<ListState<Rows>>,
    _list_events: Subscription,
    selected: Option<Row>,
    body: String,
    text_reader: Entity<TextareaState>,
    reader_mode: ReaderMode,
    html_reader: Entity<crate::html_reader::HtmlReader>,
    _html_events: Subscription,
    opened_message: Option<Arc<mailstronaut_core::mime::Projection>>,
    preferred_reader: ReaderMode,
    reader_notice: String,
    link_cancel: Option<CancellationToken>,
    search: Entity<InputState>,
    status: String,
    query: String,
    query_generation: u64,
    reading_generation: u64,
    demo: bool,
    _search_events: Subscription,
    _activation: Subscription,
    _appearance: Subscription,
    theme_override: Option<ThemeMode>,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.lifetime.cancel();
        self.account_lifetime.cancel();
        self.query_cancel.cancel();
    }
}

impl Workspace {
    pub fn new(
        path: Option<PathBuf>,
        demo: bool,
        runtime: Arc<Runtime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Theme::sync_system_appearance(Some(window), cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-k", FocusSearch, Some("Mailstronaut")),
            KeyBinding::new("ctrl-r", RefreshCache, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-d", ToggleAppearance, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-t", FocusReader, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-l", FocusLinks, Some("Mailstronaut")),
            KeyBinding::new("ctrl-l", FocusMessages, Some("Mailstronaut")),
            KeyBinding::new("ctrl-,", ShowAccounts, Some("Mailstronaut")),
            KeyBinding::new("ctrl-1", ShowInbox, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-r", ReloadConfiguration, Some("Mailstronaut")),
        ]);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search cached mail"));
        let text_reader = cx.new(|cx| TextareaState::new(window, cx));
        let html_reader =
            cx.new(|cx| crate::html_reader::HtmlReader::new(runtime.handle(), window, cx));
        let html_events = cx.subscribe_in(&html_reader, window, |this, _, event, window, cx| {
            this.html_event(event, window, cx);
        });
        let events = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.refresh(window, cx);
            }
        });
        let list = cx.new(|cx| ListState::new(Rows::default(), window, cx).searchable(false));
        let list_events = cx.subscribe_in(&list, window, |this, list, event, window, cx| {
            if let ListEvent::Confirm(index) = event
                && let Some(row) = list.read(cx).delegate().items.get(index.row).cloned()
            {
                this.open(row, window, cx);
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, _| {
            this.read_gate
                .visibility(window.is_window_active() && this.view != View::Accounts);
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_override.is_none() {
                Theme::sync_system_appearance(Some(window), cx);
                cx.notify();
            }
        });
        let mut read_gate = crate::read_gate::Gate::default();
        read_gate.visibility(window.is_window_active());
        let mut this = Self {
            focus,
            link_focus: cx.focus_handle(),
            runtime,
            owner: None,
            accounts: Vec::new(),
            declaration: source::Runtime::default(),
            config_path: path,
            loading_config: false,
            view: View::Inbox,
            account_access: crate::account_actions::Access::default(),
            account_views: Vec::new(),
            account_events: Vec::new(),
            account_lifetime: CancellationToken::new(),
            lifetime: CancellationToken::new(),
            query_cancel: CancellationToken::new(),
            read_gate,
            list,
            _list_events: list_events,
            selected: None,
            body: String::new(),
            text_reader,
            reader_mode: if demo {
                ReaderMode::Html
            } else {
                ReaderMode::Text
            },
            html_reader,
            _html_events: html_events,
            opened_message: None,
            preferred_reader: ReaderMode::Html,
            reader_notice: String::new(),
            link_cancel: None,
            search,
            status: "Opening local mail…".into(),
            query: String::new(),
            query_generation: 0,
            reading_generation: 0,
            demo,
            _search_events: events,
            _activation: activation,
            _appearance: appearance,
            theme_override: None,
        };
        if demo {
            this.init_demo(window, cx);
        } else {
            this.load_configuration(window, cx);
        }
        this
    }

    fn init_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status = "Synthetic preview · no accounts or network".into();
        let rows: Vec<_> = [
            (
                "The Mailstronaut team",
                "Welcome aboard",
                "A native space for your mail.",
            ),
            (
                "Doctoral school",
                "Réinscription en doctorat",
                "Synthetic example, no personal data.",
            ),
            (
                "Design notes",
                "A calmer inbox",
                "Built with standard GPUI Kit components.",
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (sender, subject, preview))| Row {
            account: "Demo".into(),
            id: i64::try_from(id).unwrap_or_default(),
            conversation: None,
            sender: sender.into(),
            subject: subject.into(),
            preview: preview.into(),
            unread: 1,
            messages: 1,
        })
        .collect();
        if let Some(row) = rows.first().cloned() {
            self.open(row, window, cx);
        }
        self.list.update(cx, |list, cx| {
            crate::mailbox::replace(list, rows, window, cx);
        });
    }

    fn load_configuration(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.demo || self.loading_config {
            return;
        }
        self.loading_config = true;
        self.status = "Loading native Nix configuration…".into();
        let path = self.config_path.clone();
        let owner = self.owner.clone();
        let cancel = self.lifetime.child_token();
        let job = self.runtime.spawn(async move {
            let path =
                path.ok_or_else(|| "Unable to locate native Nix configuration.".to_owned())?;
            let executable =
                std::env::current_exe().map_err(|_| "Unable to locate configuration helper.")?;
            let config = source::load(path, &executable, &cancel).await;
            let owner = if owner.is_some() || config.is_err() {
                owner
            } else {
                let root = directories::BaseDirs::new()
                    .ok_or("Unable to locate local mail storage.")?
                    .data_local_dir()
                    .join("mailstronaut");
                Some(Arc::new(
                    Worker::open(root)
                        .await
                        .map_err(|error| error.to_string())?,
                ))
            };
            Ok::<_, String>((owner, config))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.loading_config = false;
                match result {
                    Ok(Ok((owner, config))) => {
                        this.owner = owner;
                        if this.declaration.apply(config) {
                            this.retire_open(cx);
                            this.list.update(cx, |list, cx| {
                                crate::mailbox::replace(list, Vec::new(), window, cx);
                            });
                            this.text_reader
                                .update(cx, |reader, cx| reader.set_value("", window, cx));
                            if let Some(config) = this.declaration.active().cloned() {
                                this.accounts = config
                                    .accounts
                                    .iter()
                                    .map(|account| account.id.clone())
                                    .collect();
                                this.apply_appearance(&config, window, cx);
                                this.rebuild_accounts(&config, window, cx);
                            }
                        }
                        if let Some(error) = this.declaration.error() {
                            this.status = if this.declaration.active().is_some() {
                                format!("{error}. The last valid configuration remains active.")
                            } else {
                                format!("{error}. No accounts have been started.")
                            };
                        } else {
                            this.refresh(window, cx);
                        }
                    }
                    Ok(Err(error)) => this.status = error,
                    Err(_) => this.status = "Unable to open local mail.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn rebuild_accounts(
        &mut self,
        config: &Arc<Config>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.account_lifetime.cancel();
        self.account_lifetime = self.lifetime.child_token();
        self.account_events.clear();
        self.account_views.clear();
        let Some(handle) = self.handle() else {
            return;
        };
        let revision = self.declaration.revision();
        for account in &config.accounts {
            let view = cx.new(|cx| {
                crate::accounts::AccountPane::new(
                    config.clone(),
                    account.clone(),
                    handle.clone(),
                    self.runtime.clone(),
                    self.account_access.clone(),
                    self.account_lifetime.child_token(),
                    window,
                    cx,
                )
            });
            self.account_events.push(cx.subscribe_in(
                &view,
                window,
                move |this, _, _: &crate::accounts::Event, window, cx| {
                    if revision == this.declaration.revision() {
                        this.refresh_rows(window, cx);
                    }
                },
            ));
            self.account_views.push(view);
        }
    }

    fn apply_appearance(&mut self, config: &Config, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_override = match config.appearance {
            Appearance::Dark => Some(ThemeMode::Dark),
            Appearance::Light => Some(ThemeMode::Light),
            _ => None,
        };
        if let Some(mode) = self.theme_override {
            Theme::change(mode, Some(window), cx);
        } else {
            Theme::sync_system_appearance(Some(window), cx);
        }
    }

    fn toggle_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().mode.is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.theme_override = Some(mode);
        Theme::change(mode, Some(window), cx);
        cx.notify();
    }

    fn focus_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_some() && self.view != View::Accounts {
            self.cancel_link();
            self.reader_mode = ReaderMode::Text;
            self.preferred_reader = ReaderMode::Text;
            self.html_reader
                .update(cx, crate::html_reader::HtmlReader::hide);
            self.mark_loaded(window, cx);
            self.text_reader
                .update(cx, |reader, cx| reader.focus(window, cx));
            cx.notify();
        }
    }

    fn handle(&self) -> Option<Handle> {
        self.owner.as_ref().map(|owner| owner.handle())
    }

    fn retire_open(&mut self, cx: &mut Context<Self>) {
        self.cancel_link();
        self.reading_generation = self.reading_generation.wrapping_add(1);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.opened_message = None;
        self.reader_notice.clear();
        self.read_gate
            .suppress(self.declaration.revision(), self.reading_generation);
        self.selected = None;
        self.body.clear();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        if query != self.query {
            self.query = query;
            self.retire_open(cx);
            self.list.update(cx, |list, cx| {
                crate::mailbox::replace(list, Vec::new(), window, cx);
            });
        }
        self.refresh_rows(window, cx);
    }

    fn refresh_rows(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(handle) = self.handle() else {
            return;
        };
        self.query_generation = self.query_generation.wrapping_add(1);
        let generation = self.query_generation;
        let revision = self.declaration.revision();
        let accounts = self.accounts.clone();
        self.query_cancel.cancel();
        self.query_cancel = self.lifetime.child_token();
        let cancel = self.query_cancel.clone();
        let query = self.query.clone();
        self.list.update(cx, |list, cx| {
            list.delegate_mut().loading = true;
            list.delegate_mut().searching = !query.is_empty();
            cx.notify();
        });
        self.status = "Searching local cache…".into();
        let job = self.runtime.spawn(async move {
            let executable = std::env::current_exe()
                .map_err(|_| "Unable to locate the conversation helper.".to_owned())?;
            for account in &accounts {
                if let Some(snapshot) = handle
                    .conversation_snapshot(account.clone())
                    .await
                    .map_err(|error| error.to_string())?
                {
                    let groups = mailstronaut_core::threads::process::project(
                        &executable,
                        &snapshot.messages,
                        &cancel,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    handle
                        .commit_conversations(snapshot, groups)
                        .await
                        .map_err(|error| error.to_string())?;
                }
            }
            if cancel.is_cancelled() {
                return Err("Local query cancelled.".into());
            }
            handle
                .conversation_page(accounts, Scope::Inbox, query, None, 100)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if generation != this.query_generation || revision != this.declaration.revision() {
                    return;
                }
                this.list.update(cx, |list, cx| {
                    list.delegate_mut().loading = false;
                    cx.notify();
                });
                match result {
                    Ok(Ok(rows)) => {
                        let count = rows.len();
                        let rows = rows
                            .into_iter()
                            .map(|row| Row {
                                unread: row.unread,
                                conversation: row.conversation,
                                messages: row.cached_messages,
                                account: row.latest.account,
                                id: row.latest.id,
                                sender: row.latest.sender,
                                subject: row.latest.subject,
                                preview: row.latest.preview,
                            })
                            .collect();
                        this.list.update(cx, |list, cx| {
                            crate::mailbox::replace(list, rows, window, cx);
                        });
                        this.status = if this.accounts.is_empty() {
                            "Add accounts in your native Nix configuration.".into()
                        } else {
                            format!("{count} cached conversations · local search")
                        };
                    }
                    Ok(Err(error)) => this.status = error,
                    Err(_) => this.status = "Local query interrupted.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_link();
        self.reading_generation = self.reading_generation.wrapping_add(1);
        let generation = self.reading_generation;
        let revision = self.declaration.revision();
        self.read_gate.clicked(revision, generation);
        self.selected = Some(row.clone());
        self.opened_message = None;
        self.reader_notice.clear();
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.body = "Opening cached message…".into();
        self.text_reader
            .update(cx, |reader, cx| reader.set_value("", window, cx));
        if self.demo {
            // A synthetic plain-text alternative; real messages use the MIME projection.
            let mut text = "A little more space for your mail.\n\nThis synthetic email has a native, read-only text alternative.\nTables, inline CSS and Unicode: Bonjour · Ciao · 世界.\n".to_owned();
            for line in 1..=120 {
                use std::fmt::Write as _;
                _ = writeln!(
                    text,
                    "\nSynthetic reader line {line:03} · Bonjour · Ciao · 世界."
                );
            }
            text.push_str("\nEnd of synthetic message · fin · 世界.");
            self.text_reader
                .update(cx, |reader, cx| reader.set_value(text, window, cx));
            self.body.clear();
            self.reader_mode = self.preferred_reader;
            if self.reader_mode == ReaderMode::Html {
                self.present_html(cx);
            }
        } else if let Some(handle) = self.handle() {
            let job = self
                .runtime
                .spawn(async move { handle.message(row.account.clone(), row.id).await });
            cx.spawn_in(window, async move |this, cx| {
                let result = job.await;
                _ = this.update_in(cx, |this, window, cx| {
                    if generation != this.reading_generation
                        || revision != this.declaration.revision()
                    {
                        return;
                    }
                    match result {
                        Ok(Ok(message)) => {
                            let readable = message.readable_text();
                            let text = readable.map_or_else(
                                || "No readable body is cached.".into(),
                                std::borrow::Cow::into_owned,
                            );
                            this.text_reader
                                .update(cx, |reader, cx| reader.set_value(text, window, cx));
                            this.body.clear();
                            this.opened_message = Some(Arc::new(message));
                            this.reader_mode =
                                if this.preferred_reader == ReaderMode::Html && this.has_html() {
                                    ReaderMode::Html
                                } else {
                                    ReaderMode::Text
                                };
                            if this.reader_mode == ReaderMode::Text {
                                this.mark_loaded(window, cx);
                            } else {
                                this.present_html(cx);
                            }
                        }
                        Ok(Err(error)) => this.body = error.to_string(),
                        Err(_) => this.body = "Message load interrupted.".into(),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn has_html(&self) -> bool {
        self.demo
            || self.opened_message.as_ref().is_some_and(|message| {
                message.html.is_some() && !message.status.encrypted_unsupported
            })
    }

    fn present_html(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_none() || !self.has_html() || self.view != View::Inbox {
            return;
        }
        self.reader_mode = ReaderMode::Html;
        self.preferred_reader = ReaderMode::Html;
        let content = if self.demo {
            crate::html_worker::Content::Demo
        } else {
            let Some(message) = self.opened_message.clone() else {
                return;
            };
            crate::html_worker::Content::Mail(message)
        };
        self.html_reader.update(cx, |reader, cx| {
            reader.show(Arc::new(content), self.reading_generation, cx);
        });
        cx.notify();
    }

    fn html_event(
        &mut self,
        event: &crate::html_reader::Event,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let crate::html_reader::Event::Link(epoch, link) = event {
            self.open_link(*epoch, link.clone(), window, cx);
            return;
        }
        let (crate::html_reader::Event::Ready(epoch) | crate::html_reader::Event::Failed(epoch)) =
            event
        else {
            return;
        };
        if *epoch != self.reading_generation
            || self.reader_mode == ReaderMode::Text
            || self.view != View::Inbox
        {
            return;
        }
        if matches!(event, crate::html_reader::Event::Failed(_)) {
            self.reader_mode = ReaderMode::Text;
            self.reader_notice = "HTML could not be displayed. Showing cached text.".into();
            self.html_reader
                .update(cx, crate::html_reader::HtmlReader::hide);
        }
        self.mark_loaded(window, cx);
        cx.notify();
    }

    fn cancel_link(&mut self) {
        if let Some(cancel) = self.link_cancel.take() {
            cancel.cancel();
            self.reader_notice.clear();
        }
    }

    fn open_link(
        &mut self,
        epoch: u64,
        link: mailstronaut_render::Link,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.reading_generation
            || self.view != View::Inbox
            || self.reader_mode != ReaderMode::Html
            || self.link_cancel.is_some()
            || self.selected.is_none()
        {
            return;
        }
        let revision = self.declaration.revision();
        let cancel = self.lifetime.child_token();
        self.link_cancel = Some(cancel.clone());
        self.reader_notice = "Opening link in your browser…".into();
        let work_cancel = cancel.clone();
        let job = self
            .runtime
            .spawn(async move { crate::link_portal::open(link, &work_cancel).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if cancel.is_cancelled()
                    || epoch != this.reading_generation
                    || revision != this.declaration.revision()
                {
                    return;
                }
                this.link_cancel = None;
                this.reader_notice = match result {
                    Ok(Ok(())) => "Link sent to your browser.",
                    Ok(Err(error)) => error,
                    Err(_) => "Browser request was interrupted.",
                }
                .into();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn mark_loaded(&mut self, window: &Window, cx: &Context<Self>) {
        let eligible = self
            .opened_message
            .as_ref()
            .is_some_and(|message| message.readable_text().is_some());
        if self.read_gate.take(
            self.declaration.revision(),
            self.reading_generation,
            eligible,
        ) {
            self.mark_opened(window, cx);
        }
    }

    fn mark_opened(&self, window: &Window, cx: &Context<Self>) {
        let (Some(handle), Some(row)) = (self.handle(), self.selected.clone()) else {
            return;
        };
        let Some(account) = self.declaration.active().and_then(|config| {
            config
                .accounts
                .iter()
                .find(|account| account.id == row.account)
        }) else {
            return;
        };
        let Ok(context) = mailstronaut_core::store::flags::context(account) else {
            return;
        };
        let revision = self.declaration.revision();
        let job = self.runtime.spawn(async move {
            handle
                .set_flag(
                    row.account,
                    row.id,
                    mailstronaut_mail::imap::FlagChange::Seen,
                    true,
                    context,
                )
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() {
                    return;
                }
                match result {
                    Ok(Ok(())) => this.refresh_rows(window, cx),
                    Ok(Err(error)) => this.status = error.to_string(),
                    Err(_) => this.status = "Unable to save the message's read state.".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Workspace {
    fn show_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_link();
        self.view = View::Accounts;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn show_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view = View::Inbox;
        self.read_gate.visibility(window.is_window_active());
        window.focus(&self.focus, cx);
        self.refresh(window, cx);
        if self.reader_mode == ReaderMode::Html {
            self.present_html(cx);
        }
        cx.notify();
    }

    fn account_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let path = self.config_path.as_ref().map_or_else(
            || "No configuration path is available.".to_owned(),
            |path| path.display().to_string(),
        );
        let loading = self.loading_config;
        let reload_disabled = loading || self.demo;
        let workspace = cx.entity().downgrade();
        let config = self.declaration.active();
        let ownership = if config.is_some_and(|config| config.managed) {
            "Managed by your Nix declaration"
        } else {
            "Native Nix source · read-only in this view"
        };
        let appearance = config.map_or("system", |config| match config.appearance {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
            _ => "system",
        });
        let general = SettingPage::new("Configuration")
            .description("Your native Nix source is authoritative. Edit it and reload to apply changes.")
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("Native Nix")
                    .description(ownership)
                    .item(SettingItem::render(move |_, _, _| {
                        DescriptionList::vertical().columns(1).item("Source",
                            div().flex().items_center().gap_2()
                                .child(div().flex_1().min_w_0().child(path.clone()))
                                .child(Clipboard::new("copy-source").value(path.clone())
                                    .accessibility_label("Copy configuration path").tooltip("Copy configuration path"))
                                .into_any_element(), 1)
                    }).keywords(["source", "path", "Nix", "configuration", "Home Manager"]))
                    .item(SettingItem::new("Appearance", SettingField::dropdown(
                        vec![("system".into(), "System".into()), ("light".into(), "Light".into()), ("dark".into(), "Dark".into())],
                        move |_| appearance.into(), |_, _| {},
                    )).description("Declared appearance. The toolbar toggle is a temporary window preview.").disabled(true))
                    .item(SettingItem::render(move |options, _, _| {
                        let workspace = workspace.clone();
                        Button::new("reload-config")
                            .label(if loading { "Loading…" } else { "Reload configuration" })
                            .disabled(options.is_disabled() || reload_disabled)
                            .on_click(move |_, window, cx| {
                                _ = workspace.update(cx, |this, cx| this.load_configuration(window, cx));
                            })
                    }).keywords(["reload", "configuration", "Nix", "Home Manager"])),
            )
            .group(SettingGroup::new().title("Account status").item({
                let status = self.declaration.error().map_or_else(
                    || if self.account_views.is_empty() {
                        "No accounts are active. Add an account declaration, then reload.".to_owned()
                    } else {
                        "Choose an account in Settings to sign in or sync mail.".to_owned()
                    },
                    |error| if config.is_some() {
                        format!("{error}. The last valid configuration remains active.")
                    } else {
                        format!("{error}. No accounts have been started.")
                    },
                );
                SettingItem::render(move |_, _, _| div().text_sm().child(status.clone()))
                    .keywords(["accounts", "status", "errors", "reload"])
            }));
        Settings::new(("mail-settings", self.declaration.revision()))
            .sidebar_width(px(220.))
            .page(general)
            .pages(
                self.account_views
                    .iter()
                    .map(|view| crate::accounts::AccountPane::page(view, cx)),
            )
    }

    fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        Sidebar::new("mail-sidebar")
            .w_full()
            .collapsible(false)
            .header(SidebarHeader::new().child(div().font_semibold().child("Mailstronaut")))
            .child(
                SidebarGroup::new("Mail").child(
                    SidebarMenu::new().child(
                        SidebarMenuItem::new("Unified inbox")
                            .icon(IconName::Inbox)
                            .active(self.view == View::Inbox)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.show_inbox(window, cx)),
                            ),
                    ),
                ),
            )
            .footer(
                SidebarFooter::new().child(
                    Button::new("settings")
                        .ghost()
                        .icon(IconName::Settings)
                        .w_full()
                        .label("Settings")
                        .selected(self.view == View::Accounts)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_accounts(window, cx)),
                        ),
                ),
            )
    }

    fn mailbox(&self, cx: &Context<Self>) -> impl IntoElement {
        h_resizable("mail-panes")
            .child(
                resizable_panel()
                    .size(px(360.))
                    .size_range(px(280.)..px(600.))
                    .child(self.message_list(cx)),
            )
            .child(resizable_panel().child(self.reader(cx)))
    }

    fn top_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        div()
            .flex()
            .items_center()
            .gap_4()
            .p_4()
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .w(px(190.))
                    .font_semibold()
                    .child(if self.view == View::Accounts {
                        "Settings"
                    } else {
                        "Unified inbox"
                    }),
            )
            .child(div().flex_1().when(self.view == View::Inbox, |view| {
                view.child(Input::new(&self.search))
            }))
            .when(self.view == View::Inbox, |view| {
                view.child(
                    Button::new("refresh")
                        .label("Refresh cache")
                        .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                )
            })
            .child(
                Button::new("appearance")
                    .ghost()
                    .label("Appearance")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_appearance(window, cx);
                    })),
            )
    }

    fn message_list(&self, _: &Context<Self>) -> impl IntoElement {
        List::new(&self.list).size_full()
    }
    fn reader(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div()
            .id("reader-scroll")
            .size_full()
            .flex()
            .flex_col()
            .p_8()
            .child(self.selected.as_ref().map_or_else(
                || {
                    div().child(div().text_2xl().child("Room to focus.")).child(
                        div()
                            .mt_3()
                            .text_color(muted)
                            .child("Open a message to read it here."),
                    )
                },
                |row| {
                    div()
                        .mb_6()
                        .child(div().text_2xl().font_semibold().child(row.subject.clone()))
                        .child(
                            div()
                                .mt_2()
                                .text_sm()
                                .text_color(muted)
                                .child(format!("{} · {}", row.sender, row.account)),
                        )
                },
            ))
            .when_some(self.selected.as_ref(), |view, _| {
                view.child(
                    Toolbar::new("reader-tools")
                        .mb_4()
                        .content(
                            TabBar::new("reader-mode")
                                .segmented()
                                .selected_index(usize::from(self.reader_mode == ReaderMode::Html))
                                .on_click(cx.listener(|this, index, window, cx| {
                                    if *index == 0 {
                                        this.focus_reader(window, cx);
                                    } else {
                                        this.present_html(cx);
                                    }
                                }))
                                .child(Tab::new().label("Text"))
                                .child(Tab::new().label("HTML").disabled(!self.has_html())),
                        )
                        .content(
                            ToolbarGroup::new("reader-text-actions")
                                .label("Message text")
                                .child(
                                    Button::new("select-message-text")
                                        .ghost()
                                        .label("Select text")
                                        .tooltip("Text selection and copy · Ctrl+Shift+T")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.focus_reader(window, cx);
                                        })),
                                ),
                        )
                        .content(self.link_menu(cx)),
                )
            })
            .when(!self.reader_notice.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(self.reader_notice.clone()),
                )
            })
            .when(
                self.reader_mode == ReaderMode::Text && self.body.is_empty(),
                |view| {
                    view.child(
                        div().flex_1().min_h_0().child(
                            Textarea::new(&self.text_reader)
                                .readonly(true)
                                .appearance(false)
                                .bordered(false)
                                .aria_label("Message text")
                                .accessibility_id("message-text")
                                .h_full()
                                .w_full(),
                        ),
                    )
                },
            )
            .when(
                self.reader_mode == ReaderMode::Html && self.body.is_empty(),
                |view| view.child(div().flex_1().min_h_0().child(self.html_reader.clone())),
            )
            .when(!self.body.is_empty(), |view| view.child(self.body.clone()))
    }

    fn link_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let links = self.html_reader.read(cx).links();
        let epoch = self.reading_generation;
        let workspace = cx.entity().downgrade();
        let restore_focus = workspace.clone();
        Button::new("reader-web-links")
            .ghost()
            .label("Links")
            .tooltip("Open in browser · first 128 web links · Ctrl+Shift+L")
            .track_focus(&self.link_focus)
            .disabled(
                links.is_empty()
                    || self.reader_mode != ReaderMode::Html
                    || self.link_cancel.is_some(),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for link in links.iter() {
                    let workspace = workspace.clone();
                    let target = link.clone();
                    menu = menu.item(PopupMenuItem::new(link.as_str().to_owned()).on_click(
                        move |_, window, cx| {
                            _ = workspace.update(cx, |this, cx| {
                                this.open_link(epoch, target.clone(), window, cx);
                            });
                        },
                    ));
                }
                menu
            })
            .on_open_change(move |open, window, cx| {
                if !open {
                    let workspace = restore_focus.clone();
                    // This pin restores a context-only trigger handle. Restore
                    // the Button's public focus handle after menu dismissal.
                    window.defer(cx, move |window, cx| {
                        _ = workspace.update(cx, |this, cx| {
                            if epoch == this.reading_generation
                                && this.view == View::Inbox
                                && this.reader_mode == ReaderMode::Html
                                && this.link_cancel.is_none()
                            {
                                window.focus(&this.link_focus, cx);
                            }
                        });
                    });
                }
            })
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        div()
            .size_full()
            .key_context("Mailstronaut")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.view == View::Accounts {
                    this.show_inbox(window, cx);
                }
                this.search.update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &RefreshCache, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleAppearance, window, cx| {
                this.toggle_appearance(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusMessages, window, cx| {
                if this.view == View::Accounts {
                    this.show_inbox(window, cx);
                }
                this.list.update(cx, |list, cx| list.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &FocusReader, window, cx| {
                this.focus_reader(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusLinks, window, cx| {
                if this.view == View::Inbox
                    && this.reader_mode == ReaderMode::Html
                    && this.link_cancel.is_none()
                    && !this.html_reader.read(cx).links().is_empty()
                {
                    window.focus(&this.link_focus, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &ShowAccounts, window, cx| this.show_accounts(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowInbox, window, cx| this.show_inbox(window, cx)))
            .on_action(cx.listener(|this, _: &ReloadConfiguration, window, cx| {
                this.load_configuration(window, cx);
            }))
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(gpui::component::TitleBar::new().child("Mailstronaut"))
            .child(self.top_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("workspace-panes")
                        .child(
                            resizable_panel()
                                .size(px(200.))
                                .size_range(px(160.)..px(300.))
                                .child(self.sidebar(cx)),
                        )
                        .child(
                            resizable_panel().child(
                                div()
                                    .size_full()
                                    .when(self.view == View::Accounts, |view| {
                                        view.child(self.account_settings(cx))
                                    })
                                    .when(self.view == View::Inbox, |view| {
                                        view.child(self.mailbox(cx))
                                    }),
                            ),
                        ),
                ),
            )
            .child(
                div()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(muted)
                    .border_t_1()
                    .border_color(border)
                    .child(self.status.clone()),
            )
    }
}
