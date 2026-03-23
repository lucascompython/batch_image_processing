use ab_glyph::{FontRef, PxScale};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};
use memmap2::Mmap;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};
use std::io::BufWriter;
use std::path::Path;

/// Rotation direction (clockwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None,
    Cw90,
    Cw180,
    Cw270,
}

/// Where to place the text overlay on the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

/// RGBA color for text overlay.
#[derive(Debug, Clone, Copy)]
pub struct TextColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for TextColor {
    fn default() -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    }
}

/// Configuration for text overlay.
#[derive(Debug, Clone)]
pub struct TextOverlayConfig {
    pub text_template: String,
    pub position: TextPosition,
    pub font_size: f32,
    pub color: TextColor,
    pub margin: u32,
}

impl Default for TextOverlayConfig {
    fn default() -> Self {
        Self {
            text_template: "{filename}".to_string(),
            position: TextPosition::BottomRight,
            font_size: 24.0,
            color: TextColor::default(),
            margin: 10,
        }
    }
}

/// The embedded font bytes (Inter Regular).
const EMBEDDED_FONT: &[u8] = include_bytes!("../assets/Inter-Regular.ttf");

/// Load an image from disk.
///
/// # Errors
/// Returns an error if the file cannot be read or decoded.
pub fn load_image(path: &Path) -> Result<DynamicImage, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.display()))?;

    // Try blazing-fast zune-jpeg with memory mapping first
    if let Ok(mmap) = unsafe { Mmap::map(&file) } {
        let options = zune_core::options::DecoderOptions::default()
            .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA);

        let cursor = zune_core::bytestream::ZCursor::new(&mmap[..]);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(cursor, options);
        if decoder.decode_headers().is_ok()
            && let Some((w, h)) = decoder.dimensions()
            && let Ok(pixels) = decoder.decode()
            && let Some(rgba) = RgbaImage::from_raw(w as u32, h as u32, pixels)
        {
            return Ok(DynamicImage::ImageRgba8(rgba));
        }
    }

    println!("Failed to decode {}", path.display());

    // Fallback to generic `image` crate decoder if it's a PNG or fails
    ImageReader::open(path)
        .map_err(|e| format!("Failed to open {}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("Failed to decode {}: {e}", path.display()))
}

/// Save a `DynamicImage` as JPEG with the given quality (0-100).
///
/// # Errors
/// Returns an error if the file cannot be created or encoded.
pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Failed to create {}: {e}", path.display()))?;
    let writer = BufWriter::new(file);
    let encoder = JpegEncoder::new_with_quality(writer, quality);
    img.write_with_encoder(encoder)
        .map_err(|e| format!("Failed to encode JPEG {}: {e}", path.display()))
}

/// Rotate the image clockwise.
pub fn rotate_image(img: &DynamicImage, rotation: Rotation) -> DynamicImage {
    match rotation {
        Rotation::None => img.clone(),
        Rotation::Cw90 => img.rotate90(),
        Rotation::Cw180 => img.rotate180(),
        Rotation::Cw270 => img.rotate270(),
    }
}

/// Pre-render text to a transparent RGBA stamp buffer.
///
/// This avoids re-rasterizing vector glyphs for every image in a batch.
/// Returns the stamp image along with its dimensions.
pub fn render_text_stamp(config: &TextOverlayConfig, filename: &str) -> RgbaImage {
    let text = config.text_template.replace("{filename}", filename);
    let font = FontRef::try_from_slice(EMBEDDED_FONT).expect("embedded font is valid");
    let scale = PxScale::from(config.font_size);
    let (tw, th) = text_size(scale, &font, &text);

    // Create a transparent buffer big enough for text + shadow offset
    let buf_w = tw + 2;
    let buf_h = th + 2;
    let mut stamp = RgbaImage::from_pixel(buf_w, buf_h, image::Rgba([0, 0, 0, 0]));

    // Shadow
    let shadow = image::Rgba([0u8, 0, 0, 180]);
    draw_text_mut(&mut stamp, shadow, 1, 1, scale, &font, &text);
    // Foreground
    let color = image::Rgba([
        config.color.r,
        config.color.g,
        config.color.b,
        config.color.a,
    ]);
    draw_text_mut(&mut stamp, color, 0, 0, scale, &font, &text);

    stamp
}

/// Overlay a pre-rendered text stamp onto an image.
///
/// This is the fast path for batch processing: render the stamp once,
/// then call this for every image.
pub fn overlay_text_with_stamp(
    img: &DynamicImage,
    config: &TextOverlayConfig,
    stamp: &RgbaImage,
) -> DynamicImage {
    let mut rgba = img.to_rgba8();
    let (img_w, img_h) = (rgba.width(), rgba.height());
    let margin = config.margin;
    let stamp_w = stamp.width();
    let stamp_h = stamp.height();

    let (x, y) = match config.position {
        TextPosition::TopLeft => (margin as i64, margin as i64),
        TextPosition::TopRight => (
            (img_w.saturating_sub(stamp_w + margin)) as i64,
            margin as i64,
        ),
        TextPosition::BottomLeft => (
            margin as i64,
            (img_h.saturating_sub(stamp_h + margin)) as i64,
        ),
        TextPosition::BottomRight => (
            (img_w.saturating_sub(stamp_w + margin)) as i64,
            (img_h.saturating_sub(stamp_h + margin)) as i64,
        ),
        TextPosition::Center => (
            ((img_w.saturating_sub(stamp_w)) / 2) as i64,
            ((img_h.saturating_sub(stamp_h)) / 2) as i64,
        ),
    };

    image::imageops::overlay(&mut rgba, stamp, x, y);
    DynamicImage::ImageRgba8(rgba)
}

/// Overlay text onto the image, returning a new `DynamicImage`.
///
/// `filename` is used to expand the `{filename}` template variable.
/// For single-image use (preview). For batch, prefer `render_text_stamp`
/// + `overlay_text_with_stamp`.
pub fn overlay_text(
    img: &DynamicImage,
    config: &TextOverlayConfig,
    filename: &str,
) -> DynamicImage {
    let stamp = render_text_stamp(config, filename);
    overlay_text_with_stamp(img, config, &stamp)
}

/// Generate a thumbnail that fits within `max_size` pixels on the longest side.
pub fn generate_thumbnail(img: &DynamicImage, max_size: u32) -> RgbaImage {
    img.thumbnail(max_size, max_size).to_rgba8()
}

/// Export a single image to a PDF file.
///
/// The PDF page is sized to match the image at 72 DPI.
///
/// # Errors
/// Returns an error if encoding or writing fails.
pub fn export_single_image_to_pdf(img: &DynamicImage, output_path: &Path) -> Result<(), String> {
    export_images_to_pdf(&[img], output_path)
}

/// Export multiple images to a multi-page PDF.
///
/// Each page is sized to match its image at 72 DPI.
///
/// # Errors
/// Returns an error if encoding or writing fails.
pub fn export_images_to_pdf(images: &[&DynamicImage], output_path: &Path) -> Result<(), String> {
    let mut pdf = Pdf::new();

    // We need to assign Ref IDs. Start from 1.
    // Structure: catalog, page_tree, then for each image: (page, content_stream, image_xobject)
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);

    // Pre-calculate all refs
    let mut next_id = 3u32;
    let mut page_refs = Vec::with_capacity(images.len());
    let mut content_refs = Vec::with_capacity(images.len());
    let mut image_refs = Vec::with_capacity(images.len());

    for _ in images {
        page_refs.push(Ref::new(next_id as i32));
        next_id += 1;
        content_refs.push(Ref::new(next_id as i32));
        next_id += 1;
        image_refs.push(Ref::new(next_id as i32));
        next_id += 1;
    }

    // Catalog
    pdf.catalog(catalog_id).pages(page_tree_id);

    // Page tree
    let mut page_tree = pdf.pages(page_tree_id);
    page_tree.kids(page_refs.iter().copied());
    page_tree.count(images.len() as i32);
    page_tree.finish();

    // Pages + content + images
    for (i, img) in images.iter().enumerate() {
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width(), rgb.height());
        let w_pt = w as f32;
        let h_pt = h as f32;

        // Encode as JPEG for the PDF
        let mut jpeg_buf = Vec::new();
        {
            let encoder = JpegEncoder::new_with_quality(&mut jpeg_buf, 90);
            rgb.write_with_encoder(encoder)
                .map_err(|e| format!("Failed to encode image for PDF: {e}"))?;
        }

        // Image XObject
        let image_name = format!("Im{i}");
        let mut image_xobj = pdf.image_xobject(image_refs[i], &jpeg_buf);
        image_xobj.filter(pdf_writer::Filter::DctDecode);
        image_xobj.width(w as i32);
        image_xobj.height(h as i32);
        image_xobj.color_space().device_rgb();
        image_xobj.bits_per_component(8);
        image_xobj.finish();

        // Content stream: draw the image scaled to fill the page
        let mut content = Content::new();
        content.save_state();
        content.transform([w_pt, 0.0, 0.0, h_pt, 0.0, 0.0]);
        content.x_object(Name(image_name.as_bytes()));
        content.restore_state();
        let content_data = content.finish();

        pdf.stream(content_refs[i], &content_data);

        // Page
        let mut page = pdf.page(page_refs[i]);
        page.parent(page_tree_id);
        page.media_box(Rect::new(0.0, 0.0, w_pt, h_pt));
        page.contents(content_refs[i]);
        let mut resources = page.resources();
        resources
            .x_objects()
            .pair(Name(image_name.as_bytes()), image_refs[i]);
        resources.finish();
        page.finish();
    }

    let pdf_bytes = pdf.finish();
    std::fs::write(output_path, pdf_bytes)
        .map_err(|e| format!("Failed to write PDF {}: {e}", output_path.display()))
}
