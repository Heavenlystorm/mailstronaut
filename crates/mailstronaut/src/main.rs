//! Minimal native window for the rendering feasibility gate.

mod accounts;
mod composer;
mod config_cli;
mod inbox;
mod private;
mod text_reader;

use core::hash::{Hash, Hasher};
use std::{env, process::ExitCode};

use cosmic::app::{self, Core, Settings, Task};
use cosmic::iced::widget::{column, image, mouse_area, row};
use cosmic::iced::{Event, Point, Subscription, event, futures::stream, keyboard, mouse, window};
use cosmic::{Action, Element, cosmic_theme, executor, theme, widget};
use mailstronaut_render::{Renderer, Viewport};

/// Toolkit-owned application state.
struct App {
    composer: composer::Composer,
    quitting: bool,
    quit_flushing: bool,
    /// Durable unified inbox and native cached-message reader.
    inbox: inbox::Inbox,
    /// Validated account configuration and foreground credential setup.
    accounts: accounts::Accounts,
    /// COSMIC application state, including the system theme.
    core: Core,
    /// Current system theme, including changes while hidden.
    dark: bool,
    /// Latest uploaded synthetic frame.
    frame: Option<image::Handle>,
    /// Logical viewport size.
    logical: (f32, f32),
    /// The currently selected synthetic preview.
    page: Page,
    /// Most recent pointer position within the image.
    pointer: Point,
    /// Retained thread-affine engine.
    renderer: Option<Result<Renderer, String>>,
    /// Effective physical pixels per logical unit.
    scale: f32,
    /// Dimensions of the current frame and viewport.
    size: (u32, u32),
    /// Fixed synthetic decoded body shared by the HTML and native readers.
    text: String,
    /// Disposable UI-thread viewport or its startup error.
    viewport: Option<Result<Viewport, String>>,
    /// Most recently reported logical window size.
    window_size: (f32, f32),
}

/// Synthetic navigation targets.
#[derive(Clone, Copy, Debug)]
enum Page {
    Compose,
    /// Real, locally cached messages.
    Inbox,
    /// Native account setup.
    Accounts,
    /// Information about this preview.
    About,
    /// Synthetic reader preview.
    Preview,
    /// Native selectable reader for the fixed synthetic message.
    Text,
}

/// Actions from the native navigation.
#[derive(Clone, Debug)]
enum Message {
    Composer(composer::Message),
    Quit,
    CloseRequested(window::Id),
    QuitSaved(Result<(), mailstronaut_core::worker::Error>),
    QuitReady(Result<(), mailstronaut_core::worker::Error>),
    /// Native mailbox queries and explicit synchronization.
    Inbox(inbox::Message),
    /// Effective configured theme, forwarded to Servo's native theme API.
    Appearance(bool),
    /// Native configuration and credential setup.
    Accounts(accounts::Message),
    /// Pointer moved within the viewport.
    Pointer(Point),
    /// Keyboard scrolling in the native text reader.
    Read(window::Id, text_reader::Command),
    /// Native window scale query completed.
    Scale(f32),
    /// Select a synthetic page.
    Select(Page),
    /// Drive Servo after a coalesced wake.
    Wake,
    /// Wheel input over the viewport.
    Wheel(mouse::ScrollDelta),
    /// Window opened or changed its native DPI.
    Window(window::Id, window::Event),
}

impl cosmic::Application for App {
    const APP_ID: &'static str = include_str!("../../../assets/app-id");
    type Executor = executor::Default;
    type Flags = Option<std::path::PathBuf>;
    type Message = Message;

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }
    fn init(core: Core, path: Self::Flags) -> (Self, Task<Message>) {
        let (accounts, load) = accounts::Accounts::new(path);
        let (inbox, open) = inbox::Inbox::new();
        (
            Self {
                composer: composer::Composer::default(),
                quitting: false,
                quit_flushing: false,
                inbox,
                accounts,
                core,
                page: Page::Accounts,
                renderer: None,
                dark: theme::is_dark(),
                viewport: None,
                frame: None,
                size: (640, 400),
                logical: (640.0, 400.0),
                window_size: (1024.0, 768.0),
                scale: 1.0,
                pointer: Point::ORIGIN,
                text: Renderer::synthetic_text(),
            },
            Task::batch([load, open]),
        )
    }

    fn on_window_resize(&mut self, _: window::Id, width: f32, height: f32) {
        self.window_size = (width, height);
        self.logical = logical_preview_size(width, height, self.scale);
        self.resize_viewport();
    }

    fn on_app_exit(&mut self) -> Option<Message> {
        Some(Message::Quit)
    }
    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        (Some(id) == self.core.main_window_id()).then_some(Message::Quit)
    }

    fn subscription(&self) -> Subscription<Message> {
        let windows = event::listen_with(|incoming, _, id| match incoming {
            Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested(id)),
            Event::Window(
                event @ (window::Event::Opened {
                    position: _,
                    size: _,
                }
                | window::Event::Rescaled(_)
                | window::Event::Resized(_)
                | window::Event::Moved(_)),
            ) => Some(Message::Window(id, event)),
            _ => None,
        });
        let reader = if matches!(self.page, Page::Text)
            || (matches!(self.page, Page::Inbox) && self.inbox.reading())
        {
            event::listen_with(|incoming, _, id| {
                if let Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    modified_key: _,
                    physical_key: _,
                    location: _,
                    text: _,
                    repeat: _,
                }) = incoming
                {
                    return text_reader::command(key, modifiers)
                        .map(|command| Message::Read(id, command));
                }
                None
            })
        } else {
            Subscription::none()
        };
        if let Some(Ok(renderer)) = self.renderer.as_ref() {
            let wakes = Subscription::run_with(Wakes(renderer.wake_receiver()), |wakes| {
                stream::unfold(wakes.0.clone(), |receiver| async move {
                    receiver
                        .recv()
                        .await
                        .ok()
                        .map(|()| (Message::Wake, receiver))
                })
            });
            return Subscription::batch([windows, reader, wakes]);
        }
        Subscription::batch([windows, reader])
    }

    fn system_theme_mode_update(
        &mut self,
        _: &[&'static str],
        new_theme: &cosmic_theme::ThemeMode,
    ) -> Task<Message> {
        self.dark = self.accounts.effective_dark(new_theme.is_dark);
        if let Some(Ok(viewport)) = self.viewport.as_ref() {
            viewport.set_dark(self.dark);
        }
        Task::none()
    }

    fn system_theme_update(
        &mut self,
        _: &[&'static str],
        new_theme: &cosmic_theme::Theme,
    ) -> Task<Message> {
        // The pinned COSMIC callbacks run before its global theme is updated;
        // its Wayland-enabled branch also drops returned callback tasks.
        // Forward the supplied theme here, on the viewport's owning thread.
        self.dark = self.accounts.effective_dark(new_theme.is_dark);
        if let Some(Ok(viewport)) = self.viewport.as_ref() {
            viewport.set_dark(self.dark);
        }
        Task::none()
    }

    #[expect(
        clippy::float_cmp,
        reason = "Viewport dimensions are bounded and scale changes require exact event values."
    )]
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Quit => return self.request_quit(),
            Message::CloseRequested(id) => {
                if Some(id) == self.core.main_window_id() {
                    return self.request_quit();
                }
            }
            Message::QuitSaved(result) => return self.quit_saved(result),
            Message::QuitReady(result) => {
                if result.is_err() {
                    eprintln!("Mail storage stopped unexpectedly after the final draft save.");
                }
                return cosmic::iced::exit();
            }
            Message::Composer(message) => return self.compose_message(message),
            Message::Inbox(message) => {
                let task = self.inbox.update(message, self.accounts.keyring());
                return Task::batch([task, self.composer.attach(self.inbox.store())]);
            }
            Message::Appearance(dark) => {
                self.dark = dark;
                if let Some(Ok(viewport)) = self.viewport.as_ref() {
                    viewport.set_dark(dark);
                }
            }
            Message::Accounts(message) => return self.account_message(message),
            Message::Select(page) => return self.select_page(page),
            Message::Read(id, command) => {
                if (matches!(self.page, Page::Text)
                    || (matches!(self.page, Page::Inbox) && self.inbox.reading()))
                    && Some(id) == self.core.main_window_id()
                {
                    return text_reader::scroll(command, self.logical.1);
                }
            }
            Message::Pointer(point) => self.pointer = point,
            Message::Wheel(delta) => self.wheel(delta),
            Message::Window(id, event) => {
                if Some(id) == self.core.main_window_id()
                    && let window::Event::Opened { size, position: _ } = event
                {
                    self.on_window_resize(id, size.width, size.height);
                }
                if Some(id) == self.core.main_window_id()
                    && matches!(
                        event,
                        window::Event::Opened {
                            position: _,
                            size: _
                        } | window::Event::Rescaled(_)
                            | window::Event::Resized(_)
                            | window::Event::Moved(_)
                    )
                {
                    return window::scale_factor(id)
                        .map(Message::Scale)
                        .map(Action::App);
                }
            }
            Message::Scale(native) => {
                let effective = native * self.core.scale_factor();
                if effective.is_finite()
                    && (1.0 / 2048.0..=2048.0).contains(&effective)
                    && effective != self.scale
                {
                    self.scale = effective;
                    if let Some(Ok(viewport)) = self.viewport.as_ref()
                        && viewport.set_scale(effective).is_err()
                    {
                        return Task::none();
                    }
                    self.logical =
                        logical_preview_size(self.window_size.0, self.window_size.1, effective);
                    self.frame = None;
                    self.resize_viewport();
                }
            }
            Message::Wake => {
                if let Some(Ok(viewport)) = self.viewport.as_ref() {
                    if let Some(pixels) = viewport.paint() {
                        self.frame =
                            Some(image::Handle::from_rgba(self.size.0, self.size.1, pixels));
                    }
                } else if let Some(Ok(renderer)) = &self.renderer {
                    renderer.spin();
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        if self.quitting {
            return widget::text("Saving drafts and closing mail operations…").into();
        }
        let content: Element<'_, Message> = match self.page {
            Page::Compose => self
                .composer
                .view(self.logical.0, self.accounts.keyring().is_some()),
            Page::Inbox => self
                .inbox
                .view(self.logical.0, (self.logical.1 - 140.0).max(100.0)),
            Page::Accounts => self.accounts.view(),
            Page::Preview => {
                let image: Element<'_, Message> =
                    match (self.viewport.as_ref(), self.frame.as_ref()) {
                        (Some(Err(error)), _) => {
                            widget::text(format!("Servo preview failed to start: {error}")).into()
                        }
                        (_, Some(frame)) => mouse_area(
                            image(frame.clone())
                                .width(self.logical.0)
                                .height(self.logical.1),
                        )
                        .on_move(Message::Pointer)
                        .on_scroll(Message::Wheel)
                        .into(),
                        _ => widget::text("Waiting for the synthetic Servo frame...").into(),
                    };
                column![
                    widget::text("Synthetic preview"),
                    image,
                    widget::text("Native text: Synthetic Servo viewport. No mail."),
                    widget::text(
                        "Synthetic preview; keyboard and accessibility are not implemented."
                    )
                ]
                .spacing(theme::spacing().space_s)
                .padding(20)
                .into()
            }
            Page::About => column![
                widget::text("About this preview"),
                widget::text("Mailstronaut is under development.")
            ]
            .spacing(theme::spacing().space_s)
            .padding(20)
            .into(),
            Page::Text => column![
                widget::text("Text preview"),
                text_reader::view(&self.text, self.logical.0, self.logical.1),
                widget::text(
                    "Page Up/Down to scroll; Ctrl+Home/End for top/bottom; Ctrl+A/C to select/copy"
                )
            ]
            .spacing(theme::spacing().space_s)
            .padding(20)
            .into(),
        };
        row![
            column![
                widget::text("Mailstronaut"),
                widget::button::text("Preview").on_press(Message::Select(Page::Preview)),
                widget::button::text("About").on_press(Message::Select(Page::About)),
                widget::button::text("Text").on_press(Message::Select(Page::Text)),
                widget::button::text("Accounts").on_press(Message::Select(Page::Accounts)),
                widget::button::text("Inbox").on_press(Message::Select(Page::Inbox)),
                widget::button::text("Compose").on_press(Message::Select(Page::Compose))
            ]
            .spacing(theme::spacing().space_s)
            .padding(20),
            widget::container(content).width(cosmic::iced::Length::Fill)
        ]
        .spacing(20)
        .into()
    }
}

impl App {
    fn account_message(&mut self, message: accounts::Message) -> Task<Message> {
        if self.quitting {
            return Task::none();
        }
        if matches!(message, accounts::Message::Reload) && self.composer.busy() {
            self.accounts.status(
                "Finish or cancel the current compose operation before reloading accounts.",
            );
            return Task::none();
        }
        let previous = self.accounts.revision();
        let task = self.accounts.update(message);
        if previous == self.accounts.revision() {
            return task;
        }
        Task::batch([
            task,
            self.inbox.configure(self.accounts.active()),
            self.composer.configure(self.accounts.active()),
        ])
    }
    fn compose_message(&mut self, message: composer::Message) -> Task<Message> {
        if self.quitting
            && matches!(
                message,
                composer::Message::Field(..)
                    | composer::Message::Body(..)
                    | composer::Message::Switch(..)
                    | composer::Message::Send
                    | composer::Message::Submit(..)
                    | composer::Message::CancelQueued(..)
            )
        {
            return Task::none();
        }
        if self.accounts.loading()
            && matches!(
                message,
                composer::Message::Send
                    | composer::Message::Submit(..)
                    | composer::Message::Switch(..)
                    | composer::Message::CancelQueued(..)
            )
        {
            self.composer
                .status("Wait for account configuration to finish loading.".into());
            return Task::none();
        }
        let task = self.composer.update(message, self.accounts.keyring());
        Task::batch([task, self.maybe_quit()])
    }
    fn request_quit(&mut self) -> Task<Message> {
        self.quitting = true;
        self.inbox.cancel();
        self.composer.cancel();
        self.accounts.clear_input();
        self.maybe_quit()
    }
    fn maybe_quit(&mut self) -> Task<Message> {
        if self.quitting && !self.quit_flushing && !self.composer.busy() {
            self.quit_flushing = true;
            return self.composer.flush();
        }
        Task::none()
    }
    fn quit_saved(
        &mut self,
        result: Result<(), mailstronaut_core::worker::Error>,
    ) -> Task<Message> {
        match result {
            Ok(()) => {
                let owner = self.inbox.take_owner();
                Task::perform(inbox::Inbox::shutdown(owner), |result| {
                    Action::App(Message::QuitReady(result))
                })
            }
            Err(error) => {
                self.quitting = false;
                self.quit_flushing = false;
                self.page = Page::Compose;
                self.composer
                    .status(format!("Unable to save before closing: {error}"));
                Task::none()
            }
        }
    }
    /// Forward bounded native wheel input to the selected synthetic viewport.
    #[expect(
        clippy::cast_precision_loss,
        reason = "Renderer dimensions are bounded to 2048 pixels."
    )]
    fn wheel(&self, delta: mouse::ScrollDelta) {
        if let Some(Ok(viewport)) = self.viewport.as_ref() {
            let (dx, dy, lines) = match delta {
                mouse::ScrollDelta::Lines { x, y } => (x, y, true),
                mouse::ScrollDelta::Pixels { x, y } => (x, y, false),
            };
            // The pinned mouse_area publishes each wheel event twice.
            let _result = viewport.wheel(
                self.pointer.x * (self.size.0 as f32 / self.logical.0),
                self.pointer.y * (self.size.1 as f32 / self.logical.1),
                dx * 0.5,
                dy * 0.5,
                lines,
            );
        }
    }

    /// Resize the current viewport to the rounded physical extent.
    fn resize_viewport(&mut self) {
        let size = physical_size(self.logical, self.scale);
        if size != self.size {
            if let Some(Ok(viewport)) = self.viewport.as_mut()
                && viewport.resize(size.0, size.1).is_err()
            {
                return;
            }
            self.size = size;
            self.frame = None;
        }
    }

    /// Enter a native page or create the single synthetic `WebView`.
    fn select_page(&mut self, page: Page) -> Task<Message> {
        if !matches!(page, Page::Accounts) {
            self.accounts.clear_input();
        }
        self.page = page;
        if matches!(
            page,
            Page::About | Page::Text | Page::Accounts | Page::Inbox | Page::Compose
        ) {
            self.viewport = None;
            self.frame = None;
            if let Some(Ok(renderer)) = &self.renderer {
                renderer.spin();
            }
            return if matches!(page, Page::Text) {
                text_reader::open()
            } else {
                Task::none()
            };
        }
        if self.viewport.is_none() {
            self.size = physical_size(self.logical, self.scale);
            self.viewport = Some(
                match self.renderer.get_or_insert_with(Renderer::synthetic) {
                    Ok(renderer) => {
                        renderer
                            .viewport(self.size.0, self.size.1)
                            .and_then(|viewport| {
                                viewport.set_scale(self.scale)?;
                                viewport.set_dark(self.dark);
                                Ok(viewport)
                            })
                    }
                    Err(error) => Err(error.clone()),
                },
            );
            return Task::done(Action::App(Message::Wake));
        }
        Task::none()
    }
}

/// Subscription identity is stable for the retained engine.
struct Wakes(async_channel::Receiver<()>);
impl Hash for Wakes {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        u8::MIN.hash(state);
    }
}
/// Round the bounded logical viewport to physical framebuffer pixels.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Validated finite dimensions are clamped before conversion."
)]
fn physical_size(logical: (f32, f32), scale: f32) -> (u32, u32) {
    (
        (logical.0 * scale).round().clamp(1.0, 2048.0) as u32,
        (logical.1 * scale).round().clamp(1.0, 2048.0) as u32,
    )
}

/// Bound the software image to the available window area.
fn logical_preview_size(width: f32, height: f32, scale: f32) -> (f32, f32) {
    if !width.is_finite()
        || !height.is_finite()
        || !scale.is_finite()
        || !(1.0 / 2048.0..=2048.0).contains(&scale)
    {
        return (1.0, 1.0);
    }
    (
        (width - 240.0).clamp(1.0 / scale, 2048.0 / scale),
        (height - 180.0).clamp(1.0 / scale, 2048.0 / scale),
    )
}

fn main() -> ExitCode {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(status) = config_cli::dispatch(&arguments) {
        return status;
    }
    match app::run::<App>(
        Settings::default().exit_on_close(false),
        config_cli::selected_path(&arguments),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Unable to start Mailstronaut: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{logical_preview_size, physical_size};

    #[test]
    fn preview_respects_renderer_bounds() {
        assert_eq!(physical_size((101.0, 99.0), 1.5), (152, 149));
        assert_eq!(physical_size((2000.0, 2000.0), 2.0), (2048, 2048));
        assert_eq!(logical_preview_size(100.0, 100.0, 1.0), (1.0, 1.0));
        assert_eq!(logical_preview_size(880.0, 580.0, 1.0), (640.0, 400.0));
        assert_eq!(
            logical_preview_size(10000.0, 10000.0, 1.0),
            (2048.0, 2048.0)
        );
        assert_eq!(logical_preview_size(f32::NAN, 580.0, 1.0), (1.0, 1.0));
        assert_eq!(logical_preview_size(880.0, f32::INFINITY, 1.0), (1.0, 1.0));
    }
}
