//! Native plain-text compose, transactional autosave and explicit outbox submission.

use std::{sync::Arc, time::Duration};

use cosmic::{
    Action, Element,
    app::Task,
    iced::widget::{column, row},
    widget::{self, text_editor},
};
use mailstronaut_config::{Account, Config, Provider};
use mailstronaut_core::{
    store::outbox::{Draft, DraftSummary, Phase, Summary},
    worker::{self, Handle},
};
use mailstronaut_mail::{
    auth::Session,
    compose,
    smtp::{Outcome, Transport},
};
use tokio_util::sync::CancellationToken;

use crate::private::Private;

#[derive(Clone, Copy, Debug)]
pub enum Field {
    To,
    Cc,
    Bcc,
    Subject,
}
#[derive(Clone, Debug)]
pub enum Switch {
    New,
    Prepared(Private<Box<Draft>>),
    Sender(usize),
    Draft(String, String),
    Recover(String, String),
}
#[derive(Clone, Debug)]
pub enum Message {
    Switch(Switch),
    Switched(u64, Switch, Result<(), worker::Error>),
    Loaded(u64, Private<Result<Box<Draft>, worker::Error>>),
    Field(Field, Private<String>),
    Body(Private<text_editor::Action>),
    Save,
    Saved(u64, String, u64, Result<(), worker::Error>),
    Refresh,
    Lists(u64, Private<Result<Lists, worker::Error>>),
    Send,
    Submit(String, String, Phase, bool),
    Cancel,
    CancelQueued(String, String),
    QueueCancelled(u64, Result<(), worker::Error>),
    Sent(u64, Private<SendResult>),
    Noop,
}
#[derive(Clone)]
pub enum SendResult {
    Draft(Box<Draft>, String),
    Submitted(Result<Outcome, String>),
}
#[derive(Clone)]
pub struct Lists {
    drafts: Vec<DraftSummary>,
    outgoing: Vec<Summary>,
}
struct Identity {
    account: String,
    from: String,
}

#[derive(Default)]
pub struct Composer {
    config: Option<Arc<Config>>,
    store: Option<Handle>,
    identities: Vec<Identity>,
    labels: Vec<String>,
    selected: Option<usize>,
    draft: Option<Draft>,
    editor: text_editor::Content<cosmic::Renderer>,
    saved_revision: u64,
    generation: u64,
    loading: bool,
    sending: bool,
    autosave: CancellationToken,
    cancellation: CancellationToken,
    drafts: Vec<DraftSummary>,
    outgoing: Vec<Summary>,
    status: String,
}
impl Composer {
    pub fn configure(&mut self, config: Option<Arc<Config>>) -> Task<super::Message> {
        self.generation = self.generation.saturating_add(1);
        self.config = config;
        self.identities.clear();
        self.labels.clear();
        if let Some(config) = &self.config {
            for account in &config.accounts {
                for address in std::iter::once(&account.address).chain(&account.aliases) {
                    if let Ok(from) = compose::sender(address, &account.display_name) {
                        self.labels.push(format!("{from} · {}", account.id));
                        self.identities.push(Identity {
                            account: account.id.clone(),
                            from,
                        });
                    }
                }
            }
        }
        self.selected = self.draft.as_ref().map_or_else(
            || (!self.identities.is_empty()).then_some(0),
            |draft| {
                self.identities.iter().position(|identity| {
                    identity.account == draft.account && identity.from == draft.content.from
                })
            },
        );
        Task::batch([self.save(), self.refresh()])
    }
    pub fn attach(&mut self, store: Option<Handle>) -> Task<super::Message> {
        if self.store.is_none() && store.is_some() {
            self.store = store;
            return self.refresh();
        }
        Task::none()
    }
    pub const fn busy(&self) -> bool {
        self.loading || self.sending
    }
    pub fn cancel(&self) {
        self.autosave.cancel();
        self.cancellation.cancel();
    }
    pub fn status(&mut self, message: String) {
        self.status = message;
    }

    pub fn update(&mut self, message: Message, keyring: Option<Session>) -> Task<super::Message> {
        match message {
            Message::Switch(target) if !self.busy() => return self.request_switch(target),
            Message::Switched(generation, target, result) if generation == self.generation => {
                self.loading = false;
                match result {
                    Ok(()) => return self.switch(target),
                    Err(error) => self.status = error.to_string(),
                }
            }
            Message::Loaded(generation, Private(result)) if generation == self.generation => {
                self.loading = false;
                match result {
                    Ok(draft) => self.load(*draft),
                    Err(error) => self.status = error.to_string(),
                }
            }
            Message::Field(field, Private(value)) if !self.busy() => {
                if let Some(draft) = &mut self.draft {
                    match field {
                        Field::To => draft.content.to = value,
                        Field::Cc => draft.content.cc = value,
                        Field::Bcc => draft.content.bcc = value,
                        Field::Subject => draft.content.subject = value,
                    }
                    return self.edited();
                }
            }
            Message::Body(Private(action)) if !self.busy() => {
                let edited = action.is_edit();
                self.editor.perform(action);
                if edited && let Some(draft) = &mut self.draft {
                    draft.content.body = self.editor.text();
                    return self.edited();
                }
            }
            Message::Save => return self.save(),
            Message::Saved(generation, id, revision, result) if generation == self.generation => {
                return self.saved(&id, revision, result);
            }
            Message::Refresh => return self.refresh(),
            Message::Lists(generation, Private(result)) if generation == self.generation => {
                match result {
                    Ok(lists) => {
                        self.drafts = lists.drafts;
                        self.outgoing = lists.outgoing;
                    }
                    Err(error) => self.status = error.to_string(),
                }
            }
            Message::Send if !self.busy() => return self.send(keyring),
            Message::Submit(account, id, phase, consent) if !self.busy() => {
                return self.submit(&account, id, phase, consent, keyring);
            }
            Message::Cancel => self.cancellation.cancel(),
            Message::CancelQueued(account, id) if !self.busy() => {
                let Some(store) = self.store.clone() else {
                    return Task::none();
                };
                self.loading = true;
                let generation = self.generation;
                return Task::perform(
                    async move { store.cancel_submission(account, id).await },
                    move |result| event(Message::QueueCancelled(generation, result)),
                );
            }
            Message::QueueCancelled(generation, result) if generation == self.generation => {
                self.loading = false;
                self.status=match result{Ok(())=>"Current queued attempt cancelled. Earlier delivery uncertainty, if any, remains in Outbox.".into(),Err(error)=>error.to_string()};
                return self.refresh();
            }
            Message::Sent(generation, Private(result)) if generation == self.generation => {
                self.sending = false;
                match result {
                    SendResult::Draft(draft, error) => {
                        self.load(*draft);
                        self.saved_revision = 0;
                        self.status = error;
                    }
                    SendResult::Submitted(result) => self.status = outcome_text(&result),
                }
                return self.refresh();
            }
            _ => {}
        }
        Task::none()
    }

    fn saved(
        &mut self,
        id: &str,
        revision: u64,
        result: Result<(), worker::Error>,
    ) -> Task<super::Message> {
        let Some(draft) = &self.draft else {
            return Task::none();
        };
        if draft.id != id {
            return Task::none();
        }
        match result {
            Ok(()) => {
                self.saved_revision = self.saved_revision.max(revision);
                if draft.revision == revision {
                    self.status = "Draft saved locally.".into();
                }
                return self.refresh();
            }
            Err(error) if draft.revision == revision => self.status = error.to_string(),
            Err(_) => {}
        }
        Task::none()
    }
    fn load(&mut self, draft: Draft) {
        self.selected = self.identities.iter().position(|identity| {
            identity.account == draft.account && identity.from == draft.content.from
        });
        self.saved_revision = draft.revision;
        self.editor = text_editor::Content::with_text(&draft.content.body);
        self.draft = Some(draft);
        self.status = "Draft loaded.".into();
    }
    pub fn prepared(&mut self, draft: Draft) -> Task<super::Message> {
        self.update(
            Message::Switch(Switch::Prepared(Private(Box::new(draft)))),
            None,
        )
    }
    fn request_switch(&mut self, target: Switch) -> Task<super::Message> {
        self.autosave.cancel();
        if let (Some(draft), Some(store)) = (&self.draft, &self.store)
            && draft.revision > self.saved_revision
        {
            self.loading = true;
            let (draft, store, generation) = (draft.clone(), store.clone(), self.generation);
            return Task::perform(save_snapshot(store, draft), move |result| {
                event(Message::Switched(generation, target, result))
            });
        }
        self.switch(target)
    }
    fn switch(&mut self, target: Switch) -> Task<super::Message> {
        match target {
            Switch::Prepared(Private(draft)) => {
                self.load(*draft);
                self.saved_revision = 0;
                return self.save();
            }
            Switch::New => {
                let Some(identity) = self.selected.and_then(|index| self.identities.get(index))
                else {
                    self.status = "Configure a valid sender in Accounts first.".into();
                    return Task::none();
                };
                self.draft = Some(Draft::new(identity.account.clone(), identity.from.clone()));
                self.editor = text_editor::Content::new();
                self.saved_revision = 0;
                return self.save();
            }
            Switch::Sender(index) => {
                let Some(identity) = self.identities.get(index) else {
                    return Task::none();
                };
                self.selected = Some(index);
                if let Some(draft) = &mut self.draft {
                    if draft.account != identity.account {
                        let content = draft.content.clone();
                        *draft = Draft::new(identity.account.clone(), identity.from.clone());
                        draft.content = content;
                        self.saved_revision = 0;
                    }
                    draft.content.from.clone_from(&identity.from);
                    return self.edited();
                }
            }
            Switch::Draft(account, id) => {
                let Some(store) = self.store.clone() else {
                    return Task::none();
                };
                self.loading = true;
                let generation = self.generation;
                return Task::perform(
                    async move { store.draft(account, id).await.map(Box::new) },
                    move |result| event(Message::Loaded(generation, Private(result))),
                );
            }
            Switch::Recover(account, id) => {
                let Some(store) = self.store.clone() else {
                    return Task::none();
                };
                self.loading = true;
                let generation = self.generation;
                return Task::perform(
                    async move { store.recover_draft(account, id).await.map(Box::new) },
                    move |result| event(Message::Loaded(generation, Private(result))),
                );
            }
        }
        Task::none()
    }
    fn edited(&mut self) -> Task<super::Message> {
        let Some(draft) = &mut self.draft else {
            return Task::none();
        };
        draft.revision = draft.revision.saturating_add(1);
        self.status = "Unsaved changes…".into();
        self.autosave.cancel();
        self.autosave = CancellationToken::new();
        let (Some(store), Some(draft)) = (self.store.clone(), self.draft.clone()) else {
            return Task::none();
        };
        let (cancel, generation, id, revision) = (
            self.autosave.clone(),
            self.generation,
            draft.id.clone(),
            draft.revision,
        );
        Task::perform(
            async move {
                cancel
                    .run_until_cancelled(async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        save_snapshot(store, draft).await
                    })
                    .await
            },
            move |result| {
                result.map_or_else(
                    || event(Message::Noop),
                    |result| event(Message::Saved(generation, id.clone(), revision, result)),
                )
            },
        )
    }
    fn save(&self) -> Task<super::Message> {
        self.autosave.cancel();
        let (Some(store), Some(draft)) = (self.store.clone(), self.draft.clone()) else {
            return Task::none();
        };
        let (generation, id, revision) = (self.generation, draft.id.clone(), draft.revision);
        Task::perform(save_snapshot(store, draft), move |result| {
            event(Message::Saved(generation, id.clone(), revision, result))
        })
    }
    pub fn flush(&self) -> Task<super::Message> {
        self.autosave.cancel();
        let (store, draft) = (self.store.clone(), self.draft.clone());
        Task::perform(
            async move {
                match (store, draft) {
                    (Some(store), Some(draft)) => save_snapshot(store, draft).await,
                    (_, None) => Ok(()),
                    _ => Err(worker::Error::Closed),
                }
            },
            |result| Action::App(super::Message::QuitSaved(result)),
        )
    }
    fn refresh(&self) -> Task<super::Message> {
        let (Some(store), Some(config)) = (self.store.clone(), self.config.clone()) else {
            return Task::none();
        };
        let generation = self.generation;
        Task::perform(
            async move {
                let accounts = config
                    .accounts
                    .iter()
                    .map(|account| account.id.clone())
                    .collect::<Vec<_>>();
                let drafts = store.drafts(accounts.clone()).await?;
                let outgoing = store.outbox(accounts).await?;
                Ok(Lists { drafts, outgoing })
            },
            move |result| event(Message::Lists(generation, Private(result))),
        )
    }
    fn account(&self, id: &str) -> Option<Account> {
        self.config
            .as_ref()?
            .accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
    }
    fn send(&mut self, keyring: Option<Session>) -> Task<super::Message> {
        let Some(draft) = self.draft.as_ref() else {
            return Task::none();
        };
        if let Err(error) = draft.content.validate() {
            self.status = error.to_string();
            return Task::none();
        }
        let (Some(account), Some(store), Some(keyring)) =
            (self.account(&draft.account), self.store.clone(), keyring)
        else {
            self.status = "Load the account and unlock its encrypted keyring in Accounts.".into();
            return Task::none();
        };
        let Some(draft) = self.draft.take() else {
            return Task::none();
        };
        self.autosave.cancel();
        self.sending = true;
        self.cancellation = CancellationToken::new();
        let (generation, cancel) = (self.generation, self.cancellation.clone());
        self.status = "Saving draft and preparing submission…".into();
        Task::perform(
            send_new(store, draft, account, keyring, cancel),
            move |result| event(Message::Sent(generation, Private(result))),
        )
    }
    fn submit(
        &mut self,
        account: &str,
        id: String,
        phase: Phase,
        consent: bool,
        keyring: Option<Session>,
    ) -> Task<super::Message> {
        let (Some(account), Some(store), Some(keyring)) =
            (self.account(account), self.store.clone(), keyring)
        else {
            self.status = "Load the account and unlock its encrypted keyring in Accounts.".into();
            return Task::none();
        };
        self.sending = true;
        self.cancellation = CancellationToken::new();
        let (generation, cancel) = (self.generation, self.cancellation.clone());
        Task::perform(
            async move {
                let result = async {
                    if matches!(phase, Phase::Failed | Phase::DeliveryUnknown) {
                        store
                            .retry_submission(account.id.clone(), id.clone(), consent)
                            .await
                            .map_err(|error| error.to_string())?;
                    }
                    submit_frozen(store, &account, &id, keyring, cancel).await
                }
                .await;
                SendResult::Submitted(result)
            },
            move |result| event(Message::Sent(generation, Private(result))),
        )
    }

    pub fn view(&self, width: f32, keyring_ready: bool) -> Element<'_, super::Message> {
        let busy = self.busy();
        let mut content = column![
            widget::text::title1("Compose"),
            row![
                widget::button::text("New message").on_press_maybe(
                    (!busy && self.store.is_some() && !self.identities.is_empty())
                        .then_some(super::Message::Composer(Message::Switch(Switch::New)))
                ),
                widget::button::text("Refresh drafts / outbox")
                    .on_press(super::Message::Composer(Message::Refresh))
            ]
            .spacing(8)
        ]
        .spacing(10)
        .padding(20);
        if !self.labels.is_empty() {
            content = content.push(widget::dropdown(
                self.labels.as_slice(),
                self.selected,
                |index| super::Message::Composer(Message::Switch(Switch::Sender(index))),
            ));
        }
        if let Some(draft) = &self.draft {
            content = content.push(self.editor_view(draft, width, keyring_ready));
        }
        if self.sending {
            content = content.push(
                widget::button::text("Cancel submission")
                    .on_press(super::Message::Composer(Message::Cancel)),
            );
        }
        if !keyring_ready {
            content = content.push(widget::text(
                "Unlock the keyring in Accounts to send. Drafts remain local.",
            ));
        }
        content = content.push(widget::text(&self.status));
        content = content.push(widget::text::title3("Drafts"));
        for draft in &self.drafts {
            content = content.push(
                widget::button::text(format!("{} · {}", draft.account, subject(&draft.subject)))
                    .on_press_maybe((!busy).then(|| {
                        super::Message::Composer(Message::Switch(Switch::Draft(
                            draft.account.clone(),
                            draft.id.clone(),
                        )))
                    })),
            );
        }
        content = content.push(widget::text::title3("Outbox"));
        for outgoing in &self.outgoing {
            content = content.push(self.outbox_entry(outgoing, keyring_ready));
        }
        widget::scrollable(content).into()
    }
    fn outbox_entry<'a>(
        &'a self,
        outgoing: &'a Summary,
        keyring_ready: bool,
    ) -> Element<'a, super::Message> {
        let busy = self.busy();
        let mut entry = column![widget::text(format!(
            "{} · {} · {}",
            outgoing.account,
            subject(&outgoing.subject),
            phase_text(outgoing.phase)
        ))]
        .spacing(6);
        if let Some(error) = &outgoing.error {
            entry = entry.push(widget::text(error));
        }
        if outgoing.uncertain {
            entry=entry.push(widget::text("An earlier attempt may have delivered this message. Check Sent before sending again."));
        }
        if outgoing.phase == Phase::Pending {
            entry = entry.push(
                widget::button::text("Cancel queued attempt").on_press_maybe((!busy).then(|| {
                    super::Message::Composer(Message::CancelQueued(
                        outgoing.account.clone(),
                        outgoing.id.clone(),
                    ))
                })),
            );
        }
        if outgoing.phase == Phase::Failed && !outgoing.uncertain {
            entry = entry.push(widget::button::text("Edit as a new draft").on_press_maybe(
                (!busy).then(|| {
                    super::Message::Composer(Message::Switch(Switch::Recover(
                        outgoing.account.clone(),
                        outgoing.id.clone(),
                    )))
                }),
            ));
        }
        if matches!(
            outgoing.phase,
            Phase::Pending | Phase::Failed | Phase::DeliveryUnknown
        ) {
            let allow_duplicate = outgoing.phase == Phase::DeliveryUnknown || outgoing.uncertain;
            entry = entry.push(
                widget::button::text(if allow_duplicate {
                    "Send again (may duplicate)"
                } else {
                    "Send queued message"
                })
                .on_press_maybe((!busy && keyring_ready).then(|| {
                    super::Message::Composer(Message::Submit(
                        outgoing.account.clone(),
                        outgoing.id.clone(),
                        outgoing.phase,
                        allow_duplicate,
                    ))
                })),
            );
        }
        if outgoing.phase == Phase::Accepted && outgoing.sent_copy_pending {
            entry = entry.push(widget::text(
                "Accepted by the server. Sent-folder copy remains pending.",
            ));
        }
        entry.into()
    }
    fn editor_view<'a>(
        &'a self,
        draft: &'a Draft,
        width: f32,
        keyring_ready: bool,
    ) -> Element<'a, super::Message> {
        let busy = self.busy();
        let mut content = column![].spacing(10);
        for (field, label, value) in [
            (Field::To, "To", &draft.content.to),
            (Field::Cc, "Cc", &draft.content.cc),
            (Field::Bcc, "Bcc", &draft.content.bcc),
            (Field::Subject, "Subject", &draft.content.subject),
        ] {
            let input = widget::text_input(label, value);
            content = content.push(if busy {
                input
            } else {
                input.on_input(move |value| {
                    super::Message::Composer(Message::Field(field, Private(value)))
                })
            });
        }
        let editor = text_editor::text_editor(&self.editor)
            .placeholder("Write your message…")
            .width((width - 40.0).max(100.0))
            .height(220);
        content = content.push(if busy {
            editor
        } else {
            editor.on_action(|action| super::Message::Composer(Message::Body(Private(action))))
        });
        content = content.push(
            row![
                widget::button::text("Save draft")
                    .on_press_maybe((!busy).then_some(super::Message::Composer(Message::Save))),
                widget::button::text("Send").on_press_maybe(
                    (!busy && keyring_ready).then_some(super::Message::Composer(Message::Send))
                )
            ]
            .spacing(8),
        );
        content.into()
    }
}

async fn save_snapshot(store: Handle, draft: Draft) -> Result<(), worker::Error> {
    store.ensure_account(draft.account.clone()).await?;
    store.save_draft(draft).await
}
async fn send_new(
    store: Handle,
    draft: Draft,
    account: Account,
    keyring: Session,
    cancel: CancellationToken,
) -> SendResult {
    if let Err(error) = save_snapshot(store.clone(), draft.clone()).await {
        return SendResult::Draft(Box::new(draft), error.to_string());
    }
    if cancel.is_cancelled() {
        return SendResult::Draft(
            Box::new(draft),
            "Submission cancelled; draft retained.".into(),
        );
    }
    let queued = match store
        .queue_draft(
            draft.account.clone(),
            draft.id.clone(),
            draft.revision,
            matches!(account.provider, Provider::Google),
        )
        .await
    {
        Ok(queued) => queued,
        Err(error) => return SendResult::Draft(Box::new(draft), error.to_string()),
    };
    let result = submit_frozen(store, &account, &queued.id, keyring, cancel).await;
    SendResult::Submitted(result)
}
async fn submit_frozen(
    store: Handle,
    account: &Account,
    id: &str,
    keyring: Session,
    cancel: CancellationToken,
) -> Result<Outcome, String> {
    let resolved = keyring.resolve(account, true, &cancel).await;
    if cancel.is_cancelled() {
        store
            .cancel_submission(account.id.clone(), id.into())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Outcome::CancelledBeforeSubmission);
    }
    let credential = resolved.map_err(|error| error.to_string())?;
    let endpoint = account
        .outgoing_endpoint()
        .ok_or("Outgoing endpoint missing.")?;
    let transport = Transport::prepare(
        &endpoint,
        &account.tls,
        account.outgoing_user_name(),
        credential.authentication(),
    )
    .await
    .map_err(|error| error.to_string())?;
    drop(credential);
    if cancel.is_cancelled() {
        store
            .cancel_submission(account.id.clone(), id.into())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Outcome::CancelledBeforeSubmission);
    }
    let submission = store
        .begin_submission(account.id.clone(), id.into())
        .await
        .map_err(|error| error.to_string())?;
    let allowed = std::iter::once(&account.address)
        .chain(&account.aliases)
        .any(|address| submission.message.sender_is(address));
    let outcome = if allowed {
        transport.submit(&submission.message, &cancel).await
    } else {
        Outcome::CancelledBeforeSubmission
    };
    store
        .finish_submission(submission, outcome)
        .await
        .map_err(|error| error.to_string())?;
    if outcome == Outcome::AuthenticationRequired {
        keyring.invalidate(account, true).await;
    }
    if !allowed {
        return Err(
            "Sender identity is no longer enabled. No new attempt was submitted; check Outbox."
                .into(),
        );
    }
    Ok(outcome)
}
const fn event(message: Message) -> Action<super::Message> {
    Action::App(super::Message::Composer(message))
}
const fn subject(value: &str) -> &str {
    if value.is_empty() {
        "(no subject)"
    } else {
        value
    }
}
const fn phase_text(phase: Phase) -> &'static str {
    match phase {
        Phase::Pending => "Queued",
        Phase::Submitting => "Submitting",
        Phase::Accepted => "Accepted",
        Phase::DeliveryUnknown => "Delivery unknown",
        Phase::Failed => "Not accepted",
        Phase::Cancelled => "Cancelled",
    }
}
fn outcome_text(result: &Result<Outcome, String>) -> String {
    match result {
        Ok(Outcome::Accepted) => "Message accepted by the server.".into(),
        Ok(Outcome::DeliveryUnknown) => {
            "Delivery unknown. Check Sent before deciding whether to send again.".into()
        }
        Ok(Outcome::AuthenticationRequired) => {
            "Authentication refused. Update credentials before retrying.".into()
        }
        Ok(Outcome::TlsFailed) => "TLS validation failed before the current submission.".into(),
        Ok(Outcome::CapabilityUnavailable) => {
            "Server lacks a required security or message capability.".into()
        }
        Ok(Outcome::CancelledBeforeSubmission) => {
            "Current attempt cancelled before submission.".into()
        }
        Ok(Outcome::Rejected { .. }) => {
            "Server rejected submission. The message remains in Outbox.".into()
        }
        Err(error) => error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_save_callbacks_do_not_clear_newer_edits_or_replace_another_draft() {
        let mut composer = Composer::default();
        let mut draft = Draft::new("a".into(), "sender@example.test".into());
        draft.revision = 8;
        let id = draft.id.clone();
        composer.load(draft);
        let _task = composer.update(
            Message::Field(Field::Subject, Private("Latest edit".into())),
            None,
        );
        let _task = composer.update(Message::Saved(0, id.clone(), 8, Ok(())), None);
        assert_eq!(composer.saved_revision, 8);
        assert_eq!(composer.draft.as_ref().expect("draft").revision, 9);
        assert_eq!(composer.status, "Unsaved changes…");
        let next = Draft::new("a".into(), "sender@example.test".into());
        let next_id = next.id.clone();
        composer.load(next);
        let _task = composer.update(Message::Saved(0, id, 9, Err(worker::Error::Storage)), None);
        assert_eq!(composer.draft.as_ref().expect("current draft").id, next_id);
        assert_eq!(composer.status, "Draft loaded.");
    }

    #[test]
    fn failed_pre_queue_save_returns_the_complete_draft_and_keeps_it_dirty() {
        let mut composer = Composer {
            sending: true,
            ..Composer::default()
        };
        let mut draft = Draft::new("a".into(), "sender@example.test".into());
        draft.content.body = "Private unsaved edit".into();
        draft.revision = 4;
        let id = draft.id.clone();
        let _task = composer.update(
            Message::Sent(
                0,
                Private(SendResult::Draft(Box::new(draft), "Save failed".into())),
            ),
            None,
        );
        assert!(!composer.sending);
        assert_eq!(composer.saved_revision, 0);
        assert_eq!(composer.draft.as_ref().expect("preserved").id, id);
        assert_eq!(composer.editor.text(), "Private unsaved edit");
        let diagnostics = format!(
            "{:?}",
            Message::Body(Private(text_editor::Action::Edit(
                text_editor::Edit::Paste(Arc::new("Private unsaved edit".into()))
            )))
        );
        assert!(!diagnostics.contains("Private unsaved edit"));
    }
}
