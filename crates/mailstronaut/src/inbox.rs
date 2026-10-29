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
    store::flags::{self, Counts},
    store::mailbox::{Cursor, Summary},
    sync,
    worker::{self, Worker},
};
use mailstronaut_mail::{
    auth::Session,
    imap::{self, Connection, FlagChange},
};
use tokio_util::sync::CancellationToken;

use crate::private::Private as Payload;
#[derive(Clone, Debug)]
pub enum Message {
    Started(Payload<Result<Arc<Worker>, worker::Error>>),
    Refresh,
    Next,
    Rows(u64, u64, Payload<Result<Listing, worker::Error>>),
    Open(String, i64),
    Body(u64, u64, Payload<Result<Arc<Body>, worker::Error>>),
    SetFlag(FlagChange, bool),
    UndoFlags,
    FlagQueued(u64, Result<(), worker::Error>),
    ApplyFlags,
    Applied(u64, Payload<Vec<AccountResult>>),
    Sync(bool),
    Cancel,
    Synced(u64, Payload<Vec<AccountResult>>),
}
#[derive(Clone)]
pub struct Listing {
    rows: Vec<Summary>,
    flags: Counts,
}

#[derive(Clone)]
pub struct AccountResult {
    account: String,
    next: Option<sync::Position>,
    status: String,
}

pub struct Body {
    projection: Projection,
    text: String,
}

#[derive(Default)]
pub struct Inbox {
    owner: Option<Arc<Worker>>,
    config: Option<Arc<Config>>,
    generation: u64,
    listing: u64,
    reading: u64,
    rows: Vec<Summary>,
    after: Option<Cursor>,
    selected: Option<(String, i64)>,
    body: Option<Arc<Body>>,
    loading: bool,
    jobs: u32,
    edits: u32,
    query_again: bool,
    flags: Counts,
    cancel: CancellationToken,
    older: BTreeMap<String, sync::Position>,
    status: Vec<String>,
}
impl Inbox {
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
        self.selected = None;
        self.body = None;
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

    pub fn update(&mut self, message: Message, keyring: Option<Session>) -> Task<super::Message> {
        match message {
            Message::Started(Payload(result)) => match result {
                Ok(owner) => {
                    self.owner = Some(owner);
                    return self.query(false);
                }
                Err(error) => self.status = vec![error.to_string()],
            },
            Message::Refresh => return self.query(false),
            Message::Next => return self.query(true),
            Message::Rows(generation, listing, Payload(result))
                if generation == self.generation && listing == self.listing =>
            {
                self.loading = false;
                match result {
                    Ok(listing) => {
                        self.rows = listing.rows;
                        self.flags = listing.flags;
                    }
                    Err(error) => self.status = vec![error.to_string()],
                }
                if core::mem::take(&mut self.query_again) {
                    return self.query(false);
                }
            }
            Message::Open(account, id) => return self.open(account, id),
            Message::Body(generation, reading, Payload(result))
                if generation == self.generation && reading == self.reading =>
            {
                match result {
                    Ok(body) => {
                        self.body = Some(body);
                        return crate::text_reader::open();
                    }
                    Err(error) => self.status = vec![error.to_string()],
                }
            }
            Message::UndoFlags if !self.busy() => return self.undo_flags(),
            Message::SetFlag(flag, desired) if !self.busy() => return self.set_flag(flag, desired),
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
                    self.older.remove(&result.account);
                    if let Some(next) = result.next {
                        self.older.insert(result.account.clone(), next);
                    }
                    self.status
                        .push(format!("{}: {}", result.account, result.status));
                }
                return self.query(false);
            }
            _ => {}
        }
        Task::none()
    }

    fn open(&mut self, account: String, id: i64) -> Task<super::Message> {
        let Some(owner) = &self.owner else {
            return Task::none();
        };
        if !self
            .rows
            .iter()
            .any(|row| row.id == id && row.account == account)
        {
            return Task::none();
        }
        self.selected = Some((account.clone(), id));
        self.body = None;
        self.reading = self.reading.saturating_add(1);
        let (generation, reading) = (self.generation, self.reading);
        let store = owner.handle();
        Task::perform(
            async move {
                store.message(account, id).await.map(|projection| {
                    let text = projection.readable_text().map_or_else(
                        || "No readable body. Encrypted messages are not supported yet.".into(),
                        std::borrow::Cow::into_owned,
                    );
                    Arc::new(Body { projection, text })
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
            self.after = self.rows.last().map(Summary::cursor);
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
        Task::perform(
            async move {
                let rows = store.inbox(accounts.clone(), after, 100).await?;
                let flags = store.flag_counts(accounts).await?;
                Ok(Listing { rows, flags })
            },
            move |result| event(Message::Rows(generation, listing, Payload(result))),
        )
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
        self.cancel = CancellationToken::new();
        self.jobs = self.jobs.saturating_add(1);
        self.status = vec!["Fetching a bounded inbox page per account…".into()];
        let (generation, config, cancel, store, cursors) = (
            self.generation,
            config.clone(),
            self.cancel.clone(),
            owner.handle(),
            self.older.clone(),
        );
        Task::perform(
            async move {
                stream::iter(
                    config
                        .accounts
                        .clone()
                        .into_iter()
                        .filter(|account| !older || cursors.contains_key(&account.id))
                        .map(|account| {
                            let (keyring, store, cancel) =
                                (keyring.clone(), store.clone(), cancel.clone());
                            let before = if older {
                                cursors.get(&account.id).copied()
                            } else {
                                None
                            };
                            let earliest = if older {
                                0
                            } else {
                                SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map_or(0, |time| {
                                        i64::try_from(time.as_secs()).unwrap_or(i64::MAX)
                                    })
                                    .saturating_sub(i64::from(config.sync.initial_days) * 86400)
                            };
                            let max_original = config.sync.max_body_bytes;
                            async move {
                                synchronize_account(
                                    account,
                                    keyring,
                                    store,
                                    before,
                                    earliest,
                                    max_original,
                                    cancel,
                                )
                                .await
                            }
                        }),
                )
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
            },
            move |results| event(Message::Synced(generation, Payload(results))),
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

    pub fn compose_source(&self) -> Option<(&str, &Projection)> {
        Some((&self.selected.as_ref()?.0, &self.body.as_ref()?.projection))
    }
    pub fn status(&mut self, value: String) {
        self.status = vec![value];
    }
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
        if body.projection.status.body_truncated
            || body.projection.status.degraded
            || body.projection.status.headers_truncated
        {
            reader = reader.push(widget::text(
                "This message is incomplete or has unsupported formatting.",
            ));
        }
        reader = reader.push(
            row![
                widget::button::text("Reply")
                    .on_press(super::Message::Reply(mailstronaut_core::reply::Kind::Reply)),
                widget::button::text("Reply all").on_press(super::Message::Reply(
                    mailstronaut_core::reply::Kind::ReplyAll
                )),
                widget::button::text("Forward as text").on_press(super::Message::Reply(
                    mailstronaut_core::reply::Kind::ForwardText
                ))
            ]
            .spacing(8),
        );
        if let Some((account, id)) = &self.selected
            && let Some(message) = self
                .rows
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
                    .on_press_maybe((!self.busy()).then_some(
                        super::Message::Inbox(Message::SetFlag(FlagChange::Seen, !message.seen))
                    )),
                    widget::button::text(if message.starred { "Unstar" } else { "Star" })
                        .on_press_maybe((!self.busy()).then_some(super::Message::Inbox(
                            Message::SetFlag(FlagChange::Starred, !message.starred)
                        ))),
                    widget::button::text("Undo unsent flags").on_press_maybe(
                        (!self.busy() && self.flags.pending > 0)
                            .then_some(super::Message::Inbox(Message::UndoFlags))
                    )
                ]
                .spacing(8),
            );
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

    pub fn view(&self, width: f32, height: f32) -> Element<'_, super::Message> {
        let mut content=column![widget::text::title1("Unified inbox"),
            row![widget::button::text("Sync inboxes").on_press_maybe((!self.busy()).then_some(super::Message::Inbox(Message::Sync(false)))),
                widget::button::text("Fetch older page").on_press_maybe((!self.busy()&&!self.older.is_empty()).then_some(super::Message::Inbox(Message::Sync(true)))),
                widget::button::text("Cancel").on_press_maybe(self.busy().then_some(super::Message::Inbox(Message::Cancel))),
                widget::button::text(format!("Apply flags ({})",self.flags.pending+self.flags.failed)).on_press_maybe((!self.busy()&&(self.flags.pending>0||self.flags.failed>0)).then_some(super::Message::Inbox(Message::ApplyFlags)))].spacing(8),
            widget::text("Partial offline cache · opening a message currently leaves its read flag unchanged.")].spacing(12).padding(20);
        for status in &self.status {
            content = content.push(widget::text(status));
        }
        if self.loading {
            content = content.push(widget::text("Loading cached inbox…"));
        }
        if self.rows.is_empty() && !self.loading {
            content=content.push(widget::text("No cached messages on this page. Configure accounts, unlock the keyring, then sync."));
        }
        let mut list = column![].spacing(8);
        for message in &self.rows {
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
                            if message.starred { "★ " } else { "" },
                            if message.seen { "" } else { "● " },
                            message.sender,
                            message.account
                        )),
                        widget::text(title),
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
            content=content.push(widget::text(format!("{} flag operations retired because their server identity changed. Sync to reconcile.",self.flags.conflicts)));
        }
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

const fn event(message: Message) -> Action<super::Message> {
    Action::App(super::Message::Inbox(message))
}

async fn synchronize_account(
    account: Account,
    keyring: Session,
    store: worker::Handle,
    before: Option<sync::Position>,
    earliest: i64,
    max_original: u32,
    cancel: CancellationToken,
) -> AccountResult {
    let result = async {
        let mut connection = connect_account(&account, &keyring, &cancel).await?;
        sync::page(
            &store,
            &mut connection,
            &sync::Request {
                account: &account.id,
                context: &mailstronaut_core::store::flags::context(&account)
                    .map_err(|error| error.to_string())?,
                folder: account.folders.inbox.as_deref().unwrap_or("INBOX"),
                inbox: true,
                before,
                earliest,
                max_original,
            },
            &cancel,
        )
        .await
        .map_err(|error| error.to_string())
    }
    .await;
    match result {
        Ok(page) => AccountResult {
            account: account.id,
            next: page.next_before,
            status: format!(
                "{} downloaded · {} refreshed · {} omitted · partial cache",
                page.downloaded, page.refreshed, page.skipped
            ),
        },
        Err(status) => AccountResult {
            account: account.id,
            next: before,
            status,
        },
    }
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
            Payload(Ok(Arc::new(Body {
                projection,
                text: MARKER.into(),
            }))),
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
