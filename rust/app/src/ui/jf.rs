//! jellyfin-web's building blocks: responsive card sizes, the card itself
//! (artwork, indicators, progress, hover actions, centred text), section
//! titles with paging buttons, and the round icon buttons of headers and
//! detail pages. Measurements are in `docs/jellyfin-web-reference.md`.

use egui::{Color32, Id, Rect, Response, Sense, Ui, pos2, vec2};

use super::icons::{self, Icon};
use super::images::{Art, ImageCache};
use super::theme;
use super::widgets;

/// `.cardPadder`, under every card image: a dark tile with a soft shadow
/// (`box-shadow: 0 .0725em .29em rgba(0,0,0,.37)`), radius 0.2em.
/// `card` is the card's whole box: jellyfin's `.card` clips paint to it,
/// which hides the shadow left of the image (the box starts there) but keeps
/// it in the gap to the right, the margin above and under the image.
pub fn paint_card_padder(painter: &egui::Painter, image: Rect, card: Rect) {
    let shadow = Color32::from_black_alpha(94);
    painter
        .with_clip_rect(card.intersect(painter.clip_rect()))
        .add(
            egui::epaint::RectShape::filled(
                image.translate(vec2(0.0, 0.0725 * EM)),
                CARD_RADIUS,
                shadow,
            )
            .with_blur_width(CARD_SHADOW_BLUR),
        );
    painter.rect_filled(image, CARD_RADIUS, Color32::from_gray(36));
}

/// The card's corner radius, 0.2em.
pub const CARD_RADIUS: f32 = 0.2 * EM;
/// epaint's blur width matching the CSS blur radius .29em.
const CARD_SHADOW_BLUR: f32 = 2.5 * 0.29 * EM;

/// `cardBox` margins: the gap after a card is part of its width.
pub const CARD_TOP: f32 = layout_unit(8.928);
pub const CARD_GAP: f32 = 17.856;
pub const CARD_BOTTOM: f32 = 26.784;
/// `.cardText-first` as Chrome lays it out: plain text, or taller when the
/// line is a link (its inline button raises the line box).
const TITLE_ROW: f32 = 24.53;
const TITLE_ROW_LINK: f32 = 26.5;
/// `.cardText-secondary` (86% type), plain or a link.
const SECONDARY_ROW: f32 = 18.8;
const SECONDARY_ROW_LINK: f32 = 19.73;
/// Baselines, from the box that holds the text, as Chrome places them
/// (matched to its glyphs): a card title under its image, a card's second
/// line under its row's top, and a section title in a plain or linked row.
const TITLE_BASELINE: f32 = 21.65;
/// A plain line's box is shorter, so its text sits half the difference
/// higher than a link's.
const PLAIN_TITLE_RAISE: f32 = (TITLE_ROW_LINK - TITLE_ROW) / 2.0;
const SECONDARY_BASELINE: f32 = 14.6;
const PLAIN_SECONDARY_RAISE: f32 = (SECONDARY_ROW_LINK - SECONDARY_ROW) / 2.0;
const SECTION_BASELINE: f32 = 33.71;
const SECTION_BASELINE_LINKED: f32 = 41.92;
/// A section titled by a bare `h2.sectionTitle` (the library Suggestions
/// tab): the heading's margin and line, then the cards.
pub const SECTION_HEADING: f32 = 48.703;
const SECTION_BASELINE_HEADING: f32 = 42.14;
/// A section title row (`.sectionTitle-cards`: 0.5em above, a 1.35 line,
/// 0.2em below, at 22.32 px) and where its text sits.
pub const SECTION_TITLE: f32 = 45.72;
/// A section title that links somewhere (`.sectionTitleContainer-cards`:
/// 1.25em above a link button with a 0.35em margin), and its text centre.
pub const SECTION_TITLE_LINKED: f32 = 56.5;
/// The link's chevron: 1.5em, 3.89 px under the title's line top.
pub const SECTION_CHEVRON: f32 = 22.31;
pub const SECTION_CHEVRON_CENTER: f32 = 33.64;

/// jellyfin-web's root font size: 93% of 16 px.
pub const EM: f32 = 14.88;

/// A length as Chrome lays it out: in whole 1/64 px LayoutUnits, rounded
/// down. Em-based offsets land a fraction of a device pixel higher than
/// the exact value, which decides which pixel row an edge snaps to.
pub const fn layout_unit(css_px: f32) -> f32 {
    (css_px * 64.0) as i32 as f32 / 64.0
}

/// jellyfin-web's header at a window width. It changes at its 100em
/// (1600 px) breakpoint: more padding, and the tabs move up into the
/// button row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderMetrics {
    pub wide: bool,
    /// The button row's height (`.headerTop`).
    pub height: f32,
    /// The row's side padding, before the buttons' own 0.29em margins.
    pub inset: f32,
    pub button_top: f32,
    pub tab_font: f32,
    /// A tab's height: its 1.25 line plus 1.5em above and below.
    pub tab_height: f32,
    /// Where the tabs start: under the row, or up inside it.
    pub tabs_top: f32,
    /// Where a page with tabs starts its content (`padding-top`).
    pub tabbed_content_top: f32,
}

pub const HEADER_BUTTON: f32 = 41.4;
/// One header button and its 0.29em margins either side.
pub const HEADER_STEP: f32 = 50.03;
pub const HEADER_MARGIN: f32 = 4.315;

pub fn header_metrics(width: f32) -> HeaderMetrics {
    if width >= 1600.0 {
        let tab_font = EM;
        HeaderMetrics {
            wide: true,
            height: 65.16,
            inset: 0.8 * EM,
            button_top: 0.8 * EM,
            tab_font,
            tab_height: 4.25 * tab_font,
            tabs_top: 65.16 - 4.3 * EM,
            tabbed_content_top: layout_unit(6.7 * EM),
        }
    } else {
        let tab_font = EM * 0.835;
        HeaderMetrics {
            wide: false,
            height: 57.58,
            inset: 0.0,
            button_top: 8.09,
            tab_font,
            tab_height: 4.25 * tab_font,
            tabs_top: 57.58,
            tabbed_content_top: layout_unit(7.5 * EM),
        }
    }
}

/// The width CSS `vw` units and media queries measure: the whole window,
/// even the part a page scrollbar covers.
pub fn window_width(ui: &egui::Ui) -> f32 {
    ui.ctx().content_rect().width()
}

/// The page's side padding: 3.3% of the page's width (which excludes the
/// page scrollbar).
pub fn page_padding(page_width: f32) -> f32 {
    layout_unit(page_width * 0.033)
}

/// White at 80%: jellyfin-web's toolbar icons and paging text.
pub const TEXT_80: Color32 = theme::JF_TEXT;
/// A library page's toolbar: 1em padding round a row of header-sized
/// buttons.
pub const TOOLBAR_PADDING: f32 = layout_unit(EM);
pub const TOOLBAR_HEIGHT: f32 = 2.0 * TOOLBAR_PADDING + 41.375;
/// Its buttons, their margin either side, and the step between them.
pub const TOOLBAR_BUTTON: f32 = 41.359;
pub const TOOLBAR_MARGIN: f32 = 4.3125;
pub const TOOLBAR_STEP: f32 = TOOLBAR_BUTTON + 2.0 * TOOLBAR_MARGIN;
/// The toolbar's icons (`.paper-icon-button-light`, 1.67em).
pub const TOOLBAR_ICON: f32 = 24.843;
/// The paging line's block (a 1.35 line round a 20px text box) and its
/// baseline from the block's top.
pub const PAGING_BLOCK: f32 = 22.047;
pub const PAGING_BASELINE: f32 = 17.969;
/// The paging line repeated under the grid, padded like the toolbar.
pub const PAGING_BAR_HEIGHT: f32 = 2.0 * TOOLBAR_PADDING + PAGING_BLOCK;

/// The library A–Z picker (`.alphaPicker-fixed-right`), sized by the
/// window as jellyfin-web's media queries size it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaPicker {
    pub font: f32,
    /// Space above and below each letter.
    pub pad: f32,
    pub ascent: f32,
    pub row: f32,
    pub width: f32,
    /// From the page's right edge and the window's bottom edge.
    pub right: f32,
    pub bottom: f32,
}

/// The picker for a window, or `None` where jellyfin-web hides it
/// (31.25em tall or less).
pub fn alpha_picker(window_width: f32, window_height: f32) -> Option<AlphaPicker> {
    const EM16: f32 = 16.0;
    if window_height <= 31.25 * EM16 {
        return None;
    }
    let scale = match window_height {
        h if h <= 32.0 * EM16 => 0.74,
        h if h <= 37.0 * EM16 => 0.82,
        h if h <= 44.0 * EM16 => 0.90,
        h if h <= 49.0 * EM16 => 0.94,
        _ => 1.0,
    };
    let font = EM * scale;
    let pad = match window_height {
        h if h <= 44.0 * EM16 => 0.0,
        h if h <= 50.0 * EM16 => 1.0,
        _ => layout_unit(0.1 * font),
    };
    // Chrome's `line-height: normal` rounds the font's ascent and descent
    // (Noto Sans: 1.069 and 0.293 em) to whole pixels.
    let ascent = (1.069 * font).round();
    let line = ascent + (0.293 * font).round();
    Some(AlphaPicker {
        font,
        pad,
        ascent,
        row: line + 2.0 * pad,
        width: layout_unit(1.5 * font),
        right: if window_width >= 62.5 * EM16 {
            font
        } else {
            0.4 * font
        },
        bottom: if window_height <= 50.0 * EM16 {
            5.0 * font
        } else {
            5.5 * font
        },
    })
}

/// A block of text as Chrome sets it: wrapped to `width`, each line
/// `line_height` tall (CSS `line-height`, in layout units), its baseline at
/// the half-leading plus the font's ascent rounded to a whole pixel (Noto
/// Sans: 1.069em up, 0.293em down). Lines are centred on `center_x` when
/// `centered`, else start there. Returns the block's height.
#[allow(clippy::too_many_arguments)]
pub fn paragraph(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    line_height: f32,
    top: f32,
    x: f32,
    width: f32,
    centered: bool,
    color: Color32,
) -> f32 {
    let size = font.size;
    let ascent = (1.069 * size).round();
    let content = ascent + (0.293 * size).round();
    let line_height = layout_unit(line_height);
    let lines = widgets::wrap_lines(ui, text, font.clone(), width);
    for (index, line) in lines.iter().enumerate() {
        let galley = ui
            .painter()
            .layout_no_wrap(line.clone(), font.clone(), color);
        let left = if centered {
            x - galley.size().x / 2.0
        } else {
            x
        };
        let baseline = top + index as f32 * line_height + (line_height - content) / 2.0 + ascent;
        widgets::galley_on_baseline(ui.painter(), left, baseline, galley);
    }
    lines.len() as f32 * line_height
}

/// `.cardDefaultText`: the name written large on a card with no artwork
/// (2em bold, wrapping, 0.06em 0.5em padding), centred in the tile.
fn paint_default_text(ui: &Ui, image: Rect, name: &str) {
    let font = theme::bold(2.0 * EM);
    let line = layout_unit(1.35 * 2.0 * EM);
    let pad_x = layout_unit(0.5 * 2.0 * EM);
    let lines = widgets::wrap_lines(ui, name, font.clone(), image.width() - 2.0 * pad_x).len();
    let top = image.center().y - lines as f32 * line / 2.0;
    paragraph(
        ui,
        name,
        font,
        line,
        top,
        image.center().x,
        image.width() - 2.0 * pad_x,
        true,
        TEXT_80,
    );
}

/// jellyfin-web's `.noItemsMessage.centerMessage`: an `h1` and a paragraph,
/// centred in 30% of `container` with 5em above and below. Returns its
/// height.
pub fn center_message(
    ui: &Ui,
    container: Rect,
    top: f32,
    title: &str,
    body: &str,
    paint: bool,
) -> f32 {
    let width = layout_unit(container.width() * 0.3);
    let center = container.center().x;
    let padding = layout_unit(5.0 * EM);
    let h1 = 1.8 * EM;
    // `h1` keeps the browser's 0.67em margins, a `p` 1em; adjacent margins
    // collapse to the larger.
    let h1_margin = layout_unit(0.67 * h1);
    let p_margin = layout_unit(EM);
    let color = if paint { TEXT_80 } else { Color32::TRANSPARENT };
    let mut y = top + padding + h1_margin;
    y += paragraph(
        ui,
        title,
        theme::regular(h1),
        1.35 * h1,
        y,
        center,
        width,
        true,
        color,
    );
    y += h1_margin.max(p_margin);
    y += paragraph(
        ui,
        body,
        theme::regular(EM),
        1.35 * EM,
        y,
        center,
        width,
        true,
        color,
    );
    y + p_margin + padding - top
}

/// `.padded-bottom-page`: the space under a page's last section (5em).
pub const PAGE_BOTTOM: f32 = 5.0 * EM;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// 16:9: libraries, Continue Watching, Next Up, episodes.
    Backdrop,
    /// 2:3 posters.
    Portrait,
    /// 1:1: albums, music.
    Square,
    /// 1000:185 banners.
    Banner,
}

impl Shape {
    /// Image height over width.
    pub fn ratio(self) -> f32 {
        match self {
            Shape::Backdrop => 9.0 / 16.0,
            Shape::Portrait => 1.5,
            Shape::Square => 1.0,
            Shape::Banner => 0.185,
        }
    }
}

/// A shelf ("overflow") card's width including its gap, for a window
/// `window` points wide -- jellyfin-web's breakpoints, in viewport widths.
pub fn overflow_width(window: f32, shape: Shape) -> f32 {
    let vw = match shape {
        Shape::Backdrop | Shape::Banner => match window {
            w if w >= 2500.0 => 15.6,
            w if w >= 1600.0 => 18.7,
            w if w >= 1200.0 => 23.1,
            w if w >= 770.0 => 30.0,
            w if w >= 560.0 => 45.5,
            _ => 72.0,
        },
        Shape::Portrait | Shape::Square => match window {
            w if w >= 2100.0 => 9.3,
            w if w >= 1920.0 => 10.41,
            w if w >= 1600.0 => 11.6,
            w if w >= 1400.0 => 13.3,
            w if w >= 1200.0 => 15.5,
            w if w >= 800.0 => 18.5,
            w if w >= 700.0 => 23.1,
            w if w >= 400.0 => 31.2,
            _ => 40.0,
        },
    };
    window * vw / 100.0
}

/// A grid card's width including its gap: a share of the grid's width
/// chosen by the window's width.
pub fn grid_width(window: f32, grid: f32) -> f32 {
    let share = match window {
        w if w >= 2100.0 => 0.10,
        w if w >= 1920.0 => 1.0 / 9.0,
        w if w >= 1600.0 => 0.125,
        w if w >= 1400.0 => 1.0 / 7.0,
        w if w >= 1200.0 => 1.0 / 6.0,
        w if w >= 800.0 => 0.20,
        w if w >= 700.0 => 0.25,
        w if w >= 500.0 => 1.0 / 3.0,
        _ => 0.5,
    };
    grid * share
}

/// A grid card's width for any shape: posters use [`grid_width`], wide
/// shapes jellyfin-web's `backdropCard` and `bannerCard` shares.
pub fn grid_card_width(window: f32, grid: f32, shape: Shape) -> f32 {
    match shape {
        Shape::Portrait | Shape::Square => grid_width(window, grid),
        Shape::Backdrop => {
            let share = match window {
                w if w >= 2500.0 => 1.0 / 6.0,
                w if w >= 1600.0 => 0.20,
                w if w >= 1200.0 => 0.25,
                w if w >= 770.0 => 1.0 / 3.0,
                w if w >= 400.0 => 0.5,
                _ => 1.0,
            };
            grid * share
        }
        Shape::Banner => {
            let share = match window {
                w if w >= 2100.0 => 0.25,
                w if w >= 1200.0 => 1.0 / 3.0,
                w if w >= 800.0 => 0.5,
                _ => 1.0,
            };
            grid * share
        }
    }
}

/// jellyfin-web's five placeholder colours for cards without artwork.
/// `.defaultCardBackground1` … `5`, in order.
const DEFAULT_BACKGROUNDS: [Color32; 5] = [
    Color32::from_rgb(0x00, 0x45, 0x5c),
    Color32::from_rgb(0x44, 0xba, 0xe1),
    Color32::from_rgb(0x00, 0xa4, 0xdb),
    Color32::from_rgb(0x1c, 0x4c, 0x5c),
    Color32::from_rgb(0x00, 0x7e, 0xa8),
];

/// The placeholder colour jellyfin-web gives `name`: the last digit of the
/// digit sum of its middle character's code, modulo five.
pub fn default_background(name: &str) -> Color32 {
    let characters = name.encode_utf16().collect::<Vec<_>>();
    let Some(middle) = characters.get(characters.len() / 2) else {
        return DEFAULT_BACKGROUNDS[0];
    };
    let sum: u32 = middle
        .to_string()
        .chars()
        .filter_map(|digit| digit.to_digit(10))
        .sum();
    DEFAULT_BACKGROUNDS[(sum % 10 % 5) as usize]
}

/// A card shape as jellyfin-web's card builder names it, which sets how
/// big an image it asks the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageShape {
    Portrait,
    Square,
    Banner,
    Backdrop,
    OverflowPortrait,
    OverflowSquare,
    OverflowBackdrop,
}

impl ImageShape {
    /// `uiAspect`: width over height.
    pub fn aspect(self) -> f32 {
        match self {
            Self::Portrait | Self::OverflowPortrait => 2.0 / 3.0,
            Self::Square | Self::OverflowSquare => 1.0,
            Self::Backdrop | Self::OverflowBackdrop => 16.0 / 9.0,
            Self::Banner => 1000.0 / 185.0,
        }
    }

    pub fn for_card(shape: Shape, overflow: bool) -> Self {
        match (shape, overflow) {
            (Shape::Portrait, true) => Self::OverflowPortrait,
            (Shape::Square, true) => Self::OverflowSquare,
            (Shape::Backdrop, true) => Self::OverflowBackdrop,
            (Shape::Portrait, false) => Self::Portrait,
            (Shape::Square, false) => Self::Square,
            (Shape::Backdrop, false) => Self::Backdrop,
            (Shape::Banner, _) => Self::Banner,
        }
    }
}

/// jellyfin-web's `getPostersPerRow` (desktop): cards per window width.
pub fn posters_per_row(shape: ImageShape, width: f32, landscape: bool) -> f32 {
    use ImageShape::*;
    match shape {
        Portrait | Square => match width {
            w if w >= 2200.0 => 10.0,
            w if w >= 1920.0 => 9.0,
            w if w >= 1600.0 => 8.0,
            w if w >= 1400.0 => 7.0,
            w if w >= 1200.0 => 6.0,
            w if w >= 800.0 => 5.0,
            w if w >= 700.0 => 4.0,
            w if w >= 500.0 || shape == Portrait => 3.0,
            _ => 2.0,
        },
        Banner => match width {
            w if w >= 2200.0 => 4.0,
            w if w >= 1200.0 => 3.0,
            w if w >= 800.0 => 2.0,
            _ => 1.0,
        },
        Backdrop => match width {
            w if w >= 2500.0 => 6.0,
            w if w >= 1600.0 => 5.0,
            w if w >= 1200.0 => 4.0,
            w if w >= 770.0 => 3.0,
            w if w >= 420.0 => 2.0,
            _ => 1.0,
        },
        OverflowPortrait | OverflowSquare => {
            let small = shape == OverflowSquare && width < 540.0;
            100.0
                / match () {
                    _ if landscape && width >= 1700.0 => 11.6,
                    _ if landscape => 15.5,
                    _ if width >= 1400.0 => 15.0,
                    _ if width >= 1200.0 => 18.0,
                    _ if width >= 760.0 => 23.0,
                    _ if width >= 400.0 && !small => 31.5,
                    _ => 42.0,
                }
        }
        OverflowBackdrop => match () {
            _ if landscape && width >= 1700.0 => 100.0 / 18.5,
            _ if landscape => 100.0 / 23.3,
            _ if width >= 1800.0 => 100.0 / 23.5,
            _ if width >= 1400.0 => 100.0 / 30.0,
            _ if width >= 760.0 => 2.5,
            _ if width >= 640.0 => 100.0 / 56.0,
            _ => 100.0 / 72.0,
        },
    }
}

/// The CSS width jellyfin-web asks a card image for: the window divided
/// among the cards that fit, after rounding a window narrower than its
/// screen down to the hundred.
pub fn card_image_width(shape: ImageShape, window: egui::Vec2, screen_width: f32) -> f32 {
    let mut width = window.x;
    if screen_width - width > 20.0 {
        width = (width / 100.0).floor() * 100.0;
    }
    let landscape = width > 1.3 * window.y;
    (width / posters_per_row(shape, width, landscape)).round()
}

/// The fill size to request, in device pixels, as `getScaledImageUrl`
/// computes it from the CSS width and the image's aspect.
pub fn card_fill_size(
    shape: ImageShape,
    window: egui::Vec2,
    screen_width: f32,
    pixels_per_point: f32,
    primary_aspect: Option<f32>,
) -> (u32, u32) {
    let width = card_image_width(shape, window, screen_width);
    let height = match primary_aspect {
        Some(aspect) => (width / aspect).round(),
        None => width / shape.aspect(),
    };
    (
        (width * pixels_per_point).round() as u32,
        (height * pixels_per_point).round() as u32,
    )
}

/// What a card shows.
pub struct CardModel<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub art: Option<&'a Art>,
    pub watched: bool,
    pub unplayed: u32,
    pub progress: f32,
    pub favorite: bool,
    pub playable: bool,
    /// Whether hovering shows the played/favorite buttons (cast and
    /// libraries have none).
    pub actions: bool,
    /// Drawn instead of the name on a card with no artwork (libraries).
    pub placeholder_icon: Option<Icon>,
    /// Whether the card has jellyfin-web's "⋮" menu (and right-click).
    pub more: bool,
    /// Whether each line is a link, which sets its height and where its
    /// text sits.
    pub title_link: bool,
    pub subtitle_link: bool,
}

/// What the user did with a card this frame.
#[derive(Default)]
pub struct CardResponse {
    pub open: bool,
    pub play: bool,
    pub toggle_played: bool,
    pub toggle_favorite: bool,
    pub gained_focus: bool,
    /// The "⋮" or a right-click: where to anchor the menu.
    pub more: Option<Rect>,
}

/// The text under a card: how many lines, and whether each is a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardText {
    pub lines: usize,
    pub title_link: bool,
    pub subtitle_link: bool,
}

impl CardText {
    /// Lines that are all links: the tallest text a card can have.
    pub fn links(lines: usize) -> Self {
        Self {
            lines,
            title_link: true,
            subtitle_link: true,
        }
    }

    fn title_row(self) -> f32 {
        if self.title_link {
            TITLE_ROW_LINK
        } else {
            TITLE_ROW
        }
    }

    pub fn height(self) -> f32 {
        let secondary = if self.subtitle_link {
            SECONDARY_ROW_LINK
        } else {
            SECONDARY_ROW
        };
        match self.lines {
            0 => 0.0,
            1 => self.title_row(),
            _ => self.title_row() + secondary,
        }
    }
}

/// A card's full height for `width` (including the gap) and `lines` text
/// rows (1 or 2), each assumed to be a link.
pub fn card_height(width: f32, shape: Shape, lines: usize) -> f32 {
    card_height_for(width, shape, CardText::links(lines))
}

/// A card's full height for its actual text.
pub fn card_height_for(width: f32, shape: Shape, text: CardText) -> f32 {
    let image = (width - CARD_GAP) * shape.ratio();
    CARD_TOP + image + text.height() + CARD_BOTTOM
}

/// Draws one card with its top-left corner (gap included) at `origin`.
#[allow(clippy::too_many_arguments)]
pub fn card(
    ui: &mut Ui,
    images: &mut ImageCache,
    model: &CardModel<'_>,
    origin: egui::Pos2,
    width: f32,
    shape: Shape,
    lines: usize,
    id: Id,
) -> CardResponse {
    let image_width = width - CARD_GAP;
    let image = Rect::from_min_size(
        pos2(origin.x, origin.y + CARD_TOP),
        vec2(image_width, image_width * shape.ratio()),
    );
    let mut out = CardResponse::default();
    let response = ui.interact(image, id, Sense::click());
    widgets::describe_button(&response, true, model.title);
    out.gained_focus = response.gained_focus();
    // Pointer-in-image rather than `hovered()`: the overlay's own buttons sit
    // on top of the image, and egui only reports the topmost widget as
    // hovered, so `hovered()` would hide the overlay under its own buttons.
    let hovered = ui.rect_contains_pointer(image) || response.has_focus();
    let painter = ui.painter().clone();
    if ui.is_rect_visible(image) {
        let text_height = card_height(width, shape, lines) - CARD_TOP - image.height();
        let card = Rect::from_min_max(
            pos2(image.left(), origin.y),
            pos2(origin.x + width, image.bottom() + text_height),
        );
        paint_card_padder(&painter, image, card);
        let texture = model
            .art
            .and_then(|art| images.texture(art).zip(images.size(&art.key)));
        match texture {
            Some((texture, size)) => {
                widgets::image_cover(&painter, texture, size, image, CARD_RADIUS)
            }
            // No artwork at all: jellyfin-web's coloured tile with the name.
            None if model.art.is_none_or(|art| images.is_failed(&art.key)) => {
                painter.rect_filled(image, CARD_RADIUS, default_background(model.title));
                if let Some(icon) = model.placeholder_icon {
                    // `.cardImageIcon { font-size: 5em }`, centred.
                    icons::paint_centered(&painter, icon, image.center(), 5.0 * EM, theme::JF_TEXT);
                } else {
                    paint_default_text(ui, image, model.title);
                }
            }
            // Still loading: the padder shows through.
            None => {}
        }
        if model.progress > 0.0 && model.progress < 1.0 {
            let track = Rect::from_min_max(
                pos2(image.left(), image.bottom() - 4.0),
                image.right_bottom(),
            );
            painter.rect_filled(track, 0.0, theme::JF_PROGRESS_TRACK);
            painter.rect_filled(
                Rect::from_min_size(track.min, vec2(track.width() * model.progress, 4.0)),
                0.0,
                theme::JF_ACCENT,
            );
        }
        corner_indicator(&painter, image, model);
        if response.has_focus() {
            widgets::border(&painter, image, theme::JF_RADIUS, 2.0, theme::JF_ACCENT);
        }
    }
    let mut overlay_used = false;
    if hovered && ui.is_rect_visible(image) {
        painter.rect_filled(image, theme::JF_RADIUS, Color32::from_black_alpha(128));
        let icon_color = Color32::from_white_alpha(194);
        if model.playable {
            let play = Rect::from_center_size(image.center(), vec2(58.0, 58.0));
            let play_response = ui.interact(play, id.with("play"), Sense::click());
            widgets::describe_button(&play_response, true, "Play");
            painter.circle_filled(play.center(), 29.0, Color32::from_black_alpha(128));
            icons::paint_centered(
                &painter,
                Icon::Play,
                play.center(),
                34.0,
                if play_response.hovered() {
                    Color32::WHITE
                } else {
                    icon_color
                },
            );
            widgets::pointer_on_hover(&play_response);
            if play_response.clicked() {
                out.play = true;
                overlay_used = true;
            }
        }
        if model.actions || model.more {
            let mut right = image.right();
            let mut button = |icon: Icon, color: Color32, label: &str, name: &str| {
                let rect = Rect::from_min_max(
                    pos2(right - 39.0, image.bottom() - 39.0),
                    pos2(right, image.bottom()),
                );
                right -= 39.0;
                let button = ui.interact(rect, id.with(name), Sense::click());
                widgets::describe_button(&button, true, label);
                let color = if button.hovered() {
                    color.gamma_multiply(1.3)
                } else {
                    color
                };
                icons::paint_centered(&painter, icon, rect.center(), 24.8, color);
                widgets::pointer_on_hover(&button);
                button.on_hover_text(label).clicked()
            };
            if model.more {
                // The first button from the right edge.
                let rect = Rect::from_min_max(
                    pos2(image.right() - 39.0, image.bottom() - 39.0),
                    image.right_bottom(),
                );
                if button(Icon::MoreVert, icon_color, "More", "more") {
                    out.more = Some(rect);
                    overlay_used = true;
                }
            }
            let favorite_color = if model.favorite {
                theme::JF_PLAYED_RED
            } else {
                icon_color
            };
            let favorite_label = if model.favorite {
                "Remove from favorites"
            } else {
                "Add to favorites"
            };
            if model.actions && button(Icon::Favorite, favorite_color, favorite_label, "favorite") {
                out.toggle_favorite = true;
                overlay_used = true;
            }
            let played_color = if model.watched {
                theme::JF_PLAYED_RED
            } else {
                icon_color
            };
            let played_label = if model.watched {
                "Mark unplayed"
            } else {
                "Mark played"
            };
            if model.actions && button(Icon::Check, played_color, played_label, "played") {
                out.toggle_played = true;
                overlay_used = true;
            }
        }
    }
    if response.clicked() && !overlay_used {
        out.open = true;
    }
    if model.more && response.secondary_clicked() {
        let at = response.interact_pointer_pos().unwrap_or(image.center());
        out.more = Some(Rect::from_min_size(at, vec2(1.0, 1.0)));
    }
    widgets::pointer_on_hover(&response);

    // Text rows, centred like jellyfin-web's home and library cards.
    let text_width = image_width - 2.0 * 7.44;
    let mut y = image.bottom();
    if lines >= 1 {
        let title = widgets::elided(
            ui,
            model.title,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT,
            text_width,
        );
        let left = image.center().x - title.size().x / 2.0;
        let baseline = y + TITLE_BASELINE
            - if model.title_link {
                0.0
            } else {
                PLAIN_TITLE_RAISE
            };
        let text_rect = Rect::from_min_size(
            pos2(left, baseline - widgets::galley_baseline(&title)),
            title.size(),
        );
        let title_response = ui.interact(text_rect, id.with("title"), Sense::click());
        widgets::galley_on_baseline(&painter, left, baseline, title);
        widgets::pointer_on_hover(&title_response);
        if title_response.clicked() {
            out.open = true;
        }
        y += if model.title_link {
            TITLE_ROW_LINK
        } else {
            TITLE_ROW
        };
    }
    if lines >= 2 && !model.subtitle.is_empty() {
        let subtitle = widgets::elided(
            ui,
            model.subtitle,
            theme::regular(theme::JF_FONT_SECONDARY),
            theme::JF_TEXT_SECONDARY,
            text_width,
        );
        // Chrome puts the glyphs at the same place whether the line is a
        // link or not.
        let left = image.center().x - subtitle.size().x / 2.0;
        let raise = if model.subtitle_link {
            0.0
        } else {
            PLAIN_SECONDARY_RAISE
        };
        widgets::galley_on_baseline(&painter, left, y + SECONDARY_BASELINE - raise, subtitle);
    }
    out
}

/// The corner indicator: unplayed count, or a check once watched.
pub fn corner_indicator(painter: &egui::Painter, image: Rect, model: &CardModel<'_>) {
    let inset = 3.35;
    if model.unplayed > 0 {
        let text = model.unplayed.to_string();
        let galley = painter.layout_no_wrap(text, theme::regular(11.9), Color32::WHITE);
        let width = (galley.size().x + 12.0).max(24.0);
        let rect = Rect::from_min_size(
            pos2(image.right() - inset - width, image.top() + inset),
            vec2(width, 24.0),
        );
        painter.rect_filled(rect, 12.0, theme::JF_ACCENT);
        widgets::galley_centered(painter, rect, galley);
    } else if model.watched {
        let rect = Rect::from_min_size(
            pos2(image.right() - inset - 24.0, image.top() + inset),
            vec2(24.0, 24.0),
        );
        painter.circle_filled(rect.center(), 12.0, theme::JF_ACCENT);
        icons::paint_centered(painter, Icon::Check, rect.center(), 17.0, Color32::WHITE);
    }
}

/// A section title row: the title at the page padding, and ‹ › paging
/// buttons at the right end when the shelf scrolls. Returns which paging
/// button was clicked: -1 back, 1 forward, 0 neither.
pub fn section_title(
    ui: &mut Ui,
    row: Rect,
    title: &str,
    padding: f32,
    paging: Option<(bool, bool)>,
    id: Id,
) -> i32 {
    let baseline = if (row.height() - SECTION_HEADING).abs() < 0.01 {
        SECTION_BASELINE_HEADING
    } else if row.height() >= SECTION_TITLE_LINKED - 0.5 {
        SECTION_BASELINE_LINKED
    } else {
        SECTION_BASELINE
    };
    let painter = ui.painter().clone();
    let galley = painter.layout_no_wrap(
        title.to_owned(),
        theme::regular(theme::JF_FONT_SECTION),
        theme::JF_TEXT,
    );
    widgets::galley_on_baseline(&painter, row.left() + padding, row.top() + baseline, galley);
    let Some((back, forward)) = paging else {
        return 0;
    };
    let mut clicked = 0;
    // `.emby-scrollbuttons`: two 41.36 px buttons 50 px apart, 12.64 px
    // down, ending 6.32 px inside the page padding.
    let right = row.right() - padding - 6.32;
    let top = row.top() + 12.64;
    let size = vec2(41.36, 41.38);
    for (icon, enabled, rect, direction, label) in [
        (
            Icon::ChevronLeft,
            back,
            Rect::from_min_size(pos2(right - 41.36 - 49.99, top), size),
            -1,
            "Previous",
        ),
        (
            Icon::ChevronRight,
            forward,
            Rect::from_min_size(pos2(right - 41.36, top), size),
            1,
            "Next",
        ),
    ] {
        if round_icon_button(
            ui,
            rect,
            icon,
            Color32::WHITE,
            24.8,
            label,
            enabled,
            id.with(direction),
        )
        .clicked()
        {
            clicked = direction;
        }
    }
    clicked
}

/// A transparent round icon button, as in jellyfin-web's headers, shelves
/// and detail pages. Disabled buttons fade to 30%.
#[allow(clippy::too_many_arguments)]
pub fn round_icon_button(
    ui: &mut Ui,
    rect: Rect,
    icon: Icon,
    color: Color32,
    icon_size: f32,
    label: &str,
    enabled: bool,
    id: Id,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    widgets::describe_button(&response, enabled, label);
    let painter = ui.painter();
    // jellyfin-web: an accent circle at 20% under the pointer, and the
    // icon itself in the accent while hovered or focused.
    let lit = enabled && (response.hovered() || response.has_focus());
    if enabled && response.hovered() {
        painter.circle_filled(
            rect.center(),
            rect.width().min(rect.height()) / 2.0,
            theme::JF_ACCENT.gamma_multiply(0.2),
        );
    }
    let color = if !enabled {
        color.gamma_multiply(0.3)
    } else if lit {
        theme::JF_ACCENT
    } else {
        color
    };
    icons::paint_centered(painter, icon, rect.center(), icon_size, color);
    if enabled {
        widgets::pointer_on_hover(&response);
    }
    response.on_hover_text(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_cards_follow_jellyfin_breakpoints() {
        // The reference window: 1261 px wide.
        assert!((overflow_width(1261.0, Shape::Backdrop) - 291.3).abs() < 0.5);
        assert!((overflow_width(1261.0, Shape::Portrait) - 195.5).abs() < 0.5);
        assert!((overflow_width(1700.0, Shape::Portrait) - 197.2).abs() < 0.5);
        assert!((overflow_width(700.0, Shape::Backdrop) - 318.5).abs() < 0.5);
    }

    #[test]
    fn card_images_are_the_sizes_jellyfin_web_asks_for() {
        let window = egui::vec2(2536.0, 1390.0);
        assert_eq!(
            card_fill_size(ImageShape::OverflowBackdrop, window, 2536.0, 1.5, None),
            (704, 396)
        );
        assert_eq!(
            card_fill_size(
                ImageShape::OverflowPortrait,
                window,
                2536.0,
                1.5,
                Some(2.0 / 3.0)
            ),
            (441, 662)
        );
        assert_eq!(
            card_fill_size(ImageShape::OverflowBackdrop, window, 2536.0, 1.5, Some(1.0)),
            (704, 704)
        );
    }

    #[test]
    fn grid_cards_take_a_share_of_the_grid() {
        // 1251 wide content, 41 + 94 of padding: six per row.
        assert!((grid_width(1261.0, 1116.0) - 186.0).abs() < 0.5);
        assert!((grid_width(900.0, 800.0) - 160.0).abs() < 0.5);
    }

    #[test]
    fn card_height_adds_margins_image_and_text() {
        let height = card_height(291.3, Shape::Backdrop, 2);
        // Card top margin (in layout units), the 16:9 image, a linked title
        // row and a linked subtitle row, the bottom margin.
        let expected = 8.921875 + 273.444 * 0.5625 + TITLE_ROW_LINK + SECONDARY_ROW_LINK + 26.784;
        assert!((height - expected).abs() < 0.01);
    }

    #[test]
    fn placeholder_colours_follow_jellyfin_classes() {
        // "Apple TV": middle char 'e' (101) → 1+0+1 = 2 → class 3.
        assert_eq!(
            default_background("Apple TV"),
            Color32::from_rgb(0x00, 0xa4, 0xdb)
        );
        // "Playlists": 'l' (108) → 9 → class 5.
        assert_eq!(
            default_background("Playlists"),
            Color32::from_rgb(0x00, 0x7e, 0xa8)
        );
        // "HBO": 'B' (66) → 12 → 2 → class 3; "ABC": 'B' as well.
        assert_eq!(default_background("HBO"), default_background("ABC"));
    }
}
