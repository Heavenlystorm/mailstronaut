//! Remind me: scheduling, desktop delivery and the in-app list of reminders.
//!
//! The store owns durable state and every transition. This loop only wakes at the
//! earliest due instant. A muted or missing notification service leaves the reminder
//! visible here, so a reminder is never silently lost to the desktop.

pub mod desktop;

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, Disableable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    date_picker::{DatePicker, DatePickerState, DateTime},
    description_list::DescriptionList,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    label::Label,
    time_field::TimePrecision,
};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use mailstronaut_config::Config;
use mailstronaut_core::{
    reminders::{self, When},
    store::reminders::{Reminder, Schedule, State},
    worker::Handle,
};
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc, watch},
};
use tokio_util::sync::CancellationToken;

/// A sleeping loop still rechecks this often, in case the clock or timezone moved.
const MAX_SLEEP: Duration = Duration::from_secs(600);

pub enum Event {
    Open(Reminder),
    Changed,
}

enum Update {
    Snapshot(Vec<Reminder>, String),
    Open(Reminder),
    Error(String),
}

/// The scheduler and its event relay do not depend on a GPUI window. The owner
/// must keep consuming open requests, including while no window is present.
struct Service {
    cancel: CancellationToken,
    state: watch::Receiver<Option<Snapshot>>,
    events: async_channel::Receiver<Update>,
}

#[derive(Clone)]
struct Snapshot(Vec<Reminder>, String);

impl Service {
    fn start(
        runtime: &tokio::runtime::Handle,
        store: Handle,
        config: Arc<Config>,
        wake: Arc<Notify>,
        parent: &CancellationToken,
    ) -> Self {
        let cancel = parent.child_token();
        let (updates, mut receiver) = mpsc::channel(8);
        let (state, state_receiver) = watch::channel(None);
        let (events, event_receiver) = async_channel::bounded(8);
        runtime.spawn(engine(store, config, wake, updates, cancel.clone()));
        let relay_cancel = cancel.clone();
        runtime.spawn(async move {
            loop {
                let update = tokio::select! {
                    () = relay_cancel.cancelled() => break,
                    update = receiver.recv() => match update {
                        Some(update) => update,
                        None => break,
                    },
                };
                match update {
                    Update::Snapshot(records, status) => {
                        state.send_replace(Some(Snapshot(records, status)));
                    }
                    event @ (Update::Error(_) | Update::Open(_)) => {
                        tokio::select! {
                            () = relay_cancel.cancelled() => break,
                            result = events.send(event) => if result.is_err() { break; },
                        }
                    }
                }
            }
        });
        Self {
            cancel,
            state: state_receiver,
            events: event_receiver,
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Application-owned reminder state. Unlike the picker, this entity is not tied
/// to a native window and can retain the scheduler across view replacement.
pub struct Session {
    runtime: Arc<Runtime>,
    lifetime: CancellationToken,
    service: Option<Service>,
    generation: u64,
    wake: Arc<Notify>,
    records: Vec<Reminder>,
    status: SharedString,
    error: Option<SharedString>,
}

enum SessionEvent {
    Changed,
    Open(Reminder),
    Error(String),
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            lifetime: CancellationToken::new(),
            service: None,
            generation: 0,
            wake: Arc::default(),
            records: Vec::new(),
            status: "".into(),
            error: None,
        }
    }

    pub(crate) fn configure(
        &mut self,
        config: &Arc<Config>,
        store: Option<Handle>,
        cx: &mut Context<Self>,
    ) {
        self.service = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.records.clear();
        self.status = "".into();
        self.error = None;
        cx.emit(SessionEvent::Changed);
        cx.notify();
        let Some(store) = store else {
            return;
        };
        let service = Service::start(
            self.runtime.handle(),
            store,
            config.clone(),
            self.wake.clone(),
            &self.lifetime,
        );
        let mut state = service.state.clone();
        let events = service.events.clone();
        self.service = Some(service);
        cx.spawn(async move |this, cx| {
            let mut watching_state = true;
            loop {
                let update = tokio::select! {
                    changed = state.changed(), if watching_state => {
                        if changed.is_err() { watching_state = false; continue; }
                        let snapshot = state.borrow_and_update().clone();
                        match snapshot {
                            Some(Snapshot(records, status)) => Update::Snapshot(records, status),
                            None => continue,
                        }
                    }
                    event = events.recv() => match event {
                        Ok(event) => event,
                        Err(_) => break,
                    }
                };
                let applied = this.update(cx, |this, cx| {
                    if generation != this.generation {
                        return;
                    }
                    match update {
                        Update::Snapshot(records, status) => {
                            this.records = records;
                            this.status = status.into();
                            cx.emit(SessionEvent::Changed);
                            cx.notify();
                        }
                        Update::Open(record) => cx.emit(SessionEvent::Open(record)),
                        Update::Error(error) => {
                            this.error = Some(error.clone().into());
                            cx.emit(SessionEvent::Error(error));
                            cx.notify();
                        }
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(test)]
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn has_message(&self, message: i64) -> bool {
        self.records.iter().any(|record| record.message == message)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.lifetime.cancel();
    }
}

pub struct Reminders {
    runtime: Arc<Runtime>,
    lifetime: CancellationToken,
    session: Entity<Session>,
    _session_events: Subscription,
    config: Option<Arc<Config>>,
    store: Option<Handle>,
    zone: String,
    records: Vec<Reminder>,
    status: SharedString,
    error: Option<SharedString>,
    picker: Entity<DatePickerState>,
}

impl EventEmitter<Event> for Reminders {}

impl Drop for Reminders {
    fn drop(&mut self) {
        self.lifetime.cancel();
    }
}

impl Reminders {
    pub fn new(
        runtime: Arc<Runtime>,
        lifetime: CancellationToken,
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| {
            DatePickerState::new(window, cx)
                .time_precision(TimePrecision::Minute)
                .date_format("%Y-%m-%d %H:%M")
        });
        let session_events = cx.subscribe(&session, |this, session, event, cx| match event {
            SessionEvent::Changed => {
                let session = session.read(cx);
                this.records.clone_from(&session.records);
                this.status.clone_from(&session.status);
                cx.emit(Event::Changed);
                cx.notify();
            }
            SessionEvent::Open(record) => cx.emit(Event::Open(record.clone())),
            SessionEvent::Error(error) => {
                this.error = Some(error.clone().into());
                cx.emit(Event::Changed);
                cx.notify();
            }
        });
        let records = session.read(cx).records.clone();
        let status = session.read(cx).status.clone();
        let error = session.read(cx).error.clone();
        Self {
            runtime,
            session,
            _session_events: session_events,
            lifetime,
            config: None,
            store: None,
            zone: "UTC".into(),
            records,
            status,
            error,
            picker,
        }
    }

    pub fn configure(
        &mut self,
        config: &Arc<Config>,
        store: Option<Handle>,
        zone: String,
        cx: &mut Context<Self>,
    ) {
        self.config = Some(config.clone());
        self.store.clone_from(&store);
        self.zone = zone;
        self.session
            .update(cx, |session, cx| session.configure(config, store, cx));
    }

    /// Reminders the user has not dealt with yet, for the sidebar badge.
    pub fn overdue(&self) -> usize {
        let now = now();
        self.records
            .iter()
            .filter(|record| {
                record.due_at <= now && matches!(record.state, State::Pending | State::Delivered)
            })
            .count()
    }

    pub fn is_scheduled(&self, account: &str, message: i64) -> bool {
        self.records.iter().any(|record| {
            record.account == account
                && record.message == message
                && matches!(record.state, State::Pending | State::Delivering)
        })
    }

    /// Schedule or reschedule one exact original, from a preset the user chose.
    pub fn schedule(
        &mut self,
        account: String,
        message: i64,
        when: &When,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(store), Some(config)) = (self.store.clone(), self.config.clone()) else {
            return;
        };
        let zone = self.zone.clone();
        let at = reminders::resolve(
            now(),
            &zone,
            when,
            &config.reminders.morning,
            &config.reminders.evening,
        );
        let due_at = match at {
            Ok(at) => at,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.emit(Event::Changed);
                cx.notify();
                return;
            }
        };
        let job = self.runtime.spawn(async move {
            let expected = store
                .reminder_for(account.clone(), message)
                .await?
                .map(|record| record.generation);
            store
                .schedule_reminder(Schedule {
                    account,
                    message,
                    due_at,
                    time_zone: zone,
                    now: now(),
                    expected,
                })
                .await
        });
        let wake = self.session.read(cx).wake.clone();
        let label = reminders::display(due_at, &self.zone).unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                match result {
                    Ok(Ok(_)) => {
                        this.error = None;
                        this.session.update(cx, |session, _| session.error = None);
                        this.status = format!("Reminder set for {label}.").into();
                        wake.notify_one();
                    }
                    Ok(Err(error)) => this.error = Some(error.to_string().into()),
                    Err(_) => this.error = Some("Setting the reminder was interrupted.".into()),
                }
                cx.emit(Event::Changed);
                cx.notify();
            });
        })
        .detach();
    }

    /// Ask for an exact local time; the persistent picker survives dialog rebuilds.
    pub fn ask_custom(
        &self,
        account: String,
        message: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (this, picker) = (cx.entity().downgrade(), self.picker.clone());
        let zone = self.zone.clone();
        // Each opening starts with this original's future reminder, or tomorrow's
        // configured morning. Never reuse a previous original's picker value.
        if let Some(config) = &self.config {
            let now = now();
            let at = self
                .records
                .iter()
                .find(|record| record.account == account && record.message == message)
                .filter(|record| record.due_at > now)
                .map(|record| record.due_at)
                .or_else(|| {
                    reminders::resolve(
                        now,
                        &zone,
                        &When::Tomorrow,
                        &config.reminders.morning,
                        &config.reminders.evening,
                    )
                    .ok()
                });
            if let (Some(at), Ok(zone)) = (
                at.and_then(|at| chrono::DateTime::from_timestamp(at, 0)),
                zone.parse::<chrono_tz::Tz>(),
            ) {
                picker.update(cx, |picker, cx| {
                    picker.set_date_time(at.with_timezone(&zone).naive_local(), window, cx);
                });
            }
        }
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (this, picker2, account) = (this.clone(), picker.clone(), account.clone());
            alert
                .confirm()
                .title("Remind me at")
                .description(format!("Times are in {zone}."))
                .child(DatePicker::new(&picker).placeholder("Choose a date and time"))
                .ok_text("Set reminder")
                .on_ok(move |_, window, cx| {
                    let value = picker2.read(cx).date_time();
                    let DateTime::Single(Some(local)) = value else {
                        return false;
                    };
                    _ = this.update(cx, |this, cx| {
                        this.schedule(account.clone(), message, &When::Custom(local), window, cx);
                    });
                    true
                })
        });
    }

    fn change(&self, record: &Reminder, retry: bool, window: &Window, cx: &Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let (account, id, generation) =
            (record.account.clone(), record.id.clone(), record.generation);
        let job = self
            .runtime
            .spawn(async move { store.edit_reminder(account, id, generation, retry).await });
        let wake = self.session.read(cx).wake.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if !matches!(result, Ok(Ok(()))) {
                    this.error = Some("This reminder changed. Try again.".into());
                }
                wake.notify_one();
                cx.notify();
            });
        })
        .detach();
    }

    /// The list may be a moment behind the store: only open what is still current.
    fn open_current(&self, record: Reminder, window: &Window, cx: &Context<Self>) {
        let (Some(store), Some(config)) = (self.store.clone(), self.config.clone()) else {
            return;
        };
        let accounts: Vec<_> = config.accounts.iter().map(|item| item.id.clone()).collect();
        let job = self
            .runtime
            .spawn(async move { resolve_original(store, accounts, record).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if let Ok(Ok(Some(record))) = result {
                    cx.emit(Event::Open(record));
                } else {
                    this.error = Some("This reminder changed. Try again.".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn row(
        index: usize,
        record: &Reminder,
        columns: usize,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let state = match record.state {
            State::Pending if record.due_at <= now() => "Overdue",
            State::Pending | State::Delivering => "Scheduled",
            State::Delivered => "Notified",
            State::Failed => "Notification failed",
            State::Cancelled => "Cancelled",
            State::Unavailable => "Original unavailable",
        };
        let (open, cancel, retry) = (record.clone(), record.clone(), record.clone());
        let reschedule = record.clone();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(Label::new(record.title.clone()).truncate())
            .child(
                Label::new(format!("{state} · {}", record.account))
                    .text_xs()
                    .text_color(muted),
            )
            .child(Self::details(record, columns))
            .when(!record.available, |view| {
                view.child(
                    Label::new("The original message is no longer available.")
                        .text_xs()
                        .text_color(muted),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(("open-reminder", index))
                            .ghost()
                            .label("Open")
                            .disabled(!record.available)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_current(open.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new(("reschedule-reminder", index))
                            .ghost()
                            .label("Reschedule")
                            .disabled(!record.available)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.ask_custom(
                                    reschedule.account.clone(),
                                    reschedule.message,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .when(record.state == State::Failed, |view| {
                        view.child(
                            Button::new(("retry-reminder", index))
                                .ghost()
                                .label("Retry")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.change(&retry, true, window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new(("cancel-reminder", index))
                            .ghost()
                            .label(if record.state == State::Delivered {
                                "Dismiss"
                            } else {
                                "Cancel"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(&cancel, false, window, cx);
                            })),
                    ),
            )
    }

    fn details(record: &Reminder, columns: usize) -> DescriptionList {
        let utc = chrono::DateTime::from_timestamp(record.due_at, 0).map_or_else(
            || "Unavailable".into(),
            |at| at.format("%Y-%m-%d %H:%M UTC").to_string(),
        );
        DescriptionList::vertical()
            .columns(columns)
            .item(
                "Local time",
                reminders::display(record.due_at, &record.time_zone)
                    .unwrap_or_else(|_| "Unavailable".into()),
                1,
            )
            .item("Time zone", record.time_zone.clone(), 1)
            .item("UTC", utc, 1)
    }
}

impl Render for Reminders {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = if window.viewport_size().width < gpui::px(1100.) {
            1
        } else {
            3
        };
        let visible: Vec<_> = self
            .records
            .iter()
            .filter(|record| record.state != State::Cancelled)
            .cloned()
            .collect();
        div()
            .id("reminder-list")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_lg().font_semibold().child("Reminders"))
            .when_some(self.error.clone(), |view, error| {
                view.child(Alert::error("reminder-error", error).on_close(cx.listener(
                    |this, _, _, cx| {
                        this.error = None;
                        cx.notify();
                    },
                )))
            })
            .when(!self.status.is_empty(), |view| {
                view.child(
                    Label::new(self.status.clone())
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .when(visible.is_empty(), |view| {
                view.child(
                    Empty::new().header(
                        EmptyHeader::new()
                            .title(EmptyTitle::new().child("No reminders"))
                            .description(
                                EmptyDescription::new()
                                    .child("Open a message and choose Remind me."),
                            ),
                    ),
                )
            })
            .children(
                visible
                    .iter()
                    .enumerate()
                    .map(|(index, record)| Self::row(index, record, columns, cx)),
            )
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or_default()
}

/// How long to sleep before the next due reminder.
fn sleep_for(next: Option<i64>, now: i64) -> Duration {
    next.map_or(MAX_SLEEP, |due| {
        Duration::from_secs(u64::try_from(due.saturating_sub(now)).unwrap_or(0).max(1))
            .min(MAX_SLEEP)
    })
}

async fn resolve_original(
    store: Handle,
    accounts: Vec<String>,
    record: Reminder,
) -> Result<Option<Reminder>, mailstronaut_core::worker::Error> {
    if !accounts.contains(&record.account) {
        return Ok(None);
    }
    let current = store.reminder_for(record.account, record.message).await?;
    Ok(
        current
            .filter(|current| current.id == record.id && current.generation == record.generation),
    )
}

/// Resolve and apply notification actions without requiring a window. Every
/// mutation uses the generation that the notification actually identified.
async fn handle_action(
    store: &Handle,
    config: &Config,
    action: desktop::Action,
) -> Result<Option<Reminder>, &'static str> {
    let accounts = config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect();
    let Some(record) = store
        .reminder_action(accounts, action.notification, action.generation)
        .await
        .map_err(|_| "Unable to resolve the reminder action.")?
    else {
        return Ok(None);
    };
    match action.kind {
        desktop::Kind::Open => Ok(Some(record)),
        desktop::Kind::Dismiss => {
            store
                .edit_reminder(record.account, record.id, record.generation, false)
                .await
                .map_err(|_| "This reminder changed. Try again.")?;
            Ok(None)
        }
        desktop::Kind::Again => {
            let current = now();
            let zone = crate::mailbox::local_zone(config.time_zone.as_deref());
            let due_at = reminders::resolve(
                current,
                &zone,
                &When::Hour,
                &config.reminders.morning,
                &config.reminders.evening,
            )
            .map_err(|_| "Unable to reschedule the reminder.")?;
            store
                .schedule_reminder(Schedule {
                    account: record.account,
                    message: record.message,
                    due_at,
                    time_zone: zone,
                    now: current,
                    expected: Some(record.generation),
                })
                .await
                .map_err(|_| "This reminder changed. Try again.")?;
            Ok(None)
        }
    }
}

async fn engine(
    store: Handle,
    config: Arc<Config>,
    wake: Arc<Notify>,
    updates: mpsc::Sender<Update>,
    cancel: CancellationToken,
) {
    let mut client = None;
    loop {
        let (records, status, next) = step(&store, &config, &mut client, &cancel).await;
        if !crate::cancellable_send::send(&updates, Update::Snapshot(records, status), &cancel)
            .await
        {
            return;
        }
        let events = client
            .as_ref()
            .map(|client: &Arc<desktop::Client>| client.events.clone());
        let action = async {
            match events {
                Some(events) => events.recv().await.ok(),
                None => core::future::pending().await,
            }
        };
        tokio::select! {
            () = cancel.cancelled() => return,
            () = wake.notified() => {}
            () = tokio::time::sleep(sleep_for(next, now())) => {}
            action = action => {
                if let Some(action) = action {
                    let update = match handle_action(&store, &config, action).await {
                        Ok(Some(record)) => Some(Update::Open(record)),
                        Ok(None) => None,
                        Err(error) => Some(Update::Error(error.into())),
                    };
                    if let Some(update) = update && !crate::cancellable_send::send(&updates, update, &cancel).await { return; }
                }
            }
        }
    }
}

async fn step(
    store: &Handle,
    config: &Config,
    client: &mut Option<Arc<desktop::Client>>,
    cancel: &CancellationToken,
) -> (Vec<Reminder>, String, Option<i64>) {
    if client.as_ref().is_some_and(|client| !client.healthy()) {
        *client = None;
    }
    let accounts: Vec<_> = config.accounts.iter().map(|item| item.id.clone()).collect();
    let mut status = String::new();
    let withdrawals = store
        .reminder_withdrawals(accounts.clone())
        .await
        .unwrap_or_default();
    let next = store.next_reminder(accounts.clone()).await.ok().flatten();
    let due = config.notifications.enabled && next.is_some_and(|at| at <= now());
    if (!withdrawals.is_empty() || due) && client.is_none() {
        match desktop::bounded(desktop::Client::connect()).await {
            Ok(connected) => *client = Some(connected),
            Err(error) => error.clone_into(&mut status),
        }
    }
    if let Some(desktop) = client.as_ref() {
        for record in withdrawals {
            if cancel.is_cancelled() {
                break;
            }
            match desktop::bounded(desktop.withdraw(&record)).await {
                Ok(()) => {
                    _ = store.withdrew_reminder(record).await;
                }
                Err(error) => {
                    error.clone_into(&mut status);
                    break;
                }
            }
        }
        for _ in 0..if config.notifications.enabled { 4 } else { 0 } {
            if cancel.is_cancelled() {
                break;
            }
            let Ok(Some(attempt)) = store.claim_reminder(accounts.clone(), now()).await else {
                break;
            };
            let delivered = desktop::bounded(desktop.deliver(
                &attempt.reminder,
                store,
                config.notifications.show_preview,
            ))
            .await;
            let accepted = delivered.is_ok();
            if let Err(error) = delivered {
                error.clone_into(&mut status);
            }
            _ = store.finish_reminder(attempt, accepted, now()).await;
            if !accepted {
                break;
            }
        }
    }
    let records = store.reminders(accounts.clone()).await.unwrap_or_default();
    let next = store.next_reminder(accounts).await.ok().flatten();
    (records, status, next)
}

#[cfg(test)]
mod portal_tests;

#[cfg(test)]
mod tests {
    use super::{Duration, MAX_SLEEP, sleep_for};

    #[test]
    fn the_loop_wakes_for_the_earliest_reminder_and_never_sleeps_unbounded() {
        assert_eq!(sleep_for(None, 100), MAX_SLEEP);
        assert_eq!(sleep_for(Some(130), 100), Duration::from_secs(30));
        assert_eq!(sleep_for(Some(100 + 86_400), 100), MAX_SLEEP);
        // Overdue work retries soon rather than spinning.
        assert_eq!(sleep_for(Some(50), 100), Duration::from_secs(1));
    }

    #[test]
    fn scheduler_keeps_current_state_without_a_window_subscriber() {
        use mailstronaut_core::{store::Store, worker::Worker};
        use std::sync::Arc;
        use tokio::sync::Notify;
        use tokio_util::sync::CancellationToken;

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let root = tempfile::tempdir().expect("private profile");
            let mut store = Store::open(root.path()).expect("store");
            store.ensure_account("a").expect("account");
            let message = store.ingest_message("a", b"Subject: Synthetic\r\n\r\nFixture.").expect("message");
            drop(store);
            let owner = Worker::open(root.path().to_owned()).await.expect("worker");
            let handle = owner.handle();
            let config = serde_json::from_value(serde_json::json!({"schemaVersion":1,"accounts":[{"id":"a","provider":"icloud","address":"a@example.test","displayName":"A","userName":"a","auth":{"kind":"password","secretRef":"a"}}]})).expect("config");
            let wake = Arc::new(Notify::new());
            let service = super::Service::start(runtime.handle(), handle.clone(), Arc::new(config), wake.clone(), &CancellationToken::new());
            let mut initial = service.state.clone();
            tokio::time::timeout(Duration::from_secs(2), initial.changed()).await.expect("initial snapshot").expect("service alive");
            drop(initial); // No window is listening from this point.
            handle.schedule_reminder(super::Schedule {
                account: "a".into(), message, due_at: super::now() + 3600,
                time_zone: "UTC".into(), now: super::now(), expected: None,
            }).await.expect("schedule");
            wake.notify_one();
            let mut reopened = service.state.clone();
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let ready = reopened.borrow_and_update().as_ref().is_some_and(|snapshot| snapshot.0.iter().any(|record| record.message == message));
                    if ready { break; }
                    reopened.changed().await.expect("service alive");
                }
            }).await.expect("background scheduler refreshed state");
            drop(service);
            owner.shutdown().await.expect("shutdown");
        });
    }

    #[tokio::test]
    async fn reminder_engine_stops_when_its_update_consumer_stalls() {
        use mailstronaut_core::worker::Worker;
        use std::sync::Arc;
        use tokio::sync::{Notify, mpsc};
        use tokio_util::sync::CancellationToken;

        let root = tempfile::tempdir().expect("private profile");
        let owner = Worker::open(root.path().to_owned()).await.expect("worker");
        let config = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "accounts": []
        }))
        .expect("config");
        let (updates, mut receiver) = mpsc::channel(1);
        updates
            .send(super::Update::Error("occupied".into()))
            .await
            .expect("fill channel");
        let cancel = CancellationToken::new();
        let task = tokio::spawn(super::engine(
            owner.handle(),
            Arc::new(config),
            Arc::new(Notify::new()),
            updates,
            cancel.clone(),
        ));
        // The initial store read completes before the first snapshot can be sent.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!task.is_finished());
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("engine must stop")
            .expect("task");
        assert!(matches!(
            receiver.recv().await,
            Some(super::Update::Error(_))
        ));
        owner.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn notification_actions_are_fenced_and_do_not_need_a_window() {
        use super::{Schedule, State, desktop, handle_action, now};
        use mailstronaut_core::{store::Store, worker::Worker};
        let root = tempfile::tempdir().expect("private profile");
        let mut store = Store::open(root.path()).expect("store");
        store.ensure_account("a").expect("account");
        let message = store
            .ingest_message("a", b"Subject: Synthetic\r\n\r\nFixture.")
            .expect("message");
        drop(store);
        let owner = Worker::open(root.path().to_owned()).await.expect("worker");
        let handle = owner.handle();
        let config: mailstronaut_config::Config = serde_json::from_value(serde_json::json!({"schemaVersion":1,"timeZone":"UTC","accounts":[{"id":"a","provider":"icloud","address":"a@example.test","displayName":"A","userName":"a","auth":{"kind":"password","secretRef":"a"}}]})).expect("config");
        let first = handle
            .schedule_reminder(Schedule {
                account: "a".into(),
                message,
                due_at: now() + 120,
                time_zone: "UTC".into(),
                now: now(),
                expected: None,
            })
            .await
            .expect("schedule");
        let action = |generation, kind| desktop::Action {
            notification: first.notification.clone(),
            generation,
            kind,
        };
        assert!(
            handle_action(
                &handle,
                &config,
                action(first.generation, desktop::Kind::Open)
            )
            .await
            .expect("open")
            .is_some()
        );
        assert!(
            handle_action(
                &handle,
                &config,
                action(first.generation, desktop::Kind::Again)
            )
            .await
            .expect("reschedule")
            .is_none()
        );
        let next = handle
            .reminder_for("a".into(), message)
            .await
            .expect("read")
            .expect("reminder");
        assert_eq!(next.generation, first.generation + 1);
        assert!((3500..=3600).contains(&(next.due_at - now())));
        assert!(
            handle_action(
                &handle,
                &config,
                action(first.generation, desktop::Kind::Dismiss)
            )
            .await
            .expect("stale ignored")
            .is_none()
        );
        assert_eq!(
            handle
                .reminder_for("a".into(), message)
                .await
                .expect("read")
                .expect("reminder")
                .state,
            State::Pending
        );
        handle_action(
            &handle,
            &config,
            action(next.generation, desktop::Kind::Dismiss),
        )
        .await
        .expect("dismiss");
        assert_eq!(
            handle
                .reminder_for("a".into(), message)
                .await
                .expect("read")
                .expect("reminder")
                .state,
            State::Cancelled
        );
        owner.shutdown().await.expect("shutdown");
    }
}
