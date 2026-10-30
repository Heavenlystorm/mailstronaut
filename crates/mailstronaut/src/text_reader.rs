//! Native selection/copy and keyboard scrolling for the synthetic text body.

use cosmic::iced::{
    Length, Task,
    advanced::widget::{operate, operation as tree_operation},
    keyboard,
    widget::operation,
};
use cosmic::{Element, widget};

/// Scope for the toolkit's anonymous selectable-text focus target.
const BODY_SCOPE: &str = "reader-body";
/// Stable scroll state across normal application updates and theme changes.
const SCROLL_ID: &str = "reader-scroll";

/// Reader shortcuts leave selection and clipboard handling to the toolkit.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Scroll to the end of the message.
    End,
    /// Scroll down by most of the visible height.
    PageDown,
    /// Scroll up by most of the visible height.
    PageUp,
    /// Scroll to the beginning of the message.
    Start,
}

/// Match only reader scrolling keys; never consume selection or copy shortcuts.
pub fn command(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Command> {
    if modifiers.alt() || modifiers.logo() || modifiers.shift() {
        return None;
    }
    match (key, modifiers.control()) {
        (keyboard::Key::Named(keyboard::key::Named::PageDown), false) => Some(Command::PageDown),
        (keyboard::Key::Named(keyboard::key::Named::PageUp), false) => Some(Command::PageUp),
        (keyboard::Key::Named(keyboard::key::Named::Home), true) => Some(Command::Start),
        (keyboard::Key::Named(keyboard::key::Named::End), true) => Some(Command::End),
        _ => None,
    }
}

/// Focus the text and reset scrolling when opening the reader.
pub fn open<Message>() -> Task<Message>
where
    Message: Send + 'static,
{
    Task::batch([
        focus(),
        operation::snap_to(SCROLL_ID, operation::RelativeOffset::START),
    ])
}

/// Restore paragraph focus after a structural selector update, preserving scroll.
pub fn focus<Message>() -> Task<Message>
where
    Message: Send + 'static,
{
    operate(tree_operation::focusable::unfocus()).chain(operate(tree_operation::scoped(
        BODY_SCOPE.into(),
        tree_operation::focusable::focus_next(),
    )))
}

/// Use the toolkit's scroll state and bounds rather than keeping a second offset.
pub fn scroll<Message>(command: Command, height: f32) -> Task<Message>
where
    Message: 'static,
{
    let page = (height - 32.0).max(1.0);
    match command {
        Command::End => operation::snap_to(SCROLL_ID, operation::RelativeOffset::END),
        Command::Start => operation::snap_to(SCROLL_ID, operation::RelativeOffset::START),
        Command::PageDown => {
            operation::scroll_by(SCROLL_ID, operation::AbsoluteOffset { x: 0.0, y: page })
        }
        Command::PageUp => {
            operation::scroll_by(SCROLL_ID, operation::AbsoluteOffset { x: 0.0, y: -page })
        }
    }
}

/// A single selectable Unicode paragraph keeps copy/selection across line breaks.
pub fn view<Message>(body: &str, width: f32, height: f32) -> Element<'_, Message>
where
    Message: Clone + 'static,
{
    let text = widget::selectable_text(body).width(Length::Fill);
    widget::scrollable(widget::container(text).id(BODY_SCOPE))
        .id(SCROLL_ID)
        .width(width)
        .height(height)
        .into()
}
