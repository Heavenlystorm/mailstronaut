//! GPUI Kit mailbox workspace. Mail and storage remain toolkit-independent.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Selectable as _, Theme, ThemeMode,
    WindowExt as _,
    alert::Alert,
    attachment::{
        Attachment, AttachmentContent, AttachmentDescription, AttachmentGroup, AttachmentTitle,
    },
    badge::Badge,
    button::{Button, ButtonVariants as _},
    description_list::DescriptionList,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    label::Label,
    list::{List, ListEvent, ListState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::{Notification, NotificationDelivery},
    resizable::{h_resizable, resizable_panel},
    setting::{SettingGroup, SettingItem, SettingPage, Settings},
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem},
    tab::{Tab, TabBar},
    toolbar::{Toolbar, ToolbarGroup},
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};
use mailstronaut_config::{Account, Appearance, Config, source};
use mailstronaut_core::{
    reminders::When,
    reply::Kind,
    store::browse::Scope,
    store::mailbox::{Cursor, Folder, FolderRole},
    worker::{Handle, Worker},
};
use mailstronaut_mail::imap::FlagChange;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::mailbox::{Row, Rows};

/// How long a queued move can be undone before the server is told about it.
const UNDO_GRACE: Duration = Duration::from_secs(8);
/// Returning to the window checks for mail, at most this often.
const ACTIVATION_WAKE_INTERVAL: Duration = Duration::from_secs(60);

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
        ReloadConfiguration,
        #[derive(Eq)]
        /// Export a validated native snapshot from Settings.
        ExportConfiguration,
        #[derive(Eq)]
        /// Start a new local draft.
        ComposeMessage,
        #[derive(Eq)]
        /// Show saved drafts and the outbox.
        ShowDrafts,
        #[derive(Eq)]
        /// Prepare a reply to the open original.
        Reply,
        #[derive(Eq)]
        /// Prepare a reply to every visible recipient of the open original.
        ReplyAll,
        #[derive(Eq)]
        /// Prepare a plain-text forward of the open original.
        Forward,
        #[derive(Eq)]
        /// Toggle the open original between read and unread.
        ToggleRead,
        #[derive(Eq)]
        /// Toggle the star on the open original.
        ToggleStar,
        #[derive(Eq)]
        /// Move the open conversation to the account's archive.
        Archive,
        #[derive(Eq)]
        /// Move the open conversation to the account's trash.
        Trash,
        #[derive(Eq)]
        /// Show scheduled and overdue reminders.
        ShowReminders
    ]
);

#[derive(Eq, PartialEq)]
enum View {
    Inbox,
    Accounts,
    Compose,
    Reminders,
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
    folders: Vec<Folder>,
    /// Unread messages by folder ID, from the same read as `folders`.
    unread: HashMap<i64, usize>,
    scope: Scope,
    pages: crate::mailbox::Pages,
    declaration: source::Runtime,
    config_path: Option<PathBuf>,
    preferences: Entity<crate::preferences::Preferences>,
    onboarding: Entity<crate::onboarding::Onboarding>,
    _preference_events: Subscription,
    loading_config: bool,
    view: View,
    account_access: crate::account_actions::Access,
    composer: Entity<crate::composer::Composer>,
    _composer_events: Subscription,
    reminders: Entity<crate::reminders::Reminders>,
    _reminder_events: Subscription,
    account_views: Vec<Entity<crate::accounts::AccountPane>>,
    account_events: Vec<Subscription>,
    account_lifetime: CancellationToken,
    lifetime: CancellationToken,
    query_cancel: CancellationToken,
    read_gate: crate::read_gate::Gate,
    list: Entity<ListState<Rows>>,
    _list_events: Subscription,
    thread_reader: Entity<crate::thread_reader::ThreadReader>,
    _thread_events: Subscription,
    selected: Option<Row>,
    body: String,
    text_reader: Entity<TextareaState>,
    reader_mode: ReaderMode,
    html_reader: Entity<crate::html_reader::HtmlReader>,
    _html_events: Subscription,
    opened_message: Option<Arc<mailstronaut_core::mime::Projection>>,
    /// What the opened message left on the server, if it is too large to keep.
    opened_omitted: Option<mailstronaut_core::outline::Omitted>,
    /// Why the opened message was or was not flagged, if it has been analyzed.
    opened_analysis: Option<mailstronaut_priority::Analysis>,
    preferred_reader: ReaderMode,
    reader_notice: String,
    headers_open: bool,
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
    _focus_lost: Subscription,
    theme_override: Option<ThemeMode>,
    engine: Option<crate::mail_sync::Engine>,
    /// Fuzzy search once the store is open; literal search answers until then.
    fuzzy: Option<crate::mail_search::MailSearch>,
    search_pause: Task<()>,
    /// Only while `priority.enabled` is set in the configuration.
    priority: Option<crate::mail_priority::PriorityEngine>,
    /// Refreshes the list when the first thing in the priority group expires.
    priority_expiry: Task<()>,
    /// How far back older mail has been fetched, by account and folder wire name. Absent
    /// means not yet asked for, and `None` means the beginning of the folder was reached.
    older_reached: HashMap<(String, String), Option<mailstronaut_core::sync::Position>>,
    /// Present while older mail is being fetched.
    older_fetch: Option<CancellationToken>,
    announcer: Option<crate::mail_notify::Announcer>,
    /// The newest message already considered, per account: new mail is what comes after.
    announced_through: HashMap<String, i64>,
    /// Messages announced since a window was last in front, per account.
    announced_count: HashMap<String, u32>,
    syncing: BTreeSet<String>,
    sync_errors: BTreeMap<String, String>,
    last_sync: String,
    last_wake: Instant,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.lifetime.cancel();
        self.account_lifetime.cancel();
        self.query_cancel.cancel();
    }
}

const SEARCH_PAUSE: Duration = Duration::from_millis(200);
/// Mail older than this is not new, however recently it was fetched: older mail can be
/// fetched on request, and an account can come back after days offline.
const NEW_MAIL_WITHIN_SECONDS: i64 = 3 * 24 * 3600;
/// Identifies the progress notification, so that the result replaces it.
struct AttachmentDownload;
/// What one list refresh loads: folders, unread counts, the page and the priority group.
type ListPage = (
    Vec<Folder>,
    Vec<(i64, u32)>,
    Vec<mailstronaut_core::store::conversations::Summary>,
    Vec<Row>,
);
/// More than this much needing attention at once is not a useful group.
const MAX_PRIORITY_ROWS: u32 = 20;

/// The priority group as list rows, one per conversation, soonest to expire first.
async fn priority_rows(
    store: &Handle,
    accounts: Vec<String>,
) -> Result<Vec<Row>, mailstronaut_core::worker::Error> {
    let now = chrono::Utc::now().timestamp();
    let items = store
        .priority_items(accounts.clone(), now, MAX_PRIORITY_ROWS)
        .await?;
    let ids = items.iter().map(|item| item.id).collect();
    let rows = store
        .ranked_conversation_page(accounts, Scope::Inbox, ids, None, MAX_PRIORITY_ROWS)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut row = Row::from(row);
            if let Some(item) = items.iter().find(|item| item.id == row.id) {
                row.due.clone_from(&item.due);
                row.expires_at = Some(item.expires_at);
            }
            row
        })
        .collect())
}

/// A page of conversations: through the search index when there is a query and it can
/// answer, and otherwise through the store's own literal search.
async fn conversation_rows(
    store: &Handle,
    fuzzy: Option<&crate::mail_search::MailSearch>,
    accounts: Vec<String>,
    scope: Scope,
    query: String,
    after: Option<Cursor>,
) -> Result<Vec<mailstronaut_core::store::conversations::Summary>, mailstronaut_core::worker::Error>
{
    let hits = match fuzzy {
        Some(search) if !query.trim().is_empty() => {
            // The number of hits is bounded, so other accounts must not use them up.
            let searched = match &scope {
                Scope::Inbox => accounts.clone(),
                Scope::Account(account) | Scope::Folder { account, .. } => vec![account.clone()],
            };
            search.hits(query.clone(), searched).await
        }
        _ => None,
    };
    match hits {
        Some(hits) => {
            store
                .ranked_conversation_page(accounts, scope, hits, after, 100)
                .await
        }
        None => {
            store
                .conversation_page(accounts, scope, query, after, 100)
                .await
        }
    }
}

impl Workspace {
    #[expect(
        clippy::too_many_lines,
        reason = "Construct native entities and their subscriptions together before initializing the workspace that owns them."
    )]
    pub fn new(
        path: Option<PathBuf>,
        demo: bool,
        runtime: Arc<Runtime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Theme::sync_system_appearance(Some(window), cx);
        Self::bind_keys(cx);
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
            match event {
                InputEvent::PressEnter { .. } => {
                    this.search_pause = Task::ready(());
                    this.refresh(window, cx);
                }
                // A query is cheap, but several keystrokes are still one search.
                InputEvent::Change => {
                    this.search_pause = cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(SEARCH_PAUSE).await;
                        _ = this.update_in(cx, Self::refresh);
                    });
                }
                _ => {}
            }
        });
        let workspace = cx.entity().downgrade();
        let rows = Rows {
            on_dismiss: Some(Rc::new(move |message, window, cx| {
                _ = workspace.update(cx, |this, cx| this.dismiss_priority(message, window, cx));
            })),
            ..Rows::default()
        };
        let list = cx.new(|cx| ListState::new(rows, window, cx).searchable(false));
        let list_events = cx.subscribe_in(&list, window, |this, list, event, window, cx| {
            if let ListEvent::Confirm(index) = event
                && let Some(row) = list.read(cx).delegate().row(*index).cloned()
            {
                this.open(row, window, cx);
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, _| {
            this.read_gate
                .visibility(window.is_window_active() && this.view == View::Inbox);
            if let Some(announcer) = &this.announcer {
                announcer.set_window_active(window.is_window_active());
            }
            if window.is_window_active() {
                this.withdraw_announcements();
            }
            if window.is_window_active()
                && let Some(engine) = &this.engine
                && this.last_wake.elapsed() >= ACTIVATION_WAKE_INTERVAL
            {
                this.last_wake = Instant::now();
                engine.wake_all();
            }
        });
        let thread_reader =
            cx.new(|cx| crate::thread_reader::ThreadReader::new(runtime.clone(), window, cx));
        let thread_events = cx.subscribe_in(
            &thread_reader,
            window,
            |this, _, event: &crate::thread_reader::Open, window, cx| {
                if this.view == View::Inbox
                    && this
                        .selected
                        .as_ref()
                        .is_some_and(|row| row.same_group(&event.0))
                {
                    this.open(event.0.clone(), window, cx);
                }
            },
        );
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_override.is_none() {
                Theme::sync_system_appearance(Some(window), cx);
                cx.notify();
            }
        });
        // A focused control can leave the tree (a disabled pager button, a retired
        // thread list). Global bindings need the workspace back in the focus path.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            window.focus(&this.focus, cx);
        });
        let mut read_gate = crate::read_gate::Gate::default();
        read_gate.visibility(window.is_window_active());
        let lifetime = CancellationToken::new();
        let account_access = crate::account_actions::Access::default();
        let composer = cx.new(|cx| {
            crate::composer::Composer::new(
                runtime.clone(),
                account_access.clone(),
                lifetime.child_token(),
                window,
                cx,
            )
        });
        let composer_events = cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            if let crate::composer::Event::Closed(message) = event {
                window.push_notification(
                    Notification::success(message.clone()).delivery(NotificationDelivery::InApp),
                    cx,
                );
                this.navigate(this.scope.clone(), window, cx);
            }
            cx.notify();
        });
        let reminders = cx.new(|cx| {
            crate::reminders::Reminders::new(runtime.clone(), lifetime.child_token(), window, cx)
        });
        let reminder_events = cx.subscribe_in(&reminders, window, |this, _, event, window, cx| {
            if let crate::reminders::Event::Open(record) = event {
                this.open_reminder(record, window, cx);
            }
            cx.notify();
        });
        let preferences = cx.new(|cx| {
            crate::preferences::Preferences::new(
                runtime.clone(),
                lifetime.child_token(),
                window,
                cx,
            )
        });
        let preference_events =
            cx.subscribe_in(&preferences, window, |this, _, event, window, cx| {
                if let crate::preferences::Event::Saved(revision) = event
                    && *revision == this.declaration.revision()
                {
                    this.load_configuration(window, cx);
                }
                cx.notify();
            });
        let onboarding =
            cx.new(|cx| crate::onboarding::Onboarding::new(preferences.clone(), window, cx));
        let mut this = Self {
            focus,
            link_focus: cx.focus_handle(),
            runtime,
            owner: None,
            accounts: Vec::new(),
            folders: Vec::new(),
            unread: HashMap::new(),
            scope: Scope::Inbox,
            pages: crate::mailbox::Pages::default(),
            declaration: source::Runtime::default(),
            config_path: path,
            preferences,
            onboarding,
            _preference_events: preference_events,
            loading_config: false,
            view: View::Inbox,
            account_access,
            composer,
            _composer_events: composer_events,
            reminders,
            _reminder_events: reminder_events,
            account_views: Vec::new(),
            account_events: Vec::new(),
            account_lifetime: CancellationToken::new(),
            lifetime,
            query_cancel: CancellationToken::new(),
            read_gate,
            list,
            _list_events: list_events,
            thread_reader,
            _thread_events: thread_events,
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
            opened_omitted: None,
            opened_analysis: None,
            preferred_reader: ReaderMode::Html,
            reader_notice: String::new(),
            headers_open: false,
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
            _focus_lost: focus_lost,
            theme_override: None,
            engine: None,
            fuzzy: None,
            search_pause: Task::ready(()),
            priority: None,
            priority_expiry: Task::ready(()),
            older_reached: HashMap::new(),
            older_fetch: None,
            announcer: None,
            announced_through: HashMap::new(),
            announced_count: HashMap::new(),
            syncing: BTreeSet::new(),
            sync_errors: BTreeMap::new(),
            last_sync: String::new(),
            last_wake: Instant::now(),
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
            received_at: 1_700_000_000,
            seen: false,
            starred: false,
            due: None,
            expires_at: None,
        })
        .collect();
        if let Some(row) = rows.first().cloned() {
            self.open(row, window, cx);
        }
        self.list.update(cx, |list, cx| {
            crate::mailbox::replace(list, rows, window, cx);
        });
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One existing atomic configuration adoption path also updates its small preference snapshot"
    )]
    fn load_configuration(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.demo || self.loading_config || self.preferences.read(cx).busy() {
            return;
        }
        self.loading_config = true;
        self.preferences
            .update(cx, crate::preferences::Preferences::loading);
        self.status = "Loading native Nix configuration…".into();
        let path = self.config_path.clone();
        let owner = self.owner.clone();
        let cancel = self.lifetime.child_token();
        let job = self.runtime.spawn(async move {
            let path =
                path.ok_or_else(|| "Unable to locate native Nix configuration.".to_owned())?;
            let executable =
                std::env::current_exe().map_err(|_| "Unable to locate configuration helper.")?;
            let config = source::load(path.clone(), &executable, &cancel).await;
            let ownership = tokio::task::spawn_blocking(move || source::ownership(&path))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(source::Ownership::Unavailable);
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
            Ok::<_, String>((owner, config, ownership))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.loading_config = false;
                match result {
                    Ok(Ok((owner, config, ownership))) => {
                        this.owner = owner;
                        this.start_search();
                        if this.declaration.apply(config) {
                            this.retire_open(window, cx);
                            this.list.update(cx, |list, cx| {
                                crate::mailbox::replace(list, Vec::new(), window, cx);
                            });
                            if let Some(config) = this.declaration.active().cloned() {
                                this.accounts = config
                                    .accounts
                                    .iter()
                                    .map(|account| account.id.clone())
                                    .collect();
                                if match &this.scope {
                                    Scope::Inbox => false,
                                    Scope::Account(account) | Scope::Folder { account, .. } => {
                                        !this.accounts.contains(account)
                                    }
                                } {
                                    this.scope = Scope::Inbox;
                                }
                                this.pages = crate::mailbox::Pages::default();
                                let zone = crate::mailbox::local_zone(config.time_zone.as_deref());
                                let handle = this.handle();
                                this.reminders.update(cx, |reminders, cx| {
                                    reminders.configure(&config, handle, zone.clone(), window, cx);
                                });
                                this.list.update(cx, |list, _| {
                                    list.delegate_mut().time_zone = zone;
                                });
                                this.apply_appearance(&config, window, cx);
                                this.rebuild_accounts(&config, window, cx);
                                let handle = this.handle();
                                this.composer.update(cx, |composer, cx| {
                                    composer.configure(&config, handle, window, cx);
                                });
                                this.restart_sync(&config, window, cx);
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
                        let snapshot = crate::preferences::Snapshot {
                            path: this.config_path.clone(),
                            config: this.declaration.active().cloned(),
                            revision: this.declaration.revision(),
                            ownership,
                            error: this.declaration.error().map(|error| error.to_string()),
                        };
                        this.preferences.update(cx, |preferences, cx| {
                            preferences.configure(snapshot, window, cx);
                        });
                        if this.declaration.active().is_none() {
                            this.view = View::Accounts;
                            this.read_gate.visibility(false);
                        }
                    }
                    Ok(Err(error)) => this.status = error,
                    Err(_) => this.status = "Unable to open local mail.".into(),
                }
                if this.preferences.read(cx).busy() {
                    let error = this.status.clone();
                    this.preferences
                        .update(cx, |preferences, cx| preferences.failed(error, cx));
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
            let account_id = account.id.clone();
            let inbox = account
                .folders
                .inbox
                .clone()
                .unwrap_or_else(|| "INBOX".into());
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
                move |this, _, event: &crate::accounts::Event, window, cx| {
                    if revision == this.declaration.revision() {
                        match event {
                            crate::accounts::Event::CacheChanged => this.refresh_rows(window, cx),
                            crate::accounts::Event::CredentialsChanged => {
                                if let Some(engine) = &this.engine {
                                    engine.include_folder(&account_id, inbox.clone());
                                }
                            }
                        }
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
        if self.selected.is_some() && self.view == View::Inbox {
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

    fn retire_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.text_reader
            .update(cx, |reader, cx| reader.set_value("", window, cx));
        self.thread_reader
            .update(cx, crate::thread_reader::ThreadReader::clear);
        self.cancel_link();
        self.reading_generation = self.reading_generation.wrapping_add(1);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.opened_message = None;
        self.opened_omitted = None;
        self.opened_analysis = None;
        self.headers_open = false;
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
            self.pages = crate::mailbox::Pages::default();
            self.retire_open(window, cx);
            self.list.update(cx, |list, cx| {
                crate::mailbox::replace(list, Vec::new(), window, cx);
            });
        }
        self.refresh_rows(window, cx);
    }

    /// Nothing else happens when a priority's time runs out, so the list must look again.
    fn watch_priority_expiry(&mut self, group: &[Row], window: &Window, cx: &Context<Self>) {
        let now = chrono::Utc::now().timestamp();
        self.priority_expiry = group
            .iter()
            .filter_map(|row| row.expires_at)
            .min()
            .map_or_else(
                || Task::ready(()),
                |expires_at| {
                    let wait = u64::try_from(expires_at - now).unwrap_or_default() + 1;
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_secs(wait))
                            .await;
                        _ = this.update_in(cx, |this, window, cx| this.refresh_rows(window, cx));
                    })
                },
            );
    }

    fn dismiss_priority(&self, message: i64, window: &Window, cx: &Context<Self>) {
        let Some(store) = self.handle() else {
            return;
        };
        let job = self
            .runtime
            .spawn(async move { store.dismiss_priority(message).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if matches!(result, Ok(Ok(()))) {
                    this.refresh_rows(window, cx);
                } else {
                    this.status = "Unable to dismiss this message.".into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_priority(&mut self, config: &Config, window: &Window, cx: &Context<Self>) {
        self.priority = None;
        let Some(store) = self.handle().filter(|_| config.priority.enabled) else {
            return;
        };
        let (engine, mut changed) = crate::mail_priority::PriorityEngine::start(
            &self.runtime,
            store,
            config,
            self.lifetime.child_token(),
        );
        self.priority = Some(engine);
        let revision = self.declaration.revision();
        cx.spawn_in(window, async move |this, cx| {
            while changed.recv().await.is_some() {
                let applied = this.update_in(cx, |this, window, cx| {
                    if revision == this.declaration.revision() {
                        this.refresh_rows(window, cx);
                    }
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn start_search(&mut self) {
        if self.fuzzy.is_some() {
            return;
        }
        let (Some(store), Some(directories)) = (self.handle(), directories::BaseDirs::new()) else {
            return;
        };
        // A sibling of the mail store, which is the only source of what is in it.
        let directory = directories
            .data_local_dir()
            .join("mailstronaut")
            .join("search");
        self.fuzzy = Some(crate::mail_search::MailSearch::start(
            &self.runtime,
            directory,
            store,
            self.lifetime.child_token(),
        ));
    }

    fn page_status(&self, count: usize) -> String {
        let page = self.pages.number();
        let waiting = self
            .fuzzy
            .as_ref()
            .map_or(0, crate::mail_search::MailSearch::queued);
        if self.query.trim().is_empty() {
            format!("{count} cached conversations · page {page} · local cache")
        } else {
            let matches = if count == 1 { "match" } else { "matches" };
            if waiting > 0 {
                format!("{count} {matches} · page {page} · {waiting} messages not searchable yet")
            } else {
                format!("{count} {matches} · page {page} · searched local cache")
            }
        }
    }

    fn show_page(&mut self, page: ListPage, window: &mut Window, cx: &mut Context<Self>) {
        let (folders, unread, rows, group) = page;
        self.watch_priority_expiry(&group, window, cx);
        self.folders = folders;
        self.unread = unread
            .into_iter()
            .map(|(folder, count)| (folder, count as usize))
            .collect();
        let count = rows.len();
        self.pages.more = count == 100;
        let rows = rows.into_iter().map(Row::from).collect();
        self.list.update(cx, |list, cx| {
            crate::mailbox::replace_with_priority(list, rows, group, window, cx);
        });
        self.status = if self.accounts.is_empty() {
            "Add an account in Settings or your native Nix configuration.".into()
        } else {
            self.page_status(count)
        };
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
        let scope = self.scope.clone();
        let after = self.pages.after;
        let fuzzy = self.fuzzy.clone();
        // The group belongs at the top of the unified view, not in searches or later pages.
        let show_priority = self.priority.is_some()
            && self.scope == Scope::Inbox
            && query.trim().is_empty()
            && !self.pages.has_previous();
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
            let folders = handle
                .folders(accounts.clone())
                .await
                .map_err(|error| error.to_string())?;
            let unread = handle
                .unread_counts(accounts.clone())
                .await
                .map_err(|error| error.to_string())?;
            let group = if show_priority {
                priority_rows(&handle, accounts.clone())
                    .await
                    .map_err(|error| error.to_string())?
            } else {
                Vec::new()
            };
            conversation_rows(&handle, fuzzy.as_ref(), accounts, scope, query, after)
                .await
                .map(|rows| (folders, unread, rows, group))
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
                    Ok(Ok(page)) => this.show_page(page, window, cx),
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
        self.headers_open = false;
        if let Some(handle) = self.handle() {
            let zone = self.list.read(cx).delegate().time_zone.clone();
            self.thread_reader.update(cx, |thread, cx| {
                thread.show(row.clone(), handle, zone, window, cx);
            });
        }
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
            let job = self.runtime.spawn(async move {
                let message = handle.message(row.account.clone(), row.id).await?;
                let omitted = handle.omitted(row.account.clone(), row.id).await?;
                // A result from another version may not parse, and then it says nothing.
                let analysis = handle
                    .analysis_data(row.id)
                    .await?
                    .and_then(|data| serde_json::from_str(&data).ok());
                Ok::<_, mailstronaut_core::worker::Error>((message, omitted, analysis))
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = job.await;
                _ = this.update_in(cx, |this, window, cx| {
                    if generation != this.reading_generation
                        || revision != this.declaration.revision()
                    {
                        return;
                    }
                    match result {
                        Ok(Ok((message, omitted, analysis))) => {
                            this.opened_omitted = omitted;
                            this.opened_analysis = analysis;
                            let readable = message.readable_text();
                            let text = readable.map_or_else(
                                || "No readable body is cached.".into(),
                                |text| crate::mailbox::display_text(&text),
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

    fn mark_opened(&mut self, window: &Window, cx: &Context<Self>) {
        self.apply_flag(FlagChange::Seen, true, window, cx);
    }

    /// An explicit toggle stays until the next explicit open: the read gate is spent,
    /// so refreshing or reselecting this message cannot mark it read again.
    fn toggle_flag(&mut self, flag: FlagChange, window: &Window, cx: &Context<Self>) {
        let Some(row) = &self.selected else {
            return;
        };
        let desired = match flag {
            FlagChange::Seen => !row.seen,
            FlagChange::Starred => !row.starred,
        };
        self.apply_flag(flag, desired, window, cx);
    }

    fn apply_flag(&mut self, flag: FlagChange, desired: bool, window: &Window, cx: &Context<Self>) {
        let (Some(handle), Some(row)) = (self.handle(), self.selected.as_mut()) else {
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
        match flag {
            FlagChange::Seen => row.seen = desired,
            FlagChange::Starred => row.starred = desired,
        }
        let (account, id) = (row.account.clone(), row.id);
        let revision = self.declaration.revision();
        let job = self
            .runtime
            .spawn(async move { handle.set_flag(account, id, flag, desired, context).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() {
                    return;
                }
                match result {
                    Ok(Ok(())) => {
                        this.refresh_rows(window, cx);
                        this.thread_reader
                            .update(cx, |thread, cx| thread.refresh(window, cx));
                        // Changes reach the server on the account's next cycle.
                        if let Some(engine) = &this.engine {
                            engine.wake_all();
                        }
                    }
                    Ok(Err(error)) => {
                        this.status = error.to_string();
                        if let Some(row) = this.selected.as_mut().filter(|row| row.id == id) {
                            match flag {
                                FlagChange::Seen => row.seen = !desired,
                                FlagChange::Starred => row.starred = !desired,
                            }
                        }
                    }
                    Err(_) => this.status = "Unable to save the message's state.".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn account_folders(&self, account: &str) -> impl Iterator<Item = &Folder> {
        self.folders
            .iter()
            .filter(move |folder| folder.account == account)
    }

    fn role_folder(&self, role: FolderRole) -> Option<&Folder> {
        let row = self.selected.as_ref()?;
        self.folders
            .iter()
            .find(|folder| folder.account == row.account && folder.role == Some(role))
    }

    fn move_to_role(&mut self, role: FolderRole, window: &Window, cx: &mut Context<Self>) {
        let Some(destination) = self.role_folder(role).map(|folder| folder.name.clone()) else {
            self.reader_notice = format!(
                "This account has no {} folder. Set one in your Nix configuration.",
                role.label()
            );
            cx.notify();
            return;
        };
        self.move_open(destination, role.label().to_owned(), window, cx);
    }

    /// Hide the open conversation now and report it with Undo. The server is told on
    /// the account's next cycle, and only copies in the current folder move.
    fn move_open(&self, destination: String, label: String, window: &Window, cx: &Context<Self>) {
        let (Some(handle), Some(row)) = (self.handle(), self.selected.clone()) else {
            return;
        };
        let Some(context) = self.declaration.active().and_then(|config| {
            let account = config.accounts.iter().find(|item| item.id == row.account)?;
            mailstronaut_core::store::flags::context(account).ok()
        }) else {
            return;
        };
        let source = match &self.scope {
            Scope::Folder { folder, .. } => Some(*folder),
            _ => None,
        };
        let revision = self.declaration.revision();
        let (account, id) = (row.account.clone(), row.id);
        // The hold is stored with the move: IDLE can start a sync pass at any moment, and
        // opening a message is usually what makes it happen.
        let not_before = chrono::Utc::now()
            .timestamp()
            .saturating_add(i64::try_from(UNDO_GRACE.as_secs()).unwrap_or(i64::MAX));
        let job = self.runtime.spawn(async move {
            handle
                .queue_move(account, id, source, destination, context, not_before)
                .await
        });
        let undo = (row.account, row.id);
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() {
                    return;
                }
                match result {
                    Ok(Ok(_)) => this.after_move(undo, &label, window, cx),
                    Ok(Err(error)) => this.reader_notice = error.to_string(),
                    Err(_) => this.reader_notice = "Unable to move this message.".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn after_move(
        &mut self,
        undo: (String, i64),
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.retire_open(window, cx);
        self.refresh_rows(window, cx);
        // Waking the account at once would claim the move before Undo could be pressed.
        let grace = cx.background_executor().timer(UNDO_GRACE);
        cx.spawn(async move |this, cx| {
            grace.await;
            _ = this.update(cx, |this, _| {
                if let Some(engine) = &this.engine {
                    engine.wake_all();
                }
            });
        })
        .detach();
        let workspace = cx.entity().downgrade();
        window.push_notification(
            Notification::info(format!("Moved to {label}"))
                .delivery(NotificationDelivery::InApp)
                .action(move |_, _, cx| {
                    let (workspace, undo) = (workspace.clone(), undo.clone());
                    Button::new("undo-move").label("Undo").on_click(cx.listener(
                        move |notification, _, window, cx| {
                            _ = workspace.update(cx, |this, cx| {
                                this.undo_move(undo.clone(), window, cx);
                            });
                            notification.dismiss(window, cx);
                        },
                    ))
                })
                // `action` turns autohide off; Undo is only useful during the grace period.
                .autohide(true),
            cx,
        );
    }

    fn undo_move(&self, (account, id): (String, i64), window: &Window, cx: &Context<Self>) {
        let Some(handle) = self.handle() else {
            return;
        };
        let job = self
            .runtime
            .spawn(async move { handle.undo_move(account, id).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if matches!(result, Ok(Ok(()))) {
                    this.refresh_rows(window, cx);
                } else {
                    this.status = "Too late to undo: the server already has this change.".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_announcer(&mut self, window: &Window, cx: &Context<Self>) {
        if self.announcer.is_some() {
            return;
        }
        let announcer = crate::mail_notify::Announcer::start(&self.runtime);
        let opened = announcer.opened.clone();
        announcer.set_window_active(window.is_window_active());
        self.announcer = Some(announcer);
        cx.spawn_in(window, async move |this, cx| {
            while opened.recv().await.is_ok() {
                let applied = this.update_in(cx, |this, window, cx| {
                    window.activate_window();
                    this.show_inbox(window, cx);
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// The first look at an account only records where it stands, so that mail already
    /// there at startup is not announced as if it had just arrived.
    fn look_for_new_mail(&self, account: String, window: &Window, cx: &Context<Self>) {
        let Some(store) = self.handle() else {
            return;
        };
        let after = self.announced_through.get(&account).copied();
        let since = chrono::Utc::now().timestamp() - NEW_MAIL_WITHIN_SECONDS;
        let asked = account.clone();
        let job = self
            .runtime
            .spawn(async move { store.new_inbox_mail(asked, after, since, 100).await });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok((newest, mail))) = job.await else {
                return;
            };
            _ = this.update_in(cx, |this, _, _| {
                this.announced_through.insert(account.clone(), newest);
                if this
                    .announcer
                    .as_ref()
                    .is_some_and(|item| !item.window_active())
                {
                    this.announce(&account, &mail);
                }
            });
        })
        .detach();
    }

    fn announce(&mut self, account: &str, mail: &[mailstronaut_core::store::analysis::NewMail]) {
        let (Some(config), Some(announcer)) = (self.declaration.active(), &self.announcer) else {
            return;
        };
        let Some(item) = config.accounts.iter().find(|item| item.id == account) else {
            return;
        };
        if mail.is_empty() || !config.notifications.enabled {
            return;
        }
        let count = self.announced_count.entry(account.to_owned()).or_default();
        *count = count.saturating_add(u32::try_from(mail.len()).unwrap_or(u32::MAX));
        let latest = config
            .notifications
            .show_preview
            .then(|| mail.first())
            .flatten();
        announcer.show(account, &item.display_name, *count, latest);
    }

    /// Coming back to the window answers whatever was announced.
    fn withdraw_announcements(&mut self) {
        let Some(announcer) = &self.announcer else {
            return;
        };
        for (account, _) in self.announced_count.drain() {
            announcer.withdraw(&account);
        }
    }

    /// A move that will be tried again keeps its warning until it works. One this server
    /// can never do is reported once and given up on, so it is shown until dismissed.
    fn report_move_failure(
        &mut self,
        account: &str,
        failure: Option<mailstronaut_mail::imap::Error>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use mailstronaut_mail::imap::Error;
        match failure {
            Some(Error::MoveUnavailable) => {
                self.sync_errors.remove(account);
                window.push_notification(
                    Notification::warning(
                        "This mail server cannot safely move a single message, so it stayed where it was.",
                    )
                    .delivery(NotificationDelivery::InApp)
                    .autohide(false),
                    cx,
                );
            }
            Some(error) => {
                self.sync_errors.insert(
                    account.to_owned(),
                    format!("A message could not be moved, and will be tried again: {error}."),
                );
            }
            None => {
                self.sync_errors.remove(account);
            }
        }
    }

    /// Restart per-account polling for the active configuration. Explicit sync in
    /// Settings still works when `sync.automatic` is off.
    fn restart_sync(&mut self, config: &Arc<Config>, window: &Window, cx: &Context<Self>) {
        self.start_announcer(window, cx);
        self.start_priority(config, window, cx);
        self.engine = None;
        self.sync_errors.clear();
        self.syncing.clear();
        let Some(handle) = self.handle().filter(|_| config.sync.automatic) else {
            return;
        };
        let (engine, mut updates) = crate::mail_sync::Engine::start(
            &self.runtime,
            &self.account_access,
            &handle,
            config,
            self.lifetime.child_token(),
        );
        self.engine = Some(engine);
        let revision = self.declaration.revision();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(update) = updates.recv().await {
                let applied = this.update_in(cx, |this, window, cx| {
                    if revision == this.declaration.revision() {
                        this.sync_update(update, window, cx);
                    }
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn sync_update(
        &mut self,
        update: crate::mail_sync::Update,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match update {
            crate::mail_sync::Update::Started(account) => {
                self.syncing.insert(account);
            }
            crate::mail_sync::Update::Finished(account, result) => {
                self.syncing.remove(&account);
                if let Some(search) = &self.fuzzy {
                    search.wake();
                }
                if let Some(priority) = &self.priority {
                    priority.wake();
                }
                match result {
                    Ok(cycle) => {
                        self.report_move_failure(&account, cycle.move_failure, window, cx);
                        if cycle.downloaded > 0 || !self.announced_through.contains_key(&account) {
                            self.look_for_new_mail(account.clone(), window, cx);
                        }
                        self.last_sync = chrono::Local::now().format("%H:%M").to_string();
                        if cycle.downloaded > 0
                            || cycle.refreshed > 0
                            || cycle.actions > 0
                            || cycle.move_failure.is_some()
                        {
                            self.refresh_rows(window, cx);
                        }
                    }
                    Err(error) => {
                        self.sync_errors.insert(account, error);
                        // Whatever failed, the pass may have put a hidden message back.
                        self.refresh_rows(window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    fn sync_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(window, cx);
        if let Some(engine) = &self.engine {
            engine.wake_all();
        }
    }

    fn sync_status(&self) -> String {
        if !self.syncing.is_empty() {
            return format!("{} · syncing…", self.status);
        }
        if self.last_sync.is_empty() {
            return self.status.clone();
        }
        format!("{} · updated {}", self.status, self.last_sync)
    }

    fn sync_alert(&self, cx: &Context<Self>) -> impl IntoElement {
        div().when_some(self.sync_errors.iter().next(), |view, (account, error)| {
            let more = self.sync_errors.len().saturating_sub(1);
            let name = self
                .declaration
                .active()
                .and_then(|config| config.accounts.iter().find(|item| &item.id == account))
                .map_or_else(|| account.clone(), |item| item.display_name.clone());
            view.p_2().child(
                Alert::warning(
                    "sync-error",
                    if more > 0 {
                        format!("{name}: {error}. {more} more accounts also need attention.")
                    } else {
                        format!("{name}: {error}. Cached mail is still available.")
                    },
                )
                .on_close(cx.listener(|this, _, _, cx| {
                    // Dismissal hides this report; each account keeps its own retry state.
                    this.sync_errors.clear();
                    cx.notify();
                })),
            )
        })
    }
}

impl Workspace {
    fn bind_keys(cx: &mut Context<Self>) {
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
            KeyBinding::new("ctrl-shift-e", ExportConfiguration, Some("Mailstronaut")),
            KeyBinding::new("ctrl-n", ComposeMessage, Some("Mailstronaut")),
            KeyBinding::new("ctrl-2", ShowDrafts, Some("Mailstronaut")),
            KeyBinding::new("ctrl-3", ShowReminders, Some("Mailstronaut")),
            KeyBinding::new("alt-r", Reply, Some("Mailstronaut")),
            KeyBinding::new("alt-a", ReplyAll, Some("Mailstronaut")),
            KeyBinding::new("alt-f", Forward, Some("Mailstronaut")),
            KeyBinding::new("alt-u", ToggleRead, Some("Mailstronaut")),
            KeyBinding::new("alt-s", ToggleStar, Some("Mailstronaut")),
            KeyBinding::new("alt-e", Archive, Some("Mailstronaut")),
            KeyBinding::new("alt-d", Trash, Some("Mailstronaut")),
        ]);
    }

    fn show_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_link();
        self.view = View::Accounts;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.composer.update(cx, |composer, cx| composer.flush(cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn show_compose(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.cancel_link();
        self.view = View::Compose;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.composer
            .update(cx, |composer, cx| composer.refresh_lists(window, cx));
        cx.notify();
    }

    fn show_reminders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_link();
        self.view = View::Reminders;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.composer.update(cx, |composer, cx| composer.flush(cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A reminder names one exact original; opening it is an explicit open like any other.
    fn open_reminder(
        &mut self,
        record: &mailstronaut_core::store::reminders::Reminder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Scope::Inbox, window, cx);
        self.open(
            Row {
                account: record.account.clone(),
                id: record.message,
                conversation: None,
                sender: record.account.clone(),
                subject: record.title.clone(),
                preview: String::new(),
                unread: 0,
                messages: 1,
                received_at: record.due_at,
                seen: false,
                starred: false,
                due: None,
                expires_at: None,
            },
            window,
            cx,
        );
    }

    fn remind_open(&self, when: &When, window: &Window, cx: &mut Context<Self>) {
        let Some((account, id)) = self
            .selected
            .as_ref()
            .map(|row| (row.account.clone(), row.id))
        else {
            return;
        };
        self.reminders.update(cx, |reminders, cx| {
            reminders.schedule(account, id, when, window, cx);
        });
    }

    fn compose(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.accounts.is_empty() || !self.composer.read(cx).has_store() {
            self.status = "Add an account before composing a message.".into();
            cx.notify();
            return;
        }
        self.show_compose(window, cx);
        self.composer
            .update(cx, |composer, cx| composer.compose_new(window, cx));
    }

    fn respond(&mut self, kind: Kind, window: &Window, cx: &mut Context<Self>) {
        let (Some(row), Some(message), Some(config)) = (
            self.selected.as_ref(),
            self.opened_message.as_ref(),
            self.declaration.active(),
        ) else {
            return;
        };
        let Some(account) = config.accounts.iter().find(|item| item.id == row.account) else {
            return;
        };
        match mailstronaut_core::reply::prepare(account, &config.accounts, message, kind) {
            Ok(draft) => {
                self.show_compose(window, cx);
                self.composer
                    .update(cx, |composer, cx| composer.open_prepared(draft, window, cx));
            }
            Err(error) => {
                self.reader_notice = error.to_string();
                cx.notify();
            }
        }
    }

    fn show_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Scope::Inbox, window, cx);
    }

    fn navigate(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.pages = crate::mailbox::Pages::default();
            self.clear_page(window, cx);
        }
        self.view = View::Inbox;
        self.composer.update(cx, |composer, cx| composer.flush(cx));
        self.read_gate.visibility(window.is_window_active());
        window.focus(&self.focus, cx);
        self.refresh(window, cx);
        if self.reader_mode == ReaderMode::Html {
            self.present_html(cx);
        }
        cx.notify();
    }

    fn clear_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retire_open(window, cx);
        self.list.update(cx, |list, cx| {
            crate::mailbox::replace(list, Vec::new(), window, cx);
            list.scroll_to_item(IndexPath::new(0), gpui::ScrollStrategy::Top, window, cx);
        });
    }

    fn turn_page(&mut self, older: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.list.read(cx).delegate();
        if rows.loading {
            return;
        }
        if older {
            let Some(cursor) = rows
                .items
                .last()
                .filter(|_| self.pages.more)
                .map(Row::cursor)
            else {
                return;
            };
            self.pages.next(cursor);
        } else if self.pages.has_previous() {
            self.pages.previous();
        } else {
            return;
        }
        self.clear_page(window, cx);
        self.refresh_rows(window, cx);
    }

    fn scope_label(&self) -> String {
        match &self.scope {
            Scope::Inbox => "Unified inbox".into(),
            Scope::Account(id) => self
                .declaration
                .active()
                .and_then(|config| config.accounts.iter().find(|account| &account.id == id))
                .map_or_else(|| id.clone(), |account| account.display_name.clone()),
            Scope::Folder { account, folder } => self
                .folders
                .iter()
                .find(|row| &row.account == account && &row.id == folder)
                .map_or_else(|| "Cached folder".into(), Folder::label),
        }
    }

    fn account_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let loading = self.loading_config;
        let reload_disabled = loading || self.demo || self.preferences.read(cx).busy();
        let workspace = cx.entity().downgrade();
        let export_workspace = workspace.clone();
        let export_disabled = self.declaration.active().is_none() || loading;
        let config = self.declaration.active();
        let ownership = self.preferences.read(cx).ownership_label();
        let source_details = self.preferences.clone();
        let general = SettingPage::new("Configuration")
            .description("Preferences use your native Nix source. Managed and user-authored expressions stay read-only here.")
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("Native Nix")
                    .description(ownership)
                    .item(SettingItem::render(move |_, _, cx| crate::preferences::Preferences::source_details(&source_details, cx))
                        .keywords(["source", "path", "Nix", "configuration", "Home Manager"]))
                    .item(SettingItem::render(move |options, _, _| {
                        let workspace = workspace.clone();
                        Button::new("reload-config")
                            .label(if loading { "Loading…" } else { "Reload configuration" })
                            .disabled(options.is_disabled() || reload_disabled)
                            .on_click(move |_, window, cx| {
                                _ = workspace.update(cx, |this, cx| this.load_configuration(window, cx));
                            })
                    }).keywords(["reload", "configuration", "Nix", "Home Manager"]))
                    .item(SettingItem::render(move |options, _, _| {
                        let workspace = export_workspace.clone();
                        Button::new("export-nix").label("Export to Nix").tooltip("Ctrl+Shift+E").disabled(options.is_disabled() || export_disabled)
                            .on_click(move |_, window, cx| {
                                _ = workspace.update(cx, |this, cx| this.export_configuration(window, cx));
                            })
                    }).keywords(["export", "Nix", "Home Manager", "snapshot"])),
            );
        let general = if config.is_some() {
            general
                .group(crate::preferences::Preferences::appearance(
                    &self.preferences,
                    cx,
                ))
                .group(crate::preferences::Preferences::sync(&self.preferences, cx))
                .group(crate::preferences::Preferences::notifications(
                    &self.preferences,
                    cx,
                ))
                .group(crate::preferences::Preferences::times(
                    &self.preferences,
                    cx,
                ))
        } else {
            general.group(crate::preferences::Preferences::setup(
                &self.preferences,
                cx,
            ))
        };
        let general = general.group(SettingGroup::new().title("Account status").item({
            let status = self.declaration.error().map_or_else(
                || {
                    if self.account_views.is_empty() {
                        "No accounts are active. Add an account here or in your Nix declaration."
                            .to_owned()
                    } else {
                        "Choose an account in Settings to sign in or sync mail.".to_owned()
                    }
                },
                |error| {
                    if config.is_some() {
                        format!("{error}. The last valid configuration remains active.")
                    } else {
                        format!("{error}. No accounts have been started.")
                    }
                },
            );
            SettingItem::render(move |_, _, _| div().text_sm().child(status.clone()))
                .keywords(["accounts", "status", "errors", "reload"])
        }));
        let editor = self.preferences.read(cx);
        let settings = Settings::new(("mail-settings", self.declaration.revision()))
            .default_selected_index(editor.page_after_reload())
            .sidebar_width(px(220.))
            .page(general)
            .pages(
                self.account_views
                    .iter()
                    .map(|view| crate::accounts::AccountPane::page(view, cx)),
            )
            .page(crate::onboarding::Onboarding::page(&self.onboarding));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(settings))
            .child(crate::preferences::Preferences::footer(
                &self.preferences,
                cx,
            ))
    }

    fn export_configuration(&self, window: &Window, cx: &Context<Self>) {
        let Some(config) = self.declaration.active().cloned() else {
            return;
        };
        let revision = self.declaration.revision();
        let job = self
            .runtime
            .spawn_blocking(move || mailstronaut_config::native::export(&config));
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() || this.view != View::Accounts {
                    return;
                }
                match result {
                    Ok(Ok(text)) => {
                        crate::config_export::Export::show(text, this.runtime.clone(), window, cx);
                    }
                    _ => window.push_notification(
                        Notification::error("Unable to export this configuration.")
                            .delivery(NotificationDelivery::InApp),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    /// Unread mail worth a badge: mail in Sent, Drafts, Trash or Junk asks for nothing.
    fn unread_badge(&self, folder: &Folder) -> usize {
        match folder.role {
            Some(FolderRole::Sent | FolderRole::Drafts | FolderRole::Trash | FolderRole::Junk) => 0,
            _ => self.unread.get(&folder.id).copied().unwrap_or_default(),
        }
    }

    fn mail_routes(&self, attention: usize, cx: &Context<Self>) -> SidebarMenu {
        let overdue = self.reminders.read(cx).overdue();
        let unread: usize = self
            .folders
            .iter()
            .filter(|folder| folder.inbox)
            .map(|folder| self.unread_badge(folder))
            .sum();
        SidebarMenu::new()
            .child(
                SidebarMenuItem::new("Unified inbox")
                    .icon(IconName::Inbox)
                    .active(self.view == View::Inbox && self.scope == Scope::Inbox)
                    .suffix(move |_, _| Badge::new().count(unread).child(div().w_6().h_3()))
                    .on_click(cx.listener(|this, _, window, cx| this.show_inbox(window, cx))),
            )
            .child(
                SidebarMenuItem::new("Drafts & outbox")
                    .icon(IconName::Folder)
                    .active(self.view == View::Compose)
                    .suffix(move |_, _| Badge::new().count(attention).child(div().w_3().h_3()))
                    .on_click(cx.listener(|this, _, window, cx| this.show_compose(window, cx))),
            )
            .child(
                SidebarMenuItem::new("Reminders")
                    .icon(IconName::Calendar)
                    .active(self.view == View::Reminders)
                    .suffix(move |_, _| Badge::new().count(overdue).child(div().w_3().h_3()))
                    .on_click(cx.listener(|this, _, window, cx| this.show_reminders(window, cx))),
            )
    }

    fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let attention = self.composer.read(cx).attention();
        Sidebar::new("mail-sidebar")
            .w_full()
            .collapsible(false)
            .header(SidebarHeader::new().child(div().font_semibold().child("Mailstronaut")))
            .child(SidebarGroup::new("Mail").child(self.mail_routes(attention, cx)))
            .child(
                SidebarGroup::new("Accounts").child(
                    SidebarMenu::new().children(
                        self.declaration
                            .active()
                            .into_iter()
                            .flat_map(|config| &config.accounts)
                            .map(|account| {
                                let scope = Scope::Account(account.id.clone());
                                SidebarMenuItem::new(account.display_name.clone())
                                    .icon(IconName::Inbox)
                                    .accessibility_label(format!(
                                        "All cached mail · {} · {}",
                                        account.display_name, account.id
                                    ))
                                    .active(self.view == View::Inbox && self.scope == scope)
                                    .default_open(true)
                                    .click_to_open(true)
                                    .click_to_toggle(false)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigate(scope.clone(), window, cx);
                                    }))
                                    .children(
                                        self.folders
                                            .iter()
                                            .filter(|folder| folder.account == account.id)
                                            .map(|folder| {
                                                let scope = Scope::Folder {
                                                    account: account.id.clone(),
                                                    folder: folder.id,
                                                };
                                                let wire = folder.name.clone();
                                                let unread = self.unread_badge(folder);
                                                SidebarMenuItem::new(folder.label())
                                                    .icon(if folder.inbox {
                                                        IconName::Inbox
                                                    } else {
                                                        IconName::Folder
                                                    })
                                                    .suffix(move |_, _| {
                                                        Badge::new()
                                                            .count(unread)
                                                            .child(div().w_6().h_3())
                                                    })
                                                    .accessibility_label(format!(
                                                        "Cached folder · {} · {} · {unread} unread",
                                                        account.display_name,
                                                        folder.label()
                                                    ))
                                                    .active(
                                                        self.view == View::Inbox
                                                            && self.scope == scope,
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.navigate(
                                                                scope.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                            // Standard folders already sync;
                                                            // this covers any other folder.
                                                            if let (
                                                                Some(engine),
                                                                Scope::Folder { account, .. },
                                                            ) = (&this.engine, &scope)
                                                            {
                                                                engine.include_folder(
                                                                    account,
                                                                    wire.clone(),
                                                                );
                                                            }
                                                        },
                                                    ))
                                            }),
                                    )
                            }),
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
                div().w(px(190.)).font_semibold().child(
                    Label::new(match self.view {
                        View::Accounts => "Settings".into(),
                        View::Compose => "Drafts & outbox".into(),
                        View::Reminders => "Reminders".into(),
                        View::Inbox => self.scope_label(),
                    })
                    .truncate(),
                ),
            )
            .child(div().flex_1().when(self.view == View::Inbox, |view| {
                view.child(Input::new(&self.search))
            }))
            .child(
                Button::new("compose")
                    .primary()
                    .label("Compose")
                    .tooltip("Ctrl+N")
                    .disabled(self.accounts.is_empty() || !self.composer.read(cx).has_store())
                    .on_click(cx.listener(|this, _, window, cx| this.compose(window, cx))),
            )
            .when(self.view == View::Inbox, |view| {
                view.child(
                    Button::new("refresh")
                        .label(if self.engine.is_some() {
                            "Sync"
                        } else {
                            "Refresh cache"
                        })
                        .tooltip("Ctrl+R")
                        .loading(!self.syncing.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.sync_now(window, cx))),
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

    fn message_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let loading = self.list.read(cx).delegate().loading;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.sync_alert(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(List::new(&self.list).size_full()),
            )
            .when(
                self.pages.more || self.pages.has_previous() || self.can_fetch_older(),
                |view| {
                    view.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .p_2()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(
                                Button::new("newer-conversations")
                                    .ghost()
                                    .label("Newer")
                                    .disabled(loading || !self.pages.has_previous())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.turn_page(false, window, cx);
                                    })),
                            )
                            .child(Label::new(format!("Page {}", self.pages.number())).text_xs())
                            .child(if !self.pages.more && self.can_fetch_older() {
                                Button::new("fetch-older-mail")
                                    .ghost()
                                    .label("Fetch older mail")
                                    .loading(self.older_fetch.is_some())
                                    .disabled(loading || self.older_fetch.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.fetch_older(window, cx);
                                    }))
                            } else {
                                Button::new("older-conversations")
                                    .ghost()
                                    .label("Older")
                                    .disabled(loading || !self.pages.more)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.turn_page(true, window, cx);
                                    }))
                            }),
                    )
                },
            )
    }

    /// Where older mail for the current view would come from, leaving out folders already
    /// fetched back to their beginning.
    fn older_targets(&self) -> Vec<(Account, String, bool)> {
        let Some(config) = self.declaration.active() else {
            return Vec::new();
        };
        let inbox_of = |account: &Account| {
            let folder = account
                .folders
                .inbox
                .clone()
                .unwrap_or_else(|| "INBOX".into());
            (account.clone(), folder, true)
        };
        let targets: Vec<_> = match &self.scope {
            Scope::Inbox => config.accounts.iter().map(inbox_of).collect(),
            Scope::Account(id) => config
                .accounts
                .iter()
                .filter(|account| &account.id == id)
                .map(inbox_of)
                .collect(),
            Scope::Folder { account, folder } => config
                .accounts
                .iter()
                .filter(|item| &item.id == account)
                .filter_map(|item| {
                    let found = self.folders.iter().find(|row| &row.id == folder)?;
                    Some((item.clone(), found.name.clone(), found.inbox))
                })
                .collect(),
        };
        targets
            .into_iter()
            .filter(|(account, folder, _)| {
                !matches!(
                    self.older_reached
                        .get(&(account.id.clone(), folder.clone())),
                    Some(None)
                )
            })
            .collect()
    }

    fn can_fetch_older(&self) -> bool {
        self.view == View::Inbox
            && self.query.trim().is_empty()
            && self.handle().is_some()
            && !self.older_targets().is_empty()
    }

    fn fetch_older(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(store) = self.handle() else {
            return;
        };
        if self.older_fetch.is_some() {
            return;
        }
        let cancel = self.lifetime.child_token();
        self.older_fetch = Some(cancel.clone());
        let targets = self.older_targets();
        let reached = self.older_reached.clone();
        let access = self.account_access.clone();
        let job = self.runtime.spawn(async move {
            let mut progress = Vec::new();
            let mut downloaded = 0;
            for (account, folder, inbox) in targets {
                let key = (account.id.clone(), folder.clone());
                let resume = reached.get(&key).copied().flatten();
                match access
                    .fetch_older(&store, &account, &folder, inbox, resume, &cancel)
                    .await
                {
                    Ok(older) => {
                        downloaded += older.downloaded;
                        progress.push((key, older.resume));
                    }
                    Err(error) => return (progress, downloaded, Some(error)),
                }
            }
            (progress, downloaded, None)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.older_fetch = None;
                let notice = match result {
                    Ok((progress, downloaded, failure)) => {
                        this.older_reached.extend(progress);
                        if let Some(search) = &this.fuzzy {
                            search.wake();
                        }
                        if let Some(priority) = &this.priority {
                            priority.wake();
                        }
                        failure.unwrap_or_else(|| match downloaded {
                            0 => "There is no older mail on the server.".to_owned(),
                            1 => "Fetched 1 older message.".to_owned(),
                            count => format!("Fetched {count} older messages."),
                        })
                    }
                    Err(_) => "Fetching older mail was interrupted.".to_owned(),
                };
                window.push_notification(
                    Notification::new()
                        .message(notice)
                        .delivery(NotificationDelivery::InApp),
                    cx,
                );
                this.refresh_rows(window, cx);
            });
        })
        .detach();
        cx.notify();
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
                view.child(self.message_actions(cx))
                    .child(self.thread_reader.clone())
                    .child(self.reader_tools(cx))
            })
            .when_some(self.opened_message.as_ref(), |view, message| {
                view.child(self.message_details(message, cx))
                    .child(Self::received_attachments(message, cx))
                    .child(self.remote_attachments(cx))
                    .child(self.analysis_notes(cx))
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
                self.selected.is_some()
                    && self.reader_mode == ReaderMode::Text
                    && self.body.is_empty(),
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
                self.selected.is_some()
                    && self.reader_mode == ReaderMode::Html
                    && self.body.is_empty(),
                |view| view.child(div().flex_1().min_h_0().child(self.html_reader.clone())),
            )
            .when(!self.body.is_empty(), |view| view.child(self.body.clone()))
    }

    fn received_attachments(
        message: &mailstronaut_core::mime::Projection,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let listed = message.listed_attachments();
        div().when(!listed.is_empty(), |view| {
            view.mb_3()
                .child(AttachmentGroup::new("received-attachments").children(
                    listed.into_iter().map(|(index, part)| {
                        let name = part.name.clone().unwrap_or_else(|| "Attachment".into());
                        let kind = part.media_type.clone().unwrap_or_default();
                        Attachment::new()
                            .id(("received-attachment", index))
                            .tooltip("Save a copy")
                            .content(
                                AttachmentContent::new()
                                    .title(AttachmentTitle::new(name))
                                    .description(AttachmentDescription::new(format!(
                                        "{} · {kind}",
                                        crate::composer::size_label(part.decoded_size)
                                    ))),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_attachment(index, window, cx);
                            }))
                    }),
                ))
        })
    }

    /// Why this message was flagged, and each date found with the words it came from. Mail
    /// is untrusted, so the quote is shown as plain text and only ever as a quote.
    fn analysis_notes(&self, cx: &Context<Self>) -> impl IntoElement {
        let notes: Vec<String> = self
            .opened_analysis
            .as_ref()
            .map_or_else(Vec::new, |analysis| {
                crate::mail_priority::explanation(analysis)
                    .into_iter()
                    .chain(crate::mail_priority::deadline_notes(analysis))
                    .collect()
            });
        let muted = cx.theme().muted_foreground;
        div().when(!notes.is_empty(), |view| {
            view.mb_3().children(
                notes
                    .into_iter()
                    .map(|note| Label::new(note).text_sm().text_color(muted)),
            )
        })
    }

    /// Attachments of a message too large to keep are listed like any others, and
    /// downloaded from the server only when saved.
    fn remote_attachments(&self, cx: &Context<Self>) -> impl IntoElement {
        let omitted = self.opened_omitted.as_ref();
        div().when_some(omitted, |view, omitted| {
            let muted = cx.theme().muted_foreground;
            view.mb_3()
                .child(
                    Label::new(format!(
                        "This message is {}, too large to keep on this device. Its attachments stay on the server until you save them.",
                        crate::composer::size_label(omitted.size as usize)
                    ))
                    .text_sm()
                    .text_color(muted)
                    .mb_2(),
                )
                .when(!omitted.attachments.is_empty(), |view| {
                    view.child(AttachmentGroup::new("remote-attachments").children(
                        omitted.attachments.iter().enumerate().map(|(index, part)| {
                            let name = part.name.clone().unwrap_or_else(|| "Attachment".into());
                            let size = usize::try_from(part.size).unwrap_or(usize::MAX);
                            Attachment::new()
                                .id(("remote-attachment", index))
                                .tooltip("Download and save a copy")
                                .content(
                                    AttachmentContent::new()
                                        .title(AttachmentTitle::new(name))
                                        .description(AttachmentDescription::new(format!(
                                            "On the server · about {}",
                                            crate::composer::size_label(size)
                                        ))),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.save_remote_attachment(index, window, cx);
                                }))
                        }),
                    ))
                })
        })
    }

    /// The chooser comes first, so that nothing is downloaded for a save that is cancelled.
    fn save_remote_attachment(&self, index: usize, window: &Window, cx: &Context<Self>) {
        let (Some(row), Some(part), Some(store)) = (
            self.selected.clone(),
            self.opened_omitted
                .as_ref()
                .and_then(|omitted| omitted.attachments.get(index))
                .cloned(),
            self.handle(),
        ) else {
            return;
        };
        let Some(account) = self
            .declaration
            .active()
            .and_then(|config| config.accounts.iter().find(|item| item.id == row.account))
            .cloned()
        else {
            return;
        };
        let suggested = part
            .name
            .as_deref()
            .filter(|name| mailstronaut_mail::compose::valid_attachment_name(name))
            .unwrap_or("attachment")
            .to_owned();
        let directory = directories::UserDirs::new()
            .and_then(|dirs| {
                dirs.download_dir()
                    .map(std::path::Path::to_path_buf)
                    .or_else(|| Some(dirs.home_dir().to_path_buf()))
            })
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        let (runtime, access, cancel) = (
            self.runtime.clone(),
            self.account_access.clone(),
            self.lifetime.child_token(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let notice = match chosen.await {
                Ok(Ok(Some(path))) => {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::new()
                                .id::<AttachmentDownload>()
                                .message("Downloading the attachment from the server…")
                                .autohide(false)
                                .delivery(NotificationDelivery::InApp),
                            cx,
                        );
                    });
                    let saved = runtime
                        .spawn(async move {
                            let bytes = access
                                .fetch_attachment(&store, &account, row.id, &part.section, &cancel)
                                .await?;
                            tokio::fs::write(path, bytes).await.map_err(|_| {
                                "The attachment could not be written there.".to_owned()
                            })
                        })
                        .await;
                    Some(match saved {
                        Ok(Ok(())) => "Attachment saved.".to_owned(),
                        Ok(Err(error)) => error,
                        Err(_) => "Saving the attachment was interrupted.".to_owned(),
                    })
                }
                Ok(Ok(None)) => None,
                _ => Some("The file chooser is unavailable. Nothing was saved.".to_owned()),
            };
            if let Some(notice) = notice {
                _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        Notification::new()
                            .id::<AttachmentDownload>()
                            .message(notice)
                            .delivery(NotificationDelivery::InApp),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    /// Saving is explicit and goes through the portal's chooser. Attachments are never
    /// opened automatically, and a sender-supplied name is only a suggestion.
    fn save_attachment(&self, index: usize, window: &Window, cx: &Context<Self>) {
        let Some(message) = self.opened_message.clone() else {
            return;
        };
        let suggested = message
            .attachments
            .get(index)
            .and_then(|part| part.name.as_deref())
            .filter(|name| mailstronaut_mail::compose::valid_attachment_name(name))
            .unwrap_or("attachment")
            .to_owned();
        let directory = directories::UserDirs::new()
            .and_then(|dirs| {
                dirs.download_dir()
                    .map(std::path::Path::to_path_buf)
                    .or_else(|| Some(dirs.home_dir().to_path_buf()))
            })
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let notice = match chosen.await {
                Ok(Ok(Some(path))) => {
                    let saved = runtime
                        .spawn_blocking(move || {
                            let bytes = message
                                .attachment_bytes(index)
                                .map_err(|_| "This attachment could not be decoded.")?;
                            std::fs::write(path, bytes)
                                .map_err(|_| "The attachment could not be written there.")
                        })
                        .await;
                    match saved {
                        Ok(Ok(())) => Some("Attachment saved.".to_owned()),
                        Ok(Err(error)) => Some(error.to_owned()),
                        Err(_) => Some("Saving the attachment was interrupted.".to_owned()),
                    }
                }
                Ok(Ok(None)) => None,
                _ => Some("The file chooser is unavailable. Nothing was saved.".to_owned()),
            };
            if let Some(notice) = notice {
                _ = this.update(cx, |this, cx| {
                    this.reader_notice = notice;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn message_details(
        &self,
        message: &mailstronaut_core::mime::Projection,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let headers = &message.headers;
        let zone = &self.list.read(cx).delegate().time_zone;
        gpui::component::collapsible::Collapsible::new()
            .open(self.headers_open)
            .content(
                div()
                    .id("message-details-scroll")
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .mb_3()
                    .child(
                        DescriptionList::new()
                            .columns(1)
                            .bordered(false)
                            .item("From", crate::mailbox::addresses(&headers.from), 1)
                            .item("To", crate::mailbox::addresses(&headers.to), 1)
                            .item("Cc", crate::mailbox::addresses(&headers.cc), 1)
                            .item(
                                "Sent",
                                headers.date.map_or_else(
                                    || "Date unavailable".into(),
                                    |date| crate::mailbox::date(date, zone),
                                ),
                                1,
                            )
                            .item(
                                "Received",
                                self.selected.as_ref().map_or_else(
                                    || "Date unavailable".into(),
                                    |row| crate::mailbox::date(row.received_at, zone),
                                ),
                                1,
                            ),
                    ),
            )
    }

    fn message_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let unavailable = self.opened_message.is_none() || self.demo;
        let (seen, starred) = self
            .selected
            .as_ref()
            .map_or((false, false), |row| (row.seen, row.starred));
        Toolbar::new("message-actions")
            .mb_3()
            .content(
                ToolbarGroup::new("message-respond")
                    .label("Respond")
                    .child(
                        Button::new("reply")
                            .outline()
                            .label("Reply")
                            .tooltip("Alt+R")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Kind::Reply, window, cx);
                            })),
                    )
                    .child(
                        Button::new("reply-all")
                            .ghost()
                            .label("Reply all")
                            .tooltip("Alt+A")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Kind::ReplyAll, window, cx);
                            })),
                    )
                    .child(
                        Button::new("forward")
                            .ghost()
                            .label("Forward")
                            .tooltip("Alt+F · plain text, without attachments")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Kind::ForwardText, window, cx);
                            })),
                    ),
            )
            .content(self.move_controls(cx))
            .content(self.remind_control(cx))
            .content(
                ToolbarGroup::new("message-state")
                    .label("Message state")
                    .child(
                        Button::new("toggle-read")
                            .ghost()
                            .label(if seen { "Mark unread" } else { "Mark read" })
                            .tooltip("Alt+U")
                            .disabled(self.demo)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_flag(FlagChange::Seen, window, cx);
                            })),
                    )
                    .child(
                        Button::new("toggle-star")
                            .ghost()
                            .label(if starred { "Unstar" } else { "Star" })
                            .tooltip("Alt+S")
                            .disabled(self.demo)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_flag(FlagChange::Starred, window, cx);
                            })),
                    ),
            )
    }

    fn remind_control(&self, cx: &Context<Self>) -> impl IntoElement {
        let workspace = cx.entity().downgrade();
        let scheduled = self
            .selected
            .as_ref()
            .is_some_and(|row| self.reminders.read(cx).is_scheduled(&row.account, row.id));
        Button::new("remind-me")
            .ghost()
            .label(if scheduled {
                "Reminder set"
            } else {
                "Remind me"
            })
            .disabled(self.demo)
            .dropdown_menu(move |menu, _, _| {
                let preset = |menu: gpui::component::menu::PopupMenu,
                              label: &'static str,
                              when: When| {
                    let workspace = workspace.clone();
                    menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        _ = workspace.update(cx, |this, cx| this.remind_open(&when, window, cx));
                    }))
                };
                let menu = preset(menu, "In one hour", When::Hour);
                let menu = preset(menu, "This evening", When::Evening);
                let menu = preset(menu, "Tomorrow morning", When::Tomorrow);
                let workspace = workspace.clone();
                menu.item(
                    PopupMenuItem::new("Pick a time…").on_click(move |_, window, cx| {
                        _ = workspace.update(cx, |this, cx| {
                            if let Some(row) = &this.selected {
                                let (account, id) = (row.account.clone(), row.id);
                                this.reminders.update(cx, |reminders, cx| {
                                    reminders.ask_custom(account, id, window, cx);
                                });
                            }
                        });
                    }),
                )
            })
    }

    fn move_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let targets: Vec<(String, String)> = self
            .selected
            .as_ref()
            .into_iter()
            .flat_map(|row| self.account_folders(&row.account))
            .filter(|folder| !folder.inbox)
            .map(|folder| (folder.name.clone(), folder.label()))
            .collect();
        let workspace = cx.entity().downgrade();
        ToolbarGroup::new("message-move")
            .label("Move")
            .child(
                Button::new("archive")
                    .ghost()
                    .label("Archive")
                    .tooltip("Alt+E · the whole conversation")
                    .disabled(self.demo)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.move_to_role(FolderRole::Archive, window, cx);
                    })),
            )
            .child(
                Button::new("trash")
                    .ghost()
                    .label("Trash")
                    .tooltip("Alt+D · the whole conversation")
                    .disabled(self.demo)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.move_to_role(FolderRole::Trash, window, cx);
                    })),
            )
            .content(
                Button::new("move-to")
                    .ghost()
                    .label("Move to")
                    .disabled(self.demo || targets.is_empty())
                    .dropdown_menu(move |mut menu, _, _| {
                        for (name, label) in &targets {
                            let (workspace, name, title) =
                                (workspace.clone(), name.clone(), label.clone());
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                move |_, window, cx| {
                                    _ = workspace.update(cx, |this, cx| {
                                        this.move_open(name.clone(), title.clone(), window, cx);
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
    }

    fn reader_tools(&self, cx: &Context<Self>) -> impl IntoElement {
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
            .content(self.link_menu(cx))
            .content(
                Button::new("message-details")
                    .ghost()
                    .label("Details")
                    .selected(self.headers_open)
                    .disabled(self.opened_message.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.headers_open = !this.headers_open;
                        cx.notify();
                    })),
            )
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

impl Workspace {
    fn with_actions(root: gpui::Div, cx: &Context<Self>) -> gpui::Div {
        root.on_action(cx.listener(|this, _: &ComposeMessage, window, cx| {
            if this.composer.read(cx).has_store() {
                this.compose(window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ShowDrafts, window, cx| {
            this.show_compose(window, cx);
        }))
        .on_action(cx.listener(|this, _: &ShowReminders, window, cx| {
            this.show_reminders(window, cx);
        }))
        .on_action(cx.listener(|this, _: &Reply, window, cx| {
            if this.view == View::Inbox {
                this.respond(Kind::Reply, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ReplyAll, window, cx| {
            if this.view == View::Inbox {
                this.respond(Kind::ReplyAll, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Forward, window, cx| {
            if this.view == View::Inbox {
                this.respond(Kind::ForwardText, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleRead, window, cx| {
            if this.view == View::Inbox {
                this.toggle_flag(FlagChange::Seen, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleStar, window, cx| {
            if this.view == View::Inbox {
                this.toggle_flag(FlagChange::Starred, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Archive, window, cx| {
            if this.view == View::Inbox {
                this.move_to_role(FolderRole::Archive, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Trash, window, cx| {
            if this.view == View::Inbox {
                this.move_to_role(FolderRole::Trash, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
            if this.view != View::Inbox {
                this.show_inbox(window, cx);
            }
            this.search.update(cx, |input, cx| input.focus(window, cx));
        }))
        .on_action(cx.listener(|this, _: &RefreshCache, window, cx| this.sync_now(window, cx)))
        .on_action(cx.listener(|this, _: &ToggleAppearance, window, cx| {
            this.toggle_appearance(window, cx);
        }))
        .on_action(cx.listener(|this, _: &FocusMessages, window, cx| {
            if this.view != View::Inbox {
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
        .on_action(cx.listener(|this, _: &ShowAccounts, window, cx| this.show_accounts(window, cx)))
        .on_action(cx.listener(|this, _: &ShowInbox, window, cx| this.show_inbox(window, cx)))
        .on_action(cx.listener(|this, _: &ReloadConfiguration, window, cx| {
            this.load_configuration(window, cx);
        }))
        .on_action(cx.listener(|this, _: &ExportConfiguration, window, cx| {
            if this.view == View::Accounts {
                this.export_configuration(window, cx);
            }
        }))
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        Self::with_actions(div(), cx)
            .size_full()
            .key_context("Mailstronaut")
            .track_focus(&self.focus)
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
                                    })
                                    .when(self.view == View::Compose, |view| {
                                        view.child(self.composer.clone())
                                    })
                                    .when(self.view == View::Reminders, |view| {
                                        view.child(self.reminders.clone())
                                    }),
                            ),
                        ),
                ),
            )
            // The composer reports its own draft state; mailbox counts would mislead there.
            .when(self.view != View::Compose, |view| {
                view.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_xs()
                        .text_color(muted)
                        .border_t_1()
                        .border_color(border)
                        .child(self.sync_status()),
                )
            })
    }
}
