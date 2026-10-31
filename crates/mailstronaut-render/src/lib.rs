//! Bounded, passive Blitz HTML layout and CPU rasterization, without window or I/O authority.

#[cfg(test)]
mod privacy;
mod resources;
mod viewport;
pub use resources::{InlineImage, MAX_IMAGE_BYTES, MAX_INLINE_IMAGES, MAX_RESOURCE_BYTES};
pub use viewport::{Document, View};

use core::{error::Error as StdError, fmt};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use blitz_traits::net::{NetHandler, NetProvider, Request};

/// Maximum decoded HTML accepted by the reader.
pub const MAX_HTML_BYTES: usize = 256 * 1024;
/// Maximum width or height of a viewport raster.
pub const MAX_SIDE: u32 = 4096;
/// Maximum raster pixels, independently of its aspect ratio.
pub const MAX_PIXELS: u64 = 8_000_000;

/// Redacted boundary errors; mail bodies and URLs never enter diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// HTML or sanitized DOM exceeds the bounded reader budget.
    ContentLimit,
    /// Invalid, excessive or overflowing viewport dimensions.
    Dimensions,
    /// Supplied images exceed encoded, decoded, count or dimension budgets.
    ResourceLimit,
    /// Invalid/ambiguous CID, unsupported image, invalid encoding or MIME mismatch.
    InvalidResource,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ContentLimit => "email exceeds HTML reader limits",
            Self::Dimensions => "invalid HTML viewport dimensions",
            Self::ResourceLimit => "email exceeds inline image limits",
            Self::InvalidResource => "invalid or unsupported inline image",
        })
    }
}
impl StdError for Error {}

/// An owned RGBA raster; independent of the DOM and the GUI toolkit.
pub struct Frame {
    /// Physical pixel width.
    pub width: u32,
    /// Physical pixel height.
    pub height: u32,
    /// RGBA bytes in row-major order.
    pub pixels: Vec<u8>,
    /// CSS content width, bounded independently from the viewport raster.
    pub content_width: f64,
    /// CSS content height; the host may use this for its native scrollbar.
    pub content_height: f64,
    /// Denied resource attempts, without retaining untrusted URLs.
    pub blocked_resources: usize,
}
impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("blocked_resources", &self.blocked_resources)
            .finish_non_exhaustive()
    }
}

struct DenyResources {
    blocked: AtomicUsize,
    images: Arc<resources::Images>,
}
impl NetProvider for DenyResources {
    fn fetch(&self, _document: usize, request: Request, _handler: Box<dyn NetHandler>) {
        if !self.images.contains_url(request.url.as_str()) {
            self.blocked.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn is_noop(&self) -> bool {
        true
    }
}

fn sanitize(input: &str, images: Arc<resources::Images>) -> Result<String, Error> {
    if input.len() > MAX_HTML_BYTES {
        return Err(Error::ContentLimit);
    }
    let cleaned = ammonia::Builder::default()
        .add_tags(&[
            "style", "table", "thead", "tbody", "tfoot", "tr", "td", "th", "col", "colgroup",
        ])
        .rm_clean_content_tags(&["style"])
        .add_clean_content_tags(&[
            "script", "iframe", "object", "embed", "form", "button", "textarea", "select",
            "template", "video", "audio", "canvas", "svg", "math",
        ])
        .add_generic_attributes(&["style", "class", "align", "width", "height", "bgcolor"])
        .add_tag_attributes("td", &["colspan", "rowspan"])
        .add_tag_attributes("th", &["colspan", "rowspan"])
        .add_url_schemes(&["cid", "mailstronaut-resource"])
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                images.resolve(value).map(|url| url.to_owned().into())
            } else if element == "img" && attribute == "srcset" {
                None
            } else {
                Some(value.into())
            }
        })
        .clean(input);
    let output = cleaned.to_string();
    if output.len() > MAX_HTML_BYTES * 2 {
        return Err(Error::ContentLimit);
    }
    Ok(output)
}

/// Rasterize passive HTML using Blitz and the existing Vello CPU backend.
///
/// Run on a blocking rendering task; this function owns all document/font objects.
/// External resources, navigation and native shell capabilities are not supplied.
///
/// # Errors
/// Rejects oversized HTML and invalid/oversized viewport allocations.
pub fn render(html: &str, width: u32, height: u32, dark: bool) -> Result<Frame, Error> {
    render_with_images(html, &[], width, height, dark)
}

/// Rasterize passive HTML with a fresh immutable table of message-local images.
///
/// Only exact known `cid:` image sources are rewritten to this document's private
/// namespace. Other image sources are removed; all CSS/resource fetching remains
/// denied. PNG/JPEG are decoded with the image library before supplying RGBA to
/// Blitz. A table is never shared between calls or retained after rendering.
///
/// # Errors
/// Rejects invalid/ambiguous resources, resource budgets, HTML and frame limits.
pub fn render_with_images(
    html: &str,
    images: &[InlineImage<'_>],
    width: u32,
    height: u32,
    dark: bool,
) -> Result<Frame, Error> {
    let view = View {
        width,
        height,
        scale: 1.0,
        dark,
        scroll_x: 0.0,
        scroll_y: 0.0,
    };
    view.validate()?;
    Document::new(html, images)?.paint(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitization_removes_active_content_and_retains_table_styles() {
        let html=sanitize("<script>PRIVATE_SCRIPT</script><iframe src='file:///private'></iframe><form><input value='PRIVATE_INPUT'></form><table style='color: red'><tr><td colspan='2' onclick='PRIVATE_EVENT'>Safe body</td></tr></table>", Arc::default()).expect("bounded sanitizer");
        assert!(!html.contains("PRIVATE_"));
        assert!(html.contains("<table style=\"color: red\""));
        assert!(html.contains("colspan=\"2\""));
        assert!(html.contains("Safe body"));
    }
    #[test]
    fn rejects_invalid_sizes_and_oversized_input_before_rendering() {
        for (width, height) in [(0, 1), (1, 0), (4097, 1), (4096, 4096)] {
            assert!(matches!(
                render("", width, height, false),
                Err(Error::Dimensions)
            ));
        }
        assert!(matches!(
            render(&"a".repeat(MAX_HTML_BYTES + 1), 10, 10, false),
            Err(Error::ContentLimit)
        ));
    }
    #[test]
    fn paints_real_table_cells_and_denies_external_resources() {
        let html = "<style>@import url('http://127.0.0.1:9/style'); @font-face {font-family: private;src:url(file:///private-font)}td {width:80px;height:60px}body{margin:0}</style><table cellspacing='0' cellpadding='0'><tr><td style='background:#ff0000'>Hello</td><td style='background:#00ff00'>世界</td></tr></table><img src='http://127.0.0.1:9/tracker'><img src='file:///private-image'>";
        let frame = render(html, 240, 180, false).expect("Blitz raster");
        assert_eq!(frame.pixels.len(), 240 * 180 * 4);
        assert!(
            frame
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > 240 && p[1] < 10 && p[2] < 10 && p[3] > 240)
        );
        assert!(
            frame
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[1] > 240 && p[0] < 10 && p[2] < 10 && p[3] > 240)
        );
        assert!(
            frame.blocked_resources >= 1,
            "CSS request reaches the denied provider; image URLs are stripped"
        );
    }
    #[test]
    fn rejects_excessive_dom_nodes_and_nesting() {
        assert!(matches!(
            render(&"<br>".repeat(4096), 100, 100, false),
            Err(Error::ContentLimit)
        ));
        let deep = format!("{}text{}", "<div>".repeat(130), "</div>".repeat(130));
        assert!(matches!(
            render(&deep, 100, 100, false),
            Err(Error::ContentLimit)
        ));
    }

    #[test]
    fn viewport_scheme_drives_native_css_media_queries() {
        let html = "<style>body{margin:0;}div{width:80px;height:80px;background:#ff0000;}@media(prefers-color-scheme:dark){div{background:#0000ff}}</style><div></div>";
        let light = render(html, 100, 100, false).expect("light layout");
        let dark = render(html, 100, 100, true).expect("dark layout");
        let offset = (40 * 100 + 40) * 4;
        assert_eq!(&light.pixels[offset..offset + 4], &[255, 0, 0, 255]);
        assert_eq!(&dark.pixels[offset..offset + 4], &[0, 0, 255, 255]);
    }

    #[test]
    fn remote_image_and_css_attempts_never_connect_to_listener() {
        use std::{
            io::ErrorKind,
            net::{TcpListener, TcpStream},
        };
        let listener = TcpListener::bind("127.0.0.1:0").expect("synthetic listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking observer");
        let address = listener.local_addr().expect("listener address");
        // Positive control: the independent listener actually observes connections.
        let control = TcpStream::connect(address).expect("observer positive control");
        let (_accepted, _) = listener.accept().expect("positive control observed");
        drop(control);
        let html = format!(
            "<style>@import url('http://{address}/style');div{{background-image:url('http://{address}/css');width:20px;height:20px}}</style><div></div><img src='http://{address}/image'>"
        );
        let frame = render(&html, 100, 100, false).expect("passive raster");
        assert!(
            frame.blocked_resources >= 1,
            "CSS fixture reaches denied provider; image URL is stripped"
        );
        assert_eq!(
            listener.accept().expect_err("no HTML connection").kind(),
            ErrorKind::WouldBlock
        );
    }
}
