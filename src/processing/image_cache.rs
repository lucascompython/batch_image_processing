//! High-performance image caching with LRU eviction and background preloading.
//!
//! This module provides a thread-safe cache for decoded image thumbnails,
//! eliminating redundant decoding when navigating through images.

use image::{DynamicImage, RgbaImage};
use lru::LruCache;
use parking_lot::Mutex;
use rayon::prelude::*;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::image_ops::{self, Rotation, TextOverlayConfig};

/// Cached thumbnail data ready for display.
#[derive(Clone)]
pub struct CachedImage {
    /// RGBA pixel data
    pub rgba: Arc<RgbaImage>,
    /// Original image dimensions (before thumbnail)
    pub original_width: u32,
    pub original_height: u32,
}

/// Cache key combining path and rendering parameters.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    path: PathBuf,
    rotation: u8, // Rotation enum as u8
    max_size: u32,
    // Text overlay affects the cache but we simplify by not including all params
    has_text: bool,
}

impl CacheKey {
    fn new(path: &Path, rotation: Rotation, max_size: u32, has_text: bool) -> Self {
        Self {
            path: path.to_path_buf(),
            rotation: rotation as u8,
            max_size,
            has_text,
        }
    }
}

/// Thread-safe LRU cache for image thumbnails.
pub struct ImageCache {
    cache: Mutex<LruCache<CacheKey, CachedImage>>,
    max_size: u32,
}

impl ImageCache {
    /// Create a new cache with the given capacity (number of images).
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(10).unwrap()),
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
        let key = CacheKey::new(path, rotation, self.max_size, text_config.is_some());

        // Check cache first
        {
            let mut cache = self.cache.lock();
            if let Some(cached) = cache.get(&key) {
                return Some(cached.clone());
            }
        }

        // Decode image
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

        let original_width = img.width();
        let original_height = img.height();

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
            original_width,
            original_height,
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
        let paths: Vec<_> = paths.to_vec();
        let text_config = text_config.cloned();
        let max_size = self.max_size;

        // Check which paths need decoding
        let to_decode: Vec<_> = {
            let cache = self.cache.lock();
            paths
                .into_iter()
                .filter(|p| {
                    let key = CacheKey::new(p, rotation, max_size, text_config.is_some());
                    !cache.contains(&key)
                })
                .collect()
        };

        if to_decode.is_empty() {
            return;
        }

        // Decode in parallel using rayon
        let results: Vec<_> = to_decode
            .par_iter()
            .filter_map(|path| {
                let cached = self.decode_image(path, rotation, text_config.as_ref())?;
                let key = CacheKey::new(path, rotation, max_size, text_config.is_some());
                Some((key, cached))
            })
            .collect();

        // Store results in cache
        let mut cache = self.cache.lock();
        for (key, cached) in results {
            cache.put(key, cached);
        }
    }

    /// Clear the entire cache.
    pub fn clear(&self) {
        self.cache.lock().clear();
    }

    /// Invalidate cache entries for a specific path.
    pub fn invalidate(&self, path: &Path) {
        let mut cache = self.cache.lock();
        // Remove all entries for this path regardless of rotation/text config
        let keys_to_remove: Vec<_> = cache
            .iter()
            .filter(|(k, _)| k.path == path)
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys_to_remove {
            cache.pop(&key);
        }
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(10)
    }
}
