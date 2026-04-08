//! High-performance image caching with LRU eviction and background preloading.
//!
//! This module provides a thread-safe cache for decoded image thumbnails,
//! eliminating redundant decoding when navigating through images.

use image::{DynamicImage, RgbaImage};
use lru::LruCache;
use parking_lot::Mutex;
use rayon::prelude::*;
use rapidhash::fast::RapidHasher;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::image_ops::{self, Rotation, TextOverlayConfig};

/// Cached thumbnail data ready for display.
#[derive(Clone)]
pub struct CachedImage {
    /// RGBA pixel data
    pub rgba: Arc<RgbaImage>,
}

/// Cache key combining path and rendering parameters.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    path: PathBuf,
    rotation: u8, // Rotation enum as u8
    max_size: u32,
    overlay_signature: u64,
}

impl CacheKey {
    fn new(
        path: &Path,
        rotation: Rotation,
        max_size: u32,
        text_config: Option<&TextOverlayConfig>,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            rotation: rotation as u8,
            max_size,
            overlay_signature: text_signature(text_config),
        }
    }
}

type RapidBuildHasher = BuildHasherDefault<RapidHasher<'static>>;

/// Thread-safe LRU cache for image thumbnails.
pub struct ImageCache {
    cache: Mutex<LruCache<CacheKey, CachedImage, RapidBuildHasher>>,
    max_size: u32,
}

impl ImageCache {
    /// Create a new cache with the given capacity (number of images).
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: Mutex::new(LruCache::with_hasher(
                NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(10).unwrap()),
                RapidBuildHasher::default(),
            )),
            max_size: 1200,
        }
    }

    /// Get a cached image or decode it if not present.
    pub fn get_or_decode(
        &self,
        path: &Path,
        rotation: Rotation,
        text_config: Option<&TextOverlayConfig>,
    ) -> Option<CachedImage> {
        let key = CacheKey::new(path, rotation, self.max_size, text_config);

        // Check cache first
        {
            let mut cache = self.cache.lock();
            if let Some(cached) = cache.get(&key) {
                return Some(cached.clone());
            }
        }

        // Decode image (outside lock)
        let cached = self.decode_image(path, rotation, text_config)?;

        // Store in cache
        {
            let mut cache = self.cache.lock();
            cache.put(key, cached.clone());
        }

        Some(cached)
    }

    /// Decode an image without using the cache.
    fn decode_image(
        &self,
        path: &Path,
        rotation: Rotation,
        text_config: Option<&TextOverlayConfig>,
    ) -> Option<CachedImage> {
        let mut img = image_ops::load_image(path).ok()?;

        // Apply rotation
        if rotation != Rotation::None {
            img = image_ops::rotate_image(&img, rotation);
        }

        // Generate thumbnail
        let thumb = image_ops::generate_thumbnail(&img, self.max_size);
        let mut final_img = DynamicImage::ImageRgba8(thumb);

        // Apply text overlay if configured
        if let Some(tc) = text_config {
            let filename = path.file_stem().and_then(|n| n.to_str()).unwrap_or("image");
            final_img = image_ops::overlay_text(final_img, tc, filename);
        }

        Some(CachedImage {
            rgba: Arc::new(final_img.into_rgba8()),
        })
    }

    /// Preload images in background (for adjacent images).
    /// Returns immediately, decoding happens in parallel.
    pub fn preload(
        &self,
        paths: &[PathBuf],
        rotation: Rotation,
        text_config: Option<&TextOverlayConfig>,
    ) {
        let max_size = self.max_size;

        // Check which paths need decoding
        let to_decode: Vec<PathBuf> = {
            let cache = self.cache.lock();
            paths
                .iter()
                .filter(|p| {
                    let key = CacheKey::new(p, rotation, max_size, text_config);
                    !cache.contains(&key)
                })
                .cloned()
                .collect()
        };

        if to_decode.is_empty() {
            return;
        }

        // Clone text_config once for parallel use
        let text_config = text_config.cloned();

        // Decode in parallel using rayon
        let results: Vec<_> = to_decode
            .par_iter()
            .filter_map(|path| {
                let cached = self.decode_image(path, rotation, text_config.as_ref())?;
                let key = CacheKey::new(path, rotation, max_size, text_config.as_ref());
                Some((key, cached))
            })
            .collect();

        // Store results in cache
        let mut cache = self.cache.lock();
        for (key, cached) in results {
            cache.put(key, cached);
        }
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(10)
    }
}

fn text_signature(text_config: Option<&TextOverlayConfig>) -> u64 {
    let Some(cfg) = text_config else {
        return 0;
    };

    let mut hasher = RapidHasher::default();
    cfg.text_template.hash(&mut hasher);
    cfg.position.hash(&mut hasher);
    cfg.font_size.to_bits().hash(&mut hasher);
    cfg.color.r.hash(&mut hasher);
    cfg.color.g.hash(&mut hasher);
    cfg.color.b.hash(&mut hasher);
    cfg.color.a.hash(&mut hasher);
    cfg.margin.hash(&mut hasher);
    hasher.finish()
}
