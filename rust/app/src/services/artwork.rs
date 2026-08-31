//! On-disk artwork cache: content-addressed image files with format-sniffed
//! extensions and atomic, owner-only writes.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub fn image_cache_path(directory: &Path, server: &str, item_id: &str, tag: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    server.hash(&mut hasher);
    item_id.hash(&mut hasher);
    tag.hash(&mut hasher);
    directory.join(format!("{:016x}.jpg", hasher.finish()))
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
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())?;
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
    fn cached_images_keep_their_encoded_format() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(image_extension(b"\xff\xd8\xffrest"), "jpg");
        assert_eq!(image_extension(b"RIFF1234WEBPrest"), "webp");
    }
}
