//! GPUI Kit mailbox workspace. Mail and storage remain toolkit-independent.

use std::{path::PathBuf, sync::Arc};

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, Theme, ThemeMode,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    resizable::{h_resizable, resizable_panel},
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, RenderImage, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, img, prelude::FluentBuilder as _, px, uniform_list,
};
use mailstronaut_config::{Appearance, Config, source};
use mailstronaut_core::{
    store::browse::Scope,
    worker::{Handle, Worker},
};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

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
        FocusReader
    ]
);

const DEMO_HTML: &str = "<style>body{font:17px sans-serif;margin:32px;color:#243044;background:#fff}h1{font-size:30px}td{padding:18px} @media(prefers-color-scheme:dark){body{background:#20242b;color:#edf1fa}}</style><h1>A little more space for your mail.</h1><p>This synthetic email is laid out by Blitz and painted by Vello CPU.</p><table style='width:100%;border-collapse:collapse'><tr><td style='background:#dce9ff;color:#173b73'>Native Wayland</td><td style='background:#e1f2e8;color:#164c34'>Rust throughout</td></tr></table><p>Tables, inline CSS and Unicode: Bonjour · Ciao · 世界.</p><p><img src='cid:demo-badge' width='96' height='32' alt='Local raster image'> Message-local PNG · no fetching.</p>";

#[derive(Clone)]
struct Row {
    account: String,
    id: i64,
    sender: String,
    subject: String,
    preview: String,
    unread: bool,
}

pub struct Workspace {
    focus: FocusHandle,
    runtime: Arc<Runtime>,
    owner: Option<Arc<Worker>>,
    accounts: Vec<String>,
    config: Option<Arc<Config>>,
    read_gate: crate::read_gate::Gate,
    rows: Vec<Row>,
    selected: Option<Row>,
    body: String,
    text_reader: Entity<TextareaState>,
    show_text: bool,
    frame: Option<Arc<RenderImage>>,
    search: Entity<InputState>,
    status: String,
    generation: u64,
    demo: bool,
    _search_events: Subscription,
    _activation: Subscription,
    _appearance: Subscription,
    theme_override: Option<ThemeMode>,
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
        ]);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search cached mail"));
        let text_reader = cx.new(|cx| TextareaState::new(window, cx));
        let events = cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.refresh(cx);
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, _| {
            this.read_gate.visibility(window.is_window_active());
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_override.is_none() {
                Theme::sync_system_appearance(Some(window), cx);
                this.refresh_demo(cx);
            }
        });
        let mut read_gate = crate::read_gate::Gate::default();
        read_gate.visibility(window.is_window_active());
        let mut this = Self {
            focus,
            runtime,
            owner: None,
            accounts: Vec::new(),
            config: None,
            read_gate,
            rows: Vec::new(),
            selected: None,
            body: String::new(),
            text_reader,
            show_text: !demo,
            frame: None,
            search,
            status: "Opening local mail…".into(),
            generation: 0,
            demo,
            _search_events: events,
            _activation: activation,
            _appearance: appearance,
            theme_override: None,
        };
        if demo {
            this.status = "Synthetic preview · no accounts or network".into();
            this.rows = [
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
                sender: sender.into(),
                subject: subject.into(),
                preview: preview.into(),
                unread: true,
            })
            .collect();
            if let Some(row) = this.rows.first().cloned() {
                this.open(row, window, cx);
            }
        } else {
            this.start(path, cx);
        }
        this
    }

    fn start(&self, path: Option<PathBuf>, cx: &Context<Self>) {
        let job = self.runtime.spawn(async move {
            let path =
                path.ok_or_else(|| "Unable to locate native Nix configuration.".to_owned())?;
            let executable =
                std::env::current_exe().map_err(|_| "Unable to locate configuration helper.")?;
            let config = source::load(path, &executable, &CancellationToken::new())
                .await
                .map_err(|error| error.to_string())?;
            let root = directories::BaseDirs::new()
                .ok_or("Unable to locate local mail storage.")?
                .data_local_dir()
                .join("mailstronaut");
            let owner = Worker::open(root)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((Arc::new(owner), config))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok((owner, config))) => {
                        this.accounts = config.as_ref().map_or_else(Vec::new, |config| {
                            config
                                .accounts
                                .iter()
                                .map(|account| account.id.clone())
                                .collect()
                        });
                        this.owner = Some(owner);
                        if let Some(config) = config {
                            this.apply_appearance(&config, cx);
                            this.config = Some(Arc::new(config));
                        }
                        this.refresh(cx);
                    }
                    Ok(Err(error)) => this.status = error,
                    Err(_) => this.status = "Unable to open local mail.".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_appearance(&mut self, config: &Config, cx: &mut Context<Self>) {
        self.theme_override = match config.appearance {
            Appearance::Dark => Some(ThemeMode::Dark),
            Appearance::Light => Some(ThemeMode::Light),
            _ => None,
        };
        if let Some(mode) = self.theme_override {
            Theme::change(mode, None, cx);
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
        self.refresh_demo(cx);
    }

    fn refresh_demo(&mut self, cx: &Context<Self>) {
        // Theme changes never reopen real messages or authorize Seen.
        if self.demo && self.selected.is_some() {
            self.generation = self.generation.wrapping_add(1);
            self.render_demo(cx);
        }
    }

    fn focus_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_some() {
            self.show_text = true;
            self.text_reader
                .update(cx, |reader, cx| reader.focus(window, cx));
            cx.notify();
        }
    }

    fn handle(&self) -> Option<Handle> {
        self.owner.as_ref().map(|owner| owner.handle())
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.handle() else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.read_gate.suppress(generation, generation);
        let accounts = self.accounts.clone();
        let query = self.search.read(cx).value().to_string();
        self.status = "Searching local cache…".into();
        let job = self.runtime.spawn(async move {
            handle
                .conversation_page(accounts, Scope::Inbox, query, None, 100)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(Ok(rows)) => {
                        this.rows = rows
                            .into_iter()
                            .map(|row| Row {
                                unread: row.unread > 0,
                                account: row.latest.account,
                                id: row.latest.id,
                                sender: row.latest.sender,
                                subject: row.latest.subject,
                                preview: row.latest.preview,
                            })
                            .collect();
                        this.status = if this.accounts.is_empty() {
                            "Add accounts in your native Nix configuration.".into()
                        } else {
                            format!("{} cached conversations · local search", this.rows.len())
                        };
                    }
                    Ok(Err(error)) => this.status = error.to_string(),
                    Err(_) => this.status = "Local query interrupted.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.read_gate.clicked(generation, generation);
        self.selected = Some(row.clone());
        self.frame = None;
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
            self.render_demo(cx);
        } else if let Some(handle) = self.handle() {
            let job = self
                .runtime
                .spawn(async move { handle.message(row.account.clone(), row.id).await });
            cx.spawn_in(window, async move |this, cx| {
                let result = job.await;
                _ = this.update_in(cx, |this, window, cx| {
                    if generation != this.generation {
                        return;
                    }
                    match result {
                        Ok(Ok(message)) => {
                            let text = message.readable_text().map_or_else(
                                || "No readable body is cached.".into(),
                                std::borrow::Cow::into_owned,
                            );
                            this.text_reader
                                .update(cx, |reader, cx| reader.set_value(text, window, cx));
                            this.body.clear();
                            if this.read_gate.take(generation, generation, true) {
                                this.mark_opened(cx);
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

    fn render_demo(&self, cx: &Context<Self>) {
        let generation = self.generation;
        let dark = cx.theme().mode.is_dark();
        let job = self.runtime.spawn_blocking(move || {
            let images = [mailstronaut_render::InlineImage {
                content_id: "demo-badge",
                media_type: "image/png",
                bytes: include_bytes!("../../../assets/inline-fixture.png"),
            }];
            mailstronaut_render::render_with_images(DEMO_HTML, &images, 780, 600, dark)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                match result {
                    Ok(Ok(mut frame)) => {
                        // GPUI's RenderImage requires BGRA, Vello CPU returns RGBA.
                        for pixel in frame.pixels.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }
                        if let Some(pixels) =
                            image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels)
                        {
                            this.frame =
                                Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])));
                            this.body.clear();
                        }
                    }
                    _ => this.body = "Unable to render the synthetic HTML fixture.".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn mark_opened(&self, cx: &Context<Self>) {
        let (Some(handle), Some(row)) = (self.handle(), self.selected.clone()) else {
            return;
        };
        let Some(account) = self.config.as_ref().and_then(|config| {
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
        cx.spawn(async move |this, cx| {
            if let Ok(Err(error)) = job.await {
                _ = this.update(cx, |this, cx| {
                    this.status = error.to_string();
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Workspace {
    fn message_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let rows = self.rows.clone();
        let weak = cx.entity().downgrade();
        uniform_list(
            "message-list",
            rows.len(),
            move |range, _, _cx: &mut App| {
                range
                    .map(|index| {
                        let row = rows[index].clone();
                        let weak = weak.clone();
                        div()
                            .id(("message", index))
                            .h(px(112.))
                            .p_4()
                            .border_b_1()
                            .border_color(border)
                            .cursor_pointer()
                            .hover(move |style| style.bg(accent))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(row.sender.clone())
                                    .child(if row.unread { "•" } else { "" }),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .font_semibold()
                                    .truncate()
                                    .child(row.subject.clone()),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_sm()
                                    .text_color(muted)
                                    .truncate()
                                    .child(row.preview.clone()),
                            )
                            .on_click(move |_, window, cx| {
                                _ = weak.update(cx, |this, cx| this.open(row.clone(), window, cx));
                            })
                    })
                    .collect::<Vec<_>>()
            },
        )
        .size_full()
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
                    div()
                        .flex()
                        .gap_2()
                        .mb_4()
                        .child(
                            Button::new("read-text")
                                .ghost()
                                .label("Text · Ctrl+Shift+T")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.focus_reader(window, cx);
                                })),
                        )
                        .when(self.demo, |view| {
                            view.child(Button::new("read-html").ghost().label("HTML").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.show_text = false;
                                    cx.notify();
                                }),
                            ))
                        }),
                )
            })
            .when(self.show_text && self.body.is_empty(), |view| {
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
            })
            .when(!self.show_text, |view| {
                view.child(
                    div()
                        .id("html-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .when_some(self.frame.clone(), |view, image| {
                            view.child(img(image).w_full())
                        }),
                )
            })
            .when(!self.body.is_empty(), |view| view.child(self.body.clone()))
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let list = self.message_list(cx).into_any_element();
        let reader = self.reader(cx).into_any_element();
        div()
            .size_full()
            .key_context("Mailstronaut")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.search.update(cx, |input, cx| input.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &RefreshCache, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ToggleAppearance, window, cx| {
                this.toggle_appearance(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusReader, window, cx| {
                this.focus_reader(window, cx);
            }))
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(gpui::component::TitleBar::new().child("Mailstronaut"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .p_4()
                    .border_b_1()
                    .border_color(border)
                    .child(div().w(px(190.)).font_semibold().child("Mailstronaut"))
                    .child(div().flex_1().child(Input::new(&self.search)))
                    .child(
                        Button::new("refresh")
                            .label("Refresh cache")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("appearance")
                            .ghost()
                            .label("Appearance")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_appearance(window, cx);
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("mail-panes")
                        .child(
                            resizable_panel()
                                .size(px(200.))
                                .size_range(px(160.)..px(300.))
                                .child(
                                    div()
                                        .p_4()
                                        .child(
                                            Button::new("inbox")
                                                .primary()
                                                .label("Unified inbox")
                                                .on_click(
                                                    cx.listener(|this, _, _, cx| this.refresh(cx)),
                                                ),
                                        )
                                        .child(div().mt_6().text_xs().text_color(muted).child(
                                            if self.demo {
                                                "DEMO · SYNTHETIC MAIL"
                                            } else {
                                                "LOCAL MAIL CACHE"
                                            },
                                        )),
                                ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(360.))
                                .size_range(px(280.)..px(600.))
                                .child(list),
                        )
                        .child(resizable_panel().child(reader)),
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
