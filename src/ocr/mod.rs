//! OCR module for automatic motorcycle number recognition.
//!
//! Uses PaddleOCR models via pure-onnx-ocr-sync for digit detection.

use image::DynamicImage;
use pure_onnx_ocr_sync::{OcrEngine, OcrEngineBuilder};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Global OCR engine instance (lazy initialized)
static OCR_ENGINE: OnceLock<Option<OcrEngine>> = OnceLock::new();

/// Result of OCR recognition
#[derive(Debug, Clone)]
pub struct OcrResult {
    /// Detected text (filtered to digits only for motorcycle numbers)
    pub text: String,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// All detected text boxes with their confidence
    pub all_detections: Vec<Detection>,
}

/// A single text detection
#[derive(Debug, Clone)]
pub struct Detection {
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug)]
struct ModelPaths {
    det_model: PathBuf,
    rec_model: PathBuf,
    text_line_ori_model: PathBuf,
    doc_ori_model: PathBuf,
    dictionary: PathBuf,
}

/// Initialize the OCR engine (call once at startup)
pub fn init_ocr() -> Result<(), String> {
    OCR_ENGINE.get_or_init(|| {
        let model_dir = model_dir();
        let paths = match resolve_model_paths(&model_dir) {
            Ok(paths) => paths,
            Err(e) => {
                eprintln!("{e}");
                return None;
            }
        };

        let builder = OcrEngineBuilder::new()
            .det_model_path(&paths.det_model)
            .rec_model_path(&paths.rec_model)
            .text_line_ori_model_path(&paths.text_line_ori_model)
            .doc_ori_model_path(&paths.doc_ori_model)
            .dictionary_path(&paths.dictionary)
            .det_limit_side_len(960)
            .det_unclip_ratio(1.5)
            .rec_batch_size(8);

        match builder.build() {
            Ok(engine) => {
                eprintln!("OCR initialized from {}", model_dir.display());
                Some(engine)
            }
            Err(e) => {
                eprintln!("Failed to initialize OCR engine: {e}");
                None
            }
        }
    });

    if OCR_ENGINE.get().map(|e| e.is_some()).unwrap_or(false) {
        Ok(())
    } else {
        Err("OCR engine initialization failed".into())
    }
}

/// Run OCR on an image and extract likely motorcycle numbers.
///
/// Returns the best candidate number with confidence score.
pub fn recognize_number(img: &DynamicImage) -> Option<OcrResult> {
    let engine = OCR_ENGINE.get()?.as_ref()?;
    let results = engine.run_from_image(img).ok()?;

    if results.is_empty() {
        return None;
    }

    // Collect all detections
    let all_detections: Vec<Detection> = results
        .iter()
        .map(|r| Detection {
            text: r.text.clone(),
            confidence: r.confidence,
        })
        .collect();

    // Find the best candidate that looks like a motorcycle number
    // (numeric, possibly with some letters, reasonable length 1-4 digits typically)
    let best_number = results
        .iter()
        .filter_map(|result| {
            let digits = digits_only(&result.text);
            if (1..=4).contains(&digits.len()) {
                Some((digits, result.confidence))
            } else {
                None
            }
        })
        .max_by(|a, b| a.1.total_cmp(&b.1));

    best_number.map(|(text, confidence)| OcrResult {
        text,
        confidence,
        all_detections,
    })
}

/// Check if OCR is available
pub fn is_ocr_available() -> bool {
    OCR_ENGINE.get().map(|e| e.is_some()).unwrap_or(false)
}

fn model_dir() -> PathBuf {
    std::env::var_os("BIP_OCR_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models/ppocrv5"))
}

fn resolve_model_paths(model_dir: &Path) -> Result<ModelPaths, String> {
    let det_model = find_existing(model_dir, "detection model", &["det.onnx"])?;
    let rec_model = find_existing(model_dir, "recognition model", &["rec.onnx"])?;
    let text_line_ori_model = find_existing(
        model_dir,
        "text-line orientation model",
        &[
            "PP-LCNet_x0_25_textline_ori.onnx",
            "textline_ori.onnx",
            "text_line_ori.onnx",
        ],
    )?;
    let doc_ori_model = find_existing(
        model_dir,
        "document orientation model",
        &["PP-LCNet_x1_0_doc_ori.onnx", "doc_ori.onnx"],
    )?;
    let dictionary = find_existing(
        model_dir,
        "OCR dictionary",
        &["ppocrv5_dict.txt", "dict.txt"],
    )?;

    Ok(ModelPaths {
        det_model,
        rec_model,
        text_line_ori_model,
        doc_ori_model,
        dictionary,
    })
}

fn find_existing(model_dir: &Path, label: &str, candidates: &[&str]) -> Result<PathBuf, String> {
    for name in candidates {
        let path = model_dir.join(name);
        if path.exists() {
            return Ok(path);
        }
    }

    Err(format!(
        "OCR disabled: missing {label} in {} (tried: {}). Set BIP_OCR_MODEL_DIR to your model folder.",
        model_dir.display(),
        candidates.join(", ")
    ))
}

fn digits_only(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_digit()).collect()
}
