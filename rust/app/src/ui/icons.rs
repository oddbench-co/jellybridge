//! Material icons (Apache License 2.0), the set Jellyfin's web client uses.
//!
//! Each icon is rasterized once per pixel size, white on transparent, and
//! tinted when drawn, so one texture serves every color. The cache lives in
//! the egui context and holds a few dozen small textures at most.

use std::collections::HashMap;

use egui::{Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    ArrowBack,
    Play,
    Pause,
    Replay,
    Forward,
    SkipPrevious,
    SkipNext,
    VolumeUp,
    VolumeOff,
    Audiotrack,
    Subtitles,
    Settings,
    Check,
    ChevronRight,
    Favorite,
    Fullscreen,
    FullscreenExit,
}

impl Icon {
    /// Path data on a 24×24 viewbox.
    fn path(self) -> &'static str {
        match self {
            Icon::ArrowBack => "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z",
            Icon::Play => "M8 5v14l11-7z",
            Icon::Pause => "M6 19h4V5H6v14zm8-14v14h4V5h-4z",
            Icon::Replay => {
                "M12 5V1L7 6l5 5V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z"
            }
            Icon::Forward => {
                "M12 5V1l5 5-5 5V7c-3.31 0-6 2.69-6 6s2.69 6 6 6 6-2.69 6-6h2c0 4.42-3.58 8-8 8s-8-3.58-8-8 3.58-8 8-8z"
            }
            Icon::SkipPrevious => "M6 6h2v12H6zm3.5 6l8.5 6V6z",
            Icon::SkipNext => "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z",
            Icon::VolumeUp => {
                "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"
            }
            Icon::VolumeOff => {
                "M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z"
            }
            Icon::Audiotrack => {
                "M12 3v9.28c-.47-.17-.97-.28-1.5-.28C8.01 12 6 14.01 6 16.5S8.01 21 10.5 21c2.31 0 4.2-1.75 4.45-4H15V6h4V3h-7z"
            }
            Icon::Subtitles => {
                "M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zM4 12h4v2H4v-2zm10 6H4v-2h10v2zm6 0h-4v-2h4v2zm0-4H10v-2h10v2z"
            }
            Icon::Settings => {
                "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"
            }
            Icon::Check => "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z",
            Icon::ChevronRight => "M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z",
            Icon::Favorite => {
                "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"
            }
            Icon::Fullscreen => {
                "M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z"
            }
            Icon::FullscreenExit => {
                "M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z"
            }
        }
    }
}

#[derive(Clone, Default)]
struct IconTextures(HashMap<(Icon, u32), TextureHandle>);

/// Draws `icon` filling `rect` (square, in points), tinted `color`.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let ctx = painter.ctx();
    let pixels = (rect.width() * ctx.pixels_per_point()).round().max(1.0) as u32;
    let id = egui::Id::new("jellybridge-icon-textures");
    // Looked up and inserted in two steps: `load_texture` takes the context
    // lock that `data_mut` is holding, so creating it inside would deadlock.
    let cached = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<IconTextures>(id)
            .0
            .get(&(icon, pixels))
            .map(TextureHandle::id)
    });
    let texture = cached.unwrap_or_else(|| {
        let handle = ctx.load_texture(
            format!("icon-{icon:?}-{pixels}"),
            rasterize(icon, pixels),
            TextureOptions::LINEAR,
        );
        let texture = handle.id();
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<IconTextures>(id)
                .0
                .insert((icon, pixels), handle);
        });
        texture
    });
    painter.image(
        texture,
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        color,
    );
}

/// Draws `icon` at `size` points, centered on `center`.
pub fn paint_centered(painter: &Painter, icon: Icon, center: Pos2, size: f32, color: Color32) {
    paint(
        painter,
        icon,
        Rect::from_center_size(center, egui::vec2(size, size)),
        color,
    );
}

fn rasterize(icon: Icon, pixels: u32) -> ColorImage {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="{}"/></svg>"##,
        icon.path()
    );
    let size = [pixels as usize, pixels as usize];
    let (Ok(tree), Some(mut pixmap)) = (
        resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()),
        resvg::tiny_skia::Pixmap::new(pixels, pixels),
    ) else {
        return ColorImage::filled(size, Color32::TRANSPARENT);
    };
    let scale = pixels as f32 / 24.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    ColorImage::from_rgba_premultiplied(size, pixmap.data())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_to_visible_pixels() {
        for icon in [
            Icon::ArrowBack,
            Icon::Play,
            Icon::Pause,
            Icon::Replay,
            Icon::Forward,
            Icon::SkipPrevious,
            Icon::SkipNext,
            Icon::VolumeUp,
            Icon::VolumeOff,
            Icon::Audiotrack,
            Icon::Subtitles,
            Icon::Settings,
            Icon::Check,
            Icon::ChevronRight,
            Icon::Favorite,
            Icon::Fullscreen,
            Icon::FullscreenExit,
        ] {
            let image = rasterize(icon, 24);
            let opaque = image.pixels.iter().filter(|pixel| pixel.a() > 128).count();
            assert!(opaque > 10, "{icon:?} rasterized to {opaque} opaque pixels");
        }
    }
}
