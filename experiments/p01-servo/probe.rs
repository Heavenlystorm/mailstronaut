//! Compile-only Servo embedding API probe, not an email security boundary.
#![forbid(unsafe_code)]
use servo::{
    DeviceIntPoint, DeviceIntRect, EventLoopWaker, NavigationRequest, PermissionRequest, RgbaImage,
    Servo, ServoBuilder, ServoDelegate, SoftwareRenderingContext, Theme, WebResourceLoad,
    WebResourceResponse, WebView, WebViewBuilder, WebViewDelegate, WindowRenderingContext,
};
use std::{rc::Rc, sync::mpsc::Sender};

#[derive(Clone)]
struct Wake(Sender<()>);
impl EventLoopWaker for Wake {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        let _ = self.0.send(());
    }
}
struct Deny;
fn deny_load(load: WebResourceLoad) {
    let response = WebResourceResponse::new(load.request().url.clone());
    load.intercept(response).cancel();
}
impl ServoDelegate for Deny {
    fn load_web_resource(&self, load: WebResourceLoad) {
        deny_load(load);
    }
}
impl WebViewDelegate for Deny {
    fn load_web_resource(&self, _: WebView, load: WebResourceLoad) {
        deny_load(load);
    }
    fn request_navigation(&self, _: WebView, request: NavigationRequest) {
        request.deny();
    }
    fn request_permission(&self, _: WebView, request: PermissionRequest) {
        request.deny();
    }
    fn notify_new_frame_ready(&self, _: WebView) {}
}
pub fn create(sender: Sender<()>) -> Servo {
    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(Wake(sender)))
        .build();
    servo.set_delegate(Rc::new(Deny));
    servo
}
pub fn software_view(servo: &Servo, width: u32, height: u32) -> Option<WebView> {
    let context = Rc::new(SoftwareRenderingContext::new((width, height).into()).ok()?);
    Some(
        WebViewBuilder::new(servo, context)
            .delegate(Rc::new(Deny))
            .build(),
    )
}
pub fn offscreen_view(
    servo: &Servo,
    parent: &Rc<WindowRenderingContext>,
    width: u32,
    height: u32,
) -> WebView {
    let context = Rc::new(parent.offscreen_context((width, height).into()));
    WebViewBuilder::new(servo, context)
        .delegate(Rc::new(Deny))
        .build()
}
pub fn frame(servo: &Servo, view: &WebView, width: i32, height: i32) -> Option<RgbaImage> {
    servo.spin_event_loop();
    view.notify_theme_change(Theme::Dark);
    view.paint();
    let context = view.rendering_context();
    let image = context.read_to_image(DeviceIntRect::new(
        DeviceIntPoint::new(0, 0),
        DeviceIntPoint::new(width, height),
    ));
    context.present();
    image
}
