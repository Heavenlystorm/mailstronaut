//! Minimal native window for the rendering feasibility gate.

#![expect(
    clippy::multiple_crate_versions,
    reason = "The pinned libcosmic graph requires multiple incompatible versions of shared dependencies."
)]

use std::{env, ffi::OsStr};

use cosmic::app::{self, Core, Settings, Task};
use cosmic::iced::widget::{column, row};
use cosmic::{Element, executor, iced, widget};

/// Toolkit-owned application state.
struct App {
    /// COSMIC application state, including the system theme.
    core: Core,
    /// The currently selected synthetic preview.
    page: Page,
}

/// Synthetic navigation targets.
#[derive(Clone, Copy, Debug)]
enum Page {
    /// Information about this preview.
    About,
    /// Synthetic reader preview.
    Preview,
}

/// Actions from the native navigation.
#[derive(Clone, Debug)]
enum Message {
    /// Select a synthetic page.
    Select(Page),
}

#[expect(
    clippy::implicit_return,
    reason = "Idiomatic tail expressions conflict with the restriction lint."
)]
#[expect(
    clippy::missing_trait_methods,
    reason = "Default COSMIC handlers provide the normal window and theme lifecycle."
)]
impl cosmic::Application for App {
    const APP_ID: &'static str = include_str!("../../../assets/app-id");
    type Executor = executor::Default;
    type Flags = ();
    type Message = Message;

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }
    fn init(core: Core, (): ()) -> (Self, Task<Message>) {
        (
            Self {
                core,
                page: Page::Preview,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let Message::Select(page) = message;
        self.page = page;
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let (title, description) = match self.page {
            Page::Preview => (
                "Synthetic preview",
                "This native text pane is a foundation for the future reader.",
            ),
            Page::About => ("About this preview", "Mailstronaut is under development."),
        };
        row![
            column![
                widget::text("Mailstronaut"),
                widget::button::text("Preview").on_press(Message::Select(Page::Preview)),
                widget::button::text("About").on_press(Message::Select(Page::About))
            ]
            .spacing(12)
            .padding(20),
            column![
                widget::text(title),
                widget::text(description),
                widget::text("Email and HTML rendering are not available yet.")
            ]
            .spacing(12)
            .padding(20)
        ]
        .spacing(20)
        .into()
    }
}

#[expect(
    clippy::implicit_return,
    reason = "The restriction conflicts with needless_return."
)]
#[expect(
    clippy::print_stdout,
    reason = "The version command is an explicit CLI output."
)]
fn main() -> iced::Result {
    if env::args_os().nth(1).as_deref() == Some(OsStr::new("--version")) {
        println!("mailstronaut {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    app::run::<App>(Settings::default(), ())
}
