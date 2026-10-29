//! A provisional, synthetic-only software Servo viewport.
#![forbid(unsafe_code)]

use content_security_policy::Destination;
use core::{
    cell::{Cell, RefCell},
    future::{Future, ready},
    pin::Pin,
};
use http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, NetworkError, ProtocolHandler, ProtocolRegistry,
    Request, ResourceFetchTiming, Response, ResponseBody,
};
use servo::{
    DeviceIntPoint, DeviceIntRect, DevicePoint, EventLoopWaker, InputEvent, NavigationRequest,
    PermissionRequest, RenderingContext as _, Servo, ServoBuilder, ServoDelegate,
    SoftwareRenderingContext, Theme, WebResourceLoad, WebResourceResponse, WebView, WebViewBuilder,
    WebViewDelegate, WebViewId, WheelDelta, WheelEvent, WheelMode,
};
use std::sync::Mutex;
extern crate alloc;
use alloc::{rc::Rc, sync::Arc};

/// Maximum width or height of the software viewport.
const MAX_SIDE: u32 = 2048;
/// Maximum generated synthetic HTML size.
const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
/// Private fixture substitution, never a user-supplied URL resolver.
const LOCAL_IMAGE: &str = "@local-raster@";
/// One fixed, known 2×2 red PNG; arbitrary image input is not exposed.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 2, 0,
    0, 0, 253, 212, 154, 115, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 68, 12,
    16, 10, 0, 31, 238, 3, 253, 139, 95, 20, 212, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
/// Defense-in-depth policy on the document response.
const CSP: &str = "sandbox; default-src 'none'; script-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; style-src 'unsafe-inline'; img-src mailstronaut:";

/// Capacity-one event loop wake sender.
#[derive(Clone)]
struct Wake(async_channel::Sender<()>);
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
    /// Active message-local resources shared with the protocol handler.
    resources: Arc<Resources>,
    #[cfg(test)]
    probe: Option<Rc<Probe>>,
}
#[cfg(test)]
#[derive(Clone, Copy, Default)]
enum TestCsp {
    #[default]
    Production,
    Resources,
    Scripts,
}
#[cfg(test)]
struct Probe {
    csp: TestCsp,
    allowed: Vec<String>,
    callbacks: RefCell<Vec<(String, String, bool)>>,
}
#[cfg(test)]
std::thread_local! {
    static PROBE: RefCell<Option<Rc<Probe>>> = const { RefCell::new(None) };
}
/// Cancel any load not explicitly associated with the active table and view.
fn deny(
    load: WebResourceLoad,
    resources: &Resources,
    view: Option<WebViewId>,
    #[cfg(test)] probe_option: Option<&Probe>,
) {
    let request = load.request();
    let allowed = !request.is_redirect
        && request.method == http::Method::GET
        && resources
            .lookup(view, request.url.as_str(), request.destination)
            .is_some();
    #[cfg(test)]
    let allowed = allowed
        || (!request.is_redirect
            && request.method == http::Method::GET
            && view.is_some()
            && probe_option.is_some_and(|probe| probe.allowed.contains(&request.url.to_string())));
    #[cfg(test)]
    if let Some(probe) = probe_option {
        probe.callbacks.borrow_mut().push((
            request.url.to_string(),
            format!("{:?}", request.destination),
            !allowed,
        ));
    }
    if !allowed {
        let response = WebResourceResponse::new(load.request().url.clone());
        load.intercept(response).cancel();
    }
}
impl ServoDelegate for Policy {
    fn load_web_resource(&self, load: WebResourceLoad) {
        deny(
            load,
            &self.resources,
            None,
            #[cfg(test)]
            self.probe.as_deref(),
        );
    }
}
impl WebViewDelegate for Policy {
    fn load_web_resource(&self, view: WebView, load: WebResourceLoad) {
        deny(
            load,
            &self.resources,
            Some(view.id()),
            #[cfg(test)]
            self.probe.as_deref(),
        );
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
/// Immutable resources for one unguessable application namespace, not a proven browser origin.
struct Document {
    /// Exact top-level URL.
    url: url::Url,
    /// Exact local PNG URL.
    image: String,
    /// Bounded generated HTML bytes.
    body: Arc<[u8]>,
    #[cfg(test)]
    csp: TestCsp,
}
impl Document {
    /// Prepare a bounded private fixture with a fresh namespace.
    fn new(template: &str, #[cfg(test)] csp: TestCsp) -> Result<Self, String> {
        if template.len() > MAX_DOCUMENT_BYTES {
            return Err("synthetic document exceeds limit".into());
        }
        let namespace = uuid::Uuid::new_v4();
        let image = format!("mailstronaut://{namespace}/raster.png");
        let body = template.replace(LOCAL_IMAGE, &image);
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("generated document exceeds limit".into());
        }
        Ok(Self {
            url: url::Url::parse(&format!("mailstronaut://{namespace}/document"))
                .map_err(|error| error.to_string())?,
            image,
            body: body.into_bytes().into(),
            #[cfg(test)]
            csp,
        })
    }
}
/// One live binding; retirement removes all registered access to the old generation.
struct Binding {
    /// Concrete Servo view identity, never inferred from the current UI selection.
    view: WebViewId,
    /// Immutable document and known raster.
    document: Arc<Document>,
}
/// Thread-safe protocol state; poison or absent/mismatched bindings fail closed.
#[derive(Default)]
struct Resources(Mutex<Option<Binding>>);
impl Resources {
    /// Publish one immutable table bound to its concrete view.
    fn bind(&self, view: WebViewId, document: Arc<Document>) -> Result<(), String> {
        let Ok(mut slot) = self.0.lock() else {
            return Err("resource registry unavailable".into());
        };
        if slot.is_some() {
            return Err("resource generation already active".into());
        }
        *slot = Some(Binding { view, document });
        drop(slot);
        Ok(())
    }
    /// Revoke only the matching generation, leaving any other binding untouched.
    fn retire(&self, view: WebViewId) {
        if let Ok(mut slot) = self.0.lock()
            && slot.as_ref().is_some_and(|binding| binding.view == view)
        {
            *slot = None;
        }
    }
    /// Return immutable bytes only for an exact URL, destination and active view.
    fn lookup(
        &self,
        view: Option<WebViewId>,
        url: &str,
        destination: Destination,
    ) -> Option<Arc<Document>> {
        let slot = self.0.lock().ok()?;
        let binding = slot.as_ref()?;
        if Some(binding.view) != view {
            return None;
        }
        let document = &binding.document;
        let result = ((url == document.url.as_str() && destination == Destination::Document)
            || (url == document.image && destination == Destination::Image))
            .then(|| Arc::clone(document));
        drop(slot);
        result
    }
}
/// Protocol handler registered once for the retained engine.
struct MemoryProtocol(Arc<Resources>);
impl ProtocolHandler for MemoryProtocol {
    fn load<'request>(
        &'request self,
        request: &'request mut Request,
        _: &mut DoneChannel,
        _: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'request>> {
        let url = request.current_url();
        let candidate = if request.method == http::Method::GET && request.redirect_count == 0 {
            self.0
                .lookup(request.target_webview_id, url.as_str(), request.destination)
        } else {
            None
        };
        let Some(document) = candidate else {
            return Box::pin(ready(Response::network_error(
                NetworkError::ResourceLoadError("unavailable local resource".into()),
            )));
        };
        let response = {
            let is_document = url.as_str() == document.url.as_str();
            let mut response = Response::new(url, ResourceFetchTiming::new(request.timing_type()));
            *response.body.lock() = ResponseBody::Done(if is_document {
                document.body.to_vec()
            } else {
                PNG.to_vec()
            });
            response.headers.insert(
                CONTENT_TYPE,
                http::HeaderValue::from_static(if is_document {
                    "text/html; charset=utf-8"
                } else {
                    "image/png"
                }),
            );
            if is_document {
                let csp = CSP;
                #[cfg(test)]
                let csp = match document.csp {
                    TestCsp::Production => csp,
                    TestCsp::Resources => {
                        "sandbox; default-src 'none'; script-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; style-src 'unsafe-inline' http:; img-src mailstronaut: http: file: data:; font-src http:"
                    }
                    TestCsp::Scripts => {
                        "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline'; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; style-src 'unsafe-inline'; img-src mailstronaut: http:"
                    }
                };
                response
                    .headers
                    .insert(CONTENT_SECURITY_POLICY, http::HeaderValue::from_static(csp));
            }
            response
                .headers
                .insert(CACHE_CONTROL, http::HeaderValue::from_static("no-store"));
            response.status = HttpStatus::default();
            response
        };
        Box::pin(ready(response))
    }
}

/// A single thread-affine Servo engine for the fixed synthetic document.
///
/// Servo initialization is process-global and cannot be restarted after shutdown.
/// Retain this owner across empty intervals, and drive wakes on its creating thread.
pub struct Renderer(Rc<Engine>);

/// Thread-affine shared owner kept alive across viewport teardown.
struct Engine {
    /// Pinned Servo instance.
    servo: Servo,
    /// Engine-wide default-deny resource policy.
    policy: Rc<Policy>,
    /// Capacity-one wake receiver.
    wakes: async_channel::Receiver<()>,
    /// One retained software context, including while no viewport exists.
    context: RefCell<Option<Rc<SoftwareRenderingContext>>>,
    /// Whether a viewport is alive.
    active: Cell<bool>,
    /// Fixed private synthetic template reused with a fresh namespace per view.
    document: String,
}

/// A bounded software viewport owned by its creating thread.
pub struct Viewport {
    /// Disposable Servo view.
    view: WebView,
    /// Software framebuffer context.
    context: Rc<SoftwareRenderingContext>,
    /// Frame state scoped to this view.
    policy: Rc<Policy>,
    /// Engine kept alive through view destruction.
    engine: Rc<Engine>,
    /// Bounded physical size.
    size: (u32, u32),
}

impl Drop for Viewport {
    #[inline]
    fn drop(&mut self) {
        self.engine.policy.resources.retire(self.view.id());
        self.context.resize((1_u32, 1_u32).into());
        self.engine.active.set(false);
    }
}

impl Renderer {
    /// Initialize the engine with its fixed synthetic fixture.
    ///
    /// # Errors
    /// Returns an error if protocol registration fails.
    #[inline]
    pub fn synthetic() -> Result<Self, String> {
        let text = Self::synthetic_text();
        let body = sanitize(&format!(
            "<h1>Synthetic Servo viewport</h1><p>{}</p>",
            text.lines().skip(1).collect::<Vec<_>>().join("</p><p>")
        ));
        let document = format!(
            "<!doctype html><style>body {{ margin: 0; padding: 30px; background: white; color: black }} .marker {{ width: 80px; height: 80px; background: #00ff00 }} .gap {{ height: 400px }} @media (prefers-color-scheme: dark) {{ body {{ background: black; color: white }} }}</style>{body}"
        );
        Self::from_document(document)
    }
    /// Decoded text of the fixed synthetic message, shared with the native reader.
    #[inline]
    #[must_use]
    pub fn synthetic_text() -> String {
        format!(
            "Synthetic Servo viewport\n\nNo mail\n\nCafé — français, italiano: caffè. Ελληνικά. 日本語.\n\n{}End of synthetic message.",
            "Synthetic scrolling marker\n\n".repeat(80)
        )
    }
    /// Construct the engine with an internal fixture document.
    fn from_document(document: String) -> Result<Self, String> {
        // The unified mail/Servo graph enables Ring and AWS-LC. Servo's own
        // network clients use Rustls' process default, which cannot infer one
        // with both features present. Choose Ring before starting its threads;
        // an already-installed process provider remains owned by its caller.
        let _selection = rustls::crypto::ring::default_provider().install_default();
        let (sender, wakes) = async_channel::bounded(1);
        let wake = Wake(sender);
        let policy = Rc::new(Policy {
            frame: Cell::new(false),
            wake: wake.clone(),
            resources: Arc::new(Resources::default()),
            #[cfg(test)]
            probe: PROBE.with(|probe| probe.borrow().clone()),
        });
        let mut registry = ProtocolRegistry::default();
        registry
            .register(
                "mailstronaut",
                MemoryProtocol(Arc::clone(&policy.resources)),
            )
            .map_err(|e| format!("protocol: {e:?}"))?;
        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(wake))
            .protocol_registry(registry)
            .build();
        servo.set_delegate(policy.clone());
        Ok(Self(Rc::new(Engine {
            servo,
            policy,
            wakes,
            context: RefCell::new(None),
            active: Cell::new(false),
            document,
        })))
    }
    /// Create the only active viewport on this engine.
    ///
    /// # Errors
    /// Returns an error for invalid dimensions, an active viewport, or context setup failure.
    #[inline]
    pub fn viewport(&self, width: u32, height: u32) -> Result<Viewport, String> {
        self.viewport_document(width, height, &self.0.document)
    }
    /// Bind a fresh private fixture before driving any queued network callbacks.
    fn viewport_document(
        &self,
        width: u32,
        height: u32,
        template: &str,
    ) -> Result<Viewport, String> {
        check_size(width, height)?;
        if self.0.active.get() {
            return Err("viewport already active".into());
        }
        let document = Arc::new(Document::new(
            template,
            #[cfg(test)]
            self.0
                .policy
                .probe
                .as_ref()
                .map_or(TestCsp::Production, |probe| probe.csp),
        )?);
        let cached_context = self.0.context.borrow().clone();
        let context = if let Some(context) = cached_context {
            context
        } else {
            let context = Rc::new(
                SoftwareRenderingContext::new((width, height).into())
                    .map_err(|e| format!("context: {e:?}"))?,
            );
            self.0.context.replace(Some(Rc::clone(&context)));
            context
        };
        context.resize((width, height).into());
        context
            .make_current()
            .map_err(|e| format!("context: {e:?}"))?;
        let policy = Rc::new(Policy {
            frame: Cell::new(false),
            wake: self.0.policy.wake.clone(),
            resources: Arc::clone(&self.0.policy.resources),
            #[cfg(test)]
            probe: self.0.policy.probe.clone(),
        });
        let view = WebViewBuilder::new(&self.0.servo, context.clone())
            .delegate(policy.clone())
            .url(document.url.clone())
            .build();
        policy.resources.bind(view.id(), document)?;
        view.show();
        self.0.active.set(true);
        Ok(Viewport {
            view,
            context,
            policy,
            engine: Rc::clone(&self.0),
            size: (width, height),
        })
    }
    /// Drive pending engine work, including teardown while no viewport exists.
    #[inline]
    pub fn spin(&self) {
        self.0.servo.spin_event_loop();
    }
    /// Clone the coalesced wake receiver for the UI subscription.
    #[inline]
    #[must_use = "Poll the receiver to drive Servo."]
    pub fn wake_receiver(&self) -> async_channel::Receiver<()> {
        self.0.wakes.clone()
    }
}

impl Viewport {
    /// Clone the coalescing wake receiver for one async GUI subscription consumer.
    /// Servo and this viewport must still be driven on their creator thread.
    #[inline]
    #[must_use = "Poll the receiver to drive Servo without blocking the GUI."]
    pub fn wake_receiver(&self) -> async_channel::Receiver<()> {
        self.engine.wakes.clone()
    }
    /// Process pending work and return an owned viewport-sized RGBA frame when ready.
    #[inline]
    #[must_use = "Use the returned pixels to update the reader viewport."]
    #[expect(
        clippy::cast_possible_wrap,
        reason = "Viewport dimensions are validated below 2049 before conversion and absent frames are normal"
    )]
    pub fn paint(&self) -> Option<Vec<u8>> {
        self.engine.servo.spin_event_loop();
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
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        check_size(width, height)?;
        self.size = (width, height);
        self.view.resize((width, height).into());
        Ok(())
    }
    /// Set the physical pixels per logical CSS pixel.
    ///
    /// # Errors
    /// Rejects scales outside the bounded application policy.
    #[inline]
    pub fn set_scale(&self, scale: f32) -> Result<(), String> {
        if !scale.is_finite() || !(0.000_488_281_25..=2048.0).contains(&scale) {
            return Err("scale outside supported range".into());
        }
        let mut factor = self.view.hidpi_scale_factor();
        factor.0 = scale;
        self.view.set_hidpi_scale_factor(factor);
        Ok(())
    }
    /// Send a wheel event at a physical point within the viewport.
    ///
    /// # Errors
    /// Rejects non-finite or excessive deltas and points outside the physical viewport.
    #[expect(
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
        // Match iced scrollable: one line is 60 logical pixels. Servo scrolls
        // in device pixels regardless of WheelMode. Pixel input is already forwarded
        // as supplied by the platform, so only line input is scaled here.
        let multiplier = if lines {
            60.0 * self.view.hidpi_scale_factor().0
        } else {
            1.0
        };
        let physical = [dx * multiplier, dy * multiplier];
        if physical
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 2048.0)
        {
            return Err("wheel delta outside supported range".into());
        }
        self.view
            .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: f64::from(physical[0]),
                    y: f64::from(physical[1]),
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
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
fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        Err("viewport dimensions must be 1..=2048".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[expect(
    clippy::items_after_statements,
    clippy::chunks_exact_to_as_chunks,
    reason = "Synthetic test fixtures fail immediately on setup errors and inspect bounded RGBA chunks"
)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn resource_interception_subprocess() {
        use std::process::{Command, Stdio};
        if let Ok(mode) = std::env::var("MAILSTRONAUT_P01_CHILD") {
            if mode == "local" {
                local_resource_probe();
            } else {
                resource_probe(mode == "allow");
            }
            return;
        }
        for mode in ["deny", "allow", "local"] {
            let mut child = Command::new(std::env::current_exe().expect("test executable"))
                .args([
                    "--exact",
                    "tests::resource_interception_subprocess",
                    "--nocapture",
                ])
                .env("MAILSTRONAUT_P01_CHILD", mode)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("spawn probe");
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                if let Some(status) = child.try_wait().expect("child status") {
                    assert!(status.success(), "{mode} probe failed: {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().expect("kill timed out child");
                    child.wait().expect("reap timed out child");
                    panic!("{mode} probe timed out");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[test]
    fn script_layers_subprocess() {
        use std::process::{Command, Stdio};
        if let Ok(mode) = std::env::var("MAILSTRONAUT_SCRIPT_CHILD") {
            script_probe(&mode);
            return;
        }
        for mode in ["production", "positive", "sanitized"] {
            let mut child = Command::new(std::env::current_exe().expect("test executable"))
                .args(["--exact", "tests::script_layers_subprocess", "--nocapture"])
                .env("MAILSTRONAUT_SCRIPT_CHILD", mode)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("spawn script probe");
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                if let Some(status) = child.try_wait().expect("child status") {
                    assert!(status.success(), "{mode} probe failed: {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().expect("kill child");
                    child.wait().expect("reap child");
                    panic!("{mode} probe timed out");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn script_probe(mode: &str) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let urls: Vec<String> = ["inline", "event"]
            .iter()
            .map(|path| format!("http://{address}/{path}"))
            .collect();
        let probe = Rc::new(Probe {
            csp: if mode == "production" {
                TestCsp::Production
            } else {
                TestCsp::Scripts
            },
            allowed: if mode == "production" {
                Vec::new()
            } else {
                urls.clone()
            },
            callbacks: RefCell::new(Vec::new()),
        });
        PROBE.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&probe)));
        let fixture = format!(
            r#"<p>Unique safe baseline</p><script>document.getElementById('inline').style.background='red'; (new Image()).src='{}';</script><img id=trigger src='@local-raster@' onload="document.getElementById('event').style.background='blue'; (new Image()).src='{}'">"#,
            urls[0], urls[1]
        );
        let body = if mode == "sanitized" {
            sanitize(&fixture)
        } else {
            fixture
        };
        let document = format!(
            "<!doctype html><style>body {{margin:0;background:white}} p {{background:#00ff00;position:absolute;left:0;top:0;width:40px;height:40px}} #inline,#event {{position:absolute;top:60px;width:40px;height:40px;background:white}} #inline {{left:0}} #event {{left:50px}} #trigger {{position:absolute;left:110px;top:60px;width:20px;height:20px}}</style><div id=inline></div><div id=event></div>{body}"
        );
        let viewport = Renderer::from_document(document)
            .expect("engine")
            .viewport(160, 120)
            .expect("viewport");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ready = false;
        let mut text_visible = false;
        let mut inline = false;
        let mut event = false;
        let mut trigger = false;
        let mut requests = Vec::new();
        while Instant::now() < deadline {
            if let Some(frame) = viewport.paint() {
                let pixel = |x: usize, y: usize| &frame[(y * 160 + x) * 4..][..3];
                ready |= pixel(35, 35) == [0, 255, 0];
                text_visible |= (0..40).any(|y| {
                    (0..40).any(|x| {
                        let color = pixel(x, y);
                        color[0] < 80 && color[1] < 80 && color[2] < 80
                    })
                });
                inline |= pixel(10, 70) == [255, 0, 0];
                event |= pixel(60, 70) == [0, 0, 255];
                trigger |= pixel(120, 70) == [255, 0, 0];
            }
            serve_script_markers(&listener, &mut requests);
            std::thread::sleep(Duration::from_millis(5));
        }
        println!(
            "script mode={mode} ready={ready} trigger={trigger} inline={inline} event={event} requests={requests:?}"
        );
        assert!(
            ready && text_visible,
            "unique safe baseline and text did not render"
        );
        assert_eq!(trigger, mode != "sanitized", "local load-event trigger");
        if mode == "positive" {
            assert!(inline && event, "both script effects must render");
            for path in ["/inline", "/event"] {
                assert!(
                    requests.iter().any(|request| request == path),
                    "missing {path}"
                );
            }
        } else {
            assert!(!inline && !event, "active content rendered");
            assert!(
                requests.is_empty(),
                "unexpected loopback request: {requests:?}"
            );
        }
    }

    fn serve_script_markers(listener: &std::net::TcpListener, requests: &mut Vec<String>) {
        use std::io::{BufRead as _, Read as _, Write as _};
        while let Ok((mut stream, _)) = listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .expect("timeout");
            stream
                .set_write_timeout(Some(Duration::from_millis(100)))
                .expect("timeout");
            let mut line = String::new();
            let mut reader = std::io::BufReader::new((&stream).take(1024));
            reader.read_line(&mut line).expect("request line");
            drop(reader);
            assert!(line.ends_with('\n'), "incomplete or excessive request line");
            let path = line.split_whitespace().nth(1).expect("request path");
            requests.push(path.to_owned());
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PNG.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(PNG);
        }
    }

    fn resource_probe(allow: bool) {
        use std::io::{Read as _, Write as _};
        const PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2,
            8, 2, 0, 0, 0, 253, 212, 154, 115, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207,
            192, 0, 68, 12, 16, 10, 0, 31, 238, 3, 253, 139, 95, 20, 212, 0, 0, 0, 0, 73, 69, 78,
            68, 174, 66, 96, 130,
        ];
        const DATA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf7gP9i18U1AAAAABJRU5ErkJggg==";
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let file = tempfile::NamedTempFile::new().expect("file");
        std::fs::write(file.path(), PNG).expect("write raster");
        let file_url = url::Url::from_file_path(file.path())
            .expect("file URL")
            .to_string();
        let urls: Vec<String> = ["image", "background", "sheet", "import", "font"]
            .iter()
            .map(|path| format!("http://{address}/{path}"))
            .collect();
        let mut allowed = Vec::new();
        if allow {
            allowed.extend(urls.iter().cloned());
            allowed.push(file_url.clone());
            allowed.push(DATA.into());
        }
        let probe = Rc::new(Probe {
            csp: TestCsp::Resources,
            allowed,
            callbacks: RefCell::new(Vec::new()),
        });
        PROBE.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&probe)));
        let document = format!(
            r#"<!doctype html><style>@import url('{}'); @font-face {{ font-family: probe; src: url('{}') }} body {{ margin:0; font-family:probe }} .bg {{position:absolute;left:0;top:0;width:40px;height:40px;background-image:url('{}');background-size:40px 40px}} img {{position:absolute;top:50px;width:40px;height:40px}} #http {{left:0}} #file {{left:50px}} #data {{left:100px}}</style><link rel=stylesheet href='{}'><div class=bg></div><img id=http src='{}'><img id=file src='{file_url}'><img id=data src='{DATA}'><p style="position:absolute;top:100px">Font probe</p>"#,
            urls[3], urls[4], urls[1], urls[2], urls[0]
        );
        let viewport = Renderer::from_document(document)
            .expect("engine")
            .viewport(320, 200)
            .expect("viewport");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut connections = 0usize;
        let mut red = [false; 4];
        while Instant::now() < deadline {
            if let Some(frame) = viewport.paint() {
                for (flag, (x, y)) in red
                    .iter_mut()
                    .zip([(10, 10), (10, 60), (60, 60), (110, 60)])
                {
                    let pixel = &frame[(y * 320 + x) * 4..][..3];
                    *flag |= pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50;
                }
            }
            while let Ok((mut stream, _)) = listener.accept() {
                connections += 1;
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .expect("timeout");
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    PNG.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(PNG);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let callbacks = probe.callbacks.borrow();
        eprintln!(
            "P01 allow={allow} connections={connections} red={red:?} callbacks={callbacks:?}"
        );
        for (url, kind) in urls
            .iter()
            .zip(["Image", "Image", "Style", "Style", "Font"])
            .chain([(&file_url, "Image"), (&DATA.to_owned(), "Image")])
        {
            assert!(
                callbacks
                    .iter()
                    .any(|(seen, destination, cancelled)| seen == url
                        && destination == kind
                        && *cancelled != allow),
                "missing {kind} callback for {url}"
            );
        }
        if allow {
            assert_eq!(connections, 5);
            assert!(red.iter().all(|value| *value), "each raster must load");
        } else {
            assert_eq!(connections, 0);
            assert!(!red.iter().any(|value| *value));
        }
    }

    fn local_resource_probe() {
        let probe = Rc::new(Probe {
            csp: TestCsp::Production,
            allowed: Vec::new(),
            callbacks: RefCell::new(Vec::new()),
        });
        PROBE.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&probe)));
        let template = "<!doctype html><style>body { margin:0; background:white } img { position:absolute; top:0; width:40px; height:40px }</style><img src='@local-raster@'>";
        let renderer = Renderer::from_document(template.into()).expect("engine");
        let first = renderer.viewport(160, 100).expect("first viewport");
        let first_document = bound_document(&first);
        let first_id = first.view.id();
        assert_eq!(
            wait_for_local_raster(&first, [255, 255, 255]).len(),
            160 * 100 * 4
        );
        check_local_scope(&first, &first_document, &probe);
        drop(first);
        assert!(
            renderer
                .0
                .policy
                .resources
                .lookup(Some(first_id), &first_document.image, Destination::Image)
                .is_none(),
            "drop retires first namespace immediately"
        );
        renderer.spin();
        let second_template = format!(
            "{template}<img style='left:50px' src='{}'><img style='left:100px' src='@local-raster@/missing'>",
            first_document.image
        );
        let second_fixture = second_template.replace("background:white", "background:blue");
        let second = renderer
            .viewport_document(160, 100, &second_fixture)
            .expect("second viewport");
        let second_document = bound_document(&second);
        assert_ne!(first_document.url, second_document.url);
        assert!(
            second
                .policy
                .resources
                .lookup(
                    Some(second.view.id()),
                    &first_document.image,
                    Destination::Image
                )
                .is_none(),
            "B cannot read A"
        );
        assert!(
            second
                .policy
                .resources
                .lookup(Some(first_id), &second_document.image, Destination::Image)
                .is_none(),
            "delayed A cannot read B"
        );
        let pixels = wait_for_local_raster(&second, [0, 0, 255]);
        assert!(!red_at(&pixels, 60), "retired A raster rendered in B");
        assert!(!red_at(&pixels, 110), "unknown raster rendered in B");
        let until = Instant::now()
            .checked_add(Duration::from_secs(2))
            .expect("bounded deadline");
        while Instant::now() < until {
            if let Some(later_pixels) = second.paint() {
                assert!(!red_at(&later_pixels, 60), "retired A raster rendered in B");
                assert!(!red_at(&later_pixels, 110), "unknown raster rendered in B");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for denied in [
            &first_document.image,
            &format!("{}/missing", second_document.image),
        ] {
            assert!(
                probe
                    .callbacks
                    .borrow()
                    .iter()
                    .any(|entry| entry.0 == *denied && entry.1 == "Image" && entry.2),
                "missing denied request for {denied}"
            );
        }
        let second_id = second.view.id();
        drop(second);
        assert!(
            renderer
                .0
                .policy
                .resources
                .lookup(Some(second_id), &second_document.image, Destination::Image)
                .is_none()
        );
        renderer.spin();
        println!(
            "P01 local raster visible; initial document/view association, A-to-B retirement, unknown URL and wrong-view/destination checks passed under production CSP"
        );
    }

    fn bound_document(viewport: &Viewport) -> Arc<Document> {
        Arc::clone(
            &viewport
                .policy
                .resources
                .0
                .lock()
                .expect("registry")
                .as_ref()
                .expect("binding")
                .document,
        )
    }

    fn check_local_scope(first: &Viewport, document: &Document, probe: &Probe) {
        assert!(
            probe
                .callbacks
                .borrow()
                .iter()
                .any(|(url, destination, cancelled)| url == document.url.as_str()
                    && destination == "Document"
                    && !cancelled),
            "initial document has a routed, bound view"
        );
        assert!(
            probe
                .callbacks
                .borrow()
                .iter()
                .any(|(url, destination, cancelled)| url == &document.image
                    && destination == "Image"
                    && !cancelled),
            "local PNG routed through view policy"
        );
        assert!(
            first
                .policy
                .resources
                .lookup(None, &document.image, Destination::Image)
                .is_none()
        );
        assert!(
            first
                .policy
                .resources
                .lookup(
                    Some(first.view.id()),
                    &document.image,
                    Destination::Document
                )
                .is_none()
        );
        assert!(
            first
                .policy
                .resources
                .lookup(
                    Some(first.view.id()),
                    document.url.as_str(),
                    Destination::Image
                )
                .is_none()
        );
        for suffix in [
            "?query",
            "#fragment",
            "/../raster.png",
            "%2fextra",
            "/missing",
        ] {
            assert!(
                first
                    .policy
                    .resources
                    .lookup(
                        Some(first.view.id()),
                        &format!("{}{suffix}", document.image),
                        Destination::Image
                    )
                    .is_none()
            );
        }
    }

    fn wait_for_local_raster(viewport: &Viewport, background: [u8; 3]) -> Vec<u8> {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(10))
            .expect("bounded deadline");
        loop {
            if let Some(pixels) = viewport.paint()
                && red_at(&pixels, 10)
                && pixels[(80 * 160 + 150) * 4..][..3] == background
            {
                return pixels;
            }
            assert!(Instant::now() < deadline, "local PNG did not render");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn red_at(pixels: &[u8], x: usize) -> bool {
        let pixel = &pixels[(10 * 160 + x) * 4..][..3];
        pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50
    }

    #[test]
    fn generated_document_is_bounded() {
        assert!(Document::new(&"x".repeat(MAX_DOCUMENT_BYTES + 1), TestCsp::Production).is_err());
        let expansion = LOCAL_IMAGE.repeat(
            MAX_DOCUMENT_BYTES
                .checked_div(LOCAL_IMAGE.len())
                .expect("nonempty marker"),
        );
        assert!(Document::new(&expansion, TestCsp::Production).is_err());
    }

    #[test]
    fn lifecycle_subprocess() {
        use std::process::Command;
        if std::env::var_os("MAILSTRONAUT_LIFECYCLE_CHILD").is_some() {
            lifecycle_probe();
            return;
        }
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "tests::lifecycle_subprocess", "--nocapture"])
            .env("MAILSTRONAUT_LIFECYCLE_CHILD", "1")
            .spawn()
            .expect("spawn lifecycle probe");
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if let Some(status) = child.try_wait().expect("child status") {
                assert!(status.success(), "lifecycle probe failed: {status}");
                break;
            }
            if Instant::now() >= deadline {
                child.kill().expect("kill child");
                child.wait().expect("reap child");
                panic!("lifecycle probe timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn lifecycle_probe() {
        let renderer = Renderer::synthetic().expect("engine");
        let mut peak_pss: u64 = 0;
        let mut peak_threads: u64 = 0;
        let baseline_threads = sample_field("/proc/self/status", "Threads:");
        let mut retained_context = None;
        let cycles: core::ops::RangeInclusive<u32> = 1..=100;
        for cycle in cycles {
            let viewport = renderer.viewport(160, 100).expect("viewport");
            assert!(renderer.viewport(160, 100).is_err());
            assert!(
                !viewport.policy.frame.get(),
                "new view starts with a stale frame"
            );
            if let Some(context) = retained_context.as_ref() {
                assert!(Rc::ptr_eq(context, &viewport.context), "context changed");
            } else {
                retained_context = Some(Rc::clone(&viewport.context));
            }
            let delegate = Rc::downgrade(&viewport.policy);
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(pixels) = viewport.paint() {
                    assert_eq!(pixels.len(), 160 * 100 * 4);
                    break;
                }
                assert!(Instant::now() < deadline, "frame timeout at cycle {cycle}");
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(viewport);
            assert_eq!(
                retained_context.as_ref().expect("context").size(),
                (1_u32, 1_u32).into()
            );
            let delegate_deadline = Instant::now() + Duration::from_secs(5);
            while delegate.upgrade().is_some() {
                renderer.spin();
                assert!(
                    Instant::now() < delegate_deadline,
                    "delegate retained at cycle {cycle}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            // Drive asynchronous teardown in an empty interval before reopening.
            let settle_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                renderer.spin();
                if sample_field("/proc/self/status", "Threads:") <= baseline_threads + 80 {
                    break;
                }
                assert!(
                    Instant::now() < settle_deadline,
                    "threads did not settle at cycle {cycle}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            if cycle.is_multiple_of(10) {
                let pss = sample_field("/proc/self/smaps_rollup", "Pss:");
                let threads = sample_field("/proc/self/status", "Threads:");
                peak_pss = peak_pss.max(pss);
                peak_threads = peak_threads.max(threads);
                println!("lifecycle cycle={cycle} pss_kib={pss} threads={threads}");
                assert!(
                    pss < 0x0020_0000 && threads < 512,
                    "resource limit exceeded"
                );
            }
        }
        println!("lifecycle sampled peak pss_kib={peak_pss} threads={peak_threads}");
    }

    fn sample_field(path: &str, field: &str) -> u64 {
        std::fs::read_to_string(path)
            .expect("process sample")
            .lines()
            .find_map(|line| line.strip_prefix(field))
            .expect("sample field")
            .split_whitespace()
            .next()
            .expect("sample value")
            .parse()
            .expect("numeric sample")
    }

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
        let hostile = sanitize(&format!(
            r#"<p>Safe visible content</p><script>document.body.style.background='red';fetch('http://{address}/script')</script><p onclick="document.body.style.background='red'">safe</p><form action="http://{address}/form"><button>submit</button></form><iframe src="http://{address}/frame"></iframe>"#
        ));
        assert!(hostile.contains("Safe visible content"));
        assert!(
            !hostile.contains("script")
                && !hostile.contains("onclick")
                && !hostile.contains("form")
                && !hostile.contains("iframe")
        );
        let document = format!(
            r#"<!doctype html><style>@import url(http://{address}/style); body {{ margin: 0; padding: 30px; background: white; color: black }} .marker {{ width: 80px; height: 80px; background: #00ff00 }} .gap {{ height: 400px }} @media (prefers-color-scheme: dark) {{ body {{ background: black; color: white }} }}</style><div class=marker></div><div class=gap></div><link rel="preconnect" href="http://{address}"><img src="http://{address}/pixel"><img src="{file_url}"><img src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%22100%22 height=%22100%22%3E%3Crect width=%22100%22 height=%22100%22 fill=%22red%22/%3E%3C/svg%3E"><script>document.body.style.background='red'; fetch('http://{address}/script')</script><h1>Synthetic viewport</h1>{hostile}"#
        );
        let mut viewport = Renderer::from_document(document)
            .expect("engine")
            .viewport(320, 200)
            .expect("viewport");
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
        clippy::too_many_lines,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
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
        // Measure marker displacement on one engine in CSS and physical pixels.
        for (scale, width, height, x) in [
            (1.0, 320, 200, 40),
            (1.5, 480, 300, 60),
            (2.0, 640, 400, 80),
        ] {
            viewport.set_scale(scale).expect("scale");
            viewport.resize(width, height).expect("resize");
            let edge = |frame: &[u8]| {
                (0..height as usize)
                    .rev()
                    .find(|&y| green(frame, width as usize, x, y))
                    .unwrap_or(0)
            };
            let before = wait_for(viewport, |frame| {
                frame.len() == width as usize * height as usize * 4
                    && edge(frame) > (100.0 * scale) as usize
            });
            let baseline = edge(&before);
            viewport
                .wheel(x as f32, 40.0, 0.0, -0.5, true)
                .expect("line wheel");
            let after = wait_for(viewport, |frame| edge(frame) < baseline);
            let displacement = (baseline - edge(&after)) as f32 / scale;
            assert!(
                (displacement - 30.0).abs() <= 2.0,
                "line displacement: {displacement} at {scale}"
            );
            viewport
                .wheel(x as f32, 40.0, 0.0, 60.0 * scale, false)
                .expect("reset");
            drop(wait_for(viewport, |frame| edge(frame) >= baseline));
            viewport
                .wheel(x as f32, 40.0, 0.0, -30.0, false)
                .expect("pixel wheel");
            let after_pixel = wait_for(viewport, |frame| edge(frame) < baseline);
            assert!((baseline - edge(&after_pixel)).abs_diff(30) <= 2);
            viewport
                .wheel(x as f32, 40.0, 0.0, 60.0, false)
                .expect("reset");
            drop(wait_for(viewport, |frame| edge(frame) >= baseline));
        }
        viewport.set_scale(1.0).expect("restore scale");
        viewport.resize(320, 200).expect("restore size");
        assert!(viewport.wheel(1.0, 1.0, f32::MAX, 0.0, true).is_err());
        assert!(viewport.wheel(1.0, 1.0, f32::MAX, 0.0, false).is_err());
        assert!(viewport.set_scale(2049.0).is_err());
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
