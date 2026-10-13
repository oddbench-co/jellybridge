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
    Home,
    Person,
    Search,
    InfoOutline,
    Star,
    ChevronLeft,
    SortByAlpha,
    Shuffle,
    MoreVert,
    Close,
    Download,
    Link,
    Add,
    Menu,
    ViewComfy,
    FilterAlt,
    ExpandMore,
    ExpandLess,
    ArrowForward,
    Movie,
    Tv,
    MusicNote,
    Book,
    Queue,
    Folder,
    ExitToApp,
    Photo,
    /// Material "theaters": the Trailer button.
    Theaters,
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
            Icon::Home => "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z",
            Icon::Person => {
                "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"
            }
            Icon::Search => {
                "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"
            }
            Icon::InfoOutline => {
                "M11 17h2v-6h-2v6zm1-15C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zM11 9h2V7h-2v2z"
            }
            Icon::Star => {
                "M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"
            }
            Icon::ChevronLeft => "M15.41 7.41L14 6l-6 6 6 6 1.41-1.41L10.83 12z",
            Icon::SortByAlpha => {
                "M14.94 4.66h-4.72l2.36-2.36zm-4.69 14.71h4.66l-2.33 2.33zM6.1 6.27L1.6 17.73h1.84l.92-2.45h5.11l.92 2.45h1.84L7.74 6.27H6.1zm-1.13 7.37l1.94-5.18 1.94 5.18H4.97zm10.76 2.5h6.12v1.59h-8.53v-1.29l5.92-8.56h-5.88v-1.6h8.3v1.26l-5.93 8.6z"
            }
            Icon::Shuffle => {
                "M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"
            }
            Icon::MoreVert => {
                "M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"
            }
            Icon::Close => {
                "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
            }
            Icon::Download => "M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z",
            Icon::Link => {
                "M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z"
            }
            Icon::Add => "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z",
            Icon::Theaters => {
                "M18 3v2h-2V3H8v2H6V3H4v18h2v-2h2v2h8v-2h2v2h2V3h-2zM8 17H6v-2h2v2zm0-4H6v-2h2v2zm0-4H6V7h2v2zm10 8h-2v-2h2v2zm0-4h-2v-2h2v2zm0-4h-2V7h2v2z"
            }
            Icon::Menu => "M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z",
            Icon::ViewComfy => {
                "M3 9h4V5H3v4zm0 5h4v-4H3v4zm5 0h4v-4H8v4zm5 0h4v-4h-4v4zM8 9h4V5H8v4zm5-4v4h4V5h-4zm5 9h4v-4h-4v4zM3 19h4v-4H3v4zm5 0h4v-4H8v4zm5 0h4v-4h-4v4zm5 0h4v-4h-4v4zm0-14v4h4V5h-4z"
            }
            Icon::FilterAlt => {
                "M4.25 5.61C6.27 8.2 10 13 10 13v6c0 .55.45 1 1 1h2c.55 0 1-.45 1-1v-6s3.72-4.8 5.74-7.39c.51-.66.04-1.61-.79-1.61H5.04c-.83 0-1.3.95-.79 1.61z"
            }
            Icon::ExpandMore => "M16.59 8.59L12 13.17 7.41 8.59 6 10l6 6 6-6z",
            Icon::ExpandLess => "M12 8l-6 6 1.41 1.41L12 10.83l4.59 4.58L18 14z",
            Icon::ArrowForward => "M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z",
            Icon::Movie => {
                "M18 4l2 4h-3l-2-4h-2l2 4h-3l-2-4H8l2 4H7L5 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V4h-4z"
            }
            Icon::Tv => {
                "M21 3H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h5v2h8v-2h5c1.1 0 1.99-.9 1.99-2L23 5c0-1.1-.9-2-2-2zm0 14H3V5h18v12z"
            }
            Icon::MusicNote => {
                "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z"
            }
            Icon::Book => {
                "M18 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM6 4h5v8l-2.5-1.5L6 12V4z"
            }
            Icon::Queue => {
                "M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-1 9h-4v4h-2v-4H9V9h4V5h2v4h4v2z"
            }
            Icon::Folder => {
                "M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z"
            }
            Icon::ExitToApp => {
                "M10.09 15.59L11.5 17l5-5-5-5-1.41 1.41L12.67 11H3v2h9.67l-2.58 2.59zM19 3H5c-1.11 0-2 .9-2 2v4h2V5h14v14H5v-4H3v4c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2z"
            }
            Icon::Photo => {
                "M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z"
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
            Icon::Home,
            Icon::Person,
            Icon::Search,
            Icon::InfoOutline,
            Icon::Star,
            Icon::ChevronLeft,
            Icon::SortByAlpha,
        ] {
            let image = rasterize(icon, 24);
            let opaque = image.pixels.iter().filter(|pixel| pixel.a() > 128).count();
            assert!(opaque > 10, "{icon:?} rasterized to {opaque} opaque pixels");
        }
    }
}
