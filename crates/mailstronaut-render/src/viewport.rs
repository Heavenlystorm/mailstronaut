//! Retained passive document; only bounded viewport pixels leave its owner thread.

use anyrender::ImageRenderer as _;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::{
    navigation::DummyNavigationProvider,
    shell::{ColorScheme, DummyShellProvider, Viewport},
};

use super::{
    DenyResources, Error, Frame, InlineImage, Link, MAX_HTML_BYTES, MAX_LINKS, MAX_PIXELS,
    MAX_SIDE, resources, sanitize,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const MAX_DOCUMENT_SIDE: f64 = 1_000_000.0;

/// Physical viewport dimensions and a CSS-pixel scroll position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Physical raster width.
    pub width: u32,
    /// Physical raster height.
    pub height: u32,
    /// Physical pixels per logical CSS pixel, from the native window.
    pub scale: f32,
    /// Selected native color scheme, forwarded to Blitz media queries.
    pub dark: bool,
    /// Horizontal CSS scroll offset; clamped to the content extent.
    pub scroll_x: f64,
    /// Vertical CSS scroll offset; clamped to the content extent.
    pub scroll_y: f64,
}

impl View {
    pub(crate) fn validate(self) -> Result<(), Error> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_SIDE
            || self.height > MAX_SIDE
            || u64::from(self.width) * u64::from(self.height) > MAX_PIXELS
            || !self.scale.is_finite()
            || !(0.5..=8.0).contains(&self.scale)
            || !self.scroll_x.is_finite()
            || !self.scroll_y.is_finite()
            || self.scroll_x < 0.0
            || self.scroll_y < 0.0
        {
            return Err(Error::Dimensions);
        }
        Ok(())
    }
}

/// One immutable message and its reusable DOM/font/raster resources.
///
/// Construct, paint and drop on one bounded rendering worker, never the UI thread.
/// It exposes no DOM events, navigation, shell or resource-loading authority.
pub struct Document {
    dom: HtmlDocument,
    resources: Arc<DenyResources>,
    renderer: VelloCpuImageRenderer,
    links: Arc<[Link]>,
    painted_view: Option<View>,
}

impl Document {
    /// Sanitize and parse a message with a fresh, immutable local raster table.
    ///
    /// # Errors
    /// Rejects excessive HTML/DOM and invalid or excessive local resources.
    pub fn new(html: &str, images: &[InlineImage<'_>]) -> Result<Self, Error> {
        if html.len() > MAX_HTML_BYTES {
            return Err(Error::ContentLimit);
        }
        let images = Arc::new(resources::Images::new(images)?);
        let html = sanitize(html, images.clone())?;
        let resources = Arc::new(DenyResources {
            blocked: AtomicUsize::new(0),
            images: images.clone(),
        });
        let config = DocumentConfig {
            base_url: Some("mailstronaut://message/".into()),
            net_provider: Some(resources.clone()),
            navigation_provider: Some(Arc::new(DummyNavigationProvider)),
            shell_provider: Some(Arc::new(DummyShellProvider)),
            ..DocumentConfig::default()
        };
        let mut document = HtmlDocument::from_html(&html, config);
        let mut pending = vec![(document.root_node().id, 0_usize)];
        let mut count = 0;
        let mut links = Vec::new();
        while let Some((id, depth)) = pending.pop() {
            count += 1;
            let node = document.get_node(id).ok_or(Error::ContentLimit)?;
            if depth > 128 || count + pending.len() + node.children.len() > 4096 {
                return Err(Error::ContentLimit);
            }
            pending.extend(node.children.iter().rev().map(|child| (*child, depth + 1)));
            if links.len() < MAX_LINKS
                && node
                    .data
                    .is_element_with_tag_name(&blitz_dom::local_name!("a"))
                && let Some(link) = node
                    .data
                    .attr(blitz_dom::local_name!("href"))
                    .and_then(|raw| Link::parse(raw, &document))
                && !links.contains(&link)
            {
                links.push(link);
            }
        }
        images.install(&mut document);
        Ok(Self {
            dom: document,
            resources,
            renderer: VelloCpuImageRenderer::new(1, 1),
            links: links.into(),
            painted_view: None,
        })
    }

    /// Relayout for native size/theme changes and rasterize only the visible area.
    /// Oversized scroll requests clamp to content instead of allocating more pixels.
    ///
    /// # Errors
    /// Rejects invalid sizes/scales/coordinates or excessive CSS layout extents.
    pub fn paint(&mut self, view: View) -> Result<Frame, Error> {
        self.painted_view = None;
        view.validate()?;
        let viewport = Viewport::new(
            view.width,
            view.height,
            view.scale,
            if view.dark {
                ColorScheme::Dark
            } else {
                ColorScheme::Light
            },
        );
        if self.dom.viewport() != &viewport {
            self.dom.set_viewport(viewport);
            self.dom.resolve(0.0);
        }
        let layout = self.dom.root_element().final_layout();
        let content_width = f64::from(layout.size.width.max(layout.scrollable_overflow_rect.right));
        let content_height = f64::from(
            layout
                .size
                .height
                .max(layout.scrollable_overflow_rect.bottom),
        );
        if [content_width, content_height]
            .into_iter()
            .any(|n| !n.is_finite() || !(0.0..=MAX_DOCUMENT_SIDE).contains(&n))
        {
            return Err(Error::ContentLimit);
        }
        let scale = f64::from(view.scale);
        self.dom.set_viewport_scroll(blitz_dom::Point {
            x: view
                .scroll_x
                .min((content_width - f64::from(view.width) / scale).max(0.0)),
            y: view
                .scroll_y
                .min((content_height - f64::from(view.height) / scale).max(0.0)),
        });
        self.renderer.resize(view.width, view.height);
        let mut pixels = Vec::new();
        self.renderer.render_to_vec(
            |scene| {
                blitz_paint::paint_scene(
                    scene,
                    &mut self.dom,
                    scale,
                    view.width,
                    view.height,
                    0,
                    0,
                );
            },
            &mut pixels,
        );
        self.painted_view = Some(view);
        Ok(Frame {
            width: view.width,
            height: view.height,
            pixels,
            content_width,
            content_height,
            blocked_resources: self.resources.blocked.load(Ordering::Relaxed),
            links: self.links.clone(),
        })
    }

    /// Identify a web link under a CSS viewport point in the last successful frame.
    ///
    /// This reads layout only. It sends no DOM event or navigation request. The
    /// exact painted view must match; unknown, hidden and unsupported targets fail.
    #[must_use]
    pub fn link_at(&self, view: View, x: f32, y: f32) -> Option<Link> {
        if self.painted_view != Some(view)
            || !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || f64::from(x) >= f64::from(view.width) / f64::from(view.scale)
            || f64::from(y) >= f64::from(view.height) / f64::from(view.scale)
        {
            return None;
        }
        let scroll = self.dom.viewport_scroll();
        let x = f64::from(x) + scroll.x;
        let y = f64::from(y) + scroll.y;
        // Document extents and the viewport have already been bounded by paint.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Blitz's public hit API uses f32; paint bounds finite document coordinates to one million CSS pixels. Fractional layout is narrowed to that engine's precision."
        )]
        let hit = self.dom.hit(x as f32, y as f32)?;
        let mut id = Some(hit.node_id);
        while let Some(current) = id {
            let node = self.dom.get_node(current)?;
            if node
                .data
                .is_element_with_tag_name(&blitz_dom::local_name!("a"))
            {
                return Link::parse(node.data.attr(blitz_dom::local_name!("href"))?, &self.dom);
            }
            id = node.parent;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            width: 100,
            height: 80,
            scale: 1.0,
            dark: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }

    fn center(frame: &Frame) -> &[u8] {
        let index = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
        &frame.pixels[index..index + 4]
    }

    #[test]
    fn paints_a_bounded_viewport_of_a_long_retained_document_at_native_scale() {
        let html = include_str!("fixtures/viewport.html");
        let mut document = Document::new(html, &[]).expect("retained document");
        let first = document.paint(view()).expect("top viewport");
        assert_eq!(center(&first), &[255, 0, 0, 255]);
        assert!(first.content_height >= 600.0);
        assert_eq!(
            first.pixels.len(),
            100 * 80 * 4,
            "never allocate full document height"
        );
        let second = document
            .paint(View {
                scroll_y: 300.0,
                ..view()
            })
            .expect("scroll");
        assert_eq!(center(&second), &[0, 0, 255, 255]);
        let scaled = document
            .paint(View {
                width: 200,
                height: 160,
                scale: 2.0,
                scroll_y: 300.0,
                ..view()
            })
            .expect("HiDPI");
        assert_eq!(
            center(&scaled),
            &[0, 0, 255, 255],
            "CSS width stays 100 at scale two"
        );
        let wide = document
            .paint(View {
                width: 400,
                height: 160,
                scale: 2.0,
                scroll_y: 300.0,
                ..view()
            })
            .expect("resize");
        assert_eq!(
            center(&wide),
            &[0, 255, 0, 255],
            "CSS media queries use logical width"
        );
        let dark = document
            .paint(View {
                dark: true,
                scroll_y: f64::MAX,
                ..view()
            })
            .expect("clamped end");
        assert_eq!(center(&dark), &[255, 255, 0, 255]);
        assert_eq!(dark.pixels.len(), first.pixels.len());
    }

    #[test]
    fn invalid_geometry_never_poison_or_expand_a_retained_document() {
        let mut document = Document::new("<div>Safe text</div>", &[]).expect("document");
        for invalid in [
            View {
                scale: f32::NAN,
                ..view()
            },
            View {
                scale: 0.0,
                ..view()
            },
            View {
                scroll_y: f64::INFINITY,
                ..view()
            },
            View {
                scroll_x: -1.0,
                ..view()
            },
            View {
                width: MAX_SIDE + 1,
                ..view()
            },
        ] {
            assert!(matches!(document.paint(invalid), Err(Error::Dimensions)));
        }
        assert!(document.paint(view()).is_ok());
        let mut oversized =
            Document::new("<div style='height:2000000px'>Long</div>", &[]).expect("bounded DOM");
        assert!(matches!(oversized.paint(view()), Err(Error::ContentLimit)));
    }

    #[test]
    fn links_are_web_only_canonical_bounded_and_redacted() {
        let mut html = String::from("<base href='https://unexpected.test/'>");
        for raw in [
            "https://EXAMPLE.test/report?PRIVATE-TOKEN",
            "https://example.test/report?PRIVATE-TOKEN",
            "https://école.test/inscription",
            "https://user:PRIVATE-TOKEN@example.test/",
            "https://user@example.test/",
            "//unexpected.test/relative",
            "/relative",
            "#fragment",
            "mailto:reader@example.test",
            "file:///private",
            "data:text/html,private",
            "javascript:alert(1)",
            "https://example.test/a&#10;b",
            "https://example.test/a b",
            "https://example.test/a\\b",
            "https://example.test/\u{202e}hidden",
        ] {
            use std::fmt::Write as _;
            write!(html, "<a href='{raw}'>Link</a>").expect("synthetic markup");
        }
        let mut document = Document::new(&html, &[]).expect("links document");
        let frame = document.paint(view()).expect("passive pixels");
        assert_eq!(frame.links.len(), 3);
        assert_eq!(
            frame.links[0].as_str(),
            "https://example.test/report?PRIVATE-TOKEN"
        );
        assert_eq!(
            frame.links[1].as_str(),
            "https://xn--cole-9oa.test/inscription"
        );
        assert_eq!(frame.links[2].as_str(), "https://example.test/a%20b");
        assert!(!format!("{:?}", frame.links).contains("PRIVATE-TOKEN"));
        let mut many = format!(
            "<a href='https://example.test/{}'>Long</a>",
            "x".repeat(super::super::MAX_LINK_BYTES)
        );
        for index in 0..MAX_LINKS + 10 {
            use std::fmt::Write as _;
            write!(many, "<a href='https://example.test/{index}'>Link</a>")
                .expect("bounded markup");
        }
        let mut document = Document::new(&many, &[]).expect("bounded input");
        let links = document.paint(view()).expect("bounded metadata").links;
        assert_eq!(links.len(), MAX_LINKS);
        assert_eq!(links[0].as_str(), "https://example.test/0");
        assert_eq!(links[MAX_LINKS - 1].as_str(), "https://example.test/127");
    }

    #[test]
    fn link_hits_follow_the_actual_painted_view_scroll_scale_and_source() {
        let html = "<style>body{margin:0;}a{display:block;width:100px;height:40px}</style><a href='https://example.test/top'><span>Top</span></a><div style='height:200px'></div><a href='https://example.test/end'><b>End</b></a>";
        let mut document = Document::new(html, &[]).expect("document");
        assert!(
            document.link_at(view(), 10.0, 10.0).is_none(),
            "no link before painting"
        );
        let frame = document.paint(view()).expect("top frame");
        assert_eq!(
            document.link_at(view(), 10.0, 10.0),
            Some(frame.links[0].clone())
        );
        assert!(document.link_at(view(), 10.0, 70.0).is_none());
        for (x, y) in [
            (f32::NAN, 10.0),
            (10.0, f32::INFINITY),
            (-1.0, 10.0),
            (100.0, 10.0),
            (10.0, 80.0),
        ] {
            assert!(document.link_at(view(), x, y).is_none());
        }
        let end = View {
            scroll_y: f64::MAX,
            ..view()
        };
        assert!(
            document.link_at(end, 10.0, 50.0).is_none(),
            "unpainted scroll cannot hit an old frame"
        );
        document.paint(end).expect("clamped end");
        assert_eq!(
            document.link_at(end, 10.0, 50.0),
            Some(frame.links[1].clone())
        );
        assert!(document.link_at(view(), 10.0, 10.0).is_none());
        let scaled = View {
            width: 200,
            height: 160,
            scale: 2.0,
            ..end
        };
        document.paint(scaled).expect("2x native scale");
        assert_eq!(
            document.link_at(scaled, 10.0, 50.0),
            Some(frame.links[1].clone())
        );
        assert!(document.link_at(end, 10.0, 50.0).is_none());
        let mut other = Document::new("<p>Another message, no links</p>", &[]).expect("new source");
        assert!(other.paint(scaled).expect("new frame").links.is_empty());
        assert!(other.link_at(scaled, 10.0, 50.0).is_none());
    }

    #[test]
    fn the_picker_budget_does_not_disable_a_visible_link_later_in_the_document() {
        use std::fmt::Write as _;
        let mut html = "<style>body{margin:0;}</style>".to_owned();
        for index in 0..MAX_LINKS {
            write!(
                html,
                "<a style='display:none' href='https://example.test/{index}'>Hidden</a>"
            )
            .expect("synthetic markup");
        }
        html.push_str("<a style='display:block;width:100px;height:40px' href='https://example.test/unsubscribe'>Unsubscribe</a>");
        let mut document = Document::new(&html, &[]).expect("bounded document");
        let frame = document.paint(view()).expect("visible link");
        assert_eq!(frame.links.len(), MAX_LINKS);
        assert!(
            frame
                .links
                .iter()
                .all(|link| !link.as_str().contains("unsubscribe"))
        );
        assert_eq!(
            document
                .link_at(view(), 10.0, 10.0)
                .expect("current DOM link")
                .as_str(),
            "https://example.test/unsubscribe"
        );
    }
}
