//! Minimal native window for the rendering feasibility gate.

#![expect(
    clippy::multiple_crate_versions,
    reason = "The pinned libcosmic graph requires multiple incompatible versions of shared dependencies."
)]

mod config_cli;

use core::hash::{Hash, Hasher};
use std::{env, process::ExitCode};

use cosmic::app::{self, Core, Settings, Task};
use cosmic::iced::widget::{column, image, mouse_area, row};
use cosmic::iced::{Event, Point, Subscription, event, futures::stream, mouse, window};
use cosmic::{Action, Element, cosmic_theme, executor, theme, widget};
use mailstronaut_render::Viewport;

/// Toolkit-owned application state.
struct App {
    /// COSMIC application state, including the system theme.
    core: Core,
    /// Latest uploaded synthetic frame.
    frame: Option<image::Handle>,
    /// Logical viewport size.
    logical: (f32, f32),
    /// The currently selected synthetic preview.
    page: Page,
    /// Most recent pointer position within the image.
    pointer: Point,
    /// Effective physical pixels per logical unit.
    scale: f32,
    /// Dimensions of the current frame and viewport.
    size: (u32, u32),
    /// Retained UI-thread Servo instance or its startup error.
    viewport: Result<Viewport, String>,
    /// Most recently reported logical window size.
    window_size: (f32, f32),
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
    /// Pointer moved within the viewport.
    Pointer(Point),
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
                viewport: {
                    let viewport = Viewport::synthetic(640, 400);
                    if let Ok(reader) = viewport.as_ref() {
                        reader.set_dark(theme::is_dark());
                    }
                    viewport
                },
                frame: None,
                size: (640, 400),
                logical: (640.0, 400.0),
                window_size: (1024.0, 768.0),
                scale: 1.0,
                pointer: Point::ORIGIN,
            },
            Task::done(Action::App(Message::Wake)),
        )
    }

    fn on_window_resize(&mut self, _: window::Id, width: f32, height: f32) {
        self.window_size = (width, height);
        self.logical = logical_preview_size(width, height, self.scale);
        self.resize_viewport();
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        clippy::unneeded_field_pattern,
        reason = "Only window geometry events matter to this subscription."
    )]
    fn subscription(&self) -> Subscription<Message> {
        let windows = event::listen_with(|incoming, _, id| match incoming {
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
        if matches!(self.page, Page::Preview)
            && let Ok(viewport) = self.viewport.as_ref()
        {
            let wakes = Subscription::run_with(Wakes(viewport.wake_receiver()), |wakes| {
                stream::unfold(wakes.0.clone(), |receiver| async move {
                    receiver
                        .recv()
                        .await
                        .ok()
                        .map(|()| (Message::Wake, receiver))
                })
            });
            return Subscription::batch([windows, wakes]);
        }
        windows
    }

    fn system_theme_mode_update(
        &mut self,
        _: &[&'static str],
        new_theme: &cosmic_theme::ThemeMode,
    ) -> Task<Message> {
        if let Ok(viewport) = self.viewport.as_ref() {
            viewport.set_dark(new_theme.is_dark);
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
        if let Ok(viewport) = self.viewport.as_ref() {
            viewport.set_dark(new_theme.is_dark);
        }
        Task::none()
    }

    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        clippy::float_arithmetic,
        clippy::float_cmp,
        clippy::unneeded_field_pattern,
        reason = "Viewport dimensions are bounded and scale changes require exact event values."
    )]
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Select(page) => {
                self.page = page;
                if matches!(page, Page::Preview) {
                    return Task::done(Action::App(Message::Wake));
                }
            }
            Message::Pointer(point) => self.pointer = point,
            Message::Wheel(delta) => {
                if let Ok(viewport) = self.viewport.as_ref() {
                    let (dx, dy, lines) = match delta {
                        mouse::ScrollDelta::Lines { x, y } => (x, y, true),
                        mouse::ScrollDelta::Pixels { x, y } => (x, y, false),
                    };
                    // The pinned mouse_area publishes each wheel event twice.
                    // Preserve the native delta across those two deliveries.
                    if viewport
                        .wheel(
                            self.pointer.x * (self.size.0 as f32 / self.logical.0),
                            self.pointer.y * (self.size.1 as f32 / self.logical.1),
                            dx * 0.5,
                            dy * 0.5,
                            lines,
                        )
                        .is_err()
                    {
                        return Task::none();
                    }
                }
            }
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
                    if let Ok(viewport) = self.viewport.as_ref()
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
                if matches!(self.page, Page::Preview)
                    && let Ok(viewport) = self.viewport.as_ref()
                    && let Some(pixels) = viewport.paint()
                {
                    self.frame = Some(image::Handle::from_rgba(self.size.0, self.size.1, pixels));
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match self.page {
            Page::Preview => {
                let image: Element<'_, Message> =
                    match (self.viewport.as_ref(), self.frame.as_ref()) {
                        (Err(error), _) => {
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
        };
        row![
            column![
                widget::text("Mailstronaut"),
                widget::button::text("Preview").on_press(Message::Select(Page::Preview)),
                widget::button::text("About").on_press(Message::Select(Page::About))
            ]
            .spacing(theme::spacing().space_s)
            .padding(20),
            content
        ]
        .spacing(20)
        .into()
    }
}

impl App {
    /// Resize the retained engine to the rounded physical extent.
    fn resize_viewport(&mut self) {
        let size = physical_size(self.logical, self.scale);
        if size != self.size
            && let Ok(viewport) = self.viewport.as_mut()
            && viewport.resize(size.0, size.1).is_ok()
        {
            self.size = size;
            self.frame = None;
        }
    }
}

/// Subscription identity is stable for the single retained viewport.
struct Wakes(async_channel::Receiver<()>);
#[expect(
    clippy::missing_trait_methods,
    reason = "Hash's default slice implementation preserves this subscription identity."
)]
impl Hash for Wakes {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        u8::MIN.hash(state);
    }
}
#[cfg_attr(
    not(test),
    expect(
        clippy::single_call_fn,
        reason = "The framebuffer rounding is also tested."
    )
)]
/// Round the bounded logical viewport to physical framebuffer pixels.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_arithmetic,
    clippy::implicit_return,
    reason = "Validated finite dimensions are clamped before conversion."
)]
fn physical_size(logical: (f32, f32), scale: f32) -> (u32, u32) {
    (
        (logical.0 * scale).round().clamp(1.0, 2048.0) as u32,
        (logical.1 * scale).round().clamp(1.0, 2048.0) as u32,
    )
}

#[expect(
    clippy::float_arithmetic,
    clippy::implicit_return,
    reason = "Window layout uses fractional units and finite clamped dimensions."
)]
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

#[expect(
    clippy::implicit_return,
    reason = "The restriction conflicts with needless_return."
)]
#[expect(
    clippy::print_stderr,
    reason = "Graphical startup failures need a terminal diagnostic."
)]
fn main() -> ExitCode {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(status) = config_cli::dispatch(&arguments) {
        return status;
    }
    match app::run::<App>(Settings::default(), ()) {
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
