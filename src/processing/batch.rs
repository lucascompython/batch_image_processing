use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use super::image_ops::{self, Rotation, TextOverlayConfig};

/// What format to export as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Jpeg,
    Pdf,
}

/// Full configuration for a batch run.
#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub quality: u8,
    pub rotation: Rotation,
    pub text_overlay: Option<TextOverlayConfig>,
    pub output_format: OutputFormat,
    pub output_dir: PathBuf,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            quality: 70,
            rotation: Rotation::None,
            text_overlay: None,
            output_format: OutputFormat::Jpeg,
            output_dir: PathBuf::new(),
        }
    }
}

/// Result of processing a single image.
#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub source: PathBuf,
    pub success: bool,
    pub error: Option<String>,
}

/// Process a batch of images in parallel.
///
/// Returns a vector of results (one per input path) and calls `progress_callback`
/// after each image is done, with (completed_count, total_count).
pub fn process_batch(
    paths: &[PathBuf],
    config: &BatchConfig,
    progress_callback: impl Fn(usize, usize) + Send + Sync,
) -> Vec<ProcessResult> {
    let total = paths.len();
    let completed = Arc::new(AtomicUsize::new(0));

    // Pre-render a shared text stamp if the template doesn't contain {filename}
    let shared_stamp = config.text_overlay.as_ref().and_then(|tc| {
        if !tc.text_template.contains("{filename}") {
            Some(image_ops::render_text_stamp(tc, ""))
        } else {
            None
        }
    });

    // Process all images in parallel natively without pre-loading into a massive memory vector.
    paths
        .par_iter()
        .map(|source| {
            let load_result = image_ops::load_image(source);
            let result = process_single(source, load_result, config, shared_stamp.as_ref());
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            progress_callback(done, total);
            result
        })
        .collect()
}

fn process_single(
    source: &Path,
    load_result: Result<image::DynamicImage, String>,
    config: &BatchConfig,
    shared_stamp: Option<&image::RgbaImage>,
) -> ProcessResult {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");

    let mut img = match load_result {
        Ok(img) => img,
        Err(e) => {
            return ProcessResult {
                source: source.to_path_buf(),
                success: false,
                error: Some(e),
            };
        }
    };

    // Apply rotation
    if config.rotation != Rotation::None {
        img = image_ops::rotate_image(&img, config.rotation);
    }

    // Apply text overlay using cached stamp when possible
    if let Some(ref text_config) = config.text_overlay {
        if let Some(stamp) = shared_stamp {
            // Static text: reuse the shared pre-rendered stamp
            img = image_ops::overlay_text_with_stamp(img, text_config, stamp);
        } else {
            // Per-file text: render a stamp for this filename
            let stamp = image_ops::render_text_stamp(text_config, stem);
            img = image_ops::overlay_text_with_stamp(img, text_config, &stamp);
        }
    }

    // Export
    let output_path = match config.output_format {
        OutputFormat::Jpeg => config.output_dir.join(format!("{stem}.jpg")),
        OutputFormat::Pdf => config.output_dir.join(format!("{stem}.pdf")),
    };

    let result = match config.output_format {
        OutputFormat::Jpeg => image_ops::save_jpeg(&img, &output_path, config.quality),
        OutputFormat::Pdf => {
            image_ops::export_single_image_to_pdf(&img, &output_path, config.quality)
        }
    };

    match result {
        Ok(()) => ProcessResult {
            source: source.to_path_buf(),
            success: true,
            error: None,
        },
        Err(e) => ProcessResult {
            source: source.to_path_buf(),
            success: false,
            error: Some(e),
        },
    }
}
