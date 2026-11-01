//! Native scroll area around bounded Blitz viewport pixels, with no HTML events.

use std::sync::Arc;

use gpui::component::{ActiveTheme as _, scroll::ScrollableElement as _};
use gpui::{
    Bounds, Context, Corners, EventEmitter, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, MouseButton, ParentElement as _, Pixels, Render, RenderImage, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Task, Window, canvas, div, point, px,
};
use mailstronaut_render::{Link, View};
use tokio::sync::watch;

use crate::html_worker::{Content, Reply, Request};

gpui::actions!(
    html_reader,
    [
        #[derive(Eq)]
        /// Scroll the HTML viewport by one page.
        PageDown,
        #[derive(Eq)]
        /// Scroll the HTML viewport back one page.
        PageUp,
        #[derive(Eq)]
        /// Scroll to the top of the HTML document.
        DocumentHome,
        #[derive(Eq)]
        /// Scroll to the bottom of the HTML document.
        DocumentEnd
    ]
);

pub enum Event {
    Ready(u64),
    Failed(u64),
    Link(u64, Link),
}

pub struct HtmlReader {
    focus: FocusHandle,
    scroll: ScrollHandle,
    extent: gpui::Size<Pixels>,
    content: Option<Arc<Content>>,
    epoch: u64,
    serial: u64,
    requested: Option<View>,
    frame: Option<(View, Arc<RenderImage>)>,
    ready_serial: Option<u64>,
    status: String,
    links: Arc<[Link]>,
    sender: Option<watch::Sender<Option<Request>>>,
    _responses: Task<()>,
}
impl EventEmitter<Event> for HtmlReader {}

impl HtmlReader {
    pub fn new(runtime: &tokio::runtime::Handle, window: &Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("pagedown", PageDown, Some("MailHtml")),
            KeyBinding::new("pageup", PageUp, Some("MailHtml")),
            KeyBinding::new("home", DocumentHome, Some("MailHtml")),
            KeyBinding::new("end", DocumentEnd, Some("MailHtml")),
        ]);
        let (sender, responses) = match crate::html_worker::start(runtime) {
            Ok((sender, mut replies)) => {
                let task = cx.spawn_in(window, async move |this, cx| {
                    while let Some(reply) = replies.recv().await {
                        if this
                            .update_in(cx, |this, _, cx| this.receive(reply, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ = this.update_in(cx, |this, _, cx| {
                        if this.content.is_some() {
                            cx.emit(Event::Failed(this.epoch));
                        }
                    });
                });
                (Some(sender), task)
            }
            Err(_) => (None, Task::ready(())),
        };
        Self {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            extent: gpui::size(px(1.), px(1.)),
            content: None,
            epoch: 0,
            serial: 0,
            requested: None,
            frame: None,
            ready_serial: None,
            status: String::new(),
            links: Arc::default(),
            sender,
            _responses: responses,
        }
    }

    pub fn show(&mut self, content: Arc<Content>, epoch: u64, cx: &mut Context<Self>) {
        self.hide(cx);
        self.content = Some(content);
        self.epoch = epoch;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.extent = gpui::size(px(1.), px(1.));
        self.status = "Laying out HTML…".into();
        if self.sender.is_none() {
            cx.emit(Event::Failed(epoch));
        }
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.serial = self.serial.wrapping_add(1);
        self.content = None;
        self.requested = None;
        self.frame = None;
        self.links = Arc::default();
        self.ready_serial = None;
        if let Some(sender) = &self.sender {
            sender.send_replace(None);
        }
        cx.notify();
    }

    fn request(&mut self, view: View, cx: &mut Context<Self>) {
        if self.requested == Some(view) {
            return;
        }
        let (Some(sender), Some(content)) = (&self.sender, &self.content) else {
            return;
        };
        self.serial = self.serial.wrapping_add(1);
        self.requested = Some(view);
        self.frame = None;
        sender.send_replace(Some(Request {
            serial: self.serial,
            epoch: self.epoch,
            content: content.clone(),
            view,
            hit: None,
        }));
        cx.notify();
    }

    fn receive(&mut self, reply: Reply, cx: &mut Context<Self>) {
        if let Reply::Link {
            serial,
            epoch,
            view,
            link,
        } = reply
        {
            if serial == self.serial
                && epoch == self.epoch
                && self.requested == Some(view)
                && self.content.is_some()
                && self
                    .frame
                    .as_ref()
                    .is_some_and(|(painted, _)| *painted == view)
                && let Some(link) = link
            {
                cx.emit(Event::Link(epoch, link));
            }
            return;
        }
        let Reply::Painted {
            serial,
            epoch,
            view,
            result,
        } = reply
        else {
            return;
        };
        if serial != self.serial
            || epoch != self.epoch
            || self.content.is_none()
            || self.requested != Some(view)
        {
            return;
        }
        match result {
            Ok((frame, omitted)) => {
                self.links = frame.links.clone();
                self.extent = gpui::size(
                    Pixels::from(frame.content_width),
                    Pixels::from(frame.content_height),
                );
                if let Some(pixels) =
                    image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels)
                {
                    self.frame = Some((
                        view,
                        Arc::new(RenderImage::new([image::Frame::new(pixels)])),
                    ));
                    self.status = if omitted {
                        "Some local images could not be displayed. External resources are blocked."
                            .into()
                    } else {
                        String::new()
                    };
                } else {
                    cx.emit(Event::Failed(epoch));
                }
            }
            Err(error) => {
                self.status = error;
                cx.emit(Event::Failed(epoch));
            }
        }
        cx.notify();
    }

    pub fn links(&self) -> Arc<[Link]> {
        self.links.clone()
    }

    fn click_link(&self, position: gpui::Point<Pixels>, window: &Window, cx: &Context<Self>) {
        let (Some((view, _)), Some(sender), Some(content)) =
            (&self.frame, &self.sender, &self.content)
        else {
            return;
        };
        let bounds = self.scroll.bounds();
        let current = measured_view(
            bounds,
            window.scale_factor(),
            cx.theme().mode.is_dark(),
            self.scroll.offset(),
        );
        if current != Some(*view) || !bounds.contains(&position) {
            return;
        }
        let position = position - bounds.origin;
        sender.send_replace(Some(Request {
            serial: self.serial,
            epoch: self.epoch,
            content: content.clone(),
            view: *view,
            hit: Some((f32::from(position.x), f32::from(position.y))),
        }));
    }

    fn scroll_page(&self, direction: f32, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        self.scroll.set_offset(point(
            offset.x,
            (offset.y - self.scroll.bounds().size.height * (0.8 * direction))
                .clamp(-self.scroll.max_offset().y, px(0.)),
        ));
        cx.notify();
    }

    fn surface(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let scroll = self.scroll.clone();
        let paint_scroll = scroll.clone();
        let weak = cx.entity().downgrade();
        let paint_weak = weak.clone();
        let frame = self.frame.clone();
        let serial = self.serial;
        let epoch = self.epoch;
        let dark = cx.theme().mode.is_dark();
        let scale = window.scale_factor();
        div()
            .id("html-viewport")
            .relative()
            .size_full()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                this.click_link(event.position(), window, cx);
            }))
            .child(div().flex_none().w(self.extent.width).h(self.extent.height))
            .child(
                canvas(
                    move |_, _, cx| {
                        let bounds = scroll.bounds();
                        let offset = scroll.offset();
                        let view = measured_view(bounds, scale, dark, offset);
                        cx.defer(move |cx| {
                            _ = weak.update(cx, |this, cx| {
                                if let Some(view) = view {
                                    this.request(view, cx);
                                } else if this.content.is_some()
                                    && bounds.size.width > px(0.)
                                    && bounds.size.height > px(0.)
                                {
                                    cx.emit(Event::Failed(this.epoch));
                                }
                            });
                        });
                        (bounds, view)
                    },
                    move |_, (bounds, view), window, cx| {
                        let Some((frame_view, image)) = frame else {
                            return;
                        };
                        if Some(frame_view) != view || paint_scroll.bounds() != bounds {
                            return;
                        }
                        if window
                            .paint_image(bounds, bounds, Corners::default(), image, 0, false)
                            .is_ok()
                        {
                            cx.defer(move |cx| {
                                _ = paint_weak.update(cx, |this, cx| {
                                    if this.serial == serial
                                        && this.content.is_some()
                                        && this.ready_serial != Some(serial)
                                    {
                                        this.ready_serial = Some(serial);
                                        cx.emit(Event::Ready(epoch));
                                    }
                                });
                            });
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .vertical_scrollbar(&self.scroll)
            .horizontal_scrollbar(&self.scroll)
    }
}

fn measured_view(
    bounds: Bounds<Pixels>,
    scale: f32,
    dark: bool,
    offset: gpui::Point<Pixels>,
) -> Option<View> {
    if !scale.is_finite() || !(0.5..=8.0).contains(&scale) {
        return None;
    }
    if !f64::from(offset.x).is_finite() || !f64::from(offset.y).is_finite() {
        return None;
    }
    if [f64::from(bounds.size.width), f64::from(bounds.size.height)]
        .into_iter()
        .any(|n| {
            !n.is_finite()
                || n <= 0.0
                || n * f64::from(scale) > f64::from(mailstronaut_render::MAX_SIDE)
        })
    {
        return None;
    }
    let physical = bounds.size.to_device_pixels(scale);
    let width = u32::try_from(physical.width.0).ok()?;
    let height = u32::try_from(physical.height.0).ok()?;
    if width == 0
        || height == 0
        || width > mailstronaut_render::MAX_SIDE
        || height > mailstronaut_render::MAX_SIDE
        || u64::from(width) * u64::from(height) > mailstronaut_render::MAX_PIXELS
    {
        return None;
    }
    Some(View {
        width,
        height,
        scale,
        dark,
        scroll_x: (-f64::from(offset.x)).max(0.0),
        scroll_y: (-f64::from(offset.y)).max(0.0),
    })
}

impl Render for HtmlReader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("MailHtml")
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.scroll_page(1.0, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.scroll_page(-1.0, cx)))
            .on_action(cx.listener(|this, _: &DocumentHome, _, cx| {
                this.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DocumentEnd, _, cx| {
                this.scroll.set_offset(-this.scroll.max_offset());
                cx.notify();
            }))
            .child(div().flex_1().min_h_0().child(self.surface(window, cx)))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.status.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_geometry_preserves_logical_scroll_and_bounds_physical_allocations() {
        let bounds = Bounds::new(point(px(500.), px(200.)), gpui::size(px(600.), px(400.)));
        let view = measured_view(bounds, 2.0, true, point(px(-30.), px(-900.)))
            .expect("valid native geometry");
        assert_eq!((view.width, view.height), (1200, 800));
        assert_eq!((view.scroll_x, view.scroll_y), (30.0, 900.0));
        assert_eq!(view.scale, 2.0);
        assert!(view.dark);
        // A native overscroll above the start cannot request negative CSS scroll.
        assert_eq!(
            measured_view(bounds, 1.0, false, point(px(4.), px(9.)))
                .expect("overscroll clamps")
                .scroll_y,
            0.0
        );
        for scale in [f32::NAN, f32::INFINITY, 0.0, 0.49, 8.01] {
            assert!(measured_view(bounds, scale, false, point(px(0.), px(0.))).is_none());
        }
        for size in [
            gpui::size(px(0.), px(400.)),
            gpui::size(px(f32::NAN), px(400.)),
            gpui::size(px(600.), px(f32::INFINITY)),
            gpui::size(px(4097.), px(1.)),
            gpui::size(px(3000.), px(3000.)),
        ] {
            assert!(
                measured_view(
                    Bounds::new(bounds.origin, size),
                    1.0,
                    false,
                    point(px(0.), px(0.))
                )
                .is_none()
            );
        }
        assert!(measured_view(bounds, 1.0, false, point(px(f32::NAN), px(0.))).is_none());
        assert!(measured_view(bounds, 1.0, false, point(px(0.), px(f32::NEG_INFINITY))).is_none());
    }
}
