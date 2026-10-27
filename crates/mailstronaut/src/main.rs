//! Minimal native window for the rendering feasibility gate.

#![expect(
    clippy::multiple_crate_versions,
    reason = "libcosmic has incompatible transitive version requirements."
)]

use cosmic::app::{self, Core, Settings, Task};
use cosmic::{Element, executor, iced, widget};

/// State of the minimal COSMIC application.
struct App {
    /// Toolkit-owned application state.
    core: Core,
}

/// No application actions are implemented yet.
#[derive(Clone, Debug)]
enum Message {}

#[expect(
    clippy::implicit_return,
    reason = "Idiomatic tail expressions conflict with the restriction lint."
)]
#[expect(
    clippy::missing_trait_methods,
    reason = "Default COSMIC handlers are appropriate for this minimal window."
)]
impl cosmic::Application for App {
    const APP_ID: &'static str = "org.mailstronaut.Mailstronaut";
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
        (Self { core }, Task::none())
    }

    fn view(&self) -> Element<'_, Message> {
        widget::text("Mailstronaut rendering integration is not yet available.").into()
    }
}

#[expect(
    clippy::implicit_return,
    reason = "The restriction conflicts with needless_return."
)]
fn main() -> iced::Result {
    app::run::<App>(Settings::default(), ())
}
