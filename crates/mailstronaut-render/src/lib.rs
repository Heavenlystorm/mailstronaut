//! A provisional, synthetic-only software Servo viewport.
#![forbid(unsafe_code)]
#![expect(
    clippy::multiple_crate_versions,
    clippy::arbitrary_source_item_ordering,
    reason = "Servo requires incompatible dependency versions; related policy items stay grouped"
)]

use core::{cell::Cell, future::Future, pin::Pin};
use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, NetworkError, ProtocolHandler, ProtocolRegistry,
    Request, ResourceFetchTiming, Response, ResponseBody,
};
use servo::{
    DeviceIntPoint, DeviceIntRect, DevicePoint, EventLoopWaker, InputEvent, NavigationRequest,
    PermissionRequest, RenderingContext as _, Servo, ServoBuilder, ServoDelegate,
    SoftwareRenderingContext, Theme, WebResourceLoad, WebResourceResponse, WebView, WebViewBuilder,
    WebViewDelegate, WheelDelta, WheelEvent, WheelMode,
};
extern crate alloc;
use alloc::rc::Rc;

/// Maximum width or height of the software viewport.
const MAX_SIDE: u32 = 2048;
/// Sole accepted memory document URL.
const DOCUMENT: &str = "mailstronaut:document";
/// Defense-in-depth policy on the document response.
const CSP: &str = "sandbox; default-src 'none'; script-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; style-src 'unsafe-inline'";

/// Capacity-one event loop wake sender.
#[derive(Clone)]
struct Wake(async_channel::Sender<()>);
#[expect(
    clippy::implicit_return,
    clippy::let_underscore_must_use,
    clippy::let_underscore_untyped,
    reason = "The Servo waker returns a boxed clone and deliberately coalesces a full channel"
)]
impl EventLoopWaker for Wake {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        let _ = self.0.try_send(());
    }
}
/// GUI-thread delegate for frame and default-deny callbacks.
struct Policy {
    /// Coalesced frame-ready state.
    frame: Cell<bool>,
    /// Signal the GUI subscription after a frame arrives.
    wake: Wake,
}
/// Cancel every resource except the selected memory document.
fn deny(load: WebResourceLoad) {
    // The sole registered memory document is the only request allowed through
    // to the protocol registry. All other requests are explicitly cancelled.
    if load.request().url.as_str() != DOCUMENT {
        let response = WebResourceResponse::new(load.request().url.clone());
        load.intercept(response).cancel();
    }
}
#[expect(
    clippy::missing_trait_methods,
    reason = "Servo provides defaults for unrelated callbacks"
)]
impl ServoDelegate for Policy {
    fn load_web_resource(&self, load: WebResourceLoad) {
        deny(load);
    }
}
#[expect(
    clippy::missing_trait_methods,
    clippy::arbitrary_source_item_ordering,
    reason = "Servo provides unrelated defaults; related policy callbacks stay together"
)]
impl WebViewDelegate for Policy {
    fn load_web_resource(&self, _: WebView, load: WebResourceLoad) {
        deny(load);
    }
    fn request_navigation(&self, _: WebView, request: NavigationRequest) {
        request.deny();
    }
    fn request_permission(&self, _: WebView, request: PermissionRequest) {
        request.deny();
    }
    fn notify_new_frame_ready(&self, _: WebView) {
        self.frame.set(true);
        self.wake.wake();
    }
}
/// Immutable synthetic document served by the custom protocol.
struct Document(Vec<u8>);
#[expect(
    clippy::missing_trait_methods,
    clippy::implicit_return,
    clippy::absolute_paths,
    clippy::std_instead_of_core,
    reason = "Servo supplies protocol defaults and the response is an immediately ready future"
)]
impl ProtocolHandler for Document {
    fn load<'request>(
        &'request self,
        request: &'request mut Request,
        _: &mut DoneChannel,
        _: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'request>> {
        let url = request.current_url();
        let response = if url.as_str() == DOCUMENT {
            let mut response = Response::new(url, ResourceFetchTiming::new(request.timing_type()));
            *response.body.lock() = ResponseBody::Done(self.0.clone());
            response.headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response.headers.insert(
                http::header::CONTENT_SECURITY_POLICY,
                http::HeaderValue::from_static(CSP),
            );
            response.status = HttpStatus::default();
            response
        } else {
            Response::network_error(NetworkError::ResourceLoadError("unknown document".into()))
        };
        Box::pin(std::future::ready(response))
    }
}

/// A viewport on the thread on which it was created. It accepts only a fixed synthetic fixture.
pub struct Viewport {
    /// The GUI-thread-affine web view.
    view: WebView,
    /// Software viewport rendering context.
    context: Rc<SoftwareRenderingContext>,
    /// Servo instance owned by the creating thread.
    servo: Servo,
    /// Shared resource and frame delegate.
    policy: Rc<Policy>,
    /// Coalesced wake receiver.
    wakes: async_channel::Receiver<()>,
    /// Bounded physical viewport dimensions.
    size: (u32, u32),
}
#[expect(
    clippy::arbitrary_source_item_ordering,
    reason = "Lifecycle operations are grouped by use rather than alphabetically"
)]
impl Viewport {
    /// Create a viewport; dimensions must be between 1 and 2048 pixels.
    ///
    /// # Errors
    /// Returns an error for invalid dimensions or rendering-context setup failure.
    #[inline]
    #[expect(
        clippy::implicit_return,
        reason = "This fallible constructor delegates to the bounded internal constructor"
    )]
    pub fn synthetic(width: u32, height: u32) -> Result<Self, String> {
        let body = sanitize(&format!(
            "<h1>Synthetic Servo viewport</h1><p>No mail</p>{}",
            "<p>Synthetic scrolling marker</p>".repeat(80)
        ));
        let document = format!(
            "<!doctype html><style>body {{ margin: 0; padding: 30px; background: white; color: black }} .marker {{ width: 80px; height: 80px; background: #00ff00 }} .gap {{ height: 400px }} @media (prefers-color-scheme: dark) {{ body {{ background: black; color: white }} }}</style>{body}"
        );
        Self::from_document(width, height, document)
    }
    /// Create a viewport around the internal synthetic document.
    #[cfg_attr(
        not(test),
        expect(
            clippy::single_call_fn,
            reason = "The private constructor is also exercised by tests"
        )
    )]
    #[expect(
        clippy::implicit_return,
        clippy::question_mark_used,
        clippy::min_ident_chars,
        clippy::clone_on_ref_ptr,
        reason = "The internal constructor shares policy and context with Servo and propagates setup failures"
    )]
    fn from_document(width: u32, height: u32, document: String) -> Result<Self, String> {
        check_size(width, height)?;
        let (sender, wakes) = async_channel::bounded(1);
        let wake = Wake(sender);
        let policy = Rc::new(Policy {
            frame: Cell::new(false),
            wake: wake.clone(),
        });
        let context = Rc::new(
            SoftwareRenderingContext::new((width, height).into())
                .map_err(|e| format!("context: {e:?}"))?,
        );
        context
            .make_current()
            .map_err(|e| format!("context: {e:?}"))?;
        let mut registry = ProtocolRegistry::default();
        registry
            .register("mailstronaut", Document(document.into_bytes()))
            .map_err(|e| format!("protocol: {e:?}"))?;
        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(wake))
            .protocol_registry(registry)
            .build();
        servo.set_delegate(policy.clone());
        let view = WebViewBuilder::new(&servo, context.clone())
            .delegate(policy.clone())
            .url(url::Url::parse(DOCUMENT).map_err(|e| e.to_string())?)
            .build();
        view.show();
        Ok(Self {
            view,
            context,
            servo,
            policy,
            wakes,
            size: (width, height),
        })
    }
    /// Clone the coalescing wake receiver for one async GUI subscription consumer.
    /// Servo and this viewport must still be driven on their creator thread.
    #[inline]
    #[must_use = "Poll the receiver to drive Servo without blocking the GUI."]
    #[expect(
        clippy::implicit_return,
        reason = "The receiver is an async subscription handle"
    )]
    pub fn wake_receiver(&self) -> async_channel::Receiver<()> {
        self.wakes.clone()
    }
    /// Process pending work and return an owned viewport-sized RGBA frame when ready.
    #[inline]
    #[must_use = "Use the returned pixels to update the reader viewport."]
    #[expect(
        clippy::implicit_return,
        clippy::question_mark_used,
        clippy::default_numeric_fallback,
        clippy::cast_possible_wrap,
        clippy::as_conversions,
        reason = "Viewport dimensions are validated below 2049 before conversion and absent frames are normal"
    )]
    pub fn paint(&self) -> Option<Vec<u8>> {
        self.servo.spin_event_loop();
        if !self.policy.frame.replace(false) {
            return None;
        }
        self.view.paint();
        let image = self.context.read_to_image(DeviceIntRect::new(
            DeviceIntPoint::new(0, 0),
            DeviceIntPoint::new(self.size.0 as i32, self.size.1 as i32),
        ))?;
        self.context.present();
        Some(image.into_raw())
    }
    /// Resize the bounded viewport.
    ///
    /// # Errors
    /// Returns an error if either dimension is outside 1..=2048 pixels.
    #[inline]
    #[expect(
        clippy::implicit_return,
        clippy::question_mark_used,
        reason = "Resize propagates the bounded dimension validation"
    )]
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        check_size(width, height)?;
        self.size = (width, height);
        self.view.resize((width, height).into());
        Ok(())
    }
    /// Set the physical pixels per logical CSS pixel.
    ///
    /// # Errors
    /// Rejects non-finite or non-positive scales.
    #[inline]
    #[expect(
        clippy::implicit_return,
        reason = "This fallible operation returns the validated result."
    )]
    pub fn set_scale(&self, scale: f32) -> Result<(), String> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err("scale must be finite and positive".into());
        }
        let mut factor = self.view.hidpi_scale_factor();
        factor.0 = scale;
        self.view.set_hidpi_scale_factor(factor);
        Ok(())
    }
    /// Send a wheel event at a physical point within the viewport.
    ///
    /// # Errors
    /// Rejects non-finite values or points outside the physical viewport.
    #[expect(
        clippy::float_arithmetic,
        clippy::implicit_return,
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "Viewport bounds are at most 2048 pixels and wheel values use floating point."
    )]
    #[inline]
    pub fn wheel(&self, x: f32, y: f32, dx: f32, dy: f32, lines: bool) -> Result<(), String> {
        if ![x, y, dx, dy].iter().all(|value| value.is_finite())
            || x < 0.0
            || y < 0.0
            || x >= self.size.0 as f32
            || y >= self.size.1 as f32
        {
            return Err("invalid wheel position or delta".into());
        }
        let multiplier = if lines { 76.0 } else { 1.0 };
        self.view
            .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: f64::from(dx * multiplier),
                    y: f64::from(dy * multiplier),
                    z: 0.0,
                    mode: if lines {
                        WheelMode::DeltaLine
                    } else {
                        WheelMode::DeltaPixel
                    },
                },
                DevicePoint::new(x, y).into(),
            )));
        Ok(())
    }
    /// Forward the host's effective theme to Servo.
    #[inline]
    pub fn set_dark(&self, dark: bool) {
        self.view
            .notify_theme_change(if dark { Theme::Dark } else { Theme::Light });
    }
}
/// Restrict the synthetic document body to passive email elements.
#[cfg_attr(
    not(test),
    expect(
        clippy::single_call_fn,
        reason = "The sanitizer is also exercised by tests"
    )
)]
#[expect(
    clippy::implicit_return,
    clippy::absolute_paths,
    reason = "The explicit allowlist is kept separate from the synthetic constructor"
)]
fn sanitize(input: &str) -> String {
    ammonia::Builder::new()
        .tags(
            [
                "a",
                "abbr",
                "b",
                "blockquote",
                "br",
                "caption",
                "code",
                "col",
                "colgroup",
                "div",
                "em",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "hr",
                "i",
                "li",
                "ol",
                "p",
                "pre",
                "span",
                "strong",
                "table",
                "tbody",
                "td",
                "tfoot",
                "th",
                "thead",
                "tr",
                "u",
                "ul",
            ]
            .into_iter()
            .collect(),
        )
        .tag_attributes(std::collections::HashMap::new())
        .generic_attributes(std::collections::HashSet::new())
        .clean(input)
        .to_string()
}
/// Reject zero or oversized viewport dimensions.
#[expect(
    clippy::implicit_return,
    reason = "The validation returns one of two results"
)]
fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        Err("viewport dimensions must be 1..=2048".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[expect(
    clippy::std_instead_of_core,
    clippy::assertions_on_result_states,
    clippy::items_after_statements,
    clippy::expect_used,
    clippy::absolute_paths,
    clippy::indexing_slicing,
    clippy::chunks_exact_to_as_chunks,
    clippy::implicit_return,
    clippy::missing_asserts_for_indexing,
    reason = "Synthetic test fixtures fail immediately on setup errors and inspect bounded RGBA chunks"
)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn sanitizer_removes_active_content_and_css() {
        let clean = sanitize(
            r#"<table><tr><td style="background:url(http://example.invalid)">hello</td></tr></table><script>alert(1)</script><img src="file:///etc/passwd"><a href="https://example.invalid" onclick="evil()">link</a>"#,
        );
        assert!(clean.contains("<table>"));
        assert!(clean.contains("hello"));
        assert!(!clean.contains("style="));
        assert!(!clean.contains("script"));
        assert!(!clean.contains("src="));
        assert!(!clean.contains("href="));
        assert!(!clean.contains("onclick"));
    }

    #[test]
    fn bounded_and_coalesced() {
        assert!(check_size(0, 1).is_err());
        assert!(check_size(2049, 1).is_err());
        assert!(check_size(2048, 2048).is_ok());
        let (sender, receiver) = async_channel::bounded(1);
        let wake = Wake(sender);
        wake.wake();
        wake.wake();
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn synthetic_pixels_and_native_theme() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let local = tempfile::NamedTempFile::new().expect("synthetic file");
        std::fs::write(local.path(), r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="red"/></svg>"#).expect("write fixture");
        let file_url = url::Url::from_file_path(local.path()).expect("file URL");
        let document = format!(
            r#"<!doctype html><style>@import url(http://{address}/style); body {{ margin: 0; padding: 30px; background: white; color: black }} .marker {{ width: 80px; height: 80px; background: #00ff00 }} .gap {{ height: 400px }} @media (prefers-color-scheme: dark) {{ body {{ background: black; color: white }} }}</style><div class=marker></div><div class=gap></div><link rel="preconnect" href="http://{address}"><img src="http://{address}/pixel"><img src="{file_url}"><img src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%22100%22 height=%22100%22%3E%3Crect width=%22100%22 height=%22100%22 fill=%22red%22/%3E%3C/svg%3E"><script>document.body.style.background='red'; fetch('http://{address}/script')</script><h1>Synthetic viewport</h1>"#
        );
        let mut viewport = Viewport::from_document(320, 200, document).expect("viewport");
        fn frame(viewport: &Viewport, dark: bool) -> Vec<u8> {
            viewport.set_dark(dark);
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(frame) = viewport.paint() {
                    let background = &frame[0..4];
                    if ((dark && background[..3].iter().all(|value| *value < 32))
                        || (!dark && background[..3].iter().all(|value| *value > 224)))
                        && frame
                            .chunks_exact(4)
                            .any(|pixel| pixel[..3] != background[..3])
                    {
                        return frame;
                    }
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "timed out waiting for theme frame");
                if viewport.wake_receiver().try_recv().is_err() {
                    std::thread::sleep(Duration::from_millis(5).min(remaining));
                }
            }
        }
        let light = frame(&viewport, false);
        assert_eq!(light.len(), 320 * 200 * 4);
        assert!(light.chunks_exact(4).any(|pixel| pixel[0] < 128));
        assert!(
            !light
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50),
            "script or resource changed pixels"
        );
        let dark = frame(&viewport, true);
        assert!(dark.chunks_exact(4).any(|pixel| pixel[1] > 128));
        viewport.resize(160, 100).expect("resize");
        let resized = frame(&viewport, true);
        assert_eq!(resized.len(), 160 * 100 * 4);
        assert!(viewport.resize(2049, 1).is_err());
        assert!(listener.accept().is_err(), "unexpected loopback connection");
        viewport.set_dark(false);
        viewport.resize(320, 200).expect("restore size");
        check_scroll_and_scale(&mut viewport);
    }
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::single_call_fn,
        reason = "Bounded test pixels and deadlines are checked on one retained engine."
    )]
    fn check_scroll_and_scale(viewport: &mut Viewport) {
        fn wait_for(viewport: &Viewport, predicate: impl Fn(&[u8]) -> bool) -> Vec<u8> {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(frame) = viewport.paint()
                    && predicate(&frame)
                {
                    return frame;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "timed out waiting for expected pixels"
                );
                if viewport.wake_receiver().try_recv().is_err() {
                    std::thread::sleep(Duration::from_millis(5).min(remaining));
                }
            }
        }
        fn green(frame: &[u8], width: usize, x: usize, y: usize) -> bool {
            let pixel = &frame[(y * width + x) * 4..][..3];
            pixel[0] < 30 && pixel[1] > 220 && pixel[2] < 30
        }
        let initial = wait_for(viewport, |frame| green(frame, 320, 40, 40));
        assert!(!green(&initial, 320, 120, 40));
        viewport
            .wheel(50.0, 50.0, 0.0, -300.0, false)
            .expect("wheel");
        let scrolled = wait_for(viewport, |frame| !green(frame, 320, 40, 40));
        assert_ne!(initial, scrolled);
        assert!(viewport.wheel(f32::NAN, 0.0, 0.0, 1.0, false).is_err());
        assert!(viewport.set_scale(0.0).is_err());
        assert!(viewport.set_scale(f32::INFINITY).is_err());
        viewport
            .wheel(50.0, 50.0, 0.0, 1000.0, false)
            .expect("wheel up");
        drop(wait_for(viewport, |frame| green(frame, 320, 40, 40)));
        viewport.set_scale(1.5).expect("scale");
        viewport.resize(480, 300).expect("resize");
        let scaled = wait_for(viewport, |frame| {
            frame.len() == 480 * 300 * 4 && green(frame, 480, 100, 70)
        });
        assert!(!green(&scaled, 480, 170, 70));
        viewport.set_scale(2.0).expect("scale 200 percent");
        viewport.resize(640, 400).expect("resize 200 percent");
        let doubled = wait_for(viewport, |frame| {
            frame.len() == 640 * 400 * 4 && green(frame, 640, 150, 80)
        });
        assert!(!green(&doubled, 640, 230, 80));
        viewport.set_scale(1.5).expect("restore scale");
        viewport.resize(480, 300).expect("restore size");
        viewport.set_dark(true);
        let scaled_dark = wait_for(viewport, |frame| {
            frame.len() == 480 * 300 * 4 && frame[(200 * 480 + 200) * 4] < 30
        });
        assert!(green(&scaled_dark, 480, 70, 70));
    }
}
