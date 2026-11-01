//! One retained passive document on its own thread; latest request, bounded replies.

use std::sync::Arc;

use mailstronaut_core::mime::{InlineRaster, Projection};
use mailstronaut_render::{Document, Frame, InlineImage, Link, View};
use tokio::sync::{mpsc, watch};

const DEMO_HTML: &str = "<style>body{font:17px sans-serif;margin:32px;color:#243044;background:#fff}h1{font-size:30px}td{padding:18px}@media(prefers-color-scheme:dark){body{background:#20242b;color:#edf1fa}}</style><h1>A little more space for your mail.</h1><p>This synthetic email is laid out by Blitz and painted by Vello CPU.</p><table style='width:100%;border-collapse:collapse'><tr><td style='background:#dce9ff;color:#173b73'>Native Wayland</td><td style='background:#e1f2e8;color:#164c34'>Rust throughout</td></tr></table><p>Tables, inline CSS and Unicode: Bonjour · Ciao · 世界.</p><p><img src='cid:demo-badge' width='96' height='32' alt='Local raster image'> Message-local PNG · no fetching.</p>";

pub enum Content {
    Demo,
    Mail(Arc<Projection>),
}

impl Content {
    fn prepare(&self) -> Result<(Document, bool), mailstronaut_render::Error> {
        let (html, images, mut omitted) = match self {
            Self::Demo => (
                DEMO_HTML,
                vec![InlineRaster {
                    content_id: "demo-badge".into(),
                    media_type: "image/png".into(),
                    bytes: include_bytes!("../../../assets/inline-fixture.png").to_vec(),
                }],
                false,
            ),
            Self::Mail(message) => {
                let (images, omitted) = message
                    .inline_rasters()
                    .map_or_else(|_| (Vec::new(), true), |images| (images, false));
                (
                    message
                        .html
                        .as_deref()
                        .ok_or(mailstronaut_render::Error::ContentLimit)?,
                    images,
                    omitted,
                )
            }
        };
        let borrowed: Vec<_> = images
            .iter()
            .map(|image| InlineImage {
                content_id: &image.content_id,
                media_type: &image.media_type,
                bytes: &image.bytes,
            })
            .collect();
        let document = match Document::new(html, &borrowed) {
            Ok(document) => document,
            Err(
                mailstronaut_render::Error::InvalidResource
                | mailstronaut_render::Error::ResourceLimit,
            ) => {
                omitted = true;
                Document::new(html, &[])?
            }
            Err(error) => return Err(error),
        };
        Ok((document, omitted))
    }
}

#[derive(Clone)]
pub struct Request {
    pub serial: u64,
    pub epoch: u64,
    pub content: Arc<Content>,
    pub view: View,
    pub hit: Option<(f32, f32)>,
}

pub enum Reply {
    Link {
        serial: u64,
        epoch: u64,
        view: View,
        link: Option<Link>,
    },
    Painted {
        serial: u64,
        epoch: u64,
        view: View,
        result: Result<(Frame, bool), String>,
    },
    Retired,
}

pub fn start(
    runtime: &tokio::runtime::Handle,
) -> std::io::Result<(watch::Sender<Option<Request>>, mpsc::Receiver<Reply>)> {
    let (sender, requests) = watch::channel(None);
    let (replies, receiver) = mpsc::channel(1);
    let runtime = runtime.clone();
    std::thread::Builder::new()
        .name("mailstronaut-html".into())
        .spawn(move || run(&runtime, requests, &replies))?;
    Ok((sender, receiver))
}

struct Active {
    content: Arc<Content>,
    prepared: Result<(Document, bool), mailstronaut_render::Error>,
}

fn run(
    runtime: &tokio::runtime::Handle,
    mut requests: watch::Receiver<Option<Request>>,
    replies: &mpsc::Sender<Reply>,
) {
    let mut active: Option<Active> = None;
    while runtime.block_on(requests.changed()).is_ok() {
        let Some(request) = requests.borrow_and_update().clone() else {
            active = None;
            if replies.blocking_send(Reply::Retired).is_err() {
                break;
            }
            continue;
        };
        if active
            .as_ref()
            .is_none_or(|active| !Arc::ptr_eq(&active.content, &request.content))
        {
            active = Some(Active {
                content: request.content.clone(),
                prepared: request.content.prepare(),
            });
        }
        if let Some((x, y)) = request.hit {
            let link = active
                .as_ref()
                .and_then(|active| active.prepared.as_ref().ok()?.0.link_at(request.view, x, y));
            if matches!(requests.has_changed(), Ok(false))
                && replies
                    .blocking_send(Reply::Link {
                        serial: request.serial,
                        epoch: request.epoch,
                        view: request.view,
                        link,
                    })
                    .is_err()
            {
                break;
            }
            continue;
        }
        let result = active.as_mut().map_or_else(
            || Err("HTML document is unavailable.".to_owned()),
            |active| match &mut active.prepared {
                Ok((document, omitted)) => document
                    .paint(request.view)
                    .map(|mut frame| {
                        // GPUI uses BGRA. Convert on the owner thread, never in a UI callback.
                        for pixel in frame.pixels.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }
                        (frame, *omitted)
                    })
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            },
        );
        if !matches!(requests.has_changed(), Ok(false)) {
            continue;
        }
        if replies
            .blocking_send(Reply::Painted {
                serial: request.serial,
                epoch: request.epoch,
                view: request.view,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder as _;
    use std::time::Duration;

    fn source(image: bool) -> Arc<Content> {
        let mut raw = b"MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<style>body{margin:0;background:blue}</style><img src='cid:shared' width='8' height='8'>\r\n".to_vec();
        if image {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    &[255, 0, 0, 255].repeat(64),
                    8,
                    8,
                    image::ExtendedColorType::Rgba8,
                )
                .expect("synthetic PNG");
            raw.extend_from_slice(b"--x\r\nContent-Type: image/png\r\nContent-ID: <shared>\r\nContent-Transfer-Encoding: binary\r\n\r\n");
            raw.extend_from_slice(&png);
            raw.extend_from_slice(b"\r\n");
        }
        raw.extend_from_slice(b"--x--\r\n");
        Arc::new(Content::Mail(Arc::new(
            mailstronaut_core::mime::project(&raw).expect("synthetic MIME"),
        )))
    }

    fn request(serial: u64, content: Arc<Content>) -> Request {
        Request {
            serial,
            epoch: serial,
            content,
            view: View {
                width: 64,
                height: 64,
                scale: 1.0,
                dark: false,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            hit: None,
        }
    }

    async fn reply(receiver: &mut mpsc::Receiver<Reply>) -> Reply {
        tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("bounded render")
            .expect("live worker")
    }

    #[tokio::test]
    async fn switching_mime_sources_retires_images_and_hidden_source_data() {
        let (sender, mut receiver) = start(&tokio::runtime::Handle::current()).expect("worker");
        let a = source(true);
        sender.send_replace(Some(request(1, a)));
        let Reply::Painted {
            serial: 1,
            result: Ok((frame, false)),
            ..
        } = reply(&mut receiver).await
        else {
            panic!("rendered A");
        };
        assert_eq!(
            &frame.pixels[..4],
            &[0, 0, 255, 255],
            "transfer-decoded local red raster, in BGRA"
        );
        let b = source(false);
        let weak = Arc::downgrade(&b);
        sender.send_replace(Some(request(2, b)));
        let Reply::Painted {
            serial: 2,
            result: Ok((frame, false)),
            ..
        } = reply(&mut receiver).await
        else {
            panic!("rendered B");
        };
        assert_eq!(
            &frame.pixels[..4],
            &[255, 0, 0, 255],
            "B cannot reuse A's same-named image"
        );
        sender.send_replace(None);
        assert!(matches!(reply(&mut receiver).await, Reply::Retired));
        assert!(
            weak.upgrade().is_none(),
            "hidden worker releases the immutable original source"
        );
        drop(sender);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .expect("worker shutdown")
                .is_none()
        );
    }

    #[tokio::test]
    async fn queued_geometry_coalesces_before_rendering_and_output_remains_bounded() {
        let (sender, input) = watch::channel(None);
        let (output, mut receiver) = mpsc::channel(1);
        let content = source(false);
        for serial in 1..=100 {
            let mut work = request(serial, content.clone());
            work.view.width = 100;
            work.view.scale = 2.0;
            sender.send_replace(Some(work));
        }
        let runtime = tokio::runtime::Handle::current();
        let thread = std::thread::spawn(move || run(&runtime, input, &output));
        let Reply::Painted {
            serial: 100,
            view,
            result: Ok((frame, false)),
            ..
        } = reply(&mut receiver).await
        else {
            panic!("latest request only");
        };
        assert_eq!(view.scale, 2.0);
        assert_eq!(frame.width, 100);
        assert_eq!(frame.pixels.len(), 100 * 64 * 4);
        assert!(
            receiver.try_recv().is_err(),
            "superseded requests create no backlog"
        );
        drop(sender);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .expect("closed worker")
                .is_none()
        );
        thread.join().expect("owner thread terminated");
    }

    #[tokio::test]
    async fn link_requests_cannot_use_an_unpainted_source_or_retired_geometry() {
        fn linked(target: &str) -> Arc<Content> {
            let raw = format!(
                "Content-Type: text/html\r\n\r\n<style>body{{margin:0;}}a{{display:block;width:100px;height:40px}}</style><a href='https://{target}.example.test/'>Open</a>"
            );
            Arc::new(Content::Mail(Arc::new(
                mailstronaut_core::mime::project(raw.as_bytes()).expect("synthetic source"),
            )))
        }
        let (sender, mut receiver) = start(&tokio::runtime::Handle::current()).expect("worker");
        let a = linked("a");
        sender.send_replace(Some(request(1, a.clone())));
        let Reply::Painted {
            result: Ok((frame, _)),
            ..
        } = reply(&mut receiver).await
        else {
            panic!("A frame");
        };
        let mut hit = request(1, a);
        hit.hit = Some((10.0, 10.0));
        sender.send_replace(Some(hit));
        let Reply::Link {
            link: Some(link), ..
        } = reply(&mut receiver).await
        else {
            panic!("explicit A link hit");
        };
        assert_eq!(link, frame.links[0]);
        let b = linked("b");
        let mut hit = request(2, b.clone());
        hit.hit = Some((10.0, 10.0));
        sender.send_replace(Some(hit));
        assert!(
            matches!(reply(&mut receiver).await, Reply::Link { link: None, .. }),
            "new source has never painted"
        );
        sender.send_replace(Some(request(3, b.clone())));
        let Reply::Painted {
            result: Ok((frame, _)),
            ..
        } = reply(&mut receiver).await
        else {
            panic!("B frame");
        };
        assert_eq!(frame.links[0].as_str(), "https://b.example.test/");
        let mut hit = request(3, b.clone());
        hit.view.width = 100;
        hit.hit = Some((10.0, 10.0));
        sender.send_replace(Some(hit));
        assert!(
            matches!(reply(&mut receiver).await, Reply::Link { link: None, .. }),
            "unpainted size cannot hit a stale layout"
        );
        sender.send_replace(None);
        assert!(matches!(reply(&mut receiver).await, Reply::Retired));
        let mut hit = request(4, b);
        hit.hit = Some((10.0, 10.0));
        sender.send_replace(Some(hit));
        assert!(
            matches!(reply(&mut receiver).await, Reply::Link { link: None, .. }),
            "hidden document was retired"
        );
    }
}
