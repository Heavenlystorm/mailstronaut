//! Native foreground reminders using the existing store owner and desktop portal.
mod desktop;
use crate::private::Private as Payload;
use cosmic::{
    Action, Element,
    app::Task,
    iced::{
        Length, Subscription,
        futures::stream,
        widget::{column, row},
    },
    widget,
};
use mailstronaut_config::Config;
use mailstronaut_core::{
    reminders::{self, When},
    store::reminders::{Reminder, Schedule, State},
    worker,
};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Target {
    pub account: String,
    pub message: i64,
    pub title: String,
}
impl core::fmt::Debug for Target {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ReminderTarget(<private>)")
    }
}
#[derive(Clone, Debug)]
pub enum Message {
    Tick(u64),
    Refresh,
    Result(u64, Payload<Result<Snapshot, String>>),
    Preset(When),
    Custom,
    Time(Payload<String>),
    Day(jiff::civil::Date),
    Month(bool),
    Edit(Payload<Reminder>, bool),
    Open(Payload<Reminder>),
    Saved(u64, Result<(), worker::Error>),
    Desktop(Payload<desktop::Action>),
    Resolved(
        u64,
        Payload<Result<Option<Reminder>, worker::Error>>,
        desktop::Kind,
    ),
}
#[derive(Clone)]
pub struct Snapshot {
    records: Vec<Reminder>,
    selected: Option<Reminder>,
    next: Option<i64>,
    client: Option<Arc<desktop::Client>>,
    status: String,
}
pub struct Model {
    store: Option<worker::Handle>,
    config: Option<Arc<Config>>,
    generation: u64,
    busy: bool,
    stopping: bool,
    initialize_editor: bool,
    cancel: CancellationToken,
    records: Vec<Reminder>,
    client: Option<Arc<desktop::Client>>,
    target: Option<Target>,
    existing: Option<Reminder>,
    calendar: widget::calendar::CalendarModel,
    time: String,
    zone: String,
    status: String,
    queued: std::collections::VecDeque<desktop::Action>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            store: None,
            config: None,
            generation: 0,
            busy: false,
            stopping: false,
            initialize_editor: false,
            cancel: CancellationToken::new(),
            records: Vec::new(),
            client: None,
            target: None,
            existing: None,
            calendar: widget::calendar::CalendarModel::now(),
            time: "09:00".into(),
            zone: "UTC".into(),
            status: String::new(),
            queued: std::collections::VecDeque::new(),
        }
    }
}
impl Model {
    pub const fn busy(&self) -> bool {
        self.busy
    }
    pub fn cancel(&mut self) {
        self.stopping = true;
        self.cancel.cancel();
    }
    pub fn resume(&mut self) -> Task<super::Message> {
        self.stopping = false;
        self.cancel = CancellationToken::new();
        self.run()
    }
    pub fn configure(&mut self, config: Option<Arc<Config>>) -> Task<super::Message> {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.generation = self.generation.saturating_add(1);
        self.config = config;
        self.initialize_editor = false;
        self.target = None;
        self.existing = None;
        self.records.clear();
        self.queued.clear();
        self.zone = self
            .config
            .as_ref()
            .and_then(|config| config.time_zone.clone())
            .unwrap_or_else(|| {
                jiff::tz::TimeZone::system()
                    .iana_name()
                    .unwrap_or("UTC")
                    .into()
            });
        self.run()
    }
    pub fn attach(&mut self, store: Option<worker::Handle>) -> Task<super::Message> {
        if self.store.is_some() || store.is_none() {
            return Task::none();
        }
        self.store = store;
        self.run()
    }
    pub fn edit(&mut self, target: Target) -> Task<super::Message> {
        self.select_time(now().saturating_add(3600));
        self.initialize_editor = true;
        self.target = Some(target);
        self.existing = None;
        self.run()
    }
    fn select_time(&mut self, at: i64) {
        if let Ok(display) = reminders::display(at, &self.zone) {
            let mut parts = display.split_whitespace();
            if let (Some(date), Some(time)) = (parts.next(), parts.next())
                && let Ok(date) = date.parse::<jiff::civil::Date>()
                && (1..=9998).contains(&date.year())
            {
                self.calendar.set_selected_visible(date);
                self.time = time.into();
            }
        }
    }
    fn accounts(&self) -> Vec<String> {
        self.config.as_ref().map_or_else(Vec::new, |config| {
            config
                .accounts
                .iter()
                .map(|account| account.id.clone())
                .collect()
        })
    }
    fn run(&mut self) -> Task<super::Message> {
        if self.busy || self.stopping {
            return Task::none();
        }
        let (Some(store), Some(config)) = (self.store.clone(), self.config.clone()) else {
            return Task::none();
        };
        self.busy = true;
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let (target, client, cancel) = (
            self.target.clone(),
            self.client.clone(),
            self.cancel.clone(),
        );
        Task::perform(step(store, config, target, client, cancel), move |result| {
            event(Message::Result(generation, Payload(result)))
        })
    }
    pub fn defer(&self) -> Task<super::Message> {
        self.wake(None)
    }
    fn wake(&self, next: Option<i64>) -> Task<super::Message> {
        if self.stopping {
            return Task::none();
        }
        let generation = self.generation;
        let cancel = self.cancel.clone();
        let seconds = next.map_or(30, |due| {
            if due <= now() {
                30
            } else {
                due.saturating_sub(now()).clamp(1, 30)
            }
        });
        Task::perform(
            async move {
                let _completed = cancel
                    .run_until_cancelled(tokio::time::sleep(core::time::Duration::from_secs(
                        u64::try_from(seconds).unwrap_or(30),
                    )))
                    .await;
            },
            move |()| event(Message::Tick(generation)),
        )
    }
    pub fn update(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::Tick(generation) if generation == self.generation => return self.run(),
            Message::Refresh => return self.run(),
            Message::Result(generation, Payload(result)) => {
                self.busy = false;
                if generation == self.generation {
                    match result {
                        Ok(snapshot) => {
                            self.records = snapshot.records;
                            self.existing = snapshot.selected;
                            if self.initialize_editor {
                                self.initialize_editor = false;
                                let at = self.existing.as_ref().map_or_else(
                                    || now().saturating_add(3600),
                                    |record| record.due_at.max(now().saturating_add(60)),
                                );
                                self.select_time(at);
                            }
                            self.client = snapshot.client;
                            self.status = snapshot.status;
                            if let Some(action) = self.queued.pop_front() {
                                return self.action(action);
                            }
                            return self.wake(snapshot.next);
                        }
                        Err(error) => {
                            self.status = error;
                            return self.wake(None);
                        }
                    }
                }
            }
            Message::Preset(when) if !self.busy => return self.schedule(&when),
            Message::Custom if !self.busy => {
                let date = chrono::NaiveDate::parse_from_str(
                    &self.calendar.selected.to_string(),
                    "%Y-%m-%d",
                );
                let time = chrono::NaiveTime::parse_from_str(&self.time, "%H:%M");
                if let (Ok(date), Ok(time)) = (date, time) {
                    return self.schedule(&When::Custom(date.and_time(time)));
                }
                self.status = "Choose a valid date and HH:MM time.".into();
            }
            Message::Time(Payload(time)) if time.len() <= 5 => self.time = time,
            Message::Day(date) if (1..=9998).contains(&date.year()) => {
                self.calendar.set_selected_visible(date);
            }
            Message::Month(next) => {
                let date = self.calendar.visible;
                if next && !(date.year() >= 9998 && date.month() == 12) {
                    self.calendar.show_next_month();
                } else if !(next || date.year() <= 1 && date.month() == 1) {
                    self.calendar.show_prev_month();
                }
            }
            Message::Edit(Payload(record), retry) if !self.busy => {
                return self.edit_record(record, retry);
            }
            Message::Saved(generation, result) => {
                self.busy = false;
                if generation == self.generation {
                    self.status = result.map_or_else(
                        |error| error.to_string(),
                        |()| "Reminder saved locally.".into(),
                    );
                }
                return self.run();
            }
            Message::Desktop(Payload(action)) => {
                if self.busy {
                    if self.queued.len() < 16 {
                        self.queued.push_back(action);
                    }
                } else {
                    return self.action(action);
                }
            }
            Message::Open(Payload(record)) if !self.busy => {
                return self.open_record(record);
            }
            Message::Resolved(generation, Payload(result), kind) => {
                return self.resolved(generation, result, kind);
            }
            _ => {}
        }
        Task::none()
    }
    fn resolved(
        &mut self,
        generation: u64,
        result: Result<Option<Reminder>, worker::Error>,
        kind: desktop::Kind,
    ) -> Task<super::Message> {
        self.busy = false;
        if generation != self.generation {
            return self.run();
        }
        if let Ok(Some(record)) = result {
            match kind {
                desktop::Kind::Open if record.available => {
                    let open =
                        Task::done(Action::App(super::Message::OpenReminder(Payload(Target {
                            account: record.account,
                            message: record.message,
                            title: record.title,
                        }))));
                    return Task::batch([open, self.run()]);
                }
                desktop::Kind::Again => {
                    self.target = Some(Target {
                        account: record.account.clone(),
                        message: record.message,
                        title: record.title.clone(),
                    });
                    self.existing = Some(record);
                    return self.schedule(&When::Hour);
                }
                desktop::Kind::Dismiss => return self.edit_record(record, false),
                desktop::Kind::Open => self.status = "Reminder message unavailable.".into(),
            }
        }
        self.run()
    }
    fn action(&mut self, action: desktop::Action) -> Task<super::Message> {
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        self.busy = true;
        let generation = self.generation;
        let accounts = self.accounts();
        Task::perform(
            async move {
                store
                    .reminder_action(accounts, action.notification, action.generation)
                    .await
            },
            move |result| event(Message::Resolved(generation, Payload(result), action.kind)),
        )
    }
    fn open_record(&mut self, record: Reminder) -> Task<super::Message> {
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        self.busy = true;
        let generation = self.generation;
        let accounts = self.accounts();
        Task::perform(resolve_original(store, accounts, record), move |result| {
            event(Message::Resolved(
                generation,
                Payload(result),
                desktop::Kind::Open,
            ))
        })
    }
    fn schedule(&mut self, when: &When) -> Task<super::Message> {
        let (Some(store), Some(config), Some(target)) = (
            self.store.clone(),
            self.config.as_ref(),
            self.target.clone(),
        ) else {
            return Task::none();
        };
        if !config
            .accounts
            .iter()
            .any(|account| account.id == target.account)
        {
            return Task::none();
        }
        let at = match reminders::resolve(
            now(),
            &self.zone,
            when,
            &config.reminders.morning,
            &config.reminders.evening,
        ) {
            Ok(at) => at,
            Err(error) => {
                self.status = error.to_string();
                return Task::none();
            }
        };
        let request = Schedule {
            account: target.account,
            message: target.message,
            due_at: at,
            time_zone: self.zone.clone(),
            now: now(),
            expected: self.existing.as_ref().map(|record| record.generation),
        };
        self.busy = true;
        let generation = self.generation;
        Task::perform(
            async move { store.schedule_reminder(request).await.map(|_| ()) },
            move |result| event(Message::Saved(generation, result)),
        )
    }
    fn edit_record(&mut self, record: Reminder, retry: bool) -> Task<super::Message> {
        let Some(store) = self.store.clone() else {
            return Task::none();
        };
        if !self.accounts().contains(&record.account) {
            return Task::none();
        }
        self.busy = true;
        let generation = self.generation;
        Task::perform(
            async move {
                store
                    .edit_reminder(record.account, record.id, record.generation, retry)
                    .await
            },
            move |result| event(Message::Saved(generation, result)),
        )
    }
    pub fn subscription(&self) -> Subscription<super::Message> {
        let Some(client) = self.client.as_ref() else {
            return Subscription::none();
        };
        Subscription::run_with(
            Events {
                id: client.id,
                receiver: client.events.clone(),
            },
            |events| {
                stream::unfold(events.receiver.clone(), |receiver| async move {
                    receiver.recv().await.ok().map(|action| {
                        (
                            super::Message::Reminder(Message::Desktop(Payload(action))),
                            receiver,
                        )
                    })
                })
            },
        )
    }
    pub fn view(&self) -> Element<'_, super::Message> {
        let mut content=column![widget::text::title1("Reminders"),widget::text("Keep Mailstronaut open for reminders. Reading a message does not cancel its reminder."),widget::text(&self.status)].spacing(12).padding(20);
        if let Some(target) = &self.target {
            content = content
                .push(widget::text::title3(&target.title))
                .push(widget::text(format!("{} · {}", target.account, self.zone)))
                .push(
                    row![
                        button("In one hour", Message::Preset(When::Hour), !self.busy),
                        button("This evening", Message::Preset(When::Evening), !self.busy),
                        button(
                            "Tomorrow morning",
                            Message::Preset(When::Tomorrow),
                            !self.busy
                        )
                    ]
                    .spacing(8),
                )
                .push(widget::calendar::calendar(
                    &self.calendar,
                    |day| super::Message::Reminder(Message::Day(day)),
                    || super::Message::Reminder(Message::Month(false)),
                    || super::Message::Reminder(Message::Month(true)),
                    jiff::civil::Weekday::Monday,
                ))
                .push(
                    row![
                        widget::text_input("HH:MM", &self.time)
                            .on_input(|time| super::Message::Reminder(Message::Time(Payload(time))))
                            .width(120),
                        button("Set custom reminder", Message::Custom, !self.busy)
                    ]
                    .spacing(8),
                );
        } else {
            content = content.push(widget::text(
                "Open a message and choose Remind to schedule it.",
            ));
        }
        if self
            .config
            .as_ref()
            .is_some_and(|config| !config.notifications.enabled)
        {
            content = content.push(widget::text(
                "Desktop notifications are disabled. Due reminders remain here.",
            ));
        }
        content = content.push(button("Refresh reminders", Message::Refresh, !self.busy));
        let mut list = column![].spacing(10);
        for record in &self.records {
            let status = if record.available {
                match record.state {
                    State::Pending if record.due_at <= now() => "Overdue",
                    State::Pending => "Scheduled",
                    State::Delivered => "Accepted by desktop",
                    State::Delivering => "Delivering",
                    State::Failed => "Notification unavailable",
                    State::Cancelled => "Cancelled",
                    State::Unavailable => "Unavailable",
                }
            } else {
                "Message unavailable"
            };
            list = list.push(
                column![
                    widget::text(record.title.chars().take(120).collect::<String>()),
                    widget::text(format!(
                        "{} · {} · {}",
                        record.account,
                        reminders::display(record.due_at, &record.time_zone)
                            .unwrap_or_else(|_| "Invalid stored time".into()),
                        status
                    )),
                    row![
                        button(
                            "Open message",
                            Message::Open(Payload(record.clone())),
                            !self.busy && record.available
                        ),
                        button(
                            "Cancel",
                            Message::Edit(Payload(record.clone()), false),
                            !self.busy
                        ),
                        button(
                            "Retry notification",
                            Message::Edit(Payload(record.clone()), true),
                            !self.busy && record.state == State::Failed && record.available
                        )
                    ]
                    .spacing(8)
                ]
                .spacing(4),
            );
        }
        widget::scrollable(content.push(list))
            .height(Length::Fill)
            .into()
    }
}
async fn resolve_original(
    store: worker::Handle,
    accounts: Vec<String>,
    record: Reminder,
) -> Result<Option<Reminder>, worker::Error> {
    if !accounts
        .into_iter()
        .any(|account| account == record.account)
    {
        return Ok(None);
    }
    let current = store.reminder_for(record.account, record.message).await?;
    Ok(
        current
            .filter(|current| current.id == record.id && current.generation == record.generation),
    )
}
fn button(label: &str, message: Message, enabled: bool) -> Element<'_, super::Message> {
    widget::button::text(label)
        .on_press_maybe(enabled.then_some(super::Message::Reminder(message)))
        .into()
}
const fn event(message: Message) -> Action<super::Message> {
    Action::App(super::Message::Reminder(message))
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
#[derive(Clone)]
struct Events {
    id: u64,
    receiver: async_channel::Receiver<desktop::Action>,
}
impl PartialEq for Events {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Events {}
impl core::hash::Hash for Events {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(&self.id, state);
    }
}
async fn step(
    store: worker::Handle,
    config: Arc<Config>,
    target: Option<Target>,
    mut client: Option<Arc<desktop::Client>>,
    cancel: CancellationToken,
) -> Result<Snapshot, String> {
    if client.as_ref().is_some_and(|client| !client.healthy()) {
        client = None;
    }
    let accounts = config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    let mut status = String::new();
    if !cancel.is_cancelled() {
        let withdrawals = store
            .reminder_withdrawals(accounts.clone())
            .await
            .map_err(|error| error.to_string())?;
        let next = store
            .next_reminder(accounts.clone())
            .await
            .map_err(|error| error.to_string())?;
        if !withdrawals.is_empty()
            || (config.notifications.enabled && next.is_some_and(|due| due <= now()))
        {
            if client.is_none() {
                match desktop::bounded(desktop::Client::connect()).await {
                    Ok(connected) => client = Some(connected),
                    Err(error) => status = error.into(),
                }
            }
            if let Some(desktop) = client.as_ref() {
                for record in withdrawals {
                    if cancel.is_cancelled() {
                        break;
                    }
                    match desktop::bounded(desktop.withdraw(&record)).await {
                        Ok(()) => store
                            .withdrew_reminder(record)
                            .await
                            .map_err(|error| error.to_string())?,
                        Err(error) => {
                            status = error.into();
                            break;
                        }
                    }
                }
                for _ in 0..if config.notifications.enabled { 4 } else { 0 } {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let Some(attempt) = store
                        .claim_reminder(accounts.clone(), now())
                        .await
                        .map_err(|error| error.to_string())?
                    else {
                        break;
                    };
                    let accepted = match desktop::bounded(desktop.deliver(
                        &attempt.reminder,
                        &store,
                        config.notifications.show_preview,
                    ))
                    .await
                    {
                        Ok(()) => true,
                        Err(error) => {
                            status = error.into();
                            false
                        }
                    };
                    store
                        .finish_reminder(attempt, accepted, now())
                        .await
                        .map_err(|error| error.to_string())?;
                    if !accepted {
                        break;
                    }
                }
            }
        }
    }
    snapshot(&store, accounts, target, client, status).await
}
async fn snapshot(
    store: &worker::Handle,
    accounts: Vec<String>,
    target: Option<Target>,
    client: Option<Arc<desktop::Client>>,
    status: String,
) -> Result<Snapshot, String> {
    let records = store
        .reminders(accounts.clone())
        .await
        .map_err(|error| error.to_string())?;
    let next = store
        .next_reminder(accounts)
        .await
        .map_err(|error| error.to_string())?;
    let selected = if let Some(target) = target {
        store
            .reminder_for(target.account, target.message)
            .await
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    Ok(Snapshot {
        records,
        selected,
        next,
        client,
        status,
    })
}

#[cfg(test)]
mod tests;
