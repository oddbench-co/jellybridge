//! jellyfin-web's overlays: the centred dialog (sort, filter) and the action
//! sheet that opens on its button (view, item menus). Both dim the page
//! behind them and close on Escape or a click outside.

use egui::{Color32, Id, Order, Rect, Sense, Ui, pos2, vec2};

use super::icons::{self, Icon};
use super::theme;
use super::widgets;

/// The dim behind dialogs: black at 50%.
const BACKDROP: Color32 = Color32::from_black_alpha(128);
/// Action sheet rows.
const SHEET_ROW: f32 = 40.14;

/// Claims clicks on a dialog's box, so a click on its empty parts is not
/// taken for a click on the dim (which closes the dialog): egui sends a
/// click to the topmost widget under the pointer, whatever layer it is in.
fn claim(ui: &Ui, rect: Rect, id: Id) {
    ui.interact(rect, id.with("box"), Sense::click());
}

/// Dims the whole window and reports a click on the dim (outside whatever is
/// drawn above it) or Escape.
fn backdrop(ctx: &egui::Context, id: Id) -> bool {
    let screen = ctx.content_rect();
    let area = egui::Area::new(id.with("backdrop"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, BACKDROP);
            response.clicked()
        });
    area.inner || ctx.input(|input| input.key_pressed(egui::Key::Escape))
}

/// What an action sheet row shows.
pub struct SheetItem<'a> {
    pub label: &'a str,
    pub icon: Option<Icon>,
    pub checked: bool,
}

// jellyfin-web's action sheet metrics at 1em = 14.88 px.
const SHEET_PADDING: f32 = 0.4 * theme::JF_FONT;
/// Icon column: 0.45em margin after the row's 0.5em padding, 0.85em after.
const SHEET_ICON_LEFT: f32 = 16.53;
const SHEET_ICON: f32 = 1.36 * theme::JF_FONT;
const SHEET_TEXT_LEFT_ICON: f32 = 62.88;
const SHEET_TEXT_LEFT: f32 = 7.44 + 8.928;
const SHEET_TEXT_RIGHT: f32 = 14.88 + 3.72;
const SHEET_TEXT_TOP: f32 = 9.66;
const SHEET_TEXT_LINE: f32 = 20.83;

/// An action sheet opened from `anchor`. With `selected` its row is level
/// with the anchor's centre, as a select's list opens; without, the sheet is
/// centred on the anchor and pulled back inside the window, as jellyfin-web
/// places its menus. Returns the picked row, and whether the sheet closed
/// (a pick closes it too).
pub fn action_sheet(
    ctx: &egui::Context,
    id: Id,
    anchor: Rect,
    items: &[SheetItem<'_>],
    selected: Option<usize>,
) -> (Option<usize>, bool) {
    let close = backdrop(ctx, id);
    let screen = ctx.content_rect();
    let any_icon = items.iter().any(|item| item.icon.is_some() || item.checked);
    let font = theme::regular(theme::JF_FONT);
    let text_left = if any_icon { SHEET_TEXT_LEFT_ICON } else { SHEET_TEXT_LEFT };
    let galleys: Vec<_> = items
        .iter()
        .map(|item| {
            ctx.fonts_mut(|fonts| fonts.layout_no_wrap(item.label.to_owned(), font.clone(), theme::JF_TEXT))
        })
        .collect();
    let widest = galleys.iter().map(|galley| galley.size().x).fold(0.0, f32::max);
    let width = (text_left + widest + SHEET_TEXT_RIGHT).min(screen.width() * 0.9);
    let height = (items.len() as f32 * SHEET_ROW + 2.0 * SHEET_PADDING).min(screen.height() * 0.9);
    let (left, top) = match selected {
        Some(selected) => {
            let top = anchor.center().y - SHEET_PADDING - selected as f32 * SHEET_ROW - SHEET_ROW / 2.0;
            let top = top.clamp(screen.top() + 8.0, (screen.bottom() - height - 8.0).max(8.0));
            let left = anchor
                .left()
                .clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(8.0));
            (left, top)
        }
        None => {
            // jellyfin-web's actionsheet getPosition: centred on the
            // anchor, offset sizes rounded as offsetWidth/Height are.
            let mut left = anchor.center().x - width.round() / 2.0;
            let mut top = anchor.center().y - height.round() / 2.0;
            let overflow_x = left + width.round() - screen.right();
            let overflow_y = top + height.round() - screen.bottom();
            if overflow_x > 0.0 {
                left -= overflow_x + 20.0;
            }
            if overflow_y > 0.0 {
                top -= overflow_y + 20.0;
            }
            (left.max(screen.left() + 10.0), top.max(screen.top() + 10.0))
        }
    };
    let rect = Rect::from_min_size(pos2(left, top), vec2(width, height));
    let radius = 0.1 * theme::JF_FONT;
    let area = egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            claim(ui, rect, id);
            paint_dialog_shadow(ui.painter(), rect, radius);
            ui.painter().rect_filled(rect, radius, theme::JF_BG);
            let mut picked = None;
            for (index, (item, galley)) in items.iter().zip(galleys).enumerate() {
                let row = Rect::from_min_size(
                    pos2(rect.left(), rect.top() + SHEET_PADDING + index as f32 * SHEET_ROW),
                    vec2(width, SHEET_ROW),
                );
                if row.bottom() > rect.bottom() {
                    break;
                }
                let response = ui.interact(row, id.with(("row", index)), Sense::click());
                widgets::describe_button(&response, true, item.label);
                if response.has_focus() {
                    ui.painter().rect_filled(row, 0.2 * theme::JF_FONT, Color32::from_gray(0x33));
                } else if response.hovered() {
                    ui.painter().rect_filled(row, 0.0, Color32::from_gray(0x24));
                }
                let icon = if item.checked { Some(Icon::Check) } else { item.icon };
                if let Some(icon) = icon {
                    icons::paint_centered(
                        ui.painter(),
                        icon,
                        pos2(
                            row.left() + SHEET_ICON_LEFT + SHEET_ICON / 2.0,
                            row.top() + 9.95 + SHEET_ICON / 2.0,
                        ),
                        SHEET_ICON,
                        theme::JF_TEXT,
                    );
                }
                let clip = Rect::from_min_max(
                    pos2(row.left() + text_left, row.top()),
                    pos2(row.right() - SHEET_TEXT_RIGHT, row.bottom()),
                );
                widgets::galley_on_baseline(
                    &ui.painter().with_clip_rect(clip),
                    row.left() + text_left,
                    super::jf::baseline(row.top() + SHEET_TEXT_TOP, theme::JF_FONT, SHEET_TEXT_LINE),
                    galley,
                );
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    picked = Some(index);
                }
            }
            picked
        });
    let picked = area.inner;
    (picked, close || picked.is_some())
}

/// jellyfin-web's `.dialog` box shadow, all three layers.
fn paint_dialog_shadow(painter: &egui::Painter, rect: Rect, radius: f32) {
    for (offset, blur, spread, alpha) in [(16, 24, 2.0, 0.14), (6, 30, 5.0, 0.12), (8, 10, -5.0, 0.4)] {
        let shadow = egui::Shadow {
            offset: [0, offset],
            blur,
            spread: 0,
            color: Color32::from_black_alpha((alpha * 255.0f32).round() as u8),
        };
        // egui's spread is unsigned; grow or shrink the box instead.
        painter.add(shadow.as_shape(rect.expand(spread), radius));
    }
}

// jellyfin-web's alert `formDialog` metrics, measured at 1em = 14.88 px.
const ALERT_TITLE: f32 = 1.8 * theme::JF_FONT;
const ALERT_TITLE_LINE: f32 = 36.14;
/// Header padding plus the title's own side padding.
const ALERT_TITLE_LEFT: f32 = 7.44 + 26.77 + 26.784;
const ALERT_TITLE_TOP: f32 = 28.26;
const ALERT_TEXT_TOP: f32 = 94.15;
const ALERT_TEXT_LINE: f32 = 1.35 * theme::JF_FONT;
const ALERT_PADDING: f32 = 29.76;
/// Content padding, footer margin and padding, button margin.
const ALERT_BUTTON_GAP: f32 = 14.88 + 14.88 + 22.32;
const ALERT_BOTTOM: f32 = 22.32 + 14.88;
const ALERT_BUTTON: f32 = 46.86;

/// jellyfin-web's alert: a title, centred text and one focused submit
/// button, centred in the window over a 50% dim. Returns true once it is
/// dismissed: the button, Enter, Escape or a click on the dim.
pub fn message_box(ctx: &egui::Context, id: Id, title: &str, text: &str, button: &str) -> bool {
    let close = backdrop(ctx, id);
    let screen = ctx.content_rect();
    let layout = |text: &str, font: egui::FontId, wrap: f32| {
        ctx.fonts_mut(|fonts| {
            fonts.layout(text.to_owned(), font, theme::JF_TEXT, wrap)
        })
    };
    let title_galley = layout(title, theme::regular(ALERT_TITLE), f32::INFINITY);
    let max_text = (screen.width() * 0.8 - 2.0 * ALERT_PADDING).max(100.0);
    let text_galley = layout(text, theme::regular(theme::JF_FONT), max_text);
    let button_galley = ctx.fonts_mut(|fonts| {
        fonts.layout_no_wrap(button.to_owned(), theme::bold(theme::JF_FONT), Color32::WHITE)
    });
    let button_width = button_galley.size().x + 2.0 * ALERT_PADDING;
    // The dialog shrinks to its widest part: in practice the title.
    let width = (title_galley.size().x + 2.0 * ALERT_TITLE_LEFT - 26.77)
        .max(text_galley.size().x + 2.0 * ALERT_PADDING + 31.02)
        .max(button_width + 2.0 * 22.32)
        .min(screen.width() - 32.0);
    let lines = text_galley.rows.len().max(1) as f32;
    let text_bottom = ALERT_TEXT_TOP + lines * ALERT_TEXT_LINE;
    let button_top = text_bottom + ALERT_BUTTON_GAP;
    let height = button_top + ALERT_BUTTON + ALERT_BOTTOM;
    let rect = Rect::from_min_size(
        pos2(
            screen.center().x - width / 2.0,
            screen.center().y - height / 2.0,
        ),
        vec2(width, height),
    );
    let radius = 0.2 * theme::JF_FONT;
    let area = egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            claim(ui, rect, id);
            let painter = ui.painter();
            paint_dialog_shadow(painter, rect, radius);
            painter.rect_filled(rect, radius, theme::JF_BG);
            widgets::galley_on_baseline(
                painter,
                rect.left() + ALERT_TITLE_LEFT,
                super::jf::baseline(rect.top() + ALERT_TITLE_TOP, ALERT_TITLE, ALERT_TITLE_LINE),
                title_galley,
            );
            for (index, row) in text_galley.rows.iter().enumerate() {
                let galley = layout(&row.text(), theme::regular(theme::JF_FONT), f32::INFINITY);
                let x = rect.center().x - galley.size().x / 2.0;
                let top = rect.top() + ALERT_TEXT_TOP + index as f32 * ALERT_TEXT_LINE;
                widgets::galley_on_baseline(
                    painter,
                    x,
                    super::jf::baseline(top, theme::JF_FONT, ALERT_TEXT_LINE),
                    galley,
                );
            }
            let button_rect = Rect::from_min_size(
                pos2(rect.center().x - button_width / 2.0, rect.top() + button_top),
                vec2(button_width, ALERT_BUTTON),
            );
            let response = ui.interact(button_rect, id.with("button"), Sense::click());
            widgets::describe_button(&response, true, button);
            // jellyfin focuses the button on open: `.button-submit:focus`.
            ui.painter().rect_filled(button_rect, radius, Color32::from_rgb(0x0c, 0xb0, 0xe8));
            let x = button_rect.center().x - button_galley.size().x / 2.0;
            widgets::galley_on_baseline(
                ui.painter(),
                x,
                super::jf::baseline(button_rect.top() + 13.392, theme::JF_FONT, 20.08),
                button_galley,
            );
            widgets::pointer_on_hover(&response);
            response.clicked()
        });
    let enter = ctx.input(|input| input.key_pressed(egui::Key::Enter));
    close || area.inner || enter
}

/// A dialog laid out by hand: the dim, then a `size` box centred on the
/// page (the window less the page scrollbar, as jellyfin-web centres in the
/// viewport), or `top` below the window's top, with `radius` and jellyfin's
/// shadow. `content` draws into the
/// box. Returns its result and whether the dialog asked to close (Escape or
/// a click on the dim).
pub fn panel<R>(
    ctx: &egui::Context,
    id: Id,
    size: egui::Vec2,
    top: Option<f32>,
    radius: f32,
    content: impl FnOnce(&mut Ui, Rect) -> R,
) -> (R, bool) {
    let close = backdrop(ctx, id);
    let screen = ctx.content_rect();
    let page_width = screen.width() - super::scrollbar::reserved(ctx);
    let size = vec2(size.x.min(page_width - 16.0), size.y.min(screen.height() - 16.0));
    let rect = Rect::from_min_size(
        pos2(
            screen.left() + (page_width - size.x) / 2.0,
            screen.top() + top.unwrap_or((screen.height() - size.y) / 2.0),
        ),
        size,
    );
    // Above the dim's order: egui raises a pressed area within its order,
    // and the dim covers the window, so the same order would put the dim
    // over the box after the first click.
    let area = egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            claim(ui, rect, id);
            paint_dialog_shadow(ui.painter(), rect, radius);
            ui.painter().rect_filled(rect, radius, theme::JF_BG);
            ui.set_clip_rect(rect);
            content(ui, rect)
        });
    (area.inner, close)
}

// jellyfin-web's form controls in dialogs, at 1em = 14.88 px.
/// An `h2` in a dialog: 1.5em on a 30.13 px line.
pub const FORM_HEADING: f32 = 30.11;
pub const FORM_HEADING_FONT: f32 = 1.5 * theme::JF_FONT;
/// A radio or checkbox row: 24 px, with 0.5em margins that collapse.
pub const FORM_ROW: f32 = 24.0;
pub const FORM_ROW_MARGIN: f32 = 0.5 * theme::JF_FONT;
const RADIO: f32 = 16.06;
/// From the row's left edge to its label.
pub const FORM_LABEL_LEFT: f32 = RADIO + 8.035;

/// A text baseline inside a dialog, Chrome's way: the half-leading in
/// LayoutUnits (1/64 px, truncated toward zero, so a line box shorter than
/// the font loses less than a pixel), and, as dialogs are composited as their
/// own layer, rounded to device pixels where page text is floored.
/// `galley_on_baseline` floors, hence the half pixel.
pub fn layer_baseline(ui: &Ui, top: f32, size: f32, line_height: f32) -> f32 {
    let ascent = (1.069 * size).round();
    let descent = (0.293 * size).round();
    let half_leading = ((line_height - ascent - descent) / 2.0 * 64.0).trunc() / 64.0;
    top + ascent + half_leading + 0.5 / ui.ctx().pixels_per_point()
}

/// A dialog `h2` whose line box starts at `top`.
pub fn form_heading(ui: &Ui, left: f32, top: f32, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::regular(FORM_HEADING_FONT), theme::JF_TEXT);
    widgets::galley_on_baseline(
        ui.painter(),
        left,
        layer_baseline(ui, top, FORM_HEADING_FONT, 30.132),
        galley,
    );
}

/// jellyfin-web's `emby-radio` row at `row` (24 px tall): a 1.9 px ring with
/// a dot when selected, accent with a halo while focused.
/// Returns true when clicked (or Space/Enter while focused).
pub fn form_radio(ui: &Ui, row: Rect, label: &str, selected: bool, id: Id) -> bool {
    let response = ui.interact(row, id, Sense::click());
    widgets::describe_button(&response, true, label);
    let center = pos2(row.left() + RADIO / 2.0, row.top() + 3.97 + RADIO / 2.0);
    let focused = response.has_focus();
    let color = if focused { theme::JF_ACCENT } else { theme::JF_TEXT };
    if focused {
        ui.painter().circle_filled(
            center,
            RADIO * 1.75 / 2.0,
            theme::with_alpha(theme::JF_ACCENT, 0.26),
        );
    }
    // The SVG strokes 3.87 px on the circle's edge, clipped to the circle:
    // a ring 1.93 px wide inside the edge. egui strokes outside the radius.
    let ring = 3.8688 / 2.0;
    ui.painter()
        .circle_stroke(center, RADIO / 2.0 - ring, egui::Stroke::new(ring, color));
    if selected {
        ui.painter().circle_filled(center, RADIO / 4.0, color);
    }
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::regular(theme::JF_FONT), theme::JF_TEXT);
    widgets::galley_on_baseline(
        ui.painter(),
        row.left() + FORM_LABEL_LEFT,
        layer_baseline(ui, row.top(), theme::JF_FONT, FORM_ROW),
        galley,
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// Moves keyboard focus to `first` as a dialog opens, as jellyfin-web
/// focuses a dialog's first control. Asked for a few frames, since the
/// click that opened the dialog can still be settling focus. `open` false
/// forgets the dialog so the next opening focuses again.
pub fn autofocus(ctx: &egui::Context, dialog: Id, first: Id, open: bool) {
    let key = dialog.with("autofocus");
    if !open {
        ctx.data_mut(|data| data.remove::<u8>(key));
        return;
    }
    let frames = ctx.data_mut(|data| *data.get_temp_mut_or_default::<u8>(key));
    if frames < 3 {
        ctx.data_mut(|data| data.insert_temp(key, frames + 1));
        ctx.memory_mut(|memory| memory.request_focus(first));
        ctx.request_repaint();
    }
}

/// The width of the widest label, for dialogs that shrink to fit.
pub fn widest_label(ctx: &egui::Context, labels: impl IntoIterator<Item = impl Into<String>>) -> f32 {
    labels
        .into_iter()
        .map(|label| {
            ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(label.into(), theme::regular(theme::JF_FONT), Color32::WHITE)
                    .size()
                    .x
            })
        })
        .fold(0.0, f32::max)
}

// jellyfin-web's `emby-checkbox` and `emby-collapse`, at 1em = 14.88 px.
const CHECKBOX_ROW: f32 = 34.95;
const CHECKBOX: f32 = 27.22;
const CHECKBOX_LABEL_LEFT: f32 = 2.4 * theme::JF_FONT;
/// A collapsible section's button: 0.9em padding around a 1.17em `h3`, and
/// its hairline.
const COLLAPSE_BUTTON: f32 = 51.27;
const COLLAPSE_BODY_PADDING: f32 = 1.25 * theme::JF_FONT;

/// A checkbox row, jellyfin-web's: a 27 px box with a 2 px border, accent
/// with a white check when ticked, and the label. Lays out its 0.5em top
/// margin (margins between rows collapse to one). Returns true when clicked.
pub fn checkbox(ui: &mut Ui, label: &str, checked: bool, id: Id) -> bool {
    ui.add_space(FORM_ROW_MARGIN);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CHECKBOX_ROW), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    widgets::describe_button(&response, true, label);
    let boxed = Rect::from_min_size(pos2(rect.left(), rect.top() + 3.0), vec2(CHECKBOX, CHECKBOX));
    let radius = 0.14 * theme::JF_FONT;
    let focused = response.has_focus();
    if checked {
        ui.painter().rect_filled(boxed, radius, theme::JF_ACCENT);
        widgets::border(ui.painter(), boxed, radius, 2.0, Color32::WHITE);
        icons::paint_centered(ui.painter(), Icon::Check, boxed.center(), 1.6 * theme::JF_FONT, Color32::WHITE);
    } else {
        let color = if focused { theme::JF_ACCENT } else { theme::JF_TEXT };
        widgets::border(ui.painter(), boxed, radius, 2.0, color);
    }
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::regular(theme::JF_FONT), theme::JF_TEXT);
    widgets::galley_on_baseline(
        ui.painter(),
        rect.left() + CHECKBOX_LABEL_LEFT,
        layer_baseline(ui, rect.top() + 7.43, theme::JF_FONT, 1.35 * theme::JF_FONT),
        galley,
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// A collapsible section header: the `h3` title, the expand chevron (turned
/// over while open) and a hairline under it. Lays out its 0.5em top margin.
/// Returns whether it was clicked.
pub fn collapsible(ui: &mut Ui, title: &str, open: bool, id: Id) -> bool {
    ui.add_space(FORM_ROW_MARGIN);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), COLLAPSE_BUTTON), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    widgets::describe_button(&response, true, title);
    let size = 1.17 * theme::JF_FONT;
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), theme::regular(size), theme::JF_TEXT);
    widgets::galley_on_baseline(
        ui.painter(),
        rect.left() + 0.1 * theme::JF_FONT,
        layer_baseline(ui, rect.top() + 0.9 * theme::JF_FONT, size, 23.48),
        galley,
    );
    let icon = if open { Icon::ExpandLess } else { Icon::ExpandMore };
    let icon_size = 1.36 * theme::JF_FONT;
    icons::paint_centered(
        ui.painter(),
        icon,
        pos2(rect.right() - 30.33 + icon_size / 2.0, rect.top() + 15.0 + icon_size / 2.0),
        icon_size,
        theme::JF_TEXT,
    );
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(rect.left(), rect.bottom() - 1.0), rect.right_bottom()),
        0.0,
        Color32::from_white_alpha(34),
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// An open section's body: #202020 behind `content`, with 1.25em padding.
pub fn collapse_body<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().top();
    ui.add_space(COLLAPSE_BODY_PADDING);
    let inner = ui
        .scope_builder(
            egui::UiBuilder::new().max_rect(Rect::from_min_max(
                pos2(ui.max_rect().left() + COLLAPSE_BODY_PADDING, ui.cursor().top()),
                pos2(ui.max_rect().right() - COLLAPSE_BODY_PADDING, f32::INFINITY),
            )),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                let inner = content(ui);
                ui.add_space(FORM_ROW_MARGIN);
                inner
            },
        );
    ui.add_space(COLLAPSE_BODY_PADDING);
    let rect = Rect::from_min_max(
        pos2(ui.max_rect().left(), top),
        pos2(ui.max_rect().right(), inner.response.rect.bottom() + COLLAPSE_BODY_PADDING),
    );
    ui.painter().set(background, egui::Shape::rect_filled(rect, 0.0, Color32::from_gray(32)));
    inner.inner
}

