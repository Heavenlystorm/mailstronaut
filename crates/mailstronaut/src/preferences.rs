//! Staged native preferences using the standard Kit Settings fields.

use std::{path::PathBuf, sync::Arc};

use gpui::component::{
    Disableable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    searchable_list::SearchableVec,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem},
    time_field::{HourCycle, TimeField, TimeFieldEvent, TimeFieldState},
    toolbar::Toolbar,
};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, ParentElement as _, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use mailstronaut_config::{Appearance, Config, native, persistence, source::Ownership};
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

const SYSTEM_ZONE: &str = "System time zone";

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
struct Values {
    appearance: String,
    automatic: bool,
    poll_seconds: f64,
    notifications: bool,
    previews: bool,
    zone: String,
    morning: String,
    evening: String,
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
            poll_seconds: f64::from(config.sync.poll_seconds),
            notifications: config.notifications.enabled,
            previews: config.notifications.show_preview,
            zone: config.time_zone.clone().unwrap_or_default(),
            morning: config.reminders.morning.clone(),
            evening: config.reminders.evening.clone(),
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
        Ok(patch)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Idle,
    Loading,
    Saving,
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

    fn editable(&self) -> bool {
        self.snapshot.error.is_none()
            && self.snapshot.ownership == Ownership::AppOwned
            && self
                .snapshot
                .config
                .as_ref()
                .is_some_and(|config| !config.managed)
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
        let (get_poll, set_poll) =
            Self::bind(view, |v| v.poll_seconds, |v, value| v.poll_seconds = value);
        SettingGroup::new()
            .title("Synchronization")
            .item(
                SettingItem::new("Sync automatically", SettingField::switch(get, set))
                    .description("Check configured accounts while Mailstronaut is open.")
                    .disabled(disabled),
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
                SettingItem::new("Notify me about reminders", SettingField::switch(get, set))
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
        if !self.editable() || self.busy() {
            return;
        }
        let (Some(path), Some(expected), Some(draft), Some(initial)) = (
            self.snapshot.path.clone(),
            self.snapshot.config.clone(),
            &self.draft,
            &self.initial,
        ) else {
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
        let Ok(executable) = std::env::current_exe() else {
            self.error = Some("Unable to locate the configuration helper.".into());
            cx.emit(Event::Changed);
            return;
        };
        let work = self.lifetime.child_token();
        self.work = Some(work.clone());
        self.operation = Operation::Saving;
        self.error = None;
        let (generation, revision) = (self.generation, self.snapshot.revision);
        let job = self.runtime.spawn(async move {
            persistence::preferences(&executable, &path, &expected, changes, &work).await
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
                    Ok(Ok(_)) => cx.emit(Event::Saved(revision)),
                    Ok(Err(error)) => {
                        this.error = Some(error.to_string());
                        cx.emit(Event::Changed);
                    }
                    Err(_) => {
                        this.error = Some("Saving preferences was interrupted.".into());
                        cx.emit(Event::Changed);
                    }
                }
            });
        })
        .detach();
    }
}
