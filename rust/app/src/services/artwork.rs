//! On-disk artwork cache: content-addressed image files with format-sniffed
//! extensions and atomic, owner-only writes.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Names the cache file for one image.
///
/// The digest is FNV-1a rather than `DefaultHasher`, whose output is
/// explicitly unspecified between Rust releases: with that, a toolchain
/// upgrade silently renamed every entry in the cache, so the whole directory
/// leaked and every image was re-fetched. Nothing here is security-sensitive
/// -- the input is a server URL and two opaque ids, and a collision costs one
/// wrong thumbnail -- so a fast, fixed, self-contained digest is the right
/// tool.
pub fn image_cache_path(
    directory: &Path,
    server: &str,
    item_id: &str,
    tag: &str,
    max_width: u32,
) -> PathBuf {
    let mut digest = FNV_OFFSET_BASIS;
    let width = max_width.to_string();
    for part in [server, item_id, tag, width.as_str()] {
        for byte in part.as_bytes() {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(FNV_PRIME);
        }
        // A separator, so ("ab", "c") and ("a", "bc") cannot collide.
        digest ^= 0xff;
        digest = digest.wrapping_mul(FNV_PRIME);
    }
    directory.join(format!("{digest:016x}.jpg"))
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// How much artwork is kept on disk. At the sizes the app requests -- 320 px
/// posters, 1600 px backdrops -- this holds several thousand images, far more
/// than a session touches, while still being a bound.
pub const IMAGE_CACHE_BUDGET: u64 = 256 * 1024 * 1024;

/// Deletes the oldest cached images until the directory fits the budget.
///
/// Nothing ever removed an entry before, so the cache grew for the life of the
/// installation: every poster of every item ever opened, plus an orphan for
/// every image whose tag changed on the server. Entries are disposable by
/// definition -- the next fetch replaces one -- so the eviction order is
/// simply oldest modification time first, which is when the entry was written.
///
/// Blocking: call it from `spawn_blocking`, never from a runtime worker.
/// Returns the number of bytes reclaimed.
pub fn prune_image_cache(directory: &Path, budget: u64) -> u64 {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();

    let total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= budget {
        return 0;
    }

    // Prune to 80% rather than exactly to the budget, so this does not run and
    // delete a handful of files on every single launch.
    let target = budget / 5 * 4;
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut remaining = total;
    let mut reclaimed = 0;
    for (_, size, path) in files {
        if remaining <= target {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            remaining -= size;
            reclaimed += size;
        }
    }
    tracing::info!(
        reclaimed_mib = reclaimed as f64 / (1024.0 * 1024.0),
        remaining_mib = remaining as f64 / (1024.0 * 1024.0),
        "artwork cache pruned"
    );
    reclaimed
}

pub fn existing_cached_image(path: &Path) -> Option<PathBuf> {
    for extension in ["jpg", "png", "webp", "gif"] {
        let candidate = path.with_extension(extension);
        if !candidate.is_file() {
            continue;
        }
        let mut header = [0_u8; 12];
        let read = File::open(&candidate)
            .and_then(|mut file| file.read(&mut header))
            .ok()?;
        let corrected = path.with_extension(image_extension(&header[..read]));
        if corrected == candidate {
            return Some(candidate);
        }
        if corrected.is_file() || fs::rename(&candidate, &corrected).is_ok() {
            return Some(corrected);
        }
    }
    None
}

pub fn image_extension(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "gif"
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    }
}

pub fn write_cached_image(path: &Path, data: &[u8]) -> Result<PathBuf, String> {
    let path = path.with_extension(image_extension(data));
    let parent = path.parent().ok_or("image cache has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".image-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(data).map_err(|error| error.to_string())?;
        // The file is synced so a crash can never leave a truncated entry
        // that `existing_cached_image` would then serve forever. The parent
        // directory deliberately is not: losing the rename loses one cache
        // entry, which the next fetch replaces.
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_filename_never_changes() {
        // Pinned to a literal on purpose. The whole point of leaving
        // DefaultHasher is that the name must survive a compiler upgrade, and
        // only a fixed expectation can catch it if it does not.
        assert_eq!(
            image_cache_path(
                Path::new("/cache"),
                "http://jellyfin.local",
                "d3d5f6a1b2c3",
                "tag-1",
                400
            ),
            Path::new("/cache/90216ab486b4413e.jpg")
        );
    }

    #[test]
    fn pruning_removes_the_oldest_entries_until_the_budget_fits() {
        let directory = std::env::temp_dir().join(format!(
            "jellybridge-prune-{}-{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&directory).expect("temp dir");

        // 4 KiB each, written oldest first and stamped so the order is not
        // left to filesystem timestamp granularity.
        let payload = vec![0_u8; 4096];
        let mut paths = Vec::new();
        for index in 0..8 {
            let path = directory.join(format!("{index:02}.jpg"));
            fs::write(&path, &payload).expect("write");
            let stamp =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + index);
            File::options()
                .write(true)
                .open(&path)
                .expect("open")
                .set_modified(stamp)
                .expect("stamp");
            paths.push(path);
        }

        // Budget 16 KiB: prunes to 80% of it, so at most 3 files may remain.
        let reclaimed = prune_image_cache(&directory, 16 * 1024);
        assert!(reclaimed >= 5 * 4096, "reclaimed only {reclaimed} bytes");
        assert!(!paths[0].is_file(), "the oldest entry survived");
        assert!(paths[7].is_file(), "the newest entry was evicted");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_cache_inside_its_budget_is_left_alone() {
        let directory = std::env::temp_dir().join(format!(
            "jellybridge-prune-{}-{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("only.jpg");
        fs::write(&path, vec![0_u8; 4096]).expect("write");

        assert_eq!(prune_image_cache(&directory, 16 * 1024), 0);
        assert!(path.is_file());

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_requested_width_is_part_of_the_name() {
        // Otherwise switching density keeps serving the file fetched for the
        // old size, and the sharper image never appears.
        let small = image_cache_path(Path::new("/cache"), "server", "item", "tag", 320);
        let large = image_cache_path(Path::new("/cache"), "server", "item", "tag", 480);
        assert_ne!(small, large, "two sizes of one image share a cache entry");
    }

    #[test]
    fn field_boundaries_are_part_of_the_name() {
        let split_one = image_cache_path(Path::new("/cache"), "server", "ab", "c", 400);
        let split_two = image_cache_path(Path::new("/cache"), "server", "a", "bc", 400);
        assert_ne!(
            split_one, split_two,
            "ids that differ only in where they are split share a cache entry"
        );
    }

    #[test]
    fn cached_images_keep_their_encoded_format() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(image_extension(b"\xff\xd8\xffrest"), "jpg");
        assert_eq!(image_extension(b"RIFF1234WEBPrest"), "webp");
    }
}
