//! Bounded raster previews. SVG is never executed and cannot load external resources.
use dunlin_workspace::preview::PreviewKind;
use std::{
    collections::VecDeque,
    io::Cursor,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub(crate) struct Preview {
    pub bytes: Arc<Vec<u8>>,
    pub image: Arc<dunlin_ui::RenderImage>,
    pub page: usize,
    pub pages: usize,
}
pub(crate) fn decode(
    bytes: Arc<Vec<u8>>,
    kind: PreviewKind,
    page: usize,
) -> Result<Preview, String> {
    if bytes.len() as u64 > dunlin_workspace::preview::MAX_PREVIEW_BYTES {
        return Err("Preview exceeds 32 MiB".into());
    }
    let (rgba, pages) = match kind {
        PreviewKind::Png | PreviewKind::Jpeg => {
            use image::ImageDecoder;
            let format = if kind == PreviewKind::Png {
                image::ImageFormat::Png
            } else {
                image::ImageFormat::Jpeg
            };
            let mut reader = image::ImageReader::with_format(Cursor::new(bytes.as_slice()), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(8192);
            limits.max_image_height = Some(8192);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
            let orientation = decoder.orientation().map_err(|e| e.to_string())?;
            let mut image =
                image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
            image.apply_orientation(orientation);
            let image = if image.width() > 1600 || image.height() > 1600 {
                image.thumbnail(1600, 1600)
            } else {
                image
            };
            (image.into_rgba8(), 1)
        }
        PreviewKind::Svg => {
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("SVG preview exceeds 8 MiB".into());
            }
            let mut options = resvg::usvg::Options::default();
            options.image_href_resolver.resolve_string = Box::new(|_, _| None);
            options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
            let text =
                std::str::from_utf8(&bytes).map_err(|_| "SVG must be uncompressed UTF-8 XML")?;
            if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
                return Err(
                    "SVG document types and entity declarations are not allowed in previews".into(),
                );
            }
            if text.contains("<text") || text.contains(":text") {
                static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
                options.fontdb = FONTS
                    .get_or_init(|| {
                        let mut fonts = resvg::usvg::fontdb::Database::new();
                        fonts.load_system_fonts();
                        Arc::new(fonts)
                    })
                    .clone();
            }
            let tree = resvg::usvg::Tree::from_str(text, &options).map_err(|e| e.to_string())?;
            let size = tree.size();
            let scale = (1600. / size.width().max(size.height())).min(1.);
            let (width, height) = (
                (size.width() * scale).ceil().max(1.) as u32,
                (size.height() * scale).ceil().max(1.) as u32,
            );
            let mut pixmap =
                resvg::tiny_skia::Pixmap::new(width, height).ok_or("Invalid SVG dimensions")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            // tiny-skia stores premultiplied RGBA; GPUI expects straight BGRA.
            let mut rgba = pixmap.take();
            for pixel in rgba.as_chunks_mut::<4>().0 {
                let alpha = u32::from(pixel[3]);
                if alpha > 0 && alpha < 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            (
                image::RgbaImage::from_raw(width, height, rgba).ok_or("Invalid SVG bitmap")?,
                1,
            )
        }
        PreviewKind::Pdf => pdf::render(&bytes, page)?,
        _ => return Err("This file type cannot be previewed".into()),
    };
    Ok(Preview {
        bytes,
        image: render_image(rgba),
        page,
        pages,
    })
}

fn render_image(mut rgba: image::RgbaImage) -> Arc<dunlin_ui::RenderImage> {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Arc::new(dunlin_ui::RenderImage::new(vec![image::Frame::new(rgba)]))
}

/// Additional rendered PDF pages, never unbounded and never retained after a tab closes.
#[derive(Default)]
pub(crate) struct Cache {
    pages: VecDeque<(String, Preview)>,
}
impl Cache {
    const BUDGET: usize = 32 * 1024 * 1024;
    fn cost(preview: &Preview) -> usize {
        preview.image.as_bytes(0).map_or(0, <[u8]>::len)
    }
    pub fn get(&mut self, tab: &str, page: usize) -> Option<Preview> {
        let index = self
            .pages
            .iter()
            .position(|(id, p)| id == tab && p.page == page)?;
        let entry = self.pages.remove(index)?;
        let preview = entry.1.clone();
        self.pages.push_back(entry);
        Some(preview)
    }
    pub fn insert(&mut self, tab: &str, preview: Preview) {
        self.pages
            .retain(|(id, p)| id != tab || p.page != preview.page);
        if Self::cost(&preview) > Self::BUDGET {
            return;
        }
        self.pages.push_back((tab.into(), preview));
        while self.pages.len() > 8
            || self.pages.iter().map(|(_, p)| Self::cost(p)).sum::<usize>() > Self::BUDGET
        {
            self.pages.pop_front();
        }
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.pages.retain(|(id, _)| keep(id));
    }
}

#[cfg(target_os = "macos")]
mod pdf {
    use std::{ffi::c_void, ptr};
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    #[repr(C)]
    struct Transform {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        tx: f64,
        ty: f64,
    }
    type Ref = *mut c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDataProviderCreateWithData(
            info: Ref,
            data: *const c_void,
            size: usize,
            release: Option<unsafe extern "C" fn(Ref, *const c_void, usize)>,
        ) -> Ref;
        fn CGDataProviderRelease(value: Ref);
        fn CGPDFDocumentCreateWithProvider(provider: Ref) -> Ref;
        fn CGPDFDocumentRelease(value: Ref);
        fn CGPDFDocumentGetNumberOfPages(value: Ref) -> usize;
        fn CGPDFDocumentGetPage(value: Ref, page: usize) -> Ref;
        fn CGPDFDocumentIsEncrypted(value: Ref) -> bool;
        fn CGPDFPageGetBoxRect(value: Ref, box_type: i32) -> Rect;
        fn CGPDFPageGetRotationAngle(value: Ref) -> i32;
        fn CGPDFPageGetDrawingTransform(
            page: Ref,
            box_type: i32,
            rect: Rect,
            rotate: i32,
            preserve: bool,
        ) -> Transform;
        fn CGColorSpaceCreateDeviceRGB() -> Ref;
        fn CGColorSpaceRelease(value: Ref);
        fn CGBitmapContextCreate(
            data: Ref,
            width: usize,
            height: usize,
            bits: usize,
            row_bytes: usize,
            color: Ref,
            flags: u32,
        ) -> Ref;
        fn CGContextRelease(value: Ref);
        fn CGContextConcatCTM(context: Ref, transform: Transform);
        fn CGContextScaleCTM(context: Ref, sx: f64, sy: f64);
        fn CGContextDrawPDFPage(context: Ref, page: Ref);
    }
    struct Owned(Ref, unsafe extern "C" fn(Ref));
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                (self.1)(self.0);
            }
        }
    }
    pub(super) fn render(bytes: &[u8], page: usize) -> Result<(image::RgbaImage, usize), String> {
        if !bytes.starts_with(b"%PDF-") {
            return Err("Not a valid PDF document".into());
        }
        // SAFETY: the provider borrows `bytes` only within this function. Each
        // successful Create is owned until Drop; pages borrow the live document.
        unsafe {
            let provider = CGDataProviderCreateWithData(
                ptr::null_mut(),
                bytes.as_ptr().cast(),
                bytes.len(),
                None,
            );
            if provider.is_null() {
                return Err("Could not read PDF bytes".into());
            }
            let provider = Owned(provider, CGDataProviderRelease);
            let doc = CGPDFDocumentCreateWithProvider(provider.0);
            if doc.is_null() {
                return Err("Could not parse PDF".into());
            }
            let doc = Owned(doc, CGPDFDocumentRelease);
            if CGPDFDocumentIsEncrypted(doc.0) {
                return Err("Encrypted PDFs cannot be previewed".into());
            }
            let pages = CGPDFDocumentGetNumberOfPages(doc.0);
            if page == 0 || page > pages || pages > 2000 {
                return Err("PDF page is unavailable (limit: 2,000 pages)".into());
            }
            let page_ref = CGPDFDocumentGetPage(doc.0, page);
            if page_ref.is_null() {
                return Err("Could not load PDF page".into());
            }
            let rect = CGPDFPageGetBoxRect(page_ref, 1); // Crop box.
            let (mut w, mut h) = (rect.size.width.abs(), rect.size.height.abs());
            if CGPDFPageGetRotationAngle(page_ref).rem_euclid(180) == 90 {
                std::mem::swap(&mut w, &mut h);
            }
            if !w.is_finite() || !h.is_finite() || w < 1. || h < 1. {
                return Err("Invalid PDF dimensions".into());
            }
            let scale = 1600. / w.max(h);
            let (width, height) = (
                (w * scale).ceil().max(1.) as usize,
                (h * scale).ceil().max(1.) as usize,
            );
            let mut rgba = vec![255u8; width * height * 4];
            let color = CGColorSpaceCreateDeviceRGB();
            if color.is_null() {
                return Err("Could not create preview color space".into());
            }
            let color = Owned(color, CGColorSpaceRelease);
            // 32-bit big-endian RGBA, alpha last, premultiplied.
            let context = CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                color.0,
                (4 << 12) | 1,
            );
            if context.is_null() {
                return Err("Could not render PDF page".into());
            }
            let context = Owned(context, CGContextRelease);
            let target = Rect {
                origin: Point { x: 0., y: 0. },
                size: Size {
                    width: width as f64 / scale,
                    height: height as f64 / scale,
                },
            };
            // CGPDFPageGetDrawingTransform centers a small page without scaling
            // it up. Apply the raster scale explicitly, and ask CG only for the
            // point-space crop/rotation transform (including nonzero box origins).
            CGContextScaleCTM(context.0, scale, scale);
            let transform = CGPDFPageGetDrawingTransform(page_ref, 1, target, 0, true);
            CGContextConcatCTM(context.0, transform);
            CGContextDrawPDFPage(context.0, page_ref);
            drop(context);
            let image = image::RgbaImage::from_raw(width as u32, height as u32, rgba)
                .ok_or("Invalid PDF bitmap")?;
            // CG's PDF drawing transform already establishes page orientation
            // in this bitmap. Flipping the rows again inverts the document.
            Ok((image, pages))
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod pdf {
    pub(super) fn render(_: &[u8], _: usize) -> Result<(image::RgbaImage, usize), String> {
        Err("PDF previews currently require macOS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewKind, decode};
    use std::{io::Cursor, sync::Arc};

    fn rgba(preview: &super::Preview) -> image::RgbaImage {
        let size = preview.image.size(0);
        let mut bytes = preview.image.as_bytes(0).unwrap().to_vec();
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        image::RgbaImage::from_raw(size.width.into(), size.height.into(), bytes).unwrap()
    }

    #[test]
    fn png_and_jpeg_are_decoded_without_format_fallbacks() {
        for (format, kind) in [
            (image::ImageFormat::Png, PreviewKind::Png),
            (image::ImageFormat::Jpeg, PreviewKind::Jpeg),
        ] {
            let image = image::DynamicImage::new_rgb8(16, 8);
            let mut encoded = Cursor::new(Vec::new());
            image.write_to(&mut encoded, format).unwrap();
            let preview = decode(Arc::new(encoded.into_inner()), kind, 1).unwrap();
            let rendered = rgba(&preview);
            assert_eq!((rendered.width(), rendered.height()), (16, 8));
            assert_eq!(preview.pages, 1);
        }
        assert!(decode(Arc::new(b"PK archive".to_vec()), PreviewKind::Png, 1).is_err());
        assert!(decode(Arc::new(vec![]), PreviewKind::Unsupported, 1).is_err());
    }

    #[test]
    fn svg_is_rasterized_with_external_resources_and_entities_disabled() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><script>alert(1)</script><image href="file:///etc/passwd" width="20" height="10"/><image href="https://127.0.0.1:1/private"/><rect width="20" height="10" fill="red"/></svg>"#;
        let preview = decode(Arc::new(svg.to_vec()), PreviewKind::Svg, 1).unwrap();
        let image = rgba(&preview);
        assert_eq!((image.width(), image.height()), (20, 10));
        assert!(image.pixels().all(|p| p[0] > 240 && p[1] < 10));
        assert!(
            decode(
                Arc::new(b"<!DOCTYPE svg><svg/>".to_vec()),
                PreviewKind::Svg,
                1
            )
            .is_err()
        );
        assert!(decode(Arc::new(vec![0x1f, 0x8b, 0x08]), PreviewKind::Svg, 1).is_err());
    }

    #[test]
    fn straight_alpha_and_color_channels_survive_the_direct_gpu_bitmap_path() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red" opacity="0.5"/></svg>"#;
        let preview = decode(Arc::new(svg.to_vec()), PreviewKind::Svg, 1).unwrap();
        let pixel = rgba(&preview)[(0, 0)];
        assert!(pixel[0] > 250 && pixel[1] < 3 && pixel[2] < 3);
        assert!((126..=129).contains(&pixel[3]));
    }

    #[test]
    fn rendered_page_cache_is_lru_bounded_and_drops_closed_tabs() {
        let mut cache = super::Cache::default();
        let bytes = Arc::new(Vec::new());
        let image = super::render_image(image::RgbaImage::new(2, 2));
        for page in 1..=8 {
            cache.insert(
                "pdf",
                super::Preview {
                    bytes: bytes.clone(),
                    image: image.clone(),
                    page,
                    pages: 9,
                },
            );
        }
        assert!(Arc::ptr_eq(&cache.get("pdf", 1).unwrap().image, &image));
        cache.insert(
            "pdf",
            super::Preview {
                bytes,
                image,
                page: 9,
                pages: 9,
            },
        );
        assert!(cache.get("pdf", 2).is_none());
        assert!(cache.get("pdf", 1).is_some());
        cache.retain(|_| false);
        assert!(cache.pages.is_empty());
        for page in 1..=4 {
            cache.insert(
                "large",
                super::Preview {
                    bytes: Arc::new(Vec::new()),
                    image: super::render_image(image::RgbaImage::new(1600, 1600)),
                    page,
                    pages: 4,
                },
            );
        }
        assert!(
            cache
                .pages
                .iter()
                .map(|(_, p)| super::Cache::cost(p))
                .sum::<usize>()
                <= super::Cache::BUDGET
        );
        assert!(cache.get("large", 1).is_none());
    }

    #[cfg(target_os = "macos")]
    fn single_page_pdf(boxes: &str, content: &str) -> Arc<Vec<u8>> {
        use std::fmt::Write;
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".into(),
            format!("<< /Type /Page /Parent 2 0 R {boxes} /Resources << >> /Contents 4 0 R >>"),
            format!(
                "<< /Length {} >>\nstream\n{content}endstream",
                content.len()
            ),
        ];
        let mut data = "%PDF-1.4\n".to_string();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(data.len());
            writeln!(data, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let xref = data.len();
        write!(data, "xref\n0 5\n0000000000 65535 f \n").unwrap();
        for offset in offsets {
            writeln!(data, "{offset:010} 00000 n ").unwrap();
        }
        write!(
            data,
            "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .unwrap();
        Arc::new(data.into_bytes())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pdf_rotation_and_nonzero_crop_origins_preserve_orientation_aspect_and_colors() {
        for rotation in [90, 270] {
            let bytes = single_page_pdf(
                &format!("/MediaBox [0 0 200 100] /Rotate {rotation}"),
                "1 0 0 rg 0 0 50 50 re f\n",
            );
            let preview = decode(bytes, PreviewKind::Pdf, 1).unwrap();
            let image = rgba(&preview);
            assert_eq!(image.dimensions(), (800, 1600));
            let pixel = if rotation == 90 {
                image[(100, 100)]
            } else {
                image[(700, 1500)]
            };
            assert!(
                pixel[0] > 240 && pixel[1] < 10 && pixel[2] < 10,
                "rotation {rotation}: {pixel:?}"
            );
        }
        let bytes = single_page_pdf(
            "/MediaBox [10 20 210 120] /CropBox [10 20 110 120]",
            "0 1 0 rg 10 20 50 50 re f\n1 0 0 rg 60 70 50 50 re f\n",
        );
        let preview = decode(bytes, PreviewKind::Pdf, 1).unwrap();
        let image = rgba(&preview);
        assert_eq!(image.dimensions(), (1600, 1600));
        let top_right = image[(1200, 400)];
        let bottom_left = image[(400, 1200)];
        assert!(top_right[0] > 240 && top_right[1] < 10);
        assert!(bottom_left[1] > 240 && bottom_left[0] < 10);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_pdf_preview_renders_each_page_and_rejects_invalid_pages() {
        use std::fmt::Write;
        let red = "1 0 0 rg 10 10 40 40 re f\n";
        let green = "0 1 0 rg 10 10 40 40 re f\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 5 0 R >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] /Resources << >> /Contents 6 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{red}endstream", red.len()),
            format!("<< /Length {} >>\nstream\n{green}endstream", green.len()),
        ];
        let mut data = "%PDF-1.4\n".to_string();
        let mut offsets = vec![0];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(data.len());
            writeln!(data, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let xref = data.len();
        write!(data, "xref\n0 7\n0000000000 65535 f \n").unwrap();
        for offset in offsets.iter().skip(1) {
            writeln!(data, "{offset:010} 00000 n ").unwrap();
        }
        write!(
            data,
            "trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .unwrap();
        let data = Arc::new(data.into_bytes());
        for (page, dimensions) in [(1, (1600, 800)), (2, (800, 1600))] {
            let preview = decode(data.clone(), PreviewKind::Pdf, page).unwrap();
            assert_eq!(preview.pages, 2);
            let image = rgba(&preview);
            assert_eq!((image.width(), image.height()), dimensions);
            assert!(image.pixels().any(|p| p[0] < 80 || p[1] < 80));
        }
        assert!(decode(data, PreviewKind::Pdf, 3).is_err());
        assert!(decode(Arc::new(b"not a PDF".to_vec()), PreviewKind::Pdf, 1).is_err());
    }
}
