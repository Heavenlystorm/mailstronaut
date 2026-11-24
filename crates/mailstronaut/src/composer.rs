//! Native plain-text composer over the durable draft and outbox store.

mod lists;

use std::{sync::Arc, time::Duration};

use gpui::assets::IconName;
use gpui::component::{
    ActiveTheme as _, Disableable as _, IndexPath, WindowExt as _,
    alert::Alert,
    attachment::{
        Attachment, AttachmentContent, AttachmentDescription, AttachmentGroup, AttachmentMedia,
        AttachmentTitle,
    },
    button::{Button, ButtonVariant, ButtonVariants as _},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    form::{Field, Form},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    resizable::{h_resizable, resizable_panel},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    status_bar::StatusBar,
    toolbar::{Toolbar, ToolbarGroup},
};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, PathPromptOptions, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use mailstronaut_config::{Account, Config};
use mailstronaut_core::{
    attachments::read_files,
    reply,
    store::outbox::{Draft, DraftSummary, Summary},
    worker::{self, Handle},
};
use mailstronaut_mail::{compose, smtp::Outcome};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::account_actions::{Access, Sent};

const AUTOSAVE_DELAY: Duration = Duration::from_millis(500);

gpui::actions!(
    mailstronaut,
    [
        #[derive(Eq)]
        /// Show or hide the copy recipient fields.
        ToggleCopies
    ]
);

/// Emitted when the editor closes after an outcome the mailbox should report.
pub enum Event {
    Closed(SharedString),
    Changed,
}

#[derive(Clone)]
struct Identity {
    label: SharedString,
    account: String,
    from: String,
    signature: Option<String>,
}

/// At most one store or network operation owns the editor at a time.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Activity {
    Idle,
    Loading,
    Attaching,
    SendingDraft,
    SubmittingQueued,
}

enum Target {
    New(Identity),
    Prepared(Box<Draft>),
    Saved { account: String, id: String },
    Recover { account: String, id: String },
}

pub struct Composer {
    runtime: Arc<Runtime>,
    access: Access,
    lifetime: CancellationToken,
    declaration: CancellationToken,
    config: Option<Arc<Config>>,
    store: Option<Handle>,
    identities: Vec<Identity>,
    identity: Entity<SelectState<SearchableVec<SharedString>>>,
    to: Entity<InputState>,
    cc: Entity<InputState>,
    bcc: Entity<InputState>,
    subject: Entity<InputState>,
    body: Entity<TextareaState>,
    draft: Option<Draft>,
    saved_revision: u64,
    generation: u64,
    autosave: CancellationToken,
    send_cancel: CancellationToken,
    activity: Activity,
    copies_open: bool,
    drafts: Vec<DraftSummary>,
    outgoing: Vec<Summary>,
    status: SharedString,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Event> for Composer {}

impl Drop for Composer {
    fn drop(&mut self) {
        self.autosave.cancel();
        // The store owner is a separate thread, so a short wait cannot deadlock.
        // This is what keeps a keystroke typed just before closing the window.
        if let (Some(store), Some(draft)) = (self.store.clone(), self.unsaved()) {
            let (done, wait) = std::sync::mpsc::channel();
            self.runtime.spawn(async move {
                _ = save_snapshot(&store, draft).await;
                _ = done.send(());
            });
            _ = wait.recv_timeout(Duration::from_secs(2));
        }
        self.lifetime.cancel();
    }
}

impl Composer {
    pub fn new(
        runtime: Arc<Runtime>,
        access: Access,
        lifetime: CancellationToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([KeyBinding::new(
            "ctrl-shift-b",
            ToggleCopies,
            Some("Composer"),
        )]);
        let line = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let to = line("name@example.org, another@example.org", window, cx);
        let cc = line("Copy recipients", window, cx);
        let bcc = line("Blind copy recipients", window, cx);
        let subject = line("Subject", window, cx);
        let body = cx.new(|cx| TextareaState::new(window, cx));
        let identity = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SharedString>::new()),
                None,
                window,
                cx,
            )
        });
        let mut subscriptions = Vec::new();
        for field in [&to, &cc, &bcc, &subject] {
            subscriptions.push(
                cx.subscribe_in(field, window, |this, _, event, window, cx| match event {
                    InputEvent::Change => this.edited(window, cx),
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.send(window, cx),
                    _ => {}
                }),
            );
        }
        subscriptions.push(
            cx.subscribe_in(&body, window, |this, _, event, window, cx| {
                match event {
                    InputEvent::Change => this.edited(window, cx),
                    // Ctrl+Enter sends from the body, matching common mail clients.
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.send(window, cx),
                    _ => {}
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &identity,
            window,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                if let Some(choice) = this.identities.iter().find(|id| &id.label == label) {
                    let choice = choice.clone();
                    this.choose_identity(choice, window, cx);
                }
            },
        ));
        Self {
            runtime,
            access,
            autosave: lifetime.child_token(),
            send_cancel: lifetime.child_token(),
            declaration: lifetime.child_token(),
            lifetime,
            config: None,
            store: None,
            identities: Vec::new(),
            identity,
            to,
            cc,
            bcc,
            subject,
            body,
            draft: None,
            saved_revision: 0,
            generation: 0,
            activity: Activity::Idle,
            copies_open: false,
            drafts: Vec::new(),
            outgoing: Vec::new(),
            status: "".into(),
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// Apply the last valid configuration. An open draft whose account was disabled
    /// is saved and closed; its stored copy stays available if the account returns.
    pub fn configure(
        &mut self,
        config: &Arc<Config>,
        store: Option<Handle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Accepted operations finish recording their durable result. Only the
        // current declaration can publish it into this editor or start SMTP.
        self.declaration.cancel();
        self.declaration = self.lifetime.child_token();
        self.generation = self.generation.wrapping_add(1);
        let interrupted_editor =
            matches!(self.activity, Activity::SendingDraft | Activity::Attaching);
        self.activity = Activity::Idle;
        self.drafts.clear();
        self.outgoing.clear();
        self.error = None;
        self.status = "".into();
        self.config = Some(config.clone());
        self.store = store;
        self.identities = config
            .accounts
            .iter()
            .flat_map(|account| {
                std::iter::once(&account.address)
                    .chain(&account.aliases)
                    .filter_map(move |address| {
                        let from = compose::sender(address, &account.display_name).ok()?;
                        Some(Identity {
                            label: format!("{from} · {}", account.id).into(),
                            account: account.id.clone(),
                            from,
                            signature: account.signature.clone(),
                        })
                    })
            })
            .collect();
        let labels: Vec<SharedString> = self.identities.iter().map(|id| id.label.clone()).collect();
        self.identity.update(cx, |identity, cx| {
            identity.set_items(SearchableVec::new(labels), window, cx);
        });
        if let Some(draft) = &self.draft {
            if !interrupted_editor
                && config
                    .accounts
                    .iter()
                    .any(|account| account.id == draft.account)
            {
                self.sync_identity(window, cx);
            } else {
                self.flush(cx);
                self.close_editor(window, cx);
            }
        }
        self.refresh_lists(window, cx);
    }

    pub const fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// Drafts and outbox entries that still need a decision from the user.
    pub fn attention(&self) -> usize {
        self.drafts.len()
            + self
                .outgoing
                .iter()
                .filter(|entry| lists::needs_decision(entry))
                .count()
    }

    pub fn compose_new(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(identity) = self
            .selected_identity(cx)
            .or_else(|| self.identities.first().cloned())
        else {
            self.error = Some("Add an account with a valid address first.".into());
            cx.notify();
            return;
        };
        self.switch(Target::New(identity), window, cx);
    }

    pub fn open_prepared(&mut self, draft: Draft, window: &Window, cx: &mut Context<Self>) {
        self.switch(Target::Prepared(Box::new(draft)), window, cx);
    }

    fn selected_identity(&self, cx: &Context<Self>) -> Option<Identity> {
        let label = self.identity.read(cx).selected_value()?.clone();
        self.identities.iter().find(|id| id.label == label).cloned()
    }

    fn account(&self, id: &str) -> Option<Account> {
        self.config
            .as_ref()?
            .accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
    }

    const fn busy(&self) -> bool {
        !matches!(self.activity, Activity::Idle)
    }

    fn unsaved(&self) -> Option<Draft> {
        self.draft
            .as_ref()
            .filter(|draft| draft.revision > self.saved_revision)
            .cloned()
    }

    /// Save pending edits now, without waiting for the autosave delay.
    pub fn flush(&self, cx: &Context<Self>) {
        self.autosave.cancel();
        let (Some(store), Some(draft)) = (self.store.clone(), self.unsaved()) else {
            return;
        };
        let revision = draft.revision;
        let generation = self.generation;
        let job = self
            .runtime
            .spawn(async move { save_snapshot(&store, draft).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.saved(
                    revision,
                    joined(result, "Saving the draft was interrupted."),
                    cx,
                );
            });
        })
        .detach();
    }

    fn saved(&mut self, revision: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.saved_revision = self.saved_revision.max(revision);
                if self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.revision == revision)
                {
                    self.status = "Saved locally.".into();
                }
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }

    fn edited(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (to, cc, bcc, subject, body) = (
            self.to.read(cx).value().to_string(),
            self.cc.read(cx).value().to_string(),
            self.bcc.read(cx).value().to_string(),
            self.subject.read(cx).value().to_string(),
            self.body.read(cx).value().to_string(),
        );
        let Some(draft) = &mut self.draft else {
            return;
        };
        let content = &mut draft.content;
        if content.to == to
            && content.cc == cc
            && content.bcc == bcc
            && content.subject == subject
            && content.body == body
        {
            return;
        }
        content.to = to;
        content.cc = cc;
        content.bcc = bcc;
        content.subject = subject;
        content.body = body;
        draft.revision = draft.revision.saturating_add(1);
        self.status = "Saving…".into();
        self.schedule_autosave(window, cx);
        cx.notify();
    }

    fn schedule_autosave(&mut self, window: &Window, cx: &Context<Self>) {
        self.autosave.cancel();
        self.autosave = self.lifetime.child_token();
        let (Some(store), Some(draft)) = (self.store.clone(), self.draft.clone()) else {
            return;
        };
        let (cancel, revision, generation) =
            (self.autosave.clone(), draft.revision, self.generation);
        let job = self.runtime.spawn(async move {
            cancel
                .run_until_cancelled(async {
                    tokio::time::sleep(AUTOSAVE_DELAY).await;
                    save_snapshot(&store, draft).await
                })
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(None) => {}
                    Ok(Some(saved)) => {
                        this.saved(revision, saved.map_err(|error| error.to_string()), cx);
                        this.refresh_lists(window, cx);
                    }
                    Err(_) => {
                        this.saved(
                            revision,
                            Err("Saving the draft was interrupted.".into()),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    /// Finish saving the current edit before replacing the editor's contents.
    fn switch(&mut self, target: Target, window: &Window, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        self.autosave.cancel();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.activity = Activity::Loading;
        self.error = None;
        let unsaved = self.unsaved();
        let job = self.runtime.spawn(async move {
            if let Some(draft) = unsaved {
                save_snapshot(&store, draft).await?;
            }
            match target {
                Target::New(identity) => {
                    let mut draft = Draft::new(identity.account, identity.from);
                    draft.content.body = reply::with_signature("", identity.signature.as_deref());
                    save_snapshot(&store, draft.clone()).await?;
                    Ok(draft)
                }
                Target::Prepared(draft) => {
                    save_snapshot(&store, (*draft).clone()).await?;
                    Ok(*draft)
                }
                Target::Saved { account, id } => store.draft(account, id).await,
                Target::Recover { account, id } => store.recover_draft(account, id).await,
            }
        });
        Self::watch_load(job, generation, window, cx);
        cx.notify();
    }

    fn watch_load(
        job: tokio::task::JoinHandle<Result<Draft, worker::Error>>,
        generation: u64,
        window: &Window,
        cx: &Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation {
                    return;
                }
                this.activity = Activity::Idle;
                match joined(result, "Opening the draft was interrupted.") {
                    Ok(draft) => this.load(draft, window, cx),
                    Err(error) => this.error = Some(error.into()),
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn load(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        self.copies_open = !draft.content.cc.is_empty() || !draft.content.bcc.is_empty();
        self.saved_revision = draft.revision;
        let content = &draft.content;
        for (field, value) in [
            (&self.to, &content.to),
            (&self.cc, &content.cc),
            (&self.bcc, &content.bcc),
            (&self.subject, &content.subject),
        ] {
            field.update(cx, |field, cx| field.set_value(value.clone(), window, cx));
        }
        self.body.update(cx, |body, cx| {
            body.set_value(content.body.clone(), window, cx);
        });
        let start_in_body = !content.to.is_empty();
        self.draft = Some(draft);
        self.status = "Saved locally.".into();
        self.sync_identity(window, cx);
        if start_in_body {
            self.body.update(cx, |body, cx| body.focus(window, cx));
        } else {
            self.to.update(cx, |to, cx| to.focus(window, cx));
        }
    }

    fn sync_identity(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        let selected = self
            .identities
            .iter()
            .position(|id| id.account == draft.account && id.from == draft.content.from);
        self.identity.update(cx, |identity, cx| {
            identity.set_selected_index(selected.map(IndexPath::new), window, cx);
        });
    }

    fn choose_identity(&mut self, choice: Identity, window: &Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.clone() else {
            return;
        };
        if draft.account == choice.account {
            if let Some(draft) = &mut self.draft
                && draft.content.from != choice.from
            {
                draft.content.from = choice.from;
                draft.revision = draft.revision.saturating_add(1);
                self.schedule_autosave(window, cx);
            }
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let mut moved = draft.clone();
        moved.content.body = reply::replace_signature(
            &draft.content.body,
            self.account(&draft.account)
                .and_then(|account| account.signature)
                .as_deref(),
            choice.signature.as_deref(),
        );
        // Another account owns different UIDs and credentials: move the content, not
        // the identity. Files are retained and the source draft stops being offered.
        self.autosave.cancel();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.activity = Activity::Loading;
        let job = self.runtime.spawn(async move {
            store.ensure_account(choice.account.clone()).await?;
            let copy = store.copy_draft(moved, choice.account, choice.from).await?;
            store.discard_draft(draft.account, draft.id).await?;
            Ok(copy)
        });
        Self::watch_load(job, generation, window, cx);
        cx.notify();
    }

    fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.autosave.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.draft = None;
        self.saved_revision = 0;
        self.copies_open = false;
        for field in [&self.to, &self.cc, &self.bcc, &self.subject] {
            field.update(cx, |field, cx| field.set_value("", window, cx));
        }
        self.body
            .update(cx, |body, cx| body.set_value("", window, cx));
        cx.notify();
    }

    fn send(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(draft) = self.draft.clone() else {
            return;
        };
        if let Err(error) = draft.content.validate() {
            self.error = Some(format!("Not sent: {error}. Check the recipients.").into());
            cx.notify();
            return;
        }
        let (Some(store), Some(account)) = (self.store.clone(), self.account(&draft.account))
        else {
            self.error = Some("This draft's account is not enabled.".into());
            cx.notify();
            return;
        };
        self.autosave.cancel();
        self.activity = Activity::SendingDraft;
        self.error = None;
        self.status = "Sending…".into();
        self.send_cancel = self.declaration.child_token();
        let declaration = self.declaration.clone();
        let (access, cancel) = (self.access.clone(), self.send_cancel.clone());
        let job = self
            .runtime
            .spawn(async move { access.send_draft(&store, &account, draft, &cancel).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if declaration.is_cancelled() {
                    this.refresh_lists(window, cx);
                    return;
                }
                this.activity = Activity::Idle;
                match result {
                    Ok(Sent::NotQueued(error)) => {
                        this.status = "Draft kept.".into();
                        this.error = Some(error.into());
                    }
                    Ok(Sent::Queued(outcome)) => {
                        this.saved_revision = this.draft.as_ref().map_or(0, |draft| draft.revision);
                        this.close_editor(window, cx);
                        this.report(&outcome, cx);
                    }
                    Err(_) => {
                        this.error = Some("Sending was interrupted. Check the outbox.".into());
                    }
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn report(&mut self, outcome: &Result<Outcome, String>, cx: &mut Context<Self>) {
        let text: SharedString = outcome_text(outcome).into();
        self.status = "".into();
        if matches!(outcome, Ok(Outcome::Accepted)) {
            cx.emit(Event::Closed(text));
        } else {
            self.error = Some(text);
        }
    }

    fn submit(
        &mut self,
        entry: &Summary,
        retry: Option<bool>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        let (Some(store), Some(account)) = (self.store.clone(), self.account(&entry.account))
        else {
            self.error = Some("This message's account is not enabled.".into());
            cx.notify();
            return;
        };
        self.activity = Activity::SubmittingQueued;
        self.error = None;
        self.status = "Sending…".into();
        self.send_cancel = self.declaration.child_token();
        let declaration = self.declaration.clone();
        let (access, cancel, id) = (
            self.access.clone(),
            self.send_cancel.clone(),
            entry.id.clone(),
        );
        let job = self.runtime.spawn(async move {
            access
                .submit_queued(&store, &account, id, retry, &cancel)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if declaration.is_cancelled() {
                    this.refresh_lists(window, cx);
                    return;
                }
                this.activity = Activity::Idle;
                let outcome = result
                    .unwrap_or_else(|_| Err("Sending was interrupted. Check the outbox.".into()));
                this.report(&outcome, cx);
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn cancel_queued(&self, entry: &Summary, window: &Window, cx: &Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let (account, id) = (entry.account.clone(), entry.id.clone());
        let job = self
            .runtime
            .spawn(async move { store.cancel_submission(account, id).await });
        self.after_store_change(
            job,
            "Queued message cancelled. Its draft was restored.",
            window,
            cx,
        );
    }

    fn after_store_change(
        &self,
        job: tokio::task::JoinHandle<Result<(), worker::Error>>,
        success: &'static str,
        window: &Window,
        cx: &Context<Self>,
    ) {
        let declaration = self.declaration.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if declaration.is_cancelled() {
                    return;
                }
                match joined(result, "The change was interrupted.") {
                    Ok(()) => this.status = success.into(),
                    Err(error) => this.error = Some(error.into()),
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_discard(&self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let generation = self.generation;
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .confirm()
                .title("Discard this draft?")
                .description("Its text and attachments will no longer be available.")
                .ok_text("Discard")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    _ = this.update(cx, |this, cx| {
                        if generation == this.generation {
                            this.discard(window, cx);
                        }
                    });
                    true
                })
        });
    }

    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(store), Some(draft)) = (self.store.clone(), self.draft.clone()) else {
            return;
        };
        let (account, id) = (draft.account, draft.id);
        let job = self.runtime.spawn(async move {
            store.ensure_account(account.clone()).await?;
            store.discard_draft(account, id).await
        });
        self.saved_revision = u64::MAX;
        self.close_editor(window, cx);
        self.after_store_change(job, "Draft discarded.", window, cx);
    }

    fn attach(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(draft) = self.draft.clone() else {
            return;
        };
        let used: usize = draft.attachments.iter().map(|file| file.bytes).sum();
        let remaining = compose::MAX_ATTACHMENT_BYTES.saturating_sub(used);
        if draft.attachments.len() >= compose::MAX_ATTACHMENTS {
            self.error = Some("A message can have at most 16 attachments.".into());
            cx.notify();
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.autosave.cancel();
        // A portal can stay open indefinitely. Save the pending edit while the
        // chooser is open and order the attachment transaction after that write.
        let pending = self.unsaved().map(|draft| {
            let store = store.clone();
            self.runtime
                .spawn(async move { save_snapshot(&store, draft).await })
        });
        self.activity = Activity::Attaching;
        self.error = None;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        let runtime = self.runtime.clone();
        let declaration = self.declaration.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Some(selection) = declaration.run_until_cancelled(paths).await else {
                return;
            };
            let active = declaration.clone();
            let job = runtime.spawn(async move {
                if let Some(pending) = pending {
                    pending
                        .await
                        .map_err(|_| "Saving the draft was interrupted.")?
                        .map_err(|error| error.to_string())?;
                }
                let paths = match selection {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return Ok(draft),
                    _ => {
                        return Err("The file chooser is unavailable. Nothing was attached.".into());
                    }
                };
                let files = tokio::task::spawn_blocking(move || read_files(paths, remaining))
                    .await
                    .map_err(|_| "Importing the files was interrupted.".to_owned())??;
                if files.is_empty() {
                    return Ok(draft);
                }
                if active.is_cancelled() {
                    return Err("The draft's account configuration changed.".into());
                }
                store
                    .ensure_account(draft.account.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                store
                    .add_attachments(draft, files)
                    .await
                    .map_err(|e| e.to_string())
            });
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if declaration.is_cancelled() {
                    return;
                }
                this.activity = Activity::Idle;
                this.apply_attachments(result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_attachment(&mut self, id: String, window: &Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let (Some(store), Some(draft)) = (self.store.clone(), self.draft.clone()) else {
            return;
        };
        self.autosave.cancel();
        self.activity = Activity::Attaching;
        let declaration = self.declaration.clone();
        let job = self.runtime.spawn(async move {
            store
                .remove_attachment(draft, id)
                .await
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if declaration.is_cancelled() {
                    return;
                }
                this.activity = Activity::Idle;
                this.apply_attachments(result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_attachments(
        &mut self,
        result: Result<Result<Draft, String>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(Ok(draft)) => {
                self.saved_revision = draft.revision;
                self.draft = Some(draft);
                self.status = "Saved locally.".into();
            }
            Ok(Err(error)) => self.error = Some(error.into()),
            Err(_) => self.error = Some("Importing the files was interrupted.".into()),
        }
        cx.notify();
    }

    pub fn refresh_lists(&self, window: &Window, cx: &Context<Self>) {
        let declaration = self.declaration.clone();
        let (Some(store), Some(config)) = (self.store.clone(), self.config.clone()) else {
            return;
        };
        let accounts: Vec<_> = config
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect();
        let job = self.runtime.spawn(async move {
            let drafts = store.drafts(accounts.clone()).await?;
            let outgoing = store.outbox(accounts).await?;
            Ok::<_, worker::Error>((drafts, outgoing))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if declaration.is_cancelled() {
                    return;
                }
                match result {
                    Ok(Ok((drafts, outgoing))) => {
                        this.drafts = drafts;
                        this.outgoing = outgoing;
                    }
                    Ok(Err(error)) => this.error = Some(error.to_string().into()),
                    Err(_) => this.error = Some("Loading drafts was interrupted.".into()),
                }
                cx.emit(Event::Changed);
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_copies(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.is_none() || self.busy() {
            return;
        }
        self.copies_open = !self.copies_open;
        if self.copies_open {
            self.cc.update(cx, |cc, cx| cc.focus(window, cx));
        } else if self.cc.focus_handle(cx).is_focused(window)
            || self.bcc.focus_handle(cx).is_focused(window)
        {
            self.to.update(cx, |to, cx| to.focus(window, cx));
        }
        cx.notify();
    }

    fn actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.busy();
        Toolbar::new("composer-actions")
            // `content` keeps the primary style; a toolbar child is restyled as ghost.
            .content(
                ToolbarGroup::new("composer-send")
                    .content(
                        Button::new("send-message")
                            .primary()
                            .icon(IconName::Send)
                            .accessibility_label("Send message")
                            .tooltip("Send message · Ctrl+Enter")
                            .loading(self.activity == Activity::SendingDraft)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    )
                    .when(
                        matches!(
                            self.activity,
                            Activity::SendingDraft | Activity::SubmittingQueued
                        ),
                        |group| {
                            group.child(
                                Button::new("cancel-send").ghost().label("Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.send_cancel.cancel();
                                        cx.notify();
                                    }),
                                ),
                            )
                        },
                    ),
            )
            .content(
                ToolbarGroup::new("composer-draft")
                    .child(
                        Button::new("attach-file")
                            .ghost()
                            .icon(IconName::Paperclip)
                            .accessibility_label("Attach files")
                            .tooltip("Attach files · up to 16 files, 5 MiB in total")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.attach(window, cx))),
                    )
                    .child(
                        Button::new("show-copies")
                            .ghost()
                            .icon(IconName::Users)
                            .accessibility_label("Show or hide Cc and Bcc")
                            .tooltip("Cc/Bcc · Ctrl+Shift+B")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_copies(window, cx);
                            })),
                    )
                    .child(
                        Button::new("discard-draft")
                            .ghost()
                            .icon(IconName::Trash)
                            .accessibility_label("Discard draft")
                            .tooltip("Discard draft")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_discard(window, cx);
                            })),
                    ),
            )
    }

    fn fields(&self) -> impl IntoElement {
        let busy = self.busy();
        Form::horizontal()
            .label_width(px(64.))
            .child(
                Field::new().label("From").child(
                    Select::new(&self.identity)
                        .accessibility_label("Sending identity")
                        .disabled(busy),
                ),
            )
            .child(
                Field::new().label("To").child(
                    Input::new(&self.to)
                        .aria_label("To recipients")
                        .disabled(busy),
                ),
            )
            .child(
                Field::new().label("Cc").visible(self.copies_open).child(
                    Input::new(&self.cc)
                        .aria_label("Cc recipients")
                        .disabled(busy),
                ),
            )
            .child(
                Field::new().label("Bcc").visible(self.copies_open).child(
                    Input::new(&self.bcc)
                        .aria_label("Bcc recipients")
                        .disabled(busy),
                ),
            )
            .child(
                Field::new().label("Subject").child(
                    Input::new(&self.subject)
                        .aria_label("Subject")
                        .disabled(busy),
                ),
            )
    }

    fn attachments(&self, draft: &Draft, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.busy();
        AttachmentGroup::new("draft-attachments").children(draft.attachments.iter().map(|file| {
            let id = file.id.clone();
            Attachment::new()
                .media(AttachmentMedia::new().child(IconName::FileText))
                .id(SharedString::from(format!("attachment-{}", file.id)))
                .content(
                    AttachmentContent::new()
                        .title(AttachmentTitle::new(file.name.clone()))
                        .description(AttachmentDescription::new(size_label(file.bytes))),
                )
                .when(!busy, |attachment| {
                    attachment.on_remove(cx.listener(move |this, _, window, cx| {
                        this.remove_attachment(id.clone(), window, cx);
                    }))
                })
        }))
    }

    fn editor(&self, draft: &Draft, cx: &Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(self.actions(cx))
            .child(self.fields())
            .when(!draft.attachments.is_empty(), |view| {
                view.child(self.attachments(draft, cx))
            })
            .child(
                div().flex_1().min_h_0().child(
                    Textarea::new(&self.body)
                        .aria_label("Message body")
                        .accessibility_id("message-body")
                        .h_full()
                        .w_full()
                        .disabled(self.busy()),
                ),
            )
    }

    fn placeholder(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Empty::new()
                    .header(
                        EmptyHeader::new()
                            .title(EmptyTitle::new().child("No draft open"))
                            .description(EmptyDescription::new().child(
                                "Start a new message, or open a saved draft from the list.",
                            )),
                    )
                    .child(
                        Button::new("new-message-empty")
                            .primary()
                            .label("New message")
                            .disabled(self.store.is_none() || self.identities.is_empty())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.compose_new(window, cx)),
                            ),
                    ),
            )
    }
}

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = self.draft.as_ref().map_or_else(
            || self.placeholder(cx).into_any_element(),
            |draft| self.editor(draft, cx).into_any_element(),
        );
        div()
            .size_full()
            .key_context("Composer")
            .on_action(cx.listener(|this, _: &ToggleCopies, window, cx| {
                this.toggle_copies(window, cx);
            }))
            .flex()
            .flex_col()
            .text_color(cx.theme().foreground)
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .p_3()
                        .child(Alert::error("composer-error", error).on_close(cx.listener(
                            |this, _, _, cx| {
                                this.error = None;
                                cx.notify();
                            },
                        ))),
                )
            })
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("composer-panes")
                        .child(
                            resizable_panel()
                                .size(px(300.))
                                .size_range(px(220.)..px(460.))
                                .child(self.lists(cx)),
                        )
                        .child(resizable_panel().child(main)),
                ),
            )
            .when(!self.status.is_empty(), |view| {
                view.child(StatusBar::new().left(self.status.clone()))
            })
    }
}

/// Flatten a spawned store job into one redacted, displayable result.
fn joined<T, E: ToString>(
    result: Result<Result<T, E>, tokio::task::JoinError>,
    interrupted: &str,
) -> Result<T, String> {
    result.map_or_else(
        |_| Err(interrupted.to_owned()),
        |outcome| outcome.map_err(|error| error.to_string()),
    )
}

async fn save_snapshot(store: &Handle, draft: Draft) -> Result<(), worker::Error> {
    store.ensure_account(draft.account.clone()).await?;
    store.save_draft(draft).await
}

pub fn size_label(bytes: usize) -> String {
    const KIB: usize = 1024;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < KIB * KIB {
        format!("{} KiB", bytes.div_ceil(KIB))
    } else {
        let tenths = bytes * 10 / (KIB * KIB);
        format!("{}.{} MiB", tenths / 10, tenths % 10)
    }
}

fn outcome_text(result: &Result<Outcome, String>) -> String {
    match result {
        Ok(Outcome::Accepted) => "Message accepted by the server.".into(),
        Ok(Outcome::DeliveryUnknown) => {
            "Delivery unknown. Check Sent before deciding whether to send again.".into()
        }
        Ok(Outcome::AuthenticationRequired) => {
            "The server refused the sign-in. Update the account's credentials in Settings, then retry from the outbox.".into()
        }
        Ok(Outcome::TlsFailed) => {
            "The server's certificate could not be verified, so nothing was sent.".into()
        }
        Ok(Outcome::CapabilityUnavailable) => {
            "The server lacks a required security or message capability, so nothing was sent.".into()
        }
        Ok(Outcome::CancelledBeforeSubmission) => {
            "Cancelled before anything was sent. The draft was restored.".into()
        }
        Ok(Outcome::Unreachable) => {
            "Could not reach the mail server, so nothing was sent. Retry it from the outbox.".into()
        }
        Ok(Outcome::Rejected { temporary: true }) => {
            "The server temporarily rejected this message. Retry it from the outbox.".into()
        }
        Ok(Outcome::Rejected { temporary: false }) => {
            "The server rejected this message. Edit it as a new draft from the outbox.".into()
        }
        Err(error) => error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, outcome_text, size_label};

    #[test]
    fn attachment_sizes_read_naturally_at_each_unit_boundary() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(1023), "1023 B");
        assert_eq!(size_label(1024), "1 KiB");
        assert_eq!(size_label(1025), "2 KiB");
        assert_eq!(size_label(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn only_acceptance_claims_delivery_and_unknown_never_suggests_a_blind_retry() {
        assert!(outcome_text(&Ok(Outcome::Accepted)).contains("accepted"));
        let unknown = outcome_text(&Ok(Outcome::DeliveryUnknown));
        assert!(unknown.contains("Check Sent"));
        assert!(!unknown.to_lowercase().contains("retry"));
        for refused in [
            Outcome::TlsFailed,
            Outcome::CapabilityUnavailable,
            Outcome::CancelledBeforeSubmission,
            Outcome::Unreachable,
        ] {
            assert!(!outcome_text(&Ok(refused)).contains("accepted"));
        }
        assert_eq!(outcome_text(&Err("static".into())), "static");
    }
}
