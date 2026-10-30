//! Native offline inbox and explicit, bounded foreground synchronization.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use cosmic::{
    Action, Element,
    app::Task,
    iced::{
        Length,
        widget::{column, row},
    },
    widget,
};
use futures_util::{StreamExt as _, stream};
use mailstronaut_config::{Account, Config};
use mailstronaut_core::{
    actions,
    mime::Projection,
    store::browse::Scope,
    store::conversations::Summary as Conversation,
    store::flags::{self, Counts},
    store::mailbox::{Cursor, Folder, Summary},
    sync, threads,
    worker::{self, Worker},
};
use mailstronaut_mail::{
    auth::Session,
    imap::{self, Connection, FlagChange},
};
use tokio_util::sync::CancellationToken;

use crate::private::Private as Payload;
mod read;
#[derive(Clone, Debug)]
pub enum Message {
    Started(Payload<Result<Arc<Worker>, worker::Error>>),
    Refresh,
    ReadVisible(u64, u64, u64),
    Scope(u64, Scope),
    SearchChanged(Payload<String>),
    Search,
    Next,
    Rows(u64, u64, Payload<Result<Listing, worker::Error>>),
    Open(String, i64),
    Body(u64, u64, Payload<Result<Loaded, worker::Error>>),
    Members(u64, u64, Payload<Result<Vec<Summary>, worker::Error>>),
    Grouped(u64, Payload<Vec<String>>),
    SetFlag(u64, String, i64, FlagChange, bool),
    UndoFlags(u64, String, i64),
    FlagQueued(u64, Result<(), worker::Error>),
    ApplyFlags,
    Applied(u64, Payload<Vec<AccountResult>>),
    Sync(bool),
    Cancel,
    Synced(u64, Payload<Vec<AccountResult>>),
}
#[derive(Clone)]
pub struct Listing {
    rows: Vec<Conversation>,
    flags: Counts,
    folders: Vec<Folder>,
}

#[derive(Clone)]
pub struct AccountResult {
    account: String,
    next: Option<sync::Position>,
    folder: String,
    status: String,
}

#[derive(Clone)]
pub struct Loaded {
    body: Arc<Body>,
    members: Vec<Summary>,
}

pub struct Body {
    projection: Projection,
    text: String,
    readable: bool,
}

#[derive(Default)]
pub struct Inbox {
    read_gate: read::Gate,
    owner: Option<Arc<Worker>>,
    config: Option<Arc<Config>>,
    generation: u64,
    listing: u64,
    reading: u64,
    rows: Vec<Conversation>,
    scope: Scope,
    navigation: Vec<Scope>,
    navigation_labels: Vec<String>,
    folders: Vec<Folder>,
    search: String,
    active_search: String,
    after: Option<Cursor>,
    selected: Option<(String, i64)>,
    body: Option<Arc<Body>>,
    members: Vec<Summary>,
    member_labels: Vec<String>,
    group_again: bool,
    loading: bool,
    jobs: u32,
    edits: u32,
    query_again: bool,
    flags: Counts,
    cancel: CancellationToken,
    older: BTreeMap<(String, String), sync::Position>,
    status: Vec<String>,
}
impl Inbox {
    pub const fn visible(&mut self, visible: bool) {
        self.read_gate.visibility(visible);
    }
    fn read_eligible(&self) -> bool {
        self.edits == 0
            && self.body.as_ref().is_some_and(|body| body.readable)
            && self.selected.as_ref().is_some_and(|(account, id)| {
                self.members.iter().any(|message| {
                    message.account == *account
                        && message.id == *id
                        && message.inbox
                        && !message.seen
                })
            })
    }
    pub fn read_task(&mut self) -> Task<super::Message> {
        let eligible = self.read_eligible();
        let Some((generation, reading, visibility)) =
            self.read_gate.arm(self.generation, self.reading, eligible)
        else {
            return Task::none();
        };
        Task::perform(
            async { tokio::time::sleep(core::time::Duration::from_millis(750)).await },
            move |()| event(Message::ReadVisible(generation, reading, visibility)),
        )
    }
    pub fn store(&self) -> Option<worker::Handle> {
        self.owner.as_ref().map(|owner| owner.handle())
    }
    pub const fn busy(&self) -> bool {
        self.jobs != 0 || self.edits != 0
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub async fn shutdown(owner: Option<Arc<Worker>>) -> Result<(), worker::Error> {
        match owner {
            Some(owner) => {
                Arc::try_unwrap(owner)
                    .map_err(|_| worker::Error::Closed)?
                    .shutdown()
                    .await
            }
            None => Ok(()),
        }
    }
    pub const fn take_owner(&mut self) -> Option<Arc<Worker>> {
        self.owner.take()
    }
    pub fn new() -> (Self, Task<super::Message>) {
        let task = Task::perform(
            async {
                let root = directories::BaseDirs::new()
                    .ok_or(worker::Error::Storage)?
                    .data_local_dir()
                    .join("mailstronaut");
                Worker::open(root).await.map(Arc::new)
            },
            |result| event(Message::Started(Payload(result))),
        );
        (Self::default(), task)
    }
    pub fn configure(&mut self, config: Option<Arc<Config>>) -> Task<super::Message> {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.generation = self.generation.saturating_add(1);
        self.config = config;
        self.rows.clear();
        self.scope = Scope::Inbox;
        self.navigation.clear();
        self.navigation_labels.clear();
        self.folders.clear();
        self.search.clear();
        self.active_search.clear();
        self.selected = None;
        self.body = None;
        self.members.clear();
        self.member_labels.clear();
        self.group_again = true;
        self.older.clear();
        self.status.clear();
        self.loading = false;
        self.query_again = false;
        self.flags = Counts::default();
        self.after = None;
        self.query(false)
    }
    pub const fn reading(&self) -> bool {
        self.selected.is_some()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep asynchronous generation/reading fences adjacent to their event arms; operations live in separate helpers."
    )]
    pub fn update(&mut self, message: Message, keyring: Option<Session>) -> Task<super::Message> {
        match message {
            Message::Started(Payload(result)) => match result {
                Ok(owner) => {
                    self.owner = Some(owner);
                    self.group_again = true;
                    return self.query(false);
                }
                Err(error) => self.status = vec![error.to_string()],
            },
            Message::Refresh => {
                self.group_again = true;
                return self.query(false);
            }
            Message::Scope(generation, scope)
                if generation == self.generation && self.navigation.contains(&scope) =>
            {
                self.scope = scope;
                self.reset_selection();
                return self.query(false);
            }
            Message::SearchChanged(Payload(value)) => {
                if value.len() <= 1024 {
                    self.search = value;
                }
            }
            Message::Search => {
                self.active_search = self.search.clone();
                self.reset_selection();
                return self.query(false);
            }
            Message::Next => return self.query(true),
            Message::Rows(generation, listing, Payload(result))
                if generation == self.generation && listing == self.listing =>
            {
                self.loading = false;
                match result {
                    Ok(listing) => {
                        self.rows = listing.rows;
                        self.flags = listing.flags;
                        self.folders = listing.folders;
                        self.navigation();
                    }
                    Err(error) => self.status = vec![error.to_string()],
                }
                if core::mem::take(&mut self.query_again) {
                    return Task::batch([self.query(false), self.load_members()]);
                }
                return self.load_members();
            }
            Message::Open(account, id) => return self.open(account, id),
            Message::Body(generation, reading, Payload(result))
                if generation == self.generation && reading == self.reading =>
            {
                match result {
                    Ok(loaded) => {
                        self.member_snapshot(loaded.members);
                        self.body = Some(loaded.body);
                        return crate::text_reader::open();
                    }
                    Err(error) => self.status = vec![error.to_string()],
                }
            }
            Message::Members(generation, reading, Payload(result))
                if generation == self.generation && reading == self.reading =>
            {
                match result {
                    Ok(members) => {
                        let changed_shape = (self.members.len() > 1) != (members.len() > 1);
                        self.member_snapshot(members);
                        if changed_shape && self.body.is_some() {
                            return crate::text_reader::focus();
                        }
                    }
                    Err(error) => self.status = vec![error.to_string()],
                }
            }
            Message::Grouped(generation, Payload(errors)) => {
                self.jobs = self.jobs.saturating_sub(1);
                if generation == self.generation && !errors.is_empty() {
                    self.status.extend(errors);
                }
                return self.query(false);
            }
            Message::ReadVisible(generation, reading, visibility) => {
                let eligible = self.read_eligible();
                if self.read_gate.consume(
                    (generation, reading, visibility),
                    self.generation,
                    self.reading,
                    eligible,
                ) {
                    return self.set_flag(FlagChange::Seen, true);
                }
            }
            Message::UndoFlags(generation, account, id)
                if !self.busy()
                    && generation == self.generation
                    && self.selected.as_ref() == Some(&(account.clone(), id)) =>
            {
                self.read_gate.suppress(self.generation, self.reading);
                return self.undo_flags();
            }
            Message::SetFlag(generation, account, id, flag, desired)
                if !self.busy()
                    && generation == self.generation
                    && self.selected.as_ref() == Some(&(account.clone(), id)) =>
            {
                if flag == FlagChange::Seen {
                    self.read_gate.suppress(self.generation, self.reading);
                }
                return self.set_flag(flag, desired);
            }
            Message::FlagQueued(generation, result) => {
                self.edits = self.edits.saturating_sub(1);
                if generation == self.generation {
                    self.status = vec![match result {
                        Ok(()) => "Flag changes saved locally. Apply changes when online.".into(),
                        Err(error) => {
                            format!("{error}. Sync first if the account configuration changed.")
                        }
                    }];
                }
                return self.query(false);
            }
            Message::ApplyFlags if !self.busy() => return self.apply_flags(keyring),
            Message::Applied(generation, Payload(results)) => {
                self.jobs = self.jobs.saturating_sub(1);
                if generation == self.generation {
                    self.status = results
                        .into_iter()
                        .map(|result| format!("{}: {}", result.account, result.status))
                        .collect();
                }
                return self.query(false);
            }
            Message::Sync(older) if !self.busy() => return self.synchronize(keyring, older),
            Message::Cancel => self.cancel.cancel(),
            Message::Synced(generation, Payload(results)) => {
                self.jobs = self.jobs.saturating_sub(1);
                if generation != self.generation {
                    return self.query(false);
                }
                self.status.clear();
                for result in results {
                    self.status.push(format!(
                        "{} · {}: {}",
                        result.account, result.folder, result.status
                    ));
                    let key = (result.account.clone(), result.folder);
                    self.older.remove(&key);
                    if let Some(next) = result.next {
                        self.older.insert(key, next);
                    }
                }
                return self.query(false);
            }
            _ => {}
        }
        Task::none()
    }

    fn member_snapshot(&mut self, members: Vec<Summary>) {
        self.member_labels = members
            .iter()
            .enumerate()
            .map(|(index, message)| {
                format!(
                    "{} · {}{} · {}",
                    index + 1,
                    if message.seen { "" } else { "● " },
                    message.sender.chars().take(60).collect::<String>(),
                    message.subject.chars().take(80).collect::<String>()
                )
            })
            .collect();
        self.members = members;
    }
    fn reset_selection(&mut self) {
        self.reading = self.reading.saturating_add(1);
        self.selected = None;
        self.body = None;
        self.members.clear();
        self.member_labels.clear();
        self.rows.clear();
        self.listing = self.listing.saturating_add(1);
        self.loading = false;
        self.query_again = false;
    }
    fn navigation(&mut self) {
        self.navigation = vec![Scope::Inbox];
        self.navigation_labels = vec!["Unified inbox".into()];
        if let Some(config) = &self.config {
            for account in &config.accounts {
                self.navigation.push(Scope::Account(account.id.clone()));
                self.navigation_labels
                    .push(format!("{} · all cached", account.id));
                for folder in self
                    .folders
                    .iter()
                    .filter(|folder| folder.account == account.id)
                {
                    self.navigation.push(Scope::Folder {
                        account: folder.account.clone(),
                        folder: folder.id,
                    });
                    self.navigation_labels.push(format!(
                        "{} · {}",
                        account.id,
                        folder.name.chars().take(80).collect::<String>()
                    ));
                }
            }
        }
    }

    fn open(&mut self, account: String, id: i64) -> Task<super::Message> {
        let Some(owner) = &self.owner else {
            return Task::none();
        };
        if !self
            .rows
            .iter()
            .any(|row| row.latest.id == id && row.latest.account == account)
            && !self
                .members
                .iter()
                .any(|row| row.id == id && row.account == account)
        {
            return Task::none();
        }
        self.selected = Some((account.clone(), id));
        self.body = None;
        self.members.clear();
        self.member_labels.clear();
        self.reading = self.reading.saturating_add(1);
        let (generation, reading) = (self.generation, self.reading);
        let store = owner.handle();
        Task::perform(
            async move {
                let projection = store.message(account.clone(), id).await?;
                let members = store.conversation_messages(account, id).await?;
                let readable = projection.readable_text();
                let can_mark_read = readable.is_some();
                let text = readable.map_or_else(
                    || "No readable body. Encrypted messages are not supported yet.".into(),
                    std::borrow::Cow::into_owned,
                );
                Ok(Loaded {
                    body: Arc::new(Body {
                        projection,
                        text,
                        readable: can_mark_read,
                    }),
                    members,
                })
            },
            move |result| event(Message::Body(generation, reading, Payload(result))),
        )
    }

    fn query(&mut self, next: bool) -> Task<super::Message> {
        let (Some(owner), Some(config)) = (&self.owner, &self.config) else {
            return Task::none();
        };
        if self.loading {
            self.query_again = true;
            return Task::none();
        }
        self.loading = true;
        self.listing = self.listing.saturating_add(1);
        if next {
            self.after = self.rows.last().map(|row| row.latest.cursor());
        } else {
            self.after = None;
        }
        let (generation, listing, after) = (self.generation, self.listing, self.after);
        let accounts = config
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect::<Vec<_>>();
        let store = owner.handle();
        let (scope, search) = (self.scope.clone(), self.active_search.clone());
        Task::perform(
            async move {
                let rows = store
                    .conversation_page(accounts.clone(), scope, search, after, 100)
                    .await?;
                let folders = store.folders(accounts.clone()).await?;
                let flags = store.flag_counts(accounts).await?;
                Ok(Listing {
                    rows,
                    flags,
                    folders,
                })
            },
            move |result| event(Message::Rows(generation, listing, Payload(result))),
        )
    }
    fn sync_targets(&self, config: &Config, older: bool) -> Vec<(Account, String)> {
        config
            .accounts
            .iter()
            .filter(|account| match &self.scope {
                Scope::Inbox => true,
                Scope::Account(id) | Scope::Folder { account: id, .. } => account.id == *id,
            })
            .filter_map(|account| {
                let folder = match &self.scope {
                    Scope::Folder {
                        account: id,
                        folder,
                    } => self
                        .folders
                        .iter()
                        .find(|candidate| candidate.account == *id && candidate.id == *folder)?
                        .name
                        .clone(),
                    _ => account
                        .folders
                        .inbox
                        .clone()
                        .unwrap_or_else(|| "INBOX".into()),
                };
                (!older
                    || self
                        .older
                        .contains_key(&(account.id.clone(), folder.clone())))
                .then(|| (account.clone(), folder))
            })
            .collect::<Vec<_>>()
    }
    fn synchronize(&mut self, keyring: Option<Session>, older: bool) -> Task<super::Message> {
        let (Some(owner), Some(config), Some(keyring)) = (&self.owner, &self.config, keyring)
        else {
            self.status = vec![
                "Load configuration and unlock the encrypted keyring in Accounts before syncing."
                    .into(),
            ];
            return Task::none();
        };
        let targets = self.sync_targets(config, older);
        self.cancel = CancellationToken::new();
        self.jobs = self.jobs.saturating_add(1);
        self.status = vec!["Fetching a bounded page in the selected accounts/folder…".into()];
        let (generation, config, cancel, store, cursors) = (
            self.generation,
            config.clone(),
            self.cancel.clone(),
            owner.handle(),
            self.older.clone(),
        );
        Task::perform(
            async move {
                stream::iter(targets.into_iter().map(|(account, folder)| {
                    let (keyring, store, cancel) = (keyring.clone(), store.clone(), cancel.clone());
                    let before = if older {
                        cursors.get(&(account.id.clone(), folder.clone())).copied()
                    } else {
                        None
                    };
                    let earliest = if older {
                        0
                    } else {
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_or(0, |time| i64::try_from(time.as_secs()).unwrap_or(i64::MAX))
                            .saturating_sub(i64::from(config.sync.initial_days) * 86400)
                    };
                    let max_original = config.sync.max_body_bytes;
                    async move {
                        synchronize_account(
                            account,
                            folder,
                            keyring,
                            store,
                            before,
                            (earliest, max_original),
                            cancel,
                        )
                        .await
                    }
                }))
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
            },
            move |results| event(Message::Synced(generation, Payload(results))),
        )
    }

    fn load_members(&self) -> Task<super::Message> {
        let (Some(store), Some((account, id))) = (self.store(), self.selected.as_ref()) else {
            return Task::none();
        };
        let (account, id, generation, reading) =
            (account.clone(), *id, self.generation, self.reading);
        Task::perform(
            async move { store.conversation_messages(account, id).await },
            move |result| event(Message::Members(generation, reading, Payload(result))),
        )
    }
    pub fn maintenance(&mut self) -> Task<super::Message> {
        if !self.group_again || self.busy() {
            return Task::none();
        }
        let (Some(store), Some(config)) = (self.store(), self.config.as_ref()) else {
            return Task::none();
        };
        self.group_again = false;
        let accounts = config
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect::<Vec<_>>();
        if accounts.is_empty() {
            return Task::none();
        }
        self.jobs = self.jobs.saturating_add(1);
        self.cancel = CancellationToken::new();
        let (generation, cancel) = (self.generation, self.cancel.clone());
        Task::perform(
            async move {
                let mut errors = Vec::new();
                for account in accounts {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Err(error) = group_account(&store, &account, &cancel).await {
                        errors.push(format!("{account}: {error}"));
                    }
                }
                errors
            },
            move |errors| event(Message::Grouped(generation, Payload(errors))),
        )
    }
    fn undo_flags(&mut self) -> Task<super::Message> {
        let (Some(store), Some((account, id))) = (self.store(), self.selected.as_ref()) else {
            return Task::none();
        };
        let (account, id, generation) = (account.clone(), *id, self.generation);
        self.edits = self.edits.saturating_add(1);
        Task::perform(
            async move { store.undo_flags(account, id).await },
            move |result| event(Message::FlagQueued(generation, result)),
        )
    }
    fn set_flag(&mut self, flag: FlagChange, desired: bool) -> Task<super::Message> {
        let (Some(store), Some(config), Some((account, id))) =
            (self.store(), self.config.as_ref(), self.selected.as_ref())
        else {
            return Task::none();
        };
        let Some(config) = config.accounts.iter().find(|item| item.id == *account) else {
            return Task::none();
        };
        let context = match flags::context(config) {
            Ok(value) => value,
            Err(error) => {
                self.status = vec![error.to_string()];
                return Task::none();
            }
        };
        let (account, id, generation) = (account.clone(), *id, self.generation);
        self.edits = self.edits.saturating_add(1);
        Task::perform(
            async move { store.set_flag(account, id, flag, desired, context).await },
            move |result| event(Message::FlagQueued(generation, result)),
        )
    }
    fn apply_flags(&mut self, keyring: Option<Session>) -> Task<super::Message> {
        let (Some(store), Some(config), Some(keyring)) =
            (self.store(), self.config.clone(), keyring)
        else {
            self.status = vec![
                "Unlock the encrypted keyring in Accounts before applying pending flags.".into(),
            ];
            return Task::none();
        };
        self.cancel = CancellationToken::new();
        self.jobs = self.jobs.saturating_add(1);
        let (cancel, generation) = (self.cancel.clone(), self.generation);
        Task::perform(
            async move {
                stream::iter(config.accounts.clone().into_iter().map(|account| {
                    let (store, keyring, cancel) = (store.clone(), keyring.clone(), cancel.clone());
                    async move { apply_account_flags(account, keyring, store, cancel).await }
                }))
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
            },
            move |results| event(Message::Applied(generation, Payload(results))),
        )
    }

    pub fn compose_source(&self, generation: u64, reading: u64) -> Option<(&str, &Projection)> {
        if generation != self.generation || reading != self.reading {
            return None;
        }
        Some((&self.selected.as_ref()?.0, &self.body.as_ref()?.projection))
    }
    pub fn status(&mut self, value: String) {
        self.status = vec![value];
    }
    #[expect(
        clippy::too_many_lines,
        reason = "Keep native reader controls and their captured message identity together in one visible layout."
    )]
    fn reader_view(&self, width: f32, height: f32) -> Element<'_, super::Message> {
        let Some(body) = &self.body else {
            return widget::text(if self.selected.is_some() {
                "Loading cached message…"
            } else {
                "Select a message to read it offline."
            })
            .into();
        };

        let mut reader = column![widget::text::title3(
            body.projection
                .headers
                .subject
                .as_deref()
                .unwrap_or("(no subject)")
        )]
        .spacing(8);
        if self.members.len() > 1 {
            let selected = self.selected.as_ref().and_then(|(account, id)| {
                self.members
                    .iter()
                    .position(|message| message.id == *id && message.account == *account)
            });
            let ids = self
                .members
                .iter()
                .map(|message| (message.account.clone(), message.id))
                .collect::<Vec<_>>();
            reader = reader.push(widget::dropdown(
                &self.member_labels,
                selected,
                move |index| {
                    ids.get(index)
                        .map_or(super::Message::Inbox(Message::Refresh), |(account, id)| {
                            super::Message::Inbox(Message::Open(account.clone(), *id))
                        })
                },
            ));
            if self.members.len() == 200 {
                reader = reader.push(widget::text(
                    "Showing up to 200 cached conversation members.",
                ));
            }
        }
        if body.projection.status.body_truncated
            || body.projection.status.degraded
            || body.projection.status.headers_truncated
        {
            reader = reader.push(widget::text(
                "This message is incomplete or has unsupported formatting.",
            ));
        }
        reader = reader.push(reply_actions(self.generation, self.reading));
        if let Some((account, id)) = &self.selected
            && let Some(message) = self
                .members
                .iter()
                .find(|row| row.account == *account && row.id == *id)
        {
            reader = reader.push(
                row![
                    widget::button::text(if message.seen {
                        "Mark unread"
                    } else {
                        "Mark read"
                    })
                    .on_press_maybe(
                        (!self.busy() && message.inbox).then_some(super::Message::Inbox(
                            Message::SetFlag(
                                self.generation,
                                account.clone(),
                                *id,
                                FlagChange::Seen,
                                !message.seen
                            )
                        ))
                    ),
                    widget::button::text(if message.starred { "Unstar" } else { "Star" })
                        .on_press_maybe((!self.busy() && message.inbox).then_some(
                            super::Message::Inbox(Message::SetFlag(
                                self.generation,
                                account.clone(),
                                *id,
                                FlagChange::Starred,
                                !message.starred
                            ))
                        )),
                    widget::button::text("Undo unsent flags").on_press_maybe(
                        (!self.busy() && message.inbox && self.flags.pending > 0).then_some(
                            super::Message::Inbox(Message::UndoFlags(
                                self.generation,
                                account.clone(),
                                *id
                            ))
                        )
                    )
                ]
                .spacing(8),
            );
            if !message.inbox {
                reader = reader.push(widget::text(
                    "Cached ancestor outside Inbox · flags are read-only here.",
                ));
            }
        }
        // Retain decoded content in widget state; no HTML engine receives it.
        reader
            .push(crate::text_reader::view(
                &body.text,
                (width - 380.0).max(150.0),
                height.max(100.0),
            ))
            .into()
    }

    fn heading(&self) -> Element<'_, super::Message> {
        let generation = self.generation;
        let navigation = self.navigation.clone();
        let selected = self
            .navigation
            .iter()
            .position(|scope| *scope == self.scope);
        row![
            widget::text::title1("Mail"),
            widget::dropdown(&self.navigation_labels, selected, move |index| {
                super::Message::Inbox(Message::Scope(
                    generation,
                    navigation.get(index).cloned().unwrap_or_default(),
                ))
            })
            .width(240),
            widget::text_input("Search cached mail", &self.search)
                .on_input(|value| super::Message::Inbox(Message::SearchChanged(Payload(value))))
                .on_submit(|_| super::Message::Inbox(Message::Search))
                .width(Length::Fill),
            widget::button::text("Search").on_press(super::Message::Inbox(Message::Search))
        ]
        .spacing(12)
        .into()
    }

    pub fn view(&self, width: f32, height: f32) -> Element<'_, super::Message> {
        let mut content=column![self.heading(),
            row![widget::button::text(match self.scope{Scope::Inbox=>"Sync inboxes",Scope::Account(_)=>"Sync account inbox",Scope::Folder{..}=>"Sync folder"}).on_press_maybe((!self.busy()).then_some(super::Message::Inbox(Message::Sync(false)))),
                widget::button::text("Fetch older page").on_press_maybe((!self.busy()&&self.config.as_ref().is_some_and(|config|!self.sync_targets(config,true).is_empty())).then_some(super::Message::Inbox(Message::Sync(true)))),
                widget::button::text("Cancel").on_press_maybe(self.busy().then_some(super::Message::Inbox(Message::Cancel))),
                widget::button::text(format!("Apply flags ({})",self.flags.pending+self.flags.failed)).on_press_maybe((!self.busy()&&(self.flags.pending>0||self.flags.failed>0)).then_some(super::Message::Inbox(Message::ApplyFlags)))].spacing(8),
            widget::text("Partial offline cache · focused reading marks only the selected Inbox message read after a short delay.")].spacing(12).padding(20);
        let mut notices = column![].spacing(12);
        for status in &self.status {
            notices = notices.push(widget::text(status));
        }
        if self.loading {
            notices = notices.push(widget::text("Loading cached inbox…"));
        }
        if self.rows.is_empty() && !self.loading {
            notices=notices.push(widget::text("No matching cached messages on this page. Search is local; clear the query or sync this selection."));
        }
        let mut list = column![].spacing(8);
        for conversation in &self.rows {
            let message = &conversation.latest;
            let title = if message.subject.is_empty() {
                "(no subject)"
            } else {
                &message.subject
            };
            list = list.push(
                widget::button::custom(
                    column![
                        widget::text(format!(
                            "{}{}{} · {}",
                            if conversation.starred { "★ " } else { "" },
                            if conversation.unread == 0 { "" } else { "● " },
                            message.sender,
                            message.account
                        )),
                        widget::text(if conversation.cached_messages > 1 {
                            format!(
                                "{title} · {} messages ({} unread)",
                                conversation.cached_messages, conversation.unread
                            )
                        } else {
                            title.to_owned()
                        }),
                        widget::text(&message.preview)
                    ]
                    .spacing(4),
                )
                .on_press(super::Message::Inbox(Message::Open(
                    message.account.clone(),
                    message.id,
                )))
                .width(Length::Fill),
            );
        }
        if self.flags.conflicts > 0 {
            notices=notices.push(widget::text(format!("{} flag operations retired because their server identity changed. Sync to reconcile.",self.flags.conflicts)));
        }
        // Keep the reader's widget-tree position stable when status/loading/empty
        // notices change, so an automatic Seen commit cannot discard text focus.
        content = content.push(notices);
        let reader = self.reader_view(width, height);
        content = content.push(
            row![
                widget::scrollable(list)
                    .width(340)
                    .height(height.max(100.0)),
                widget::container(reader).width(Length::Fill)
            ]
            .spacing(16),
        );
        content
            .push(
                row![
                    widget::button::text("Newest cached page")
                        .on_press(super::Message::Inbox(Message::Refresh)),
                    widget::button::text("Next cached page").on_press_maybe(
                        (!self.loading && self.rows.len() == 100)
                            .then_some(super::Message::Inbox(Message::Next))
                    )
                ]
                .spacing(8),
            )
            .into()
    }
}

fn reply_actions(generation: u64, reading: u64) -> Element<'static, super::Message> {
    row![
        widget::button::text("Reply").on_press(super::Message::Reply(
            generation,
            reading,
            mailstronaut_core::reply::Kind::Reply
        )),
        widget::button::text("Reply all").on_press(super::Message::Reply(
            generation,
            reading,
            mailstronaut_core::reply::Kind::ReplyAll
        )),
        widget::button::text("Forward as text").on_press(super::Message::Reply(
            generation,
            reading,
            mailstronaut_core::reply::Kind::ForwardText
        ))
    ]
    .spacing(8)
    .into()
}

const fn event(message: Message) -> Action<super::Message> {
    Action::App(super::Message::Inbox(message))
}

async fn synchronize_account(
    account: Account,
    folder: String,
    keyring: Session,
    store: worker::Handle,
    before: Option<sync::Position>,
    limits: (i64, u32),
    cancel: CancellationToken,
) -> AccountResult {
    let result = async {
        let mut connection = connect_account(&account, &keyring, &cancel).await?;
        let context = flags::context(&account).map_err(|error| error.to_string())?;
        let inbox = account.folders.inbox.as_deref().unwrap_or("INBOX");
        sync::discover(
            &store,
            &mut connection,
            &account.id,
            &context,
            inbox,
            &cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
        let page = sync::page(
            &store,
            &mut connection,
            &sync::Request {
                account: &account.id,
                context: &context,
                folder: &folder,
                inbox: folder == inbox
                    || (inbox.eq_ignore_ascii_case("INBOX")
                        && folder.eq_ignore_ascii_case("INBOX")),
                before,
                earliest: limits.0,
                max_original: limits.1,
            },
            &cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
        drop(connection);
        let warning = group_account(&store, &account.id, &cancel).await.err();
        Ok::<_, String>((page, warning))
    }
    .await;
    match result {
        Ok((page, warning)) => AccountResult {
            account: account.id,
            folder,
            next: page.next_before,
            status: format!(
                "{} downloaded · {} refreshed · {} omitted · partial cache{}",
                page.downloaded,
                page.refreshed,
                page.skipped,
                warning.map_or_else(String::new, |warning| format!(" · {warning}"))
            ),
        },
        Err(status) => AccountResult {
            account: account.id,
            folder,
            next: before,
            status,
        },
    }
}

async fn group_account(
    store: &worker::Handle,
    account: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let Some(snapshot) = store
        .conversation_snapshot(account.into())
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let groups = if snapshot.messages.is_empty() {
        threads::Groups { groups: Vec::new() }
    } else {
        let executable =
            std::env::current_exe().map_err(|_| "Conversation helper path unavailable.")?;
        threads::process::project(&executable, &snapshot.messages, cancel)
            .await
            .map_err(|error| error.to_string())?
    };
    store
        .commit_conversations(snapshot, groups)
        .await
        .map_err(|error| error.to_string())
}

async fn connect_account(
    account: &Account,
    keyring: &Session,
    cancel: &CancellationToken,
) -> Result<Connection, String> {
    let credential = keyring
        .resolve(account, false, cancel)
        .await
        .map_err(|error| error.to_string())?;
    let endpoint = account
        .incoming_endpoint()
        .ok_or("Incoming endpoint missing.")?;
    let connection = Connection::connect(
        &endpoint,
        &account.tls,
        &account.user_name,
        credential.authentication(),
        cancel,
    )
    .await;
    drop(credential);
    let connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            if error == imap::Error::Authentication {
                keyring.invalidate(account, false).await;
            }
            return Err(error.to_string());
        }
    };
    Ok(connection)
}
async fn apply_account_flags(
    account: Account,
    keyring: Session,
    store: worker::Handle,
    cancel: CancellationToken,
) -> AccountResult {
    let result = async {
        let counts = store
            .flag_counts(vec![account.id.clone()])
            .await
            .map_err(|error| error.to_string())?;
        if counts.pending == 0 && counts.failed == 0 {
            return Ok(0);
        }
        let context = flags::context(&account).map_err(|error| error.to_string())?;
        let mut connection = connect_account(&account, &keyring, &cancel).await?;
        actions::apply(
            &store,
            &mut connection,
            &actions::Request {
                account: &account.id,
                context: &context,
                retry: true,
            },
            &cancel,
        )
        .await
        .map_err(|error| error.to_string())
    }
    .await;
    AccountResult {
        account: account.id,
        folder: String::new(),
        next: None,
        status: match result {
            Ok(count) => format!(
                "{count} flag assignments confirmed; further queued work may need another batch."
            ),
            Err(error) => error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retiring_a_configuration_keeps_inflight_work_visible_to_quit_until_its_callbacks_arrive() {
        let mut inbox = Inbox {
            generation: 2,
            jobs: 1,
            edits: 1,
            ..Inbox::default()
        };
        let _task = inbox.update(Message::Applied(1, Payload(Vec::new())), None);
        assert!(inbox.busy());
        let _task = inbox.update(Message::FlagQueued(1, Err(worker::Error::Storage)), None);
        assert!(!inbox.busy());
        assert!(
            inbox.status.is_empty(),
            "retired errors never replace current UI status"
        );
    }
    #[test]
    fn scope_targets_keep_account_folder_and_paging_identity_together() {
        let account = |id: &str| serde_json::json!({"id":id,"provider":"icloud","address":format!("{id}@example.test"),"displayName":id,"userName":id,"auth":{"kind":"password","secretRef":"synthetic"}});
        let config: Config = serde_json::from_value(
            serde_json::json!({"schemaVersion":1,"accounts":[account("a"),account("b")]}),
        )
        .expect("synthetic accounts");
        let mut inbox = Inbox {
            scope: Scope::Folder {
                account: "a".into(),
                folder: 7,
            },
            folders: vec![Folder {
                id: 7,
                account: "a".into(),
                name: "Sent".into(),
                uid_validity: None,
                inbox: false,
            }],
            ..Inbox::default()
        };
        let targets = inbox.sync_targets(&config, false);
        assert_eq!(targets.len(), 1);
        assert_eq!(
            (&targets[0].0.id, &targets[0].1),
            (&"a".to_owned(), &"Sent".to_owned())
        );
        let position = sync::Position {
            context: [0; 32],
            folder: 1,
            uid_validity: core::num::NonZeroU32::new(42).expect("namespace"),
            before: core::num::NonZeroU32::new(2).expect("cursor"),
        };
        inbox.older.insert(("a".into(), "INBOX".into()), position);
        assert_eq!(
            inbox.sync_targets(&config, true).len(),
            0,
            "INBOX's cursor is never borrowed by Sent"
        );
        inbox.older.insert(("a".into(), "Sent".into()), position);
        assert_eq!(inbox.sync_targets(&config, true).len(), 1);
        inbox.scope = Scope::Account("b".into());
        let targets = inbox.sync_targets(&config, false);
        assert_eq!(
            (&targets[0].0.id, &targets[0].1),
            (&"b".to_owned(), &"INBOX".to_owned())
        );
        inbox.scope = Scope::Folder {
            account: "b".into(),
            folder: 7,
        };
        assert_eq!(
            inbox.sync_targets(&config, false).len(),
            0,
            "a foreign/missing folder never silently syncs INBOX instead"
        );
    }
    #[test]
    fn search_events_are_redacted_and_retired_scopes_cannot_replace_current_selection() {
        assert!(
            !format!(
                "{:?}",
                Message::SearchChanged(Payload("SYNTHETIC-PRIVATE-QUERY".into()))
            )
            .contains("SYNTHETIC-PRIVATE-QUERY")
        );
        let mut inbox = Inbox {
            generation: 2,
            navigation: vec![Scope::Inbox, Scope::Account("a".into())],
            ..Inbox::default()
        };
        let _task = inbox.update(Message::Scope(1, Scope::Account("a".into())), None);
        assert_eq!(inbox.scope, Scope::Inbox);
        let _task = inbox.update(Message::Scope(2, Scope::Account("disabled".into())), None);
        assert_eq!(inbox.scope, Scope::Inbox);
    }
    const MARKER: &str = "SYNTHETIC-BODY-MUST-NOT-ENTER-EVENT-LOGS";
    #[test]
    fn body_results_are_redacted_and_old_configurations_cannot_replace_the_reader() {
        let projection = mailstronaut_core::mime::project(
            format!("Subject: {MARKER}\r\n\r\n{MARKER}").as_bytes(),
        )
        .expect("synthetic projection");
        let message = Message::Body(
            1,
            1,
            Payload(Ok(Loaded {
                body: Arc::new(Body {
                    projection,
                    text: MARKER.into(),
                    readable: true,
                }),
                members: Vec::new(),
            })),
        );
        assert!(!format!("{message:?}").contains(MARKER));
        let mut inbox = Inbox {
            generation: 2,
            reading: 1,
            ..Inbox::default()
        };
        let _task = inbox.update(message, None);
        assert!(inbox.body.is_none());
        let message = Message::Rows(1, 1, Payload(Err(worker::Error::Storage)));
        let _task = inbox.update(message, None);
        assert!(
            inbox.status.is_empty(),
            "a retired account query cannot replace current status"
        );
    }
}
