//! Immutable, document-local raster images. No URL is interpreted as a path.

use std::{collections::BTreeMap, io::Cursor, sync::Arc};

use blitz_dom::{
    BaseDocument,
    net::{Resource, ResourceLoadResponse},
    util::ImageType,
};
use image::{ImageFormat, ImageReader, Limits};

use crate::{Error, MAX_SIDE};

/// Maximum number of supplied inline images per message.
pub const MAX_INLINE_IMAGES: usize = 64;
/// Maximum encoded bytes in one PNG or JPEG.
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// Aggregate encoded bytes and retained RGBA pixels in a message's resource table.
/// Decoder scratch space and renderer/font memory are additional, bounded separately.
pub const MAX_RESOURCE_BYTES: usize = 16 * 1024 * 1024;

/// A transfer-decoded, message-local MIME image, supplied by the host.
///
/// IDs are opaque and case-sensitive. Only conservative ASCII IDs up to 255 bytes
/// are accepted; percent encoding, path separators and URL delimiters are rejected.
/// Never supply an attachment filename or a filesystem path in place of its CID.
pub struct InlineImage<'a> {
    /// The MIME Content-ID without angle brackets or a `cid:` prefix.
    pub content_id: &'a str,
    /// Declared MIME type; must match the PNG/JPEG signature.
    pub media_type: &'a str,
    /// Transfer-decoded encoded image bytes, not a path or remotely fetched data.
    pub bytes: &'a [u8],
}

struct Raster {
    url: String,
    width: u32,
    height: u32,
    pixels: Arc<Vec<u8>>,
}

#[derive(Default)]
pub struct Images(BTreeMap<String, Raster>);

impl Images {
    pub(crate) fn new(images: &[InlineImage<'_>]) -> Result<Self, Error> {
        if images.len() > MAX_INLINE_IMAGES {
            return Err(Error::ResourceLimit);
        }
        let generation = uuid::Uuid::new_v4();
        let mut output = Self::default();
        let mut total = 0_usize;
        for (index, image) in images.iter().enumerate() {
            if !valid_id(image.content_id) || output.0.contains_key(image.content_id) {
                return Err(Error::InvalidResource);
            }
            if image.bytes.len() > MAX_IMAGE_BYTES {
                return Err(Error::ResourceLimit);
            }
            let format = image::guess_format(image.bytes).map_err(|_| Error::InvalidResource)?;
            if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg)
                || !image.media_type.eq_ignore_ascii_case(match format {
                    ImageFormat::Png => "image/png",
                    _ => "image/jpeg",
                })
            {
                return Err(Error::InvalidResource);
            }
            // Inspect dimensions before decoding or allocating the output pixels.
            let mut header = ImageReader::with_format(Cursor::new(image.bytes), format);
            header.limits(image_limits());
            let (width, height) = header
                .into_dimensions()
                .map_err(|error| image_error(&error))?;
            if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
                return Err(Error::ResourceLimit);
            }
            let decoded = usize::try_from(u64::from(width) * u64::from(height) * 4)
                .map_err(|_| Error::ResourceLimit)?;
            total = total
                .checked_add(image.bytes.len())
                .and_then(|n| n.checked_add(decoded))
                .filter(|n| *n <= MAX_RESOURCE_BYTES)
                .ok_or(Error::ResourceLimit)?;
            let mut reader = ImageReader::with_format(Cursor::new(image.bytes), format);
            reader.limits(image_limits());
            let pixels = reader
                .decode()
                .map_err(|error| image_error(&error))?
                .into_rgba8();
            if pixels.dimensions() != (width, height) || pixels.len() != decoded {
                return Err(Error::InvalidResource);
            }
            output.0.insert(
                image.content_id.to_owned(),
                Raster {
                    url: format!("mailstronaut-resource://message/{generation}/{index}"),
                    width,
                    height,
                    pixels: Arc::new(pixels.into_raw()),
                },
            );
        }
        Ok(output)
    }

    pub(crate) fn resolve(&self, source: &str) -> Option<&str> {
        let (scheme, id) = source.split_once(':')?;
        if !scheme.eq_ignore_ascii_case("cid") || !valid_id(id) {
            return None;
        }
        self.0.get(id).map(|image| image.url.as_str())
    }

    pub(crate) fn contains_url(&self, url: &str) -> bool {
        self.0.values().any(|image| image.url == url)
    }

    pub(crate) fn install(&self, document: &mut BaseDocument) {
        for image in self.0.values() {
            // Use Blitz's public decoded-raster cache API. Never deliver encoded
            // bytes to its unrestricted image decoder or to a network handler.
            // Image responses use their exact URL; no request/node ID is needed.
            document.load_resource(ResourceLoadResponse {
                request_id: 0,
                node_id: None,
                resolved_url: Some(image.url.clone()),
                result: Ok(Resource::Image(
                    ImageType::Image,
                    image.width,
                    image.height,
                    image.pixels.clone(),
                )),
            });
        }
    }
}

fn image_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    // image's allocation limit is best-effort, unlike its strict dimensions.
    // Apply it to header inspection too, not only the later pixel decode.
    limits.max_alloc = Some(MAX_RESOURCE_BYTES as u64);
    limits
}

const fn image_error(error: &image::ImageError) -> Error {
    if matches!(error, image::ImageError::Limits(_)) {
        Error::ResourceLimit
    } else {
        Error::InvalidResource
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
        && id != "."
        && id != ".."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Frame, render_with_images};

    fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let pixels = image::RgbImage::from_pixel(width, height, image::Rgb([255, 0, 0]));
        let mut bytes = Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, format)
            .expect("synthetic encoding");
        bytes.into_inner()
    }

    fn inline<'a>(id: &'a str, bytes: &'a [u8]) -> InlineImage<'a> {
        InlineImage {
            content_id: id,
            media_type: "image/png",
            bytes,
        }
    }

    fn pixel(frame: &Frame, x: usize, y: usize) -> &[u8] {
        let offset = (y * usize::try_from(frame.width).expect("small frame") + x) * 4;
        &frame.pixels[offset..offset + 4]
    }

    const HTML: &str = "<style>body{margin:0;background:#fff}img{display:block;width:32px;height:32px}</style><img src='cid:logo@example.test'>";

    #[test]
    fn png_and_jpeg_paint_from_local_decoded_resources() {
        for (format, media_type) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
        ] {
            let bytes = encoded(8, 8, format);
            let images = [InlineImage {
                content_id: "logo@example.test",
                media_type,
                bytes: &bytes,
            }];
            let frame =
                render_with_images(HTML, &images, 100, 100, false).expect("local image raster");
            let p = pixel(&frame, 16, 16);
            assert!(
                p[0] > 240 && p[1] < 10 && p[2] < 10 && p[3] == 255,
                "actual inline pixels: {p:?}"
            );
            assert_eq!(
                frame.blocked_resources, 0,
                "supplied image needs no transport"
            );
        }
        let pixels = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 128]));
        let mut encoded = Cursor::new(Vec::new());
        pixels
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("alpha fixture");
        let frame = render_with_images(
            HTML,
            &[inline("logo@example.test", encoded.get_ref())],
            100,
            100,
            false,
        )
        .expect("transparent PNG");
        assert_eq!(
            pixel(&frame, 16, 16),
            &[255, 127, 127, 255],
            "straight RGBA composes correctly on white"
        );
    }

    #[test]
    fn missing_ids_encoding_and_foreign_urls_do_not_reuse_an_old_image() {
        let bytes = encoded(8, 8, ImageFormat::Png);
        let images = [inline("logo@example.test", &bytes)];
        let a = render_with_images(HTML, &images, 100, 100, false).expect("A image");
        assert_eq!(pixel(&a, 16, 16), &[255, 0, 0, 255]);
        let retired = Images::new(&images)
            .expect("resource table")
            .resolve("cid:logo@example.test")
            .expect("known CID")
            .to_owned();
        for source in [
            "cid:missing",
            "cid:logo%40example.test",
            "cid:../logo@example.test",
            retired.as_str(),
            "file:///private/logo.png",
            "data:image/svg+xml,<svg/>",
            "https://example.invalid/tracker",
        ] {
            let html = format!(
                "<style>body{{margin:0;background:#fff}}img{{display:block;width:32px;height:32px}}</style><img src='{source}'><div style='position:absolute;left:60px;top:0;width:20px;height:20px;background:#0000ff'></div>"
            );
            let b = render_with_images(&html, &images, 100, 100, false).expect("isolated B");
            assert_eq!(pixel(&b, 70, 10), &[0, 0, 255, 255], "distinct B marker");
            assert_eq!(
                pixel(&b, 16, 16),
                &[255, 255, 255, 255],
                "unknown/foreign source stays absent"
            );
        }
        let b = render_with_images(HTML, &[], 100, 100, false).expect("B has no image table");
        assert_eq!(
            pixel(&b, 16, 16),
            &[255, 255, 255, 255],
            "A's CID cannot survive into B"
        );
        let a_again = render_with_images(HTML, &images, 100, 100, false).expect("fresh A");
        assert_eq!(pixel(&a_again, 16, 16), &[255, 0, 0, 255]);
    }

    #[test]
    fn ambiguous_invalid_types_and_malformed_images_fail_without_private_diagnostics() {
        let bytes = encoded(8, 8, ImageFormat::Png);
        assert!(matches!(
            Images::new(&[inline("duplicate", &bytes), inline("duplicate", &bytes)]),
            Err(Error::InvalidResource)
        ));
        for id in [
            "",
            ".",
            "..",
            "../logo",
            "logo/path",
            "logo\\path",
            "logo%40host",
            "logo?query",
            "logo#fragment",
            "<logo>",
            "with space",
            "世界",
        ] {
            assert!(matches!(
                Images::new(&[inline(id, &bytes)]),
                Err(Error::InvalidResource)
            ));
        }
        for (media_type, data) in [
            ("image/jpeg", bytes.as_slice()),
            ("image/svg+xml", b"<svg/>".as_slice()),
            ("image/png", b"PRIVATE_BODY".as_slice()),
        ] {
            let error = render_with_images(
                "",
                &[InlineImage {
                    content_id: "PRIVATE_ID",
                    media_type,
                    bytes: data,
                }],
                10,
                10,
                false,
            )
            .expect_err("invalid image");
            assert_eq!(error, Error::InvalidResource);
            assert!(!format!("{error:?} {error}").contains("PRIVATE_"));
        }
        let a = Images::new(&[inline("logo", &bytes)]).expect("A");
        let b = Images::new(&[inline("logo", &bytes)]).expect("B");
        assert_ne!(
            a.resolve("cid:logo"),
            b.resolve("cid:logo"),
            "fresh namespaces"
        );
        assert_eq!(a.resolve("CID:logo"), a.resolve("cid:logo"));
        assert_eq!(a.resolve("cid:Logo"), None, "opaque ID case is preserved");
    }

    #[test]
    fn encoded_decoded_dimension_and_count_budgets_are_enforced() {
        let large = vec![0; MAX_IMAGE_BYTES + 1];
        assert!(matches!(
            Images::new(&[inline("large", &large)]),
            Err(Error::ResourceLimit)
        ));
        let many = (0..=MAX_INLINE_IMAGES)
            .map(|_| inline("unused", &[]))
            .collect::<Vec<_>>();
        assert!(matches!(Images::new(&many), Err(Error::ResourceLimit)));
        let wide = encoded(MAX_SIDE + 1, 1, ImageFormat::Png);
        assert!(matches!(
            Images::new(&[inline("wide", &wide)]),
            Err(Error::ResourceLimit)
        ));
        let bomb = encoded(4096, 1024, ImageFormat::Png);
        assert!(bomb.len() < MAX_IMAGE_BYTES);
        assert!(
            matches!(
                Images::new(&[inline("pixels", &bomb)]),
                Err(Error::ResourceLimit)
            ),
            "small encoding cannot bypass decoded budget"
        );
        let tile = encoded(1024, 1024, ImageFormat::Png);
        let group = [
            inline("a", &tile),
            inline("b", &tile),
            inline("c", &tile),
            inline("d", &tile),
        ];
        assert!(
            matches!(Images::new(&group), Err(Error::ResourceLimit)),
            "aggregate limit applies across images"
        );
    }
}
