//! jellyfin-web's building blocks: responsive card sizes, the card itself
//! (artwork, indicators, progress, hover actions, centred text), section
//! titles with paging buttons, and the round icon buttons of headers and
//! detail pages. Measurements are in `docs/jellyfin-web-reference.md`.

use egui::{Align2, Color32, Id, Rect, Response, Sense, Ui, pos2, vec2};

use super::icons::{self, Icon};
use super::images::{Art, ImageCache};
use super::theme;
use super::widgets;

/// `cardBox` margins: the gap after a card is part of its width.
pub const CARD_TOP: f32 = 8.928;
pub const CARD_GAP: f32 = 17.856;
pub const CARD_BOTTOM: f32 = 26.784;
const TITLE_ROW: f32 = 26.0;
const SECONDARY_ROW: f32 = 20.0;

/// The page's side padding, 3.3vw.
pub fn page_padding(window: f32) -> f32 {
    window * 0.033
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// 16:9: libraries, Continue Watching, Next Up, episodes.
    Backdrop,
    /// 2:3 posters.
    Portrait,
    /// 1:1: albums, music.
    Square,
}

impl Shape {
    /// Image height over width.
    pub fn ratio(self) -> f32 {
        match self {
            Shape::Backdrop => 9.0 / 16.0,
            Shape::Portrait => 1.5,
            Shape::Square => 1.0,
        }
    }
}

/// A shelf ("overflow") card's width including its gap, for a window
/// `window` points wide -- jellyfin-web's breakpoints, in viewport widths.
pub fn overflow_width(window: f32, shape: Shape) -> f32 {
    let vw = match shape {
        Shape::Backdrop => match window {
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
}

/// What the user did with a card this frame.
#[derive(Default)]
pub struct CardResponse {
    pub open: bool,
    pub play: bool,
    pub toggle_played: bool,
    pub toggle_favorite: bool,
    pub gained_focus: bool,
}

/// A card's full height for `width` (including the gap) and `lines` text
/// rows (1 or 2).
pub fn card_height(width: f32, shape: Shape, lines: usize) -> f32 {
    let image = (width - CARD_GAP) * shape.ratio();
    CARD_TOP + image + text_rows(lines) + CARD_BOTTOM
}

fn text_rows(lines: usize) -> f32 {
    match lines {
        0 => 0.0,
        1 => TITLE_ROW,
        _ => TITLE_ROW + SECONDARY_ROW,
    }
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
        let texture = model
            .art
            .and_then(|art| images.texture(art).zip(images.size(&art.key)));
        match texture {
            Some((texture, size)) => {
                widgets::image_cover(&painter, texture, size, image, theme::JF_RADIUS)
            }
            None => {
                painter.rect_filled(image, theme::JF_RADIUS, theme::JF_CARD_EMPTY);
            }
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
        if model.actions {
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
            if button(Icon::Favorite, favorite_color, favorite_label, "favorite") {
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
            if button(Icon::Check, played_color, played_label, "played") {
                out.toggle_played = true;
                overlay_used = true;
            }
        }
    }
    if response.clicked() && !overlay_used {
        out.open = true;
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
        let row = Rect::from_min_size(pos2(image.left(), y), vec2(image_width, TITLE_ROW));
        let text_rect = Rect::from_center_size(row.center(), title.size());
        let title_response = ui.interact(text_rect, id.with("title"), Sense::click());
        painter.galley(text_rect.min, title, Color32::WHITE);
        widgets::pointer_on_hover(&title_response);
        if title_response.clicked() {
            out.open = true;
        }
        y += TITLE_ROW;
    }
    if lines >= 2 && !model.subtitle.is_empty() {
        let subtitle = widgets::elided(
            ui,
            model.subtitle,
            theme::regular(theme::JF_FONT_SECONDARY),
            theme::JF_TEXT_SECONDARY,
            text_width,
        );
        let row = Rect::from_min_size(pos2(image.left(), y), vec2(image_width, SECONDARY_ROW));
        widgets::galley_centered(&painter, row, subtitle);
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
    let painter = ui.painter().clone();
    painter.text(
        pos2(row.left() + padding, row.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::regular(theme::JF_FONT_SECTION),
        theme::JF_TEXT,
    );
    let Some((back, forward)) = paging else {
        return 0;
    };
    let mut clicked = 0;
    let right = row.right() - 57.0;
    for (icon, enabled, rect, direction, label) in [
        (
            Icon::ChevronLeft,
            back,
            Rect::from_min_size(pos2(right - 82.0, row.center().y - 20.5), vec2(41.0, 41.0)),
            -1,
            "Previous",
        ),
        (
            Icon::ChevronRight,
            forward,
            Rect::from_min_size(pos2(right - 41.0, row.center().y - 20.5), vec2(41.0, 41.0)),
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
    if enabled && response.hovered() {
        painter.circle_filled(
            rect.center(),
            rect.width().min(rect.height()) / 2.0,
            Color32::from_white_alpha(20),
        );
    }
    if response.has_focus() {
        painter.circle_stroke(
            rect.center(),
            rect.width().min(rect.height()) / 2.0,
            egui::Stroke::new(2.0, theme::JF_ACCENT),
        );
    }
    let color = if enabled {
        color
    } else {
        color.gamma_multiply(0.3)
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
    fn grid_cards_take_a_share_of_the_grid() {
        // 1251 wide content, 41 + 94 of padding: six per row.
        assert!((grid_width(1261.0, 1116.0) - 186.0).abs() < 0.5);
        assert!((grid_width(900.0, 800.0) - 160.0).abs() < 0.5);
    }

    #[test]
    fn card_height_adds_margins_image_and_text() {
        let height = card_height(291.3, Shape::Backdrop, 2);
        assert!((height - (8.928 + 273.444 * 0.5625 + 46.0 + 26.784)).abs() < 0.1);
    }
}
