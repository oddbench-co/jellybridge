//! Design tokens: the single source of truth for color, spacing, radius, and
//! type. Drawing code references these instead of hard-coding values;
//! near-duplicate shades were deliberately collapsed onto this palette.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Style, Visuals};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// `#rrggbbaa`, unmultiplied like the CSS-style colors it was written as.
const fn rgba(hex: u32) -> Color32 {
    Color32::from_rgba_unmultiplied_const(
        (hex >> 24) as u8,
        (hex >> 16) as u8,
        (hex >> 8) as u8,
        hex as u8,
    )
}

// jellyfin-web dark theme (see docs/jellyfin-web-reference.md)
pub const JF_BG: Color32 = rgb(0x101010); // page ground
pub const JF_HEADER: Color32 = rgb(0x202020); // header bar
pub const JF_TEXT: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 204); // 80%
pub const JF_TEXT_SECONDARY: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 128); // 50%
pub const JF_TEXT_LABEL: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 179); // 70%
pub const JF_ACCENT: Color32 = rgb(0x00a4dc);
pub const JF_PLAYED_RED: Color32 = rgb(0xcc3333);
pub const JF_RAISED: Color32 = rgb(0x303030); // raised buttons
pub const JF_INPUT: Color32 = rgb(0x292929); // inputs and selects
pub const JF_RIBBON: Color32 = Color32::from_rgba_unmultiplied_const(32, 32, 32, 204);
pub const JF_PROGRESS_TRACK: Color32 = Color32::from_rgba_unmultiplied_const(51, 51, 51, 204);
pub const JF_BADGE: Color32 = Color32::from_rgba_unmultiplied_const(170, 170, 190, 51);
pub const JF_BADGE_TEXT: Color32 = rgb(0xdddddd);
/// Root font size: 93% of 16 px.
pub const JF_FONT: f32 = 14.88;
pub const JF_FONT_SECONDARY: f32 = 12.8;
pub const JF_FONT_SECTION: f32 = 22.32;
pub const JF_FONT_TITLE: f32 = 26.78;
pub const JF_FONT_SUBTITLE: f32 = 17.41;
pub const JF_RADIUS: f32 = 3.0;

// Background ramp (darkest → lightest)
pub const BG: Color32 = rgb(0x101010); // window and page ground
pub const SURFACE: Color32 = rgb(0x1b2428); // panels, cards
pub const SURFACE_RAISED: Color32 = rgb(0x232c30); // inputs, chips at rest
pub const SURFACE_HOVER: Color32 = rgb(0x2b383d); // hovered controls
pub const BORDER: Color32 = rgb(0x36454b); // hairlines, outlines
pub const BORDER_STRONG: Color32 = rgb(0x52666f); // emphasized outlines, icons

// Text ramp (brightest → faintest)
pub const TEXT_BRIGHT: Color32 = rgb(0xffffff); // buttons, hero titles
pub const TEXT_PRIMARY: Color32 = rgb(0xf5f8f9); // headings, body
pub const TEXT_SECONDARY: Color32 = rgb(0xc8d1d5); // supporting copy
pub const TEXT_MUTED: Color32 = rgb(0xaeb8bd); // labels, captions
pub const TEXT_FAINT: Color32 = rgb(0x82929a); // hints, disabled

// Accent (Jellyfin blue)
pub const ACCENT: Color32 = rgb(0x00a4dc);
pub const ACCENT_BRIGHT: Color32 = rgb(0x62ccee); // links, highlights on dark
pub const ACCENT_DEEP: Color32 = rgb(0x006f98); // pressed / selected fill

// Feedback
pub const RATING_GOLD: Color32 = rgb(0xf2b01e); // community-rating star
pub const DANGER_TEXT: Color32 = rgb(0xffb1b1);

// Player
pub const VIDEO_BG: Color32 = rgb(0x050607); // letterbox behind video
pub const SCRIM: Color32 = rgb(0x000000); // use with `with_alpha`
pub const OVERLAY: Color32 = rgba(0x171e21e5); // floating player chrome
pub const OVERLAY_RAISED: Color32 = rgba(0x2b393fef);

// Radii
pub const RADIUS_SMALL: f32 = 6.0;
pub const RADIUS_MEDIUM: f32 = 8.0;
pub const RADIUS_LARGE: f32 = 12.0;

// Type scale
pub const FONT_CAPTION: f32 = 11.0;
pub const FONT_BODY: f32 = 12.0;
pub const FONT_BODY_LARGE: f32 = 13.0;
pub const FONT_SUBTITLE: f32 = 14.0;
pub const FONT_TITLE: f32 = 16.0;
pub const FONT_HEADING: f32 = 21.0;

/// `color` at `alpha` (0–1) opacity, keeping its hue.
pub fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        (f32::from(a) * alpha.clamp(0.0, 1.0)).round() as u8,
    )
}

const BOLD_FAMILY: &str = "jellybridge-semibold";
const REGULAR_FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
const BOLD_FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Bold.ttf");

/// Regular weight text.
pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// Semibold (600) text, the weight jellyfin-web uses for every emphasis;
/// heavier weights resolve to it too.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD_FAMILY.into()))
}

/// Noto Sans, jellyfin-web's face, with egui's bundled fonts kept behind it
/// for symbols and emoji.
fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "jellybridge-regular".to_owned(),
        FontData::from_static(REGULAR_FONT).into(),
    );
    fonts.font_data.insert(
        BOLD_FAMILY.to_owned(),
        FontData::from_static(BOLD_FONT).into(),
    );
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with_fallbacks = |first: &str| {
        std::iter::once(first.to_owned())
            .chain(fallbacks.iter().cloned())
            .collect::<Vec<_>>()
    };
    fonts.families.insert(
        FontFamily::Proportional,
        with_fallbacks("jellybridge-regular"),
    );
    fonts.families.insert(
        FontFamily::Name(BOLD_FAMILY.into()),
        with_fallbacks(BOLD_FAMILY),
    );
    fonts
}

/// The egui style for the few stock widgets still in use (text fields,
/// combo boxes, scroll bars, spinners), expressed in the tokens above.
fn style() -> Style {
    let mut style = Style {
        visuals: Visuals::dark(),
        ..Style::default()
    };
    // A vertical-only page must not turn a horizontal trackpad swipe into
    // vertical scrolling: horizontal deltas belong to the shelves.
    style.always_scroll_the_only_direction = false;
    style.animation_time = 0.12;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size = egui::vec2(32.0, 32.0);
    style.spacing.combo_height = 320.0;
    // egui caps an area it has not measured yet at 600×400, which clipped
    // the sort and filter dialogs; they bound their own height instead.
    style.spacing.default_area_size = egui::vec2(4096.0, 4096.0);
    style.spacing.scroll.floating = true;
    style.spacing.scroll.bar_width = 8.0;
    style.text_styles = [
        (egui::TextStyle::Small, regular(FONT_CAPTION)),
        (egui::TextStyle::Body, regular(FONT_SUBTITLE)),
        (egui::TextStyle::Button, regular(FONT_SUBTITLE)),
        (egui::TextStyle::Heading, bold(FONT_HEADING)),
        (
            egui::TextStyle::Monospace,
            FontId::monospace(FONT_BODY_LARGE),
        ),
    ]
    .into();

    let visuals = &mut style.visuals;
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = RADIUS_MEDIUM.into();
    visuals.menu_corner_radius = RADIUS_MEDIUM.into();
    visuals.extreme_bg_color = SURFACE_RAISED;
    visuals.text_edit_bg_color = Some(SURFACE_RAISED);
    visuals.faint_bg_color = SURFACE;
    visuals.hyperlink_color = ACCENT_BRIGHT;
    visuals.error_fg_color = DANGER_TEXT;
    visuals.selection.bg_fill = ACCENT_DEEP;
    visuals.selection.stroke = Stroke::new(1.0, TEXT_BRIGHT);
    visuals.text_cursor.stroke = Stroke::new(2.0, ACCENT_BRIGHT);
    visuals.override_text_color = None;
    let radius = RADIUS_SMALL.into();
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = SURFACE;
    widgets.noninteractive.weak_bg_fill = SURFACE;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);
    widgets.noninteractive.corner_radius = radius;
    widgets.inactive.bg_fill = SURFACE_RAISED;
    widgets.inactive.weak_bg_fill = SURFACE_RAISED;
    widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    widgets.inactive.corner_radius = radius;
    widgets.hovered.bg_fill = SURFACE_HOVER;
    widgets.hovered.weak_bg_fill = SURFACE_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    widgets.hovered.corner_radius = radius;
    widgets.hovered.expansion = 0.0;
    widgets.active.bg_fill = ACCENT_DEEP;
    widgets.active.weak_bg_fill = ACCENT_DEEP;
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    widgets.active.corner_radius = radius;
    widgets.active.expansion = 0.0;
    widgets.open.bg_fill = SURFACE_HOVER;
    widgets.open.weak_bg_fill = SURFACE_HOVER;
    widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.open.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    widgets.open.corner_radius = radius;
    style
}

/// Installs the fonts and the style. Called once, before the first frame.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.set_style_of(egui::Theme::Dark, style());
    ctx.set_theme(egui::Theme::Dark);
}
