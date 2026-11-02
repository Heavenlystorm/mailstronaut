//! Native account declarations; credentials remain in the existing account pages.

mod model;

use gpui::component::{
    Disableable as _, IndexPath,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    collapsible::Collapsible,
    form::{Field, Form},
    input::{
        Input, InputGroup, InputGroupAddon, InputGroupAddonAlignment, InputGroupButton, InputState,
        NumberInput, Textarea, TextareaState,
    },
    searchable_list::SearchableListItem,
    select::{Select, SelectEvent, SelectState},
    setting::{SettingGroup, SettingItem, SettingPage},
    switch::Switch,
};
use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions,
    Render, SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};

use crate::preferences::{Event, Preferences};

#[derive(Clone)]
struct Choice {
    id: &'static str,
    label: &'static str,
}
impl SearchableListItem for Choice {
    type Value = &'static str;
    fn title(&self) -> SharedString {
        self.label.into()
    }
    fn value(&self) -> &Self::Value {
        &self.id
    }
}
type Menu = Entity<SelectState<Vec<Choice>>>;

struct Inputs {
    id: Entity<InputState>,
    name: Entity<InputState>,
    address: Entity<InputState>,
    login: Entity<InputState>,
    smtp_login: Entity<InputState>,
    imap_host: Entity<InputState>,
    imap_port: Entity<InputState>,
    smtp_host: Entity<InputState>,
    smtp_port: Entity<InputState>,
    client: Entity<InputState>,
    authorization: Entity<InputState>,
    token: Entity<InputState>,
    scopes: Entity<InputState>,
    aliases: Entity<TextareaState>,
    ca: Entity<InputState>,
    sent: Entity<InputState>,
    archive: Entity<InputState>,
    trash: Entity<InputState>,
}
impl Inputs {
    fn new(window: &mut Window, cx: &mut Context<Onboarding>) -> Self {
        let line =
            |placeholder: &'static str, window: &mut Window, cx: &mut Context<Onboarding>| {
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
            };
        let port = |default: &'static str, window: &mut Window, cx: &mut Context<Onboarding>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(default)
                    .step(1.)
                    .min(1.)
                    .max(65535.)
            })
        };
        Self {
            id: line("personal, work, university…", window, cx),
            name: line("Your name", window, cx),
            address: line("you@example.org", window, cx),
            login: line("Defaults to your email address", window, cx),
            smtp_login: line("Defaults to the incoming login", window, cx),
            imap_host: line("imap.example.org", window, cx),
            imap_port: port("993", window, cx),
            smtp_host: line("smtp.example.org", window, cx),
            smtp_port: port("587", window, cx),
            client: line("Registered public desktop client ID", window, cx),
            authorization: line("https://identity.example.org/authorize", window, cx),
            token: line("https://identity.example.org/token", window, cx),
            scopes: line("Space-separated mail scopes", window, cx),
            aliases: cx.new(|cx| TextareaState::new(window, cx)),
            ca: line("System trust store", window, cx),
            sent: line("Discovered from the server", window, cx),
            archive: line("Discovered from the server", window, cx),
            trash: line("Discovered from the server", window, cx),
        }
    }
}

pub struct Onboarding {
    editor: Entity<Preferences>,
    inputs: Inputs,
    provider: Menu,
    authentication: Menu,
    imap_tls: Menu,
    smtp_tls: Menu,
    advanced: bool,
    separate_password: bool,
    certificates: Vec<String>,
    choosing_ca: bool,
    error: Option<&'static str>,
    submitted: Option<String>,
    incoming_reference: String,
    outgoing_reference: String,
    _events: Vec<Subscription>,
}

impl Onboarding {
    pub fn new(editor: Entity<Preferences>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let menu = |choices: &[(&'static str, &'static str)],
                    selected: usize,
                    window: &mut Window,
                    cx: &mut Context<Self>| {
            let choices = choices
                .iter()
                .map(|&(id, label)| Choice { id, label })
                .collect::<Vec<_>>();
            cx.new(|cx| SelectState::new(choices, Some(IndexPath::new(selected)), window, cx))
        };
        let provider = menu(
            &[
                ("google", "Google"),
                ("icloud", "iCloud"),
                ("manual", "Manual configuration"),
            ],
            2,
            window,
            cx,
        );
        let authentication = menu(
            &[
                ("password", "Password / app password"),
                ("microsoft", "Microsoft OAuth"),
                ("custom", "Custom OAuth"),
            ],
            0,
            window,
            cx,
        );
        let security = [("tls", "TLS"), ("starttls", "Required STARTTLS")];
        let imap_tls = menu(&security, 0, window, cx);
        let smtp_tls = menu(&security, 1, window, cx);
        let mut events = Vec::new();
        for field in [&provider, &authentication, &imap_tls, &smtp_tls] {
            events.push(
                cx.subscribe(field, |this, _, _: &SelectEvent<Vec<Choice>>, cx| {
                    this.error = None;
                    cx.notify();
                }),
            );
        }
        events.push(cx.subscribe_in(
            &editor,
            window,
            |this, editor, event: &Event, window, cx| {
                if matches!(event, Event::Saved(_))
                    && this
                        .submitted
                        .as_deref()
                        .is_some_and(|id| editor.read(cx).preferred_account() == Some(id))
                {
                    this.reset(window, cx);
                }
                cx.notify();
            },
        ));
        Self {
            editor,
            inputs: Inputs::new(window, cx),
            provider,
            authentication,
            imap_tls,
            smtp_tls,
            advanced: false,
            separate_password: false,
            certificates: Vec::new(),
            choosing_ca: false,
            error: None,
            submitted: None,
            incoming_reference: uuid::Uuid::new_v4().to_string(),
            outgoing_reference: uuid::Uuid::new_v4().to_string(),
            _events: events,
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inputs = Inputs::new(window, cx);
        for (menu, index) in [
            (&self.provider, 2),
            (&self.authentication, 0),
            (&self.imap_tls, 0),
            (&self.smtp_tls, 1),
        ] {
            menu.update(cx, |menu, cx| {
                menu.set_selected_index(Some(IndexPath::new(index)), window, cx);
            });
        }
        self.submitted = None;
        self.incoming_reference = uuid::Uuid::new_v4().to_string();
        self.outgoing_reference = uuid::Uuid::new_v4().to_string();
        self.certificates.clear();
        self.separate_password = false;
        self.advanced = false;
        self.error = None;
    }

    pub fn page(view: &Entity<Self>) -> SettingPage {
        let view = view.clone();
        SettingPage::new("Add account")
            .description(
                "Save a native account declaration, then sign in through its account page. Home Manager reapplies declared account lists on activation; export your changes to retain them.",
            )
            .resettable(false)
            .group(SettingGroup::new().item(
                SettingItem::render(move |_, _, _| view.clone()).keywords([
                    "add", "setup", "Google", "iCloud", "manual", "IMAP", "SMTP", "account",
                    "provider", "email", "name", "login", "authentication", "password", "OAuth",
                    "TLS", "certificate", "CA", "folder", "alias", "client ID",
                ]),
            ))
    }

    fn selected(menu: &Menu, cx: &App) -> &'static str {
        menu.read(cx).selected_value().copied().unwrap_or("")
    }

    fn draft(&self, cx: &App) -> model::Draft {
        let value = |input: &Entity<InputState>| input.read(cx).value().to_string();
        model::Draft {
            incoming_reference: self.incoming_reference.clone(),
            outgoing_reference: self.outgoing_reference.clone(),
            provider: Self::selected(&self.provider, cx),
            auth: Self::selected(&self.authentication, cx),
            id: value(&self.inputs.id),
            name: value(&self.inputs.name),
            address: value(&self.inputs.address),
            login: value(&self.inputs.login),
            smtp_login: value(&self.inputs.smtp_login),
            imap_host: value(&self.inputs.imap_host),
            imap_port: value(&self.inputs.imap_port),
            imap_security: Self::selected(&self.imap_tls, cx),
            smtp_host: value(&self.inputs.smtp_host),
            smtp_port: value(&self.inputs.smtp_port),
            smtp_security: Self::selected(&self.smtp_tls, cx),
            client_id: value(&self.inputs.client),
            authorization_url: value(&self.inputs.authorization),
            token_url: value(&self.inputs.token),
            scopes: value(&self.inputs.scopes),
            sasl: "xoauth2",
            separate_password: self.separate_password,
            aliases: self.inputs.aliases.read(cx).value().to_string(),
            ca_files: self.certificates.clone(),
            sent: value(&self.inputs.sent),
            archive: value(&self.inputs.archive),
            trash: value(&self.inputs.trash),
        }
    }

    fn add(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.editor.read(cx).editable() || self.editor.read(cx).busy() || self.choosing_ca {
            return;
        }
        match self.draft(cx).account() {
            Ok(account) => {
                self.error = None;
                self.submitted = Some(account.id.clone());
                self.editor
                    .update(cx, |editor, cx| editor.add_account(account, window, cx));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn browse_ca(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.editor.read(cx).editable() || self.editor.read(cx).busy() || self.choosing_ca {
            return;
        }
        self.choosing_ca = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Select PEM certificates".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = paths.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.choosing_ca = false;
                match selected {
                    Ok(Ok(None)) => {}
                    Ok(Ok(Some(paths))) if paths.len() <= 16 => {
                        let values: Option<Vec<String>> = paths
                            .iter()
                            .map(|path| {
                                path.to_str()
                                    .filter(|_| path.is_absolute())
                                    .map(str::to_owned)
                            })
                            .collect();
                        if let Some(values) = values {
                            this.certificates = values;
                            let display = this.certificates.join("; ");
                            this.inputs
                                .ca
                                .update(cx, |input, cx| input.set_value(display, window, cx));
                            this.error = None;
                        } else {
                            this.error = Some("Choose absolute UTF-8 certificate paths.");
                        }
                    }
                    Ok(Ok(Some(_))) => this.error = Some("Choose at most 16 certificate files."),
                    _ => {
                        this.error = Some(
                            "The file chooser is unavailable. Certificate settings were preserved.",
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn line(label: &'static str, input: &Entity<InputState>, disabled: bool) -> Field {
        Field::new()
            .label(label)
            .child(Input::new(input).aria_label(label).disabled(disabled))
    }

    fn select(label: &'static str, menu: &Menu, disabled: bool) -> Field {
        Field::new().label(label).child(
            Select::new(menu)
                .accessibility_label(label)
                .disabled(disabled)
                .w_full(),
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the declarative advanced account form together"
    )]
    fn advanced(&self, disabled: bool, cx: &Context<Self>) -> impl IntoElement + use<> {
        let provider = Self::selected(&self.provider, cx);
        let oauth = provider == "google"
            || (provider == "manual" && Self::selected(&self.authentication, cx) != "password");
        let password =
            provider == "manual" && Self::selected(&self.authentication, cx) == "password";
        Collapsible::new()
            .open(self.advanced)
            .child(
                Button::new("advanced-account")
                    .ghost()
                    .label(if self.advanced {
                        "Hide advanced settings"
                    } else {
                        "Advanced settings"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
            .content(
                Form::vertical()
                    .columns(2)
                    .when(oauth, |form| form.child(Self::line("Custom OAuth client ID", &self.inputs.client, disabled).required(true).col_span(2)
                        .description("Developer override for a registered desktop application. This identifies the application, not your Google account.")))
                    .child(Self::line("Incoming login", &self.inputs.login, disabled))
                    .child(Self::line(
                        "Outgoing login",
                        &self.inputs.smtp_login,
                        disabled,
                    ))
                    .child(
                        Field::new().col_span(2).visible(password).child(
                            Switch::new("separate-smtp-password")
                                .label("Use a different outgoing password")
                                .checked(self.separate_password)
                                .disabled(disabled)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.separate_password = *checked;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        Field::new()
                            .label("Aliases (one address per line)")
                            .col_span(2)
                            .child(
                                Textarea::new(&self.inputs.aliases)
                                    .aria_label("Aliases, one email address per line")
                                    .h_20()
                                    .disabled(disabled),
                            ),
                    )
                    .child(
                        Field::new()
                            .label("Additional CA certificates")
                            .col_span(2)
                            .description(
                                "Optional PEM files supplement normal certificate verification.",
                            )
                            .child(
                                InputGroup::new("account-ca")
                                    .readonly(true)
                                    .disabled(disabled)
                                    .input(
                                        Input::new(&self.inputs.ca)
                                            .aria_label("Additional CA certificates"),
                                    )
                                    .addon(
                                        InputGroupAddon::new("ca-actions")
                                            .align(InputGroupAddonAlignment::InlineEnd)
                                            .child(
                                                InputGroupButton::new("browse-ca")
                                                    .label("Browse")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.browse_ca(window, cx);
                                                        },
                                                    )),
                                            )
                                            .child(
                                                InputGroupButton::new("clear-ca")
                                                    .label("Clear")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.certificates.clear();
                                                            this.inputs.ca.update(
                                                                cx,
                                                                |input, cx| {
                                                                    input.set_value("", window, cx);
                                                                },
                                                            );
                                                            this.error = None;
                                                            cx.notify();
                                                        },
                                                    )),
                                            ),
                                    ),
                            ),
                    )
                    .child(Self::line("Sent folder", &self.inputs.sent, disabled))
                    .child(Self::line("Archive folder", &self.inputs.archive, disabled))
                    .child(Self::line("Trash folder", &self.inputs.trash, disabled)),
            )
    }
}

impl Render for Onboarding {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.read(cx);
        let editable = editor.editable();
        let busy = editor.busy() || self.choosing_ca;
        let dirty = editor.dirty();
        let disabled = !editable || busy;
        let provider = Self::selected(&self.provider, cx);
        let manual = provider == "manual";
        let custom = manual && Self::selected(&self.authentication, cx) == "custom";
        let google_unconfigured =
            provider == "google" && self.inputs.client.read(cx).value().trim().is_empty();
        let hint = if !editable {
            Some(
                "Create an app-owned native source in Configuration, or manage these accounts in your Nix declaration.",
            )
        } else if dirty {
            Some("Save or discard pending preferences before adding an account.")
        } else {
            None
        };
        div().flex().flex_col().gap_4()
            .when_some(hint, |view, hint| view.child(Alert::info("account-ownership", hint)))
            .when_some(self.error, |view, error| view.child(Alert::error("account-validation", error)))
            .child(Form::vertical().columns(2)
                .child(Self::select("Provider", &self.provider, disabled).col_span(2))
                .child(Self::line("Account ID", &self.inputs.id, disabled).required(true).description("A stable name for your Nix declaration."))
                .child(Self::line("Your name", &self.inputs.name, disabled).required(true))
                .child(Self::line("Email address", &self.inputs.address, disabled).col_span(2).required(true))
                .when(manual, |form| form
                    .child(Self::line("Incoming server", &self.inputs.imap_host, disabled).required(true))
                    .child(Field::new().label("IMAP port").child(NumberInput::new(&self.inputs.imap_port).accessibility_label("IMAP port").disabled(disabled)))
                    .child(Self::select("Incoming security", &self.imap_tls, disabled).col_span(2))
                    .child(Self::line("Outgoing server", &self.inputs.smtp_host, disabled).required(true))
                    .child(Field::new().label("SMTP port").child(NumberInput::new(&self.inputs.smtp_port).accessibility_label("SMTP port").disabled(disabled)))
                    .child(Self::select("Outgoing security", &self.smtp_tls, disabled).col_span(2))
                    .child(Self::select("Authentication", &self.authentication, disabled).col_span(2)))
                .when(custom, |form| form
                    .child(Self::line("Authorization URL", &self.inputs.authorization, disabled).required(true).col_span(2))
                    .child(Self::line("Token URL", &self.inputs.token, disabled).required(true).col_span(2))
                    .child(Self::line("Scopes", &self.inputs.scopes, disabled).required(true).col_span(2)))
                .when(provider == "icloud", |form| form.child(Field::new().col_span(2).description("After saving, use an Apple app-specific password in the encrypted account controls."))))
            .when(provider == "google", |view| view.child(Alert::info("google-registration",
                "Google browser sign-in is not ready in this build: Mailstronaut needs its own registered OAuth application. You should not need to create a client ID. Developers can supply a custom registration in Advanced settings.")))
            .child(self.advanced(disabled, cx))
            .child(Button::new("add-account").primary().label(if busy { "Working…" } else if google_unconfigured { "Google sign-in unavailable" } else { "Add account" })
                .disabled(disabled || dirty || google_unconfigured).on_click(cx.listener(|this, _, window, cx| this.add(window, cx))))
    }
}
