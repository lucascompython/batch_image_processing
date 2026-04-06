//! JPEG decode/encode benchmark comparing three pipelines:
//! 1. `image` crate (internally uses zune-jpeg decoder + jpeg-encoder encoder)
//! 2. Manual `zune-jpeg` decoder + `jpeg-encoder` encoder
//! 3. `turbojpeg` (libjpeg-turbo C wrapper — hardware SIMD)
//!
//! Usage: cargo run --release --bin jpeg_bench -- <path_to_jpeg>

use std::path::PathBuf;
use std::time::Instant;

const ITERATIONS: u32 = 10;
const QUALITY: u8 = 70;

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("Usage: jpeg_bench <path_to_jpeg>");

    let raw_bytes = std::fs::read(&path).expect("Failed to read input file");
    let file_size_mb = raw_bytes.len() as f64 / (1024.0 * 1024.0);

    println!("=== JPEG Benchmark ===");
    println!(
        "Input: {} ({:.2} MB)",
        path.file_name().unwrap().to_str().unwrap(),
        file_size_mb
    );
    println!("Iterations: {ITERATIONS}");
    println!("Quality: {QUALITY}%");
    println!();

    // ── 1. image crate (decode + encode) ─────────────────────────────────
    {
        println!("--- image crate (zune-jpeg decode + jpeg-encoder encode) ---");

        // Decode benchmark
        let start = Instant::now();
        let mut decoded = None;
        for _ in 0..ITERATIONS {
            let cursor = std::io::Cursor::new(&raw_bytes);
            let reader = image::ImageReader::with_format(cursor, image::ImageFormat::Jpeg);
            let img = reader.decode().expect("image decode failed");
            decoded = Some(img);
        }
        let decode_elapsed = start.elapsed();
        let decode_avg_ms = decode_elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;

        // Encode benchmark
        let img = decoded.unwrap();
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let mut out = Vec::new();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY);
            img.write_with_encoder(encoder)
                .expect("image encode failed");
        }
        let encode_elapsed = start.elapsed();
        let encode_avg_ms = encode_elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;

        println!("  Decode: {decode_avg_ms:.2} ms avg");
        println!("  Encode: {encode_avg_ms:.2} ms avg");
        println!("  Total:  {:.2} ms avg", decode_avg_ms + encode_avg_ms);
        println!();
    }

    // ── 2. Manual zune-jpeg + jpeg-encoder ───────────────────────────────
    {
        println!("--- zune-jpeg decode + jpeg-encoder encode (manual) ---");

        // Decode benchmark
        let start = Instant::now();
        let mut last_pixels: Vec<u8> = Vec::new();
        let mut last_w = 0usize;
        let mut last_h = 0usize;
        for _ in 0..ITERATIONS {
            let options = zune_core::options::DecoderOptions::default()
                .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGB);
            let cursor = zune_core::bytestream::ZCursor::new(&raw_bytes[..]);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(cursor, options);
            decoder
                .decode_headers()
                .expect("zune decode_headers failed");
            let (w, h) = decoder.dimensions().expect("no dimensions");
            let pixels = decoder.decode().expect("zune decode failed");
            last_pixels = pixels;
            last_w = w;
            last_h = h;
        }
        let decode_elapsed = start.elapsed();
        let decode_avg_ms = decode_elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;

        // Encode benchmark
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let encoder = jpeg_encoder::Encoder::new(Vec::new(), QUALITY);
            encoder
                .encode(
                    &last_pixels,
                    last_w as u16,
                    last_h as u16,
                    jpeg_encoder::ColorType::Rgb,
                )
                .expect("jpeg-encoder encode failed");
        }
        let encode_elapsed = start.elapsed();
        let encode_avg_ms = encode_elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;

        println!("  Decode: {decode_avg_ms:.2} ms avg");
        println!("  Encode: {encode_avg_ms:.2} ms avg");
        println!("  Total:  {:.2} ms avg", decode_avg_ms + encode_avg_ms);
        println!();
    }

    // ── 3. turbojpeg (libjpeg-turbo) ─────────────────────────────────────
    {
        println!("--- turbojpeg (libjpeg-turbo — full C SIMD) ---");

        // Decode benchmark
        let start = Instant::now();
        let mut last_image: Option<turbojpeg::Image<Vec<u8>>> = None;
        for _ in 0..ITERATIONS {
            let img: turbojpeg::Image<Vec<u8>> =
                turbojpeg::decompress(&raw_bytes, turbojpeg::PixelFormat::RGB)
                    .expect("turbojpeg decompress failed");
            last_image = Some(img);
        }
        let decode_elapsed = start.elapsed();
        let decode_avg_ms = decode_elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;

        // Encode benchmark
        let img = last_image.unwrap();
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let img_ref = img.as_deref();
            turbojpeg::compress(img_ref, QUALITY as i32, turbojpeg::Subsamp::Sub2x2)
                .expect("turbojpeg compress failed");
        }
        let encode_elapsed = start.elapsed();
        let encode_avg_ms = encode_elapsed.as_secs_f64() * 1000.0 / ITERATIONS as f64;

        println!("  Decode: {decode_avg_ms:.2} ms avg");
        println!("  Encode: {encode_avg_ms:.2} ms avg");
        println!("  Total:  {:.2} ms avg", decode_avg_ms + encode_avg_ms);
        println!();
    }

    println!("=== Done ===");
}
