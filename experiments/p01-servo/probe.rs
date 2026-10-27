//! Compile-only Servo 0.7 delegate probe; not an email security boundary.
use std::rc::Rc;
use servo::{NavigationRequest, PermissionRequest, Servo, ServoBuilder, ServoDelegate, WebResourceLoad, WebResourceResponse, WebView, WebViewDelegate};

struct Deny;
fn deny_load(load: WebResourceLoad) {
    let response = WebResourceResponse::new(load.request().url.clone());
    load.intercept(response).cancel();
}
impl ServoDelegate for Deny {
    fn load_web_resource(&self, load: WebResourceLoad) { deny_load(load); }
}
impl WebViewDelegate for Deny {
    fn load_web_resource(&self, _: WebView, load: WebResourceLoad) { deny_load(load); }
    fn request_navigation(&self, _: WebView, request: NavigationRequest) { request.deny(); }
    fn request_permission(&self, _: WebView, request: PermissionRequest) { request.deny(); }
}
pub fn create() -> Servo {
    let servo = ServoBuilder::default().build();
    servo.set_delegate(Rc::new(Deny));
    servo
}
