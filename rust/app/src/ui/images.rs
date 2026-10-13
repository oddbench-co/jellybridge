//! Artwork: what a row shows ([`Art`]), the bounded texture cache that holds
//! it, and the fetch → disk cache → decode pipeline behind it.
//!
//! Rows carry an [`Art`] (cache key plus where to get it) instead of pixels.
//! Drawing asks the cache for the key; a miss is recorded as *wanted*, and the
//! window starts the fetch after the frame. So artwork is requested only for
//! what is actually drawn -- a 500-item library no longer fetches 500
//! posters to show the first 20 -- and nothing needs patching into rows when
//! an image lands: the next frame finds it in the cache.

use std::collections::{HashMap, HashSet};

use egui::{ColorImage, TextureHandle, TextureId, TextureOptions};

use crate::app::AppContext;
use crate::features::detail::ImageRef;
use crate::services::artwork::{existing_cached_image, image_cache_path, write_cached_image};

/// How many bytes of decoded artwork stay resident (as textures).
///
/// A 320 px poster decodes to ~590 KB and a 1600 px backdrop to ~5.7 MB, so
/// this holds a full 120-card grid page (~71 MB) with room to spare, which is
/// what keeps scrolling back from re-decoding.
pub const IMAGE_CACHE_BUDGET: usize = 96 * 1024 * 1024;

/// Artwork is drawn smaller than it was fetched (cards ask for the size
/// jellyfin-web asks for), so minification samples a mipmap like a browser's
/// image scaler instead of skipping texels and shimmering.
const ARTWORK_TEXTURE: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

/// Artwork a row shows: its cache identity and where to fetch it from.
#[derive(Clone, Debug, PartialEq)]
pub struct Art {
    pub key: String,
    pub source: ArtSource,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArtSource {
    /// Detail-page artwork: hero images, shelves, cast photos.
    Detail { image: ImageRef, max_width: u32 },
    /// The signed-in user's picture.
    User { tag: String },
    /// An image asked for exactly as jellyfin-web asks (its detail page's
    /// backdrop and logo): an optional width cap and a quality, nothing else.
    Web {
        image: ImageRef,
        max_width: Option<u32>,
        quality: u8,
    },
    /// A card image filled to the exact size jellyfin-web asks for.
    Card {
        image: ImageRef,
        fill_width: u32,
        fill_height: u32,
    },
}

/// How much decoded artwork goes to the GPU in one frame. An upload with its
/// mipmap chain costs about 2 ms per megabyte here; this keeps a frame of
/// uploads inside a 240 Hz refresh.
const UPLOAD_BYTES_PER_FRAME: usize = 1024 * 1024;

/// Decoded artwork as GPU textures, bounded by the bytes it holds.
///
/// Entry counts do not describe this cache -- entries differ by an order of
/// magnitude in size -- so eviction is by bytes, least recently used first.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<String, Entry>,
    bytes: usize,
    /// Monotonic use counter. Cheaper than maintaining a linked list for a
    /// cache that only evicts when it is over budget.
    clock: u64,
    /// Highest `bytes` ever reached, reported when it grows, so a profiling
    /// run records the peak without sampling.
    reported_high_water: usize,
    /// Fetches in flight, so a key drawn every frame is requested once.
    pending: HashSet<String>,
    /// Keys whose fetch failed. Not retried until the cache is cleared, or
    /// a broken image would be re-requested on every frame it is visible.
    failed: HashSet<String>,
    /// Misses drawn this frame, drained by the window after the frame.
    wanted: Vec<Art>,
    /// Decoded images waiting for the GPU: [`ImageCache::upload_ready`]
    /// takes them a few per frame, so a burst of arrivals (a shelf scrolled
    /// into view) never stretches one frame past the refresh interval.
    ready: std::collections::VecDeque<(String, ColorImage, bool)>,
}

struct Entry {
    texture: TextureHandle,
    bytes: usize,
    used: u64,
}

impl ImageCache {
    /// The texture for `art`, or `None` while it loads (the miss is queued).
    pub fn texture(&mut self, art: &Art) -> Option<TextureId> {
        if let Some(texture) = self.get(&art.key) {
            return Some(texture);
        }
        if !self.pending.contains(&art.key) && !self.failed.contains(&art.key) {
            self.pending.insert(art.key.clone());
            self.wanted.push(art.clone());
        }
        None
    }

    /// Whether `key` could not be fetched or decoded.
    pub fn is_failed(&self, key: &str) -> bool {
        self.failed.contains(key)
    }

    /// The texture's size in pixels, if resident -- for aspect-aware layout.
    pub fn size(&self, key: &str) -> Option<[usize; 2]> {
        self.entries.get(key).map(|entry| entry.texture.size())
    }

    fn get(&mut self, key: &str) -> Option<TextureId> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.used = clock;
        Some(entry.texture.id())
    }

    /// Takes this frame's misses; each is requested exactly once.
    pub fn take_wanted(&mut self) -> Vec<Art> {
        std::mem::take(&mut self.wanted)
    }

    /// Uploads a decoded image and makes it resident.
    pub fn finish(
        &mut self,
        ctx: &egui::Context,
        key: String,
        image: Option<ColorImage>,
        mipmaps: bool,
    ) {
        let Some(image) = image else {
            self.pending.remove(&key);
            self.failed.insert(key);
            return;
        };
        // Stays pending until uploaded, so it is not requested again.
        self.ready.push_back((key, image, mipmaps));
        ctx.request_repaint();
    }

    /// Uploads queued images, at most [`UPLOAD_BYTES_PER_FRAME`] of them a
    /// frame (and always the first, whatever its size). Returns whether
    /// more wait, so the window draws again for them.
    pub fn upload_ready(&mut self, ctx: &egui::Context) -> bool {
        let mut budget = UPLOAD_BYTES_PER_FRAME;
        let mut first = true;
        while let Some((_, image, _)) = self.ready.front() {
            let bytes = image.width() * image.height() * 4;
            if !first && bytes > budget {
                break;
            }
            first = false;
            budget = budget.saturating_sub(bytes);
            let Some((key, image, mipmaps)) = self.ready.pop_front() else { break };
            self.pending.remove(&key);
            self.upload(ctx, key, image, mipmaps);
        }
        !self.ready.is_empty()
    }

    fn upload(&mut self, ctx: &egui::Context, key: String, image: ColorImage, mipmaps: bool) {
        let base = image.width() * image.height() * 4;
        let (bytes, options) = if mipmaps {
            // A mipmap chain adds a third on top of the base level.
            (base * 4 / 3, ARTWORK_TEXTURE)
        } else {
            (base, TextureOptions::LINEAR)
        };
        let texture = ctx.load_texture(&key, image, options);
        self.insert(key, texture, bytes);
    }

    fn insert(&mut self, key: String, texture: TextureHandle, bytes: usize) {
        self.clock += 1;
        let entry = Entry {
            texture,
            bytes,
            used: self.clock,
        };
        if let Some(previous) = self.entries.insert(key, entry) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.bytes += bytes;
        self.report_growth();
        self.evict_to_budget();
    }

    /// Drops least-recently-used entries until the cache fits. Pruning to 80%
    /// of the budget rather than exactly to it keeps a cache sitting on the
    /// limit from evicting on every single insert. Dropping the handle frees
    /// the texture on the GPU.
    fn evict_to_budget(&mut self) {
        if self.bytes <= IMAGE_CACHE_BUDGET {
            return;
        }
        let target = IMAGE_CACHE_BUDGET / 5 * 4;
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.used, key.clone()))
            .collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        for (_, key) in by_age {
            if self.bytes <= target {
                break;
            }
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
        self.reported_high_water = self.bytes;
    }

    /// Drops everything (used on logout). In-flight fetches still land, but
    /// into an empty cache, which is harmless.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.pending.clear();
        self.ready.clear();
        self.failed.clear();
        self.wanted.clear();
    }

    /// Logs each new 8 MiB high-water mark. Reporting every insert would spam
    /// a profiling log with hundreds of lines; reporting nothing would leave
    /// the peak unmeasurable.
    fn report_growth(&mut self) {
        const STEP: usize = 8 * 1024 * 1024;
        if self.bytes < self.reported_high_water + STEP {
            return;
        }
        self.reported_high_water = self.bytes - self.bytes % STEP;
        tracing::info!(
            mib = self.bytes as f64 / (1024.0 * 1024.0),
            entries = self.entries.len(),
            "image_cache_high_water"
        );
    }
}

/// Starts fetching one artwork: disk cache probe, network fetch under the
/// download permit, disk write, then decode -- all off the UI thread -- and
/// hands the pixels to the window.
pub fn fetch(context: &AppContext, art: Art) {
    let Some(api) = context.api() else {
        let key = art.key;
        context
            .ui
            .update(move |state| state.images.finish(&state.ctx.clone(), key, None, true));
        return;
    };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let server = api.session().server_url.as_str().to_owned();
        let (cache_id, tag, width) = match &art.source {
            // Keyed on type and index too: an item's backdrop, logo and
            // poster share its id and must not share a disk entry.
            ArtSource::Detail { image, max_width } => (
                format!(
                    "{}-{}-{}",
                    image.item_id,
                    image.image_type.jellyfin_name(),
                    image.index.unwrap_or(0)
                ),
                image.tag.clone(),
                *max_width,
            ),
            ArtSource::User { tag } => (format!("user-{tag}"), tag.clone(), 0),
            ArtSource::Web {
                image,
                max_width,
                quality,
            } => (
                format!(
                    "{}-{}-{}-web{}q{quality}",
                    image.item_id,
                    image.image_type.jellyfin_name(),
                    image.index.unwrap_or(0),
                    max_width.unwrap_or(0),
                ),
                image.tag.clone(),
                max_width.unwrap_or(0),
            ),
            ArtSource::Card {
                image,
                fill_width,
                fill_height,
            } => (
                format!(
                    "{}-{}-{}-fill{}x{}",
                    image.item_id,
                    image.image_type.jellyfin_name(),
                    image.index.unwrap_or(0),
                    fill_width,
                    fill_height
                ),
                image.tag.clone(),
                *fill_width,
            ),
        };
        let path = image_cache_path(&ctx.image_cache, &server, &cache_id, &tag, width);
        // `existing_cached_image` and `write_cached_image` are blocking
        // std::fs, and the write fsyncs; running them directly on a runtime
        // worker stalls every other task sharing that thread.
        let probe = path.clone();
        let cached = tokio::task::spawn_blocking(move || existing_cached_image(&probe))
            .await
            .ok()
            .flatten();
        let file = if let Some(path) = cached {
            Ok(path)
        } else {
            // The permit bounds concurrent downloads, so it is taken here
            // rather than at the top of the task: artwork already on disk
            // must not wait behind six in-flight fetches for a permit it
            // then does not use.
            let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
                return;
            };
            let bytes = match &art.source {
                ArtSource::Detail { image, max_width } => {
                    api.fetch_detail_image(image, *max_width).await
                }
                ArtSource::Web {
                    image,
                    max_width,
                    quality,
                } => api.fetch_web_image(image, *max_width, *quality).await,
                ArtSource::User { tag } => api.fetch_user_image(tag).await,
                ArtSource::Card {
                    image,
                    fill_width,
                    fill_height,
                } => api.fetch_card_image(image, *fill_width, *fill_height).await,
            };
            match bytes {
                Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => {
                    tokio::task::spawn_blocking(move || write_cached_image(&path, &bytes))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()))
                }
                Ok(_) => Err("artwork exceeded 20 MiB".to_owned()),
                Err(error) => Err(error.to_string()),
            }
        };
        // Decoding is the expensive part (a 1600 px backdrop is ~20 ms), so it
        // runs on the blocking pool; the UI thread only uploads the texture.
        let decoded = match file {
            Ok(path) => tokio::task::spawn_blocking(move || decode(&path))
                .await
                .unwrap_or_else(|error| Err(error.to_string())),
            Err(error) => Err(error),
        };
        let image = decoded
            .inspect_err(|error| tracing::debug!(key = %art.key, %error, "artwork unavailable"))
            .ok();
        let key = art.key;
        // A full-window backdrop is drawn at its own size: a mipmap chain
        // would only cost memory.
        let mipmaps = !matches!(art.source, ArtSource::Web { .. });
        ctx.ui.update(move |state| {
            let egui_ctx = state.ctx.clone();
            state.images.finish(&egui_ctx, key, image, mipmaps);
        });
    });
}

fn decode(path: &std::path::Path) -> Result<ColorImage, String> {
    let image = image::open(path).map_err(|error| error.to_string())?;
    let size = [image.width() as usize, image.height() as usize];
    // Posters and backdrops are opaque JPEGs; only logos carry alpha.
    Ok(if image.color().has_alpha() {
        ColorImage::from_rgba_unmultiplied(size, image.to_rgba8().as_raw())
    } else {
        ColorImage::from_rgb(size, image.to_rgb8().as_raw())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A texture of a known decoded size, for the cache tests.
    fn texture_of(ctx: &egui::Context, mib: usize) -> (TextureHandle, usize) {
        let bytes = mib * 1024 * 1024;
        let image = ColorImage::filled([1, 1], egui::Color32::BLACK);
        (
            ctx.load_texture("test", image, TextureOptions::LINEAR),
            bytes,
        )
    }

    #[test]
    fn the_image_cache_evicts_the_least_recently_used_entry_first() {
        let ctx = egui::Context::default();
        let mut cache = ImageCache::default();
        for index in 0..12 {
            let (texture, bytes) = texture_of(&ctx, 8);
            cache.insert(format!("cold-{index}"), texture, bytes);
        }
        // 96 MB is exactly the budget, so nothing has been evicted yet.
        assert_eq!(cache.entries.len(), 12);

        // Touch the first one so it is no longer the oldest, then overflow.
        assert!(cache.get("cold-0").is_some());
        let (texture, bytes) = texture_of(&ctx, 8);
        cache.insert("hot".to_owned(), texture, bytes);

        assert!(
            cache.bytes <= IMAGE_CACHE_BUDGET,
            "cache is {} bytes, over its budget",
            cache.bytes
        );
        assert!(
            cache.entries.contains_key("hot"),
            "the new entry was evicted"
        );
        assert!(
            cache.entries.contains_key("cold-0"),
            "a recently used entry was evicted before older ones"
        );
        assert!(
            !cache.entries.contains_key("cold-1"),
            "the least recently used entry survived"
        );
    }

    #[test]
    fn the_image_cache_accounts_bytes_across_replacement_and_clear() {
        let ctx = egui::Context::default();
        let mut cache = ImageCache::default();
        let (texture, bytes) = texture_of(&ctx, 8);
        cache.insert("one".to_owned(), texture, bytes);
        let (texture, bytes) = texture_of(&ctx, 8);
        cache.insert("two".to_owned(), texture, bytes);
        assert_eq!(cache.bytes, 16 * 1024 * 1024);

        // Replacing an entry must not double-count the one it displaced.
        let (texture, bytes) = texture_of(&ctx, 4);
        cache.insert("one".to_owned(), texture, bytes);
        assert_eq!(cache.bytes, 12 * 1024 * 1024);
        assert_eq!(cache.entries.len(), 2);

        cache.clear();
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn a_missing_image_is_requested_once_and_a_failed_one_is_not_retried() {
        let ctx = egui::Context::default();
        let mut cache = ImageCache::default();
        let art = Art {
            key: "poster".to_owned(),
            source: ArtSource::Card {
                image: crate::features::detail::ImageRef {
                    item_id: "item".to_owned(),
                    image_type: crate::features::detail::ImageType::Primary,
                    index: None,
                    tag: "tag".to_owned(),
                },
                fill_width: 320,
                fill_height: 480,
            },
        };
        assert!(cache.texture(&art).is_none());
        assert!(cache.texture(&art).is_none());
        assert_eq!(
            cache.take_wanted(),
            vec![art.clone()],
            "drawn twice, wanted once"
        );

        cache.finish(&ctx, art.key.clone(), None, true);
        assert!(cache.texture(&art).is_none());
        assert!(
            cache.take_wanted().is_empty(),
            "a failed fetch is not retried"
        );

        cache.clear();
        assert!(cache.texture(&art).is_none());
        assert_eq!(cache.take_wanted().len(), 1, "clearing allows a retry");

        cache.finish(
            &ctx,
            art.key.clone(),
            Some(ColorImage::filled([2, 3], egui::Color32::WHITE)),
            true,
        );
        // Decoded but not yet uploaded: still pending, not asked for again.
        assert!(cache.texture(&art).is_none());
        assert!(cache.take_wanted().is_empty());
        assert!(!cache.upload_ready(&ctx));
        assert!(cache.texture(&art).is_some());
        assert_eq!(cache.size("poster"), Some([2, 3]));
    }

    #[test]
    fn uploads_are_spread_over_frames() {
        let ctx = egui::Context::default();
        let mut cache = ImageCache::default();
        // Three 600 KB images: one fits a frame's budget, two do not.
        for key in ["a", "b", "c"] {
            cache.finish(&ctx, key.to_owned(), Some(ColorImage::filled([300, 500], egui::Color32::WHITE)), true);
        }
        assert!(cache.upload_ready(&ctx), "more wait after the first frame");
        assert!(cache.size("a").is_some() && cache.size("b").is_none());
        assert!(cache.upload_ready(&ctx));
        assert!(!cache.upload_ready(&ctx), "the third frame empties the queue");
        assert!(cache.size("c").is_some());
    }
}
