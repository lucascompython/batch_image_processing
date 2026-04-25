use ab_glyph::{FontRef, PxScale};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};
use memmap2::MmapOptions;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};
use std::fs::File;
use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

/// Rotation direction (clockwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None,
    Cw90,
    Cw180,
    Cw270,
}

/// Where to place the text overlay on the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
static TURBO_SCALING_FACTORS: OnceLock<Vec<turbojpeg::ScalingFactor>> = OnceLock::new();

/// Load an image from disk.
///
/// # Errors
/// Returns an error if the file cannot be read or decoded.
pub fn load_image(path: &Path) -> Result<DynamicImage, String> {
    let raw = map_image_file(path)?;
    decode_image_from_bytes(path, &raw, None)
}

/// Load an image optimized for preview generation.
///
/// For JPEGs, this uses DCT scaling during decode to avoid full-size decode for huge files.
pub fn load_image_for_preview(path: &Path, max_side: u32) -> Result<DynamicImage, String> {
    let raw = map_image_file(path)?;
    decode_image_from_bytes(path, &raw, Some(max_side))
}

fn map_image_file(path: &Path) -> Result<memmap2::Mmap, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    // SAFETY: We create a read-only map over an immutable file descriptor and keep
    // the map alive for the full decode scope. No mutation aliases are created.
    unsafe { MmapOptions::new().map(&file) }
        .map_err(|e| format!("Failed to map {}: {e}", path.display()))
}

fn decode_image_from_bytes(
    path: &Path,
    raw: &[u8],
    preview_max_side: Option<u32>,
) -> Result<DynamicImage, String> {
    let orientation = read_image_orientation_from_bytes(raw);

    if let Some(mut img) = decode_with_turbojpeg(raw, preview_max_side) {
        img.apply_orientation(orientation);
        return Ok(img);
    }

    // fallback to generic image decoder if turbojpeg fails or format is non-JPEG.
    let mut decoder = ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess format for {}: {e}", path.display()))?
        .into_decoder()
        .map_err(|e| format!("Failed to create decoder {}: {e}", path.display()))?;

    let orientation = decoder.orientation().unwrap_or(orientation);
    let mut img = DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("Failed to decode {}: {e}", path.display()))?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn decode_with_turbojpeg(raw: &[u8], preview_max_side: Option<u32>) -> Option<DynamicImage> {
    if let Some(max_side) = preview_max_side {
        let mut decompressor = turbojpeg::Decompressor::new().ok()?;
        let header = decompressor.read_header(raw).ok()?;
        let scaling = select_scaling_factor(header, max_side);
        if scaling != turbojpeg::ScalingFactor::ONE {
            decompressor.set_scaling_factor(scaling).ok()?;
            let scaled = header.scaled(scaling);
            let pixel_count = scaled.width.checked_mul(scaled.height)?;
            let len = pixel_count.checked_mul(4)?;
            let mut image = turbojpeg::Image {
                pixels: vec![0u8; len],
                width: scaled.width,
                pitch: scaled.width.checked_mul(4)?,
                height: scaled.height,
                format: turbojpeg::PixelFormat::RGBA,
            };
            decompressor.decompress(raw, image.as_deref_mut()).ok()?;
            let rgba =
                RgbaImage::from_raw(scaled.width as u32, scaled.height as u32, image.pixels)?;
            return Some(DynamicImage::ImageRgba8(rgba));
        }
    }

    let decoded = turbojpeg::decompress(raw, turbojpeg::PixelFormat::RGBA).ok()?;
    let rgba = RgbaImage::from_raw(decoded.width as u32, decoded.height as u32, decoded.pixels)?;
    Some(DynamicImage::ImageRgba8(rgba))
}

fn select_scaling_factor(
    header: turbojpeg::DecompressHeader,
    max_side: u32,
) -> turbojpeg::ScalingFactor {
    let max_side = max_side as usize;
    if header.width <= max_side && header.height <= max_side {
        return turbojpeg::ScalingFactor::ONE;
    }

    let factors =
        TURBO_SCALING_FACTORS.get_or_init(turbojpeg::Decompressor::supported_scaling_factors);

    let mut best_above: Option<(turbojpeg::ScalingFactor, usize)> = None;
    let mut best_below: Option<(turbojpeg::ScalingFactor, usize)> = None;
    for &factor in factors {
        let w = factor.scale(header.width);
        let h = factor.scale(header.height);
        let scaled_max = w.max(h);

        if scaled_max >= max_side {
            match best_above {
                Some((_, current)) if scaled_max >= current => {}
                _ => best_above = Some((factor, scaled_max)),
            }
        } else {
            match best_below {
                Some((_, current)) if scaled_max <= current => {}
                _ => best_below = Some((factor, scaled_max)),
            }
        }
    }

    if let Some((factor, _)) = best_above {
        factor
    } else if let Some((factor, _)) = best_below {
        factor
    } else {
        turbojpeg::ScalingFactor::ONE
    }
}

fn read_image_orientation_from_bytes(raw: &[u8]) -> Orientation {
    let Ok(reader) = ImageReader::new(Cursor::new(raw)).with_guessed_format() else {
        return Orientation::NoTransforms;
    };
    let Ok(mut decoder) = reader.into_decoder() else {
        return Orientation::NoTransforms;
    };
    decoder.orientation().unwrap_or(Orientation::NoTransforms)
}

pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: u8) -> Result<(), String> {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let turbo_img = turbojpeg::Image {
        pixels: rgb.as_raw().as_slice(),
        width: w as usize,
        pitch: w as usize * 3,
        height: h as usize,
        format: turbojpeg::PixelFormat::RGB,
    };
    let jpeg_data = turbojpeg::compress(turbo_img, quality as i32, turbojpeg::Subsamp::Sub2x2)
        .map_err(|e| format!("Failed to encode JPEG {}: {e}", path.display()))?;
    std::fs::write(path, &*jpeg_data)
        .map_err(|e| format!("Failed to write JPEG {}: {e}", path.display()))
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
    img: DynamicImage,
    config: &TextOverlayConfig,
    stamp: &RgbaImage,
) -> DynamicImage {
    // Avoid a full pixel-by-pixel clone if the image is already RGBA
    let mut rgba = match img {
        DynamicImage::ImageRgba8(existing) => existing,
        other => other.to_rgba8(),
    };
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
pub fn overlay_text(img: DynamicImage, config: &TextOverlayConfig, filename: &str) -> DynamicImage {
    let stamp = render_text_stamp(config, filename);
    overlay_text_with_stamp(img, config, &stamp)
}

/// Generate a thumbnail that fits within `max_size` pixels on the longest side.
/// Uses Triangle (bilinear) filter - ~3x faster than the default Lanczos3,
/// visually identical at preview sizes.
pub fn generate_thumbnail(img: &DynamicImage, max_size: u32) -> RgbaImage {
    img.resize(max_size, max_size, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

/// Export a single image to a PDF file.
///
/// The PDF page is sized to match the image at 72 DPI.
///
/// # Errors
/// Returns an error if encoding or writing fails.
pub fn export_single_image_to_pdf(
    img: &DynamicImage,
    output_path: &Path,
    quality: u8,
) -> Result<(), String> {
    export_images_to_pdf(&[img], output_path, quality)
}

/// Export multiple images to a multi-page PDF.
///
/// Each page is sized to match its image at 72 DPI.
///
/// # Errors
/// Returns an error if encoding or writing fails.
pub fn export_images_to_pdf(
    images: &[&DynamicImage],
    output_path: &Path,
    quality: u8,
) -> Result<(), String> {
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

        // Encode as JPEG for the PDF using turbojpeg
        let turbo_img = turbojpeg::Image {
            pixels: rgb.as_raw().as_slice(),
            width: w as usize,
            pitch: w as usize * 3,
            height: h as usize,
            format: turbojpeg::PixelFormat::RGB,
        };
        let jpeg_buf = turbojpeg::compress(turbo_img, quality as i32, turbojpeg::Subsamp::Sub2x2)
            .map_err(|e| format!("Failed to encode image for PDF: {e}"))?;

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
