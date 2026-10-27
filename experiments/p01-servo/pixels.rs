//! Standalone synthetic pixel experiment; not an email renderer or security boundary.
#![forbid(unsafe_code)]
use servo::{
    DeviceIntPoint, DeviceIntRect, EventLoopWaker, RenderingContext, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
};
use std::{
    cell::Cell,
    io::Write,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Wake(mpsc::SyncSender<()>);
impl EventLoopWaker for Wake {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        let _ = self.0.try_send(());
    }
}
#[derive(Default)]
struct Frame(Cell<bool>);
impl WebViewDelegate for Frame {
    fn notify_new_frame_ready(&self, _: WebView) {
        self.0.set(true);
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let context = Rc::new(
        SoftwareRenderingContext::new((320, 200).into())
            .map_err(|error| format!("software context: {error:?}"))?,
    );
    context
        .make_current()
        .map_err(|error| format!("make current: {error:?}"))?;
    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(Wake(sender)))
        .build();
    let frame = Rc::new(Frame::default());
    let url = url::Url::parse(
        "data:text/html,%3Cbody%20style='background:%23fff;color:%23000'%3E%3Ch1%3ESynthetic%20Servo%20probe%3C/h1%3E%3Cp%3ENo%20mail%3C/p%3E%3C/body%3E",
    )?;
    let view = WebViewBuilder::new(&servo, context.clone())
        .delegate(frame.clone())
        .url(url)
        .build();
    view.show();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        servo.spin_event_loop();
        if frame.0.replace(false) {
            view.paint();
            if let Some(image) = context.read_to_image(DeviceIntRect::new(
                DeviceIntPoint::new(0, 0),
                DeviceIntPoint::new(320, 200),
            )) {
                let first = image.pixels().next().ok_or("empty image")?;
                if image.pixels().any(|pixel| pixel != first) {
                    let mut out = std::fs::File::create("servo-synthetic.ppm")?;
                    write!(out, "P6\n320 200\n255\n")?;
                    for pixel in image.pixels() {
                        out.write_all(&pixel.0[..3])?;
                    }
                    println!("wrote servo-synthetic.ppm");
                    return Ok(());
                }
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for nonuniform frame".into());
        }
        let _ = receiver.recv_timeout(remaining);
    }
}
