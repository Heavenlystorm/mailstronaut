//! Staged native preferences using the standard Kit Settings fields.

use std::{path::PathBuf, sync::Arc};

use gpui::component::{
    Disableable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    description_list::DescriptionList,
    searchable_list::SearchableVec,
    setting::{NumberFieldOptions, SelectIndex, SettingField, SettingGroup, SettingItem},
    time_field::{HourCycle, TimeField, TimeFieldEvent, TimeFieldState},
    toolbar::Toolbar,
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement as _, ParentElement as _,
    PathPromptOptions, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use mailstronaut_config::{Account, Appearance, Config, native, persistence, source::Ownership};
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

const SYSTEM_ZONE: &str = "System time zone";
const MODEL_SEARCH: [&str; 6] = [
    "priority",
    "local models",
    "model revision",
    "model directory",
    "classifier",
    "ML",
];

pub enum Event {
    Changed,
    Saved(u64),
}

pub struct Snapshot {
    pub path: Option<PathBuf>,
    pub config: Option<Arc<Config>>,
    pub revision: u64,
    pub ownership: Ownership,
    pub error: Option<String>,
}

#[derive(Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent preferences mirror boolean configuration fields, not a state machine"
)]
struct Values {
    appearance: String,
    automatic: bool,
    background: bool,
    poll_seconds: f64,
    initial_days: f64,
    notifications: bool,
    previews: bool,
    zone: String,
    morning: String,
    evening: String,
    meaning: bool,
    meaning_directory: String,
    priority: bool,
    model_directory: String,
    model_revision: String,
}

impl Values {
    fn from_config(config: &Config) -> Self {
        Self {
            appearance: match config.appearance {
                Appearance::Dark => "dark",
                Appearance::Light => "light",
                _ => "system",
            }
            .into(),
            automatic: config.sync.automatic,
            background: config.background.enabled,
            poll_seconds: f64::from(config.sync.poll_seconds),
            initial_days: f64::from(config.sync.initial_days.unwrap_or_default()),
            notifications: config.notifications.enabled,
            previews: config.notifications.show_preview,
            zone: config.time_zone.clone().unwrap_or_default(),
            morning: config.reminders.morning.clone(),
            evening: config.reminders.evening.clone(),
            meaning: config.search.semantic,
            meaning_directory: config.search.model_directory.clone().unwrap_or_default(),
            priority: config.priority.enabled,
            model_directory: config.priority.model_directory.clone().unwrap_or_default(),
            model_revision: config.priority.model_revision.clone().unwrap_or_default(),
        }
    }

    #[expect(
        clippy::float_cmp,
        reason = "Compare staged widget values exactly; a tolerance could hide an unsaved edit"
    )]
    fn patch(&self, baseline: &Self) -> Result<Value, &'static str> {
        let mut patch = json!({});
        if self.appearance != baseline.appearance {
            native::merge(&mut patch, json!({"appearance": self.appearance}));
        }
        if self.automatic != baseline.automatic {
            native::merge(&mut patch, json!({"sync": {"automatic": self.automatic}}));
        }
        if self.background != baseline.background {
            native::merge(
                &mut patch,
                json!({"background": {"enabled": self.background}}),
            );
        }
        if self.poll_seconds != baseline.poll_seconds {
            // Kit supplies f64; standard parsing accepts only a whole, nonnegative
            // u32 without a lossy numeric cast. The canonical schema sets bounds.
            let seconds = self
                .poll_seconds
                .to_string()
                .parse::<u32>()
                .map_err(|_| "sync.pollSeconds requires a whole number of seconds.")?;
            native::merge(&mut patch, json!({"sync": {"pollSeconds": seconds}}));
        }
        if self.initial_days != baseline.initial_days {
            let days = self
                .initial_days
                .to_string()
                .parse::<u32>()
                .map_err(|_| "sync.initialDays requires a whole number of days.")?;
            native::merge(
                &mut patch,
                json!({"sync": {"initialDays": if days == 0 { None } else { Some(days) }}}),
            );
        }
        if self.notifications != baseline.notifications {
            native::merge(
                &mut patch,
                json!({"notifications": {"enabled": self.notifications}}),
            );
        }
        if self.previews != baseline.previews {
            native::merge(
                &mut patch,
                json!({"notifications": {"showPreview": self.previews}}),
            );
        }
        if self.zone != baseline.zone {
            let zone = self.zone.trim();
            native::merge(
                &mut patch,
                json!({"timeZone": if zone.is_empty() { None } else { Some(zone) }}),
            );
        }
        if self.morning != baseline.morning {
            native::merge(
                &mut patch,
                json!({"reminders": {"morning": self.morning.trim()}}),
            );
        }
        if self.evening != baseline.evening {
            native::merge(
                &mut patch,
                json!({"reminders": {"evening": self.evening.trim()}}),
            );
        }
        if self.meaning != baseline.meaning || self.meaning_directory != baseline.meaning_directory
        {
            let directory = self.meaning_directory.trim();
            native::merge(
                &mut patch,
                json!({"search": {"semantic": self.meaning, "modelDirectory": if directory.is_empty() { None } else { Some(directory) }}}),
            );
        }
        if self.priority != baseline.priority {
            native::merge(&mut patch, json!({"priority": {"enabled": self.priority}}));
        }
        if self.model_directory != baseline.model_directory
            || self.model_revision != baseline.model_revision
        {
            let directory = self.model_directory.trim();
            let revision = self.model_revision.trim();
            native::merge(
                &mut patch,
                json!({"priority": {
                    "modelDirectory": if directory.is_empty() { None } else { Some(directory) },
                    "modelRevision": if revision.is_empty() { None } else { Some(revision) }
                }}),
            );
        }
        Ok(patch)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Idle,
    Loading,
    Saving,
    AddingAccount,
    Initializing,
    ChoosingSearchModel,
}

enum Destination {
    Account(String),
    AddAccount,
}

enum WriteRequest {
    Preferences(Value),
    Account(Box<Account>),
    Logins {
        id: String,
        incoming: String,
        outgoing: Option<String>,
    },
    Initialize,
}

pub struct Preferences {
    runtime: Arc<Runtime>,
    lifetime: CancellationToken,
    work: Option<CancellationToken>,
    snapshot: Snapshot,
    initial: Option<Values>,
    draft: Option<Values>,
    operation: Operation,
    error: Option<String>,
    last_reload: Option<i64>,
    generation: u64,
    destination: Option<Destination>,
    morning: Entity<TimeFieldState>,
    evening: Entity<TimeFieldState>,
    zone: Entity<ComboboxState<SearchableVec<SharedString>>>,
    _field_events: Vec<Subscription>,
}
impl EventEmitter<Event> for Preferences {}

impl Preferences {
    pub fn new(
        runtime: Arc<Runtime>,
        lifetime: CancellationToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let morning = cx.new(|cx| TimeFieldState::new(window, cx).hour_cycle(HourCycle::H23));
        let evening = cx.new(|cx| TimeFieldState::new(window, cx).hour_cycle(HourCycle::H23));
        let mut zones = vec![SharedString::from(SYSTEM_ZONE)];
        zones.extend(
            chrono_tz::TZ_VARIANTS
                .iter()
                .map(|zone| SharedString::from(zone.name())),
        );
        let zone = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(zones), Vec::new(), window, cx).searchable(true)
        });
        let mut events = Vec::new();
        for (field, is_morning) in [(&morning, true), (&evening, false)] {
            events.push(
                cx.subscribe(field, move |this, _, event: &TimeFieldEvent, cx| {
                    if this.editable()
                        && !this.busy()
                        && let Some(draft) = &mut this.draft
                    {
                        let TimeFieldEvent::Change(time) = event;
                        let value = time.format("%H:%M").to_string();
                        if is_morning {
                            draft.morning = value;
                        } else {
                            draft.evening = value;
                        }
                        this.error = None;
                        cx.emit(Event::Changed);
                    }
                }),
            );
        }
        events.push(cx.subscribe(
            &zone,
            |this, _, event: &ComboboxEvent<SearchableVec<SharedString>>, cx| {
                if this.editable()
                    && !this.busy()
                    && let Some(draft) = &mut this.draft
                {
                    // This pin can reuse a filtered row index and skip Change.
                    // Confirm still supplies the actual selection on dismissal.
                    let (ComboboxEvent::Change(values) | ComboboxEvent::Confirm(values)) = event;
                    draft.zone = values
                        .first()
                        .filter(|value| value.as_ref() != SYSTEM_ZONE)
                        .map_or_else(String::new, ToString::to_string);
                    this.error = None;
                    cx.emit(Event::Changed);
                }
            },
        ));
        Self {
            runtime,
            lifetime,
            work: None,
            snapshot: Snapshot {
                path: None,
                config: None,
                revision: 0,
                ownership: Ownership::Unavailable,
                error: None,
            },
            initial: None,
            draft: None,
            operation: Operation::Idle,
            error: None,
            last_reload: None,
            generation: 0,
            destination: None,
            morning,
            evening,
            zone,
            _field_events: events,
        }
    }

    pub fn configure(&mut self, snapshot: Snapshot, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(work) = self.work.take() {
            work.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        self.operation = Operation::Idle;
        self.last_reload = Some(chrono::Utc::now().timestamp());
        self.error = snapshot.error.clone();
        if snapshot.error.is_none() || self.initial.is_none() {
            self.initial = snapshot.config.as_deref().map(Values::from_config);
            self.draft = self.initial.clone();
        }
        self.snapshot = snapshot;
        self.reset_fields(window, cx);
        cx.emit(Event::Changed);
    }

    fn reset_fields(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        for (field, value) in [
            (&self.morning, &draft.morning),
            (&self.evening, &draft.evening),
        ] {
            if let Ok(time) = chrono::NaiveTime::parse_from_str(value, "%H:%M") {
                field.update(cx, |field, cx| field.set_time(time, window, cx));
            }
        }
        let zone = if draft.zone.is_empty() {
            SYSTEM_ZONE
        } else {
            &draft.zone
        };
        self.zone.update(cx, |field, cx| {
            field.set_selected_values(&[SharedString::from(zone.to_owned())], window, cx);
        });
    }

    pub fn loading(&mut self, cx: &mut Context<Self>) {
        self.operation = Operation::Loading;
        self.error = None;
        cx.emit(Event::Changed);
    }

    pub fn failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.operation = Operation::Idle;
        self.snapshot.ownership = Ownership::Unavailable;
        self.snapshot.error = Some(error.clone());
        self.error = Some(error);
        self.last_reload = Some(chrono::Utc::now().timestamp());
        cx.emit(Event::Changed);
    }

    pub fn busy(&self) -> bool {
        self.operation != Operation::Idle
    }

    pub fn editable(&self) -> bool {
        self.snapshot.error.is_none()
            && self.snapshot.ownership == Ownership::AppOwned
            && self
                .snapshot
                .config
                .as_ref()
                .is_some_and(|config| !config.managed)
    }

    pub fn dirty(&self) -> bool {
        self.initial != self.draft
    }

    pub fn can_initialize(&self) -> bool {
        !self.busy()
            && self.snapshot.ownership == Ownership::Missing
            && self.snapshot.config.is_none()
            && self.snapshot.error.is_none()
    }

    pub fn preferred_account(&self) -> Option<&str> {
        match &self.destination {
            Some(Destination::Account(id)) => Some(id),
            _ => None,
        }
    }

    const fn prefers_setup(&self) -> bool {
        matches!(self.destination, Some(Destination::AddAccount))
    }

    pub fn page_after_reload(&self) -> SelectIndex {
        let page_ix = self.snapshot.config.as_ref().map_or(0, |config| {
            self.preferred_account()
                .and_then(|id| config.accounts.iter().position(|account| account.id == id))
                .map_or_else(
                    || {
                        if self.prefers_setup() && self.editable() {
                            config.accounts.len() + 1
                        } else {
                            0
                        }
                    },
                    |index| index + 1,
                )
        });
        SelectIndex {
            page_ix,
            group_ix: None,
        }
    }

    pub fn source_details(view: &Entity<Self>, cx: &App) -> DescriptionList {
        let state = view.read(cx);
        let path = state.snapshot.path.as_ref().map_or_else(
            || "No configuration path is available.".to_owned(),
            |path| path.display().to_string(),
        );
        DescriptionList::vertical()
            .columns(1)
            .item(
                "Source",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(path.clone()))
                    .child(
                        Clipboard::new("copy-source")
                            .value(path)
                            .accessibility_label("Copy configuration path")
                            .tooltip("Copy configuration path"),
                    )
                    .into_any_element(),
                1,
            )
            .item("Last reload", state.last_reload(), 1)
    }

    pub fn add_account(&mut self, account: Account, window: &Window, cx: &mut Context<Self>) {
        if self.dirty() {
            self.error =
                Some("Save or discard pending preferences before adding an account.".into());
            cx.emit(Event::Changed);
            return;
        }
        self.submit(WriteRequest::Account(Box::new(account)), window, cx);
    }

    pub fn save_logins(
        &mut self,
        id: String,
        incoming: String,
        outgoing: Option<String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.dirty() {
            self.error =
                Some("Save or discard pending preferences before changing account logins.".into());
            cx.emit(Event::Changed);
            return;
        }
        self.submit(
            WriteRequest::Logins {
                id,
                incoming,
                outgoing,
            },
            window,
            cx,
        );
    }

    pub fn setup(view: &Entity<Self>, cx: &App) -> SettingGroup {
        let state = view.read(cx);
        let disabled = !state.can_initialize();
        let creating = state.operation == Operation::Initializing;
        let editor = view.clone();
        SettingGroup::new().title("Get started")
            .description("Install your Home Manager declaration, or create an app-owned native Nix file and add accounts here.")
            .item(SettingItem::render(move |options, _, _| {
                let editor = editor.clone();
                Button::new("initialize-config")
                    .label(if creating { "Creating…" } else { "Create native configuration" })
                    .disabled(disabled || options.is_disabled())
                    .on_click(move |_, window, cx| editor.update(cx, |this, cx| this.submit(WriteRequest::Initialize, window, cx)))
            }).keywords(["create", "initialize", "setup", "configuration"]))
    }

    pub fn ownership_label(&self) -> &'static str {
        if self
            .snapshot
            .config
            .as_ref()
            .is_some_and(|config| config.managed)
        {
            Ownership::Managed.label()
        } else if self.snapshot.ownership == Ownership::AppOwned {
            "Editable native Nix file · Home Manager declarations reapply on activation"
        } else {
            self.snapshot.ownership.label()
        }
    }

    pub fn last_reload(&self) -> String {
        self.last_reload.map_or_else(
            || "Not loaded".into(),
            |time| {
                let zone = crate::mailbox::local_zone(
                    self.snapshot
                        .config
                        .as_ref()
                        .and_then(|config| config.time_zone.as_deref()),
                );
                crate::mailbox::date(time, &zone)
            },
        )
    }

    fn bind<T: Clone + Default + 'static>(
        view: &Entity<Self>,
        get: fn(&Values) -> T,
        set: fn(&mut Values, T),
    ) -> (impl Fn(&App) -> T + 'static, impl Fn(T, &mut App) + 'static) {
        let (read, write) = (view.clone(), view.clone());
        (
            move |cx| read.read(cx).draft.as_ref().map_or_else(T::default, get),
            move |value, cx| {
                write.update(cx, |this, cx| {
                    if this.editable()
                        && !this.busy()
                        && let Some(draft) = &mut this.draft
                    {
                        set(draft, value);
                        this.error = None;
                        cx.emit(Event::Changed);
                    }
                });
            },
        )
    }

    pub fn appearance(view: &Entity<Self>, cx: &App) -> SettingGroup {
        let disabled = !view.read(cx).editable() || view.read(cx).busy();
        let (get, set) = Self::bind::<SharedString>(
            view,
            |v| v.appearance.clone().into(),
            |v, value| v.appearance = value.to_string(),
        );
        SettingGroup::new().title("Appearance").item(
            SettingItem::new(
                "Appearance",
                SettingField::dropdown(
                    vec![
                        ("system".into(), "System".into()),
                        ("light".into(), "Light".into()),
                        ("dark".into(), "Dark".into()),
                    ],
                    get,
                    set,
                ),
            )
            .description(
                "Save to apply your preferred theme. The toolbar toggle is a temporary preview.",
            )
            .disabled(disabled),
        )
    }

    pub fn sync(view: &Entity<Self>, cx: &App) -> SettingGroup {
        let disabled = !view.read(cx).editable() || view.read(cx).busy();
        let (get, set) = Self::bind(view, |v| v.automatic, |v, value| v.automatic = value);
        let (get_background, set_background) =
            Self::bind(view, |v| v.background, |v, value| v.background = value);
        let (get_poll, set_poll) =
            Self::bind(view, |v| v.poll_seconds, |v, value| v.poll_seconds = value);
        let (get_days, set_days) =
            Self::bind(view, |v| v.initial_days, |v, value| v.initial_days = value);
        SettingGroup::new()
            .title("Synchronization")
            .item(
                SettingItem::new("Sync automatically", SettingField::switch(get, set))
                    .description("Check configured accounts while Mailstronaut is running.")
                    .disabled(disabled),
            )
            .item(
                SettingItem::new("Keep running after closing", SettingField::switch(get_background, set_background))
                    .description(if ashpd::is_sandboxed() {
                        "Background permission for Flatpak is not available yet."
                    } else {
                        "Keep syncing and reminders active after the window closes. Press Ctrl+Q to quit completely."
                    })
                    .disabled(disabled || ashpd::is_sandboxed()),
            )
            .item(
                SettingItem::new(
                    "Poll interval (seconds)",
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: 30.,
                            max: 86400.,
                            step: 30.,
                        },
                        get_poll,
                        set_poll,
                    ),
                )
                .disabled(disabled),
            )
            .item(
                SettingItem::new("History limit (days)", SettingField::number_input(
                    NumberFieldOptions { min: 0., max: 3650., step: 1. }, get_days, set_days,
                )).description("0 fetches all history. A limit affects future fetching; it does not delete cached mail.").disabled(disabled),
            )
    }

    pub fn notifications(view: &Entity<Self>, cx: &App) -> SettingGroup {
        let disabled = !view.read(cx).editable() || view.read(cx).busy();
        let (get, set) = Self::bind(
            view,
            |v| v.notifications,
            |v, value| v.notifications = value,
        );
        let (get_preview, set_preview) =
            Self::bind(view, |v| v.previews, |v, value| v.previews = value);
        SettingGroup::new()
            .title("Notifications")
            .item(
                SettingItem::new(
                    "Notify me about new mail and reminders",
                    SettingField::switch(get, set),
                )
                .disabled(disabled),
            )
            .item(
                SettingItem::new(
                    "Include message previews",
                    SettingField::switch(get_preview, set_preview),
                )
                .description("Notification previews can reveal private mail on screen.")
                .disabled(disabled),
            )
    }

    pub fn search(
        view: &Entity<Self>,
        cx: &App,
        state: Option<crate::mail_semantic::State>,
    ) -> SettingGroup {
        let disabled = !view.read(cx).editable() || view.read(cx).busy();
        let (enabled, set_enabled) = Self::bind(view, |v| v.meaning, |v, value| v.meaning = value);
        let (directory, set_directory) = Self::bind::<SharedString>(
            view,
            |v| v.meaning_directory.clone().into(),
            |v, value| v.meaning_directory = value.to_string(),
        );
        let chooser = view.clone();
        SettingGroup::new().title("Search")
            .item(SettingItem::new("Search by meaning", SettingField::switch(enabled,set_enabled))
                .description("Enable the Related search button for cross-language suggestions. Results may include unrelated mail. Runs on this device; quotes keep searches literal.")
                .keywords(["semantic", "meaning", "search", "model"]).disabled(disabled))
            .item(SettingItem::new("Search model directory", SettingField::input(directory,set_directory))
                .description("Explicitly install the pinned semantic-model Nix package and use its directory. No automatic downloads; independent of priority mail.")
                .keywords(["semantic", "meaning", "search", "model"]).disabled(disabled))
            .item(SettingItem::new("Installed search model", SettingField::render(move |options, _, _| {
                let chooser = chooser.clone();
                Button::new("choose-search-model").label("Choose folder…").disabled(options.is_disabled())
                    .on_click(move |_, window, cx| chooser.update(cx, |this, cx| this.choose_search_model(window, cx)))
            })).keywords(["semantic", "meaning", "search", "model"]).disabled(disabled))
            .item(SettingItem::render(move |_,_,_| {
                match &state {
                    Some(state) if matches!(state, crate::mail_semantic::State::Unavailable(_)) => Alert::warning("meaning-model-state",state.label()),
                    Some(state) => Alert::info("meaning-model-state",state.label()),
                    None => Alert::info("meaning-model-state","Meaning search is disabled. Save preferences to apply changes."),
                }
            }).keywords(["meaning", "semantic", "search", "model"]))
    }

    fn choose_search_model(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.editable() || self.busy() {
            return;
        }
        self.operation = Operation::ChoosingSearchModel;
        let generation = self.generation;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose the installed search model folder".into()),
        });
        cx.emit(Event::Changed);
        cx.spawn_in(window, async move |this, cx| {
            let chosen = paths.await;
            _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation || this.operation != Operation::ChoosingSearchModel
                {
                    return;
                }
                this.operation = Operation::Idle;
                match chosen {
                    Ok(Ok(None)) => {}
                    Ok(Ok(Some(paths))) => {
                        let path = match paths.as_slice() {
                            [path] if path.is_absolute() => path.to_str(),
                            _ => None,
                        };
                        if let Some(path) = path {
                            if let Some(draft) = &mut this.draft {
                                path.clone_into(&mut draft.meaning_directory);
                                this.error = None;
                            }
                        } else {
                            this.error =
                                Some("Choose one folder with an absolute UTF-8 path.".into());
                        }
                    }
                    _ => {
                        this.error = Some(
                            "Unable to choose a model folder. The current setting was preserved."
                                .into(),
                        );
                    }
                }
                cx.emit(Event::Changed);
            });
        })
        .detach();
    }

    pub fn priority(view: &Entity<Self>, cx: &App) -> SettingGroup {
        let disabled = !view.read(cx).editable() || view.read(cx).busy();
        let (enabled, set_enabled) =
            Self::bind(view, |v| v.priority, |v, value| v.priority = value);
        let (directory, set_directory) = Self::bind::<SharedString>(
            view,
            |v| v.model_directory.clone().into(),
            |v, value| v.model_directory = value.to_string(),
        );
        let (revision, set_revision) = Self::bind::<SharedString>(
            view,
            |v| v.model_revision.clone().into(),
            |v, value| v.model_revision = value.to_string(),
        );
        SettingGroup::new().title("Priority and local models")
            .item(SettingItem::render(|_, _, _| Alert::info("local-model-status",
                "Priority currently uses local rules. Model inference is not implemented yet; model declarations below are reserved and do not load or download a model."))
                .keywords(MODEL_SEARCH))
            .item(SettingItem::new("Priority mail", SettingField::switch(enabled, set_enabled))
                .description("Use local rules to highlight unread, time-sensitive mail.")
                .keywords(MODEL_SEARCH).disabled(disabled))
            .item(SettingItem::new("Local model directory", SettingField::input(directory, set_directory))
                .description("Optional absolute path. Set together with a pinned revision; no automatic downloads.")
                .keywords(MODEL_SEARCH).disabled(disabled))
            .item(SettingItem::new("Model revision", SettingField::input(revision, set_revision))
                .description("The exact model revision for your native Nix declaration. Not executed by this build.")
                .keywords(MODEL_SEARCH).disabled(disabled))
    }

    pub fn times(view: &Entity<Self>, cx: &App) -> SettingGroup {
        let disabled = !view.read(cx).editable() || view.read(cx).busy();
        let zone = view.read(cx).zone.clone();
        let morning = view.read(cx).morning.clone();
        let evening = view.read(cx).evening.clone();
        SettingGroup::new()
            .title("Time and reminders")
            .item(
                SettingItem::new(
                    "Time zone",
                    SettingField::render(move |options, _, _| {
                        Combobox::new(&zone)
                            .w(px(240.))
                            .disabled(options.is_disabled())
                    }),
                )
                .description("Search an IANA time zone or choose the system time zone.")
                .disabled(disabled),
            )
            .item(
                SettingItem::new(
                    "Morning",
                    SettingField::render(move |options, _, _| {
                        TimeField::new(&morning)
                            .w(px(110.))
                            .disabled(options.is_disabled())
                    }),
                )
                .disabled(disabled),
            )
            .item(
                SettingItem::new(
                    "Evening",
                    SettingField::render(move |options, _, _| {
                        TimeField::new(&evening)
                            .w(px(110.))
                            .disabled(options.is_disabled())
                    }),
                )
                .disabled(disabled),
            )
    }

    // This Kit version has no Settings footer. Keep the standard Toolbar outside
    // the filtered pages, so Save and Discard stay reachable while searching.
    pub fn footer(view: &Entity<Self>, cx: &App) -> impl gpui::IntoElement {
        let (save, discard) = (view.clone(), view.clone());
        let state = view.read(cx);
        let dirty = state.initial != state.draft;
        let busy = state.busy();
        let saving = state.operation == Operation::Saving;
        let editable = state.editable();
        let error = state.error.clone();
        let toolbar = Toolbar::new("preference-actions")
            .content(
                Button::new("save-preferences")
                    .label(if saving {
                        "Saving…"
                    } else {
                        "Save preferences"
                    })
                    .disabled(busy || !editable)
                    .on_click(move |_, window, cx| {
                        save.update(cx, |this, cx| this.save(window, cx));
                    }),
            )
            .content(
                Button::new("discard-preferences")
                    .ghost()
                    .label("Discard changes")
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        discard.update(cx, |this, cx| {
                            this.draft = this.initial.clone();
                            this.error.clone_from(&this.snapshot.error);
                            this.reset_fields(window, cx);
                            cx.emit(Event::Changed);
                        });
                    }),
            );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(error, |view, message| {
                view.child(Alert::error("preference-error", message))
            })
            .when(dirty || saving, |view| view.child(toolbar))
    }

    fn save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (Some(draft), Some(initial)) = (&self.draft, &self.initial) else {
            return;
        };
        let changes = match draft.patch(initial) {
            Ok(changes) => changes,
            Err(message) => {
                self.error = Some(message.into());
                cx.emit(Event::Changed);
                return;
            }
        };
        self.submit(WriteRequest::Preferences(changes), window, cx);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One bounded operation coordinates source ownership, cancellation, and revision-fenced completion for all native writes"
    )]
    fn submit(&mut self, request: WriteRequest, window: &Window, cx: &mut Context<Self>) {
        let initializing = matches!(request, WriteRequest::Initialize);
        if self.busy()
            || if initializing {
                !self.can_initialize()
            } else {
                !self.editable()
            }
        {
            return;
        }
        let Some(path) = self.snapshot.path.clone() else {
            return;
        };
        let Ok(executable) = std::env::current_exe() else {
            self.error = Some("Unable to locate the configuration helper.".into());
            cx.emit(Event::Changed);
            return;
        };
        let expected = self.snapshot.config.clone();
        let preferred = match &request {
            WriteRequest::Account(account) => Some(Destination::Account(account.id.clone())),
            WriteRequest::Logins { id, .. } => Some(Destination::Account(id.clone())),
            WriteRequest::Initialize => Some(Destination::AddAccount),
            WriteRequest::Preferences(_) => None,
        };
        self.operation = match &request {
            WriteRequest::Initialize => Operation::Initializing,
            WriteRequest::Account(_) => Operation::AddingAccount,
            WriteRequest::Logins { .. } | WriteRequest::Preferences(_) => Operation::Saving,
        };
        let work = self.lifetime.child_token();
        self.work = Some(work.clone());
        self.error = None;
        let (generation, revision) = (self.generation, self.snapshot.revision);
        let job = self.runtime.spawn(async move {
            match request {
                WriteRequest::Initialize => {
                    if work.is_cancelled() {
                        return Err(persistence::Error::Evaluation(
                            mailstronaut_config::process::EvaluationError::Cancelled,
                        ));
                    }
                    tokio::task::spawn_blocking(move || persistence::initialize(&path))
                        .await
                        .map_err(|_| persistence::Error::Task)?
                }
                WriteRequest::Account(account) => persistence::add_account(
                    &executable,
                    &path,
                    expected.as_deref().ok_or(persistence::Error::Task)?,
                    *account,
                    &work,
                )
                .await
                .map(|_| ()),
                WriteRequest::Logins {
                    id,
                    incoming,
                    outgoing,
                } => persistence::account_logins(
                    &executable,
                    &path,
                    expected.as_deref().ok_or(persistence::Error::Task)?,
                    &id,
                    incoming,
                    outgoing,
                    &work,
                )
                .await
                .map(|_| ()),
                WriteRequest::Preferences(changes) => persistence::preferences(
                    &executable,
                    &path,
                    expected.as_deref().ok_or(persistence::Error::Task)?,
                    changes,
                    &work,
                )
                .await
                .map(|_| ()),
            }
        });
        cx.emit(Event::Changed);
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.operation = Operation::Idle;
                this.work = None;
                match result {
                    Ok(Ok(())) => {
                        this.destination = preferred;
                        cx.emit(Event::Saved(revision));
                    }
                    Ok(Err(error)) => {
                        this.error = Some(error.to_string());
                        cx.emit(Event::Changed);
                    }
                    Err(_) => {
                        this.error = Some("Saving native configuration was interrupted.".into());
                        cx.emit(Event::Changed);
                    }
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_preference_patches_only_the_explicit_setting() {
        let config: Config =
            serde_json::from_value(json!({"schemaVersion": 1, "accounts": []})).expect("config");
        let baseline = Values::from_config(&config);
        let mut changed = baseline.clone();
        changed.background = true;
        assert_eq!(
            changed.patch(&baseline).expect("enable"),
            json!({"background": {"enabled": true}})
        );
        assert_eq!(
            baseline.patch(&changed).expect("disable"),
            json!({"background": {"enabled": false}})
        );
    }

    #[test]
    fn model_preferences_patch_only_changed_fields_and_clear_pair() {
        let config: Config =
            serde_json::from_value(json!({"schemaVersion": 1, "accounts": []})).expect("config");
        let baseline = Values::from_config(&config);
        let mut draft = baseline.clone();
        draft.priority = true;
        draft.model_directory = " /models/local-classifier ".into();
        draft.model_revision = " pinned-revision ".into();
        assert_eq!(
            draft.patch(&baseline).expect("patch"),
            json!({"priority": {
                "enabled": true, "modelDirectory": "/models/local-classifier", "modelRevision": "pinned-revision"
            }})
        );
        let mut cleared = draft.clone();
        cleared.model_directory.clear();
        cleared.model_revision.clear();
        assert_eq!(
            cleared.patch(&draft).expect("clear"),
            json!({"priority": {
                "modelDirectory": null, "modelRevision": null
            }})
        );
        assert_eq!(draft.patch(&draft).expect("unchanged"), json!({}));
    }

    #[test]
    fn history_limit_can_be_set_and_cleared_without_changing_other_settings() {
        let config: Config =
            serde_json::from_value(json!({"schemaVersion": 1, "accounts": []})).expect("config");
        let baseline = Values::from_config(&config);
        let mut draft = baseline.clone();
        draft.initial_days = 30.;
        assert_eq!(
            draft.patch(&baseline).expect("limit"),
            json!({"sync": {"initialDays": 30}})
        );
        assert_eq!(
            baseline.patch(&draft).expect("clear"),
            json!({"sync": {"initialDays": null}})
        );
        draft.initial_days = 1.5;
        assert!(draft.patch(&baseline).is_err());
    }
}
