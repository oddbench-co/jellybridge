//! jellyfin-web's overlays: the centred dialog (sort, filter) and the action
//! sheet that opens on its button (view, item menus). Both dim the page
//! behind them and close on Escape or a click outside.

use egui::{Align2, Color32, Id, Order, Rect, Sense, Ui, pos2, vec2};

use super::icons::{self, Icon};
use super::theme;
use super::widgets;

/// The dim behind dialogs: black at 50%.
const BACKDROP: Color32 = Color32::from_black_alpha(128);
/// Action sheet rows.
const SHEET_ROW: f32 = 40.0;

fn shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 16],
        blur: 24,
        spread: 2,
        color: Color32::from_black_alpha(90),
    }
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

/// A dialog centred in the window, `width` wide, filled by `content`.
/// `height` fixes its height (content scrolls inside); otherwise it fits its
/// content. Returns `content`'s result and whether the dialog asked to
/// close (Escape, or a click on the dim around it).
pub fn modal<R>(
    ctx: &egui::Context,
    id: Id,
    width: f32,
    height: Option<f32>,
    content: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    let max_height = ctx.content_rect().height() - 48.0;
    let fixed = height.map(|height| height.min(max_height));
    let response = egui::Modal::new(id)
        .backdrop_color(BACKDROP)
        .frame(
            egui::Frame::new()
                .fill(theme::JF_BG)
                .corner_radius(theme::JF_RADIUS)
                .shadow(shadow()),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(fixed.unwrap_or(max_height));
            if let Some(fixed) = fixed {
                scroll = scroll
                    .min_scrolled_height(fixed)
                    .auto_shrink([false, false]);
            }
            scroll.show(ui, content).inner
        });
    let close = response.should_close();
    (response.inner, close)
}

/// What an action sheet row shows.
pub struct SheetItem<'a> {
    pub label: &'a str,
    pub icon: Option<Icon>,
    pub checked: bool,
}

/// An action sheet anchored on `anchor`, its `selected` row level with the
/// anchor's centre as jellyfin-web places it. Returns the picked row, and
/// whether the sheet closed (a pick closes it too).
pub fn action_sheet(
    ctx: &egui::Context,
    id: Id,
    anchor: Rect,
    items: &[SheetItem<'_>],
    selected: usize,
) -> (Option<usize>, bool) {
    let close = backdrop(ctx, id);
    let screen = ctx.content_rect();
    let any_icon = items.iter().any(|item| item.icon.is_some() || item.checked);
    let font = theme::regular(theme::JF_FONT);
    let text_left = if any_icon { 63.0 } else { 17.0 };
    let widest = items
        .iter()
        .map(|item| {
            ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(item.label.to_owned(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
        })
        .fold(0.0, f32::max);
    let width = (text_left + widest + 24.0).max(168.0);
    let height = items.len() as f32 * SHEET_ROW + 12.0;
    let top = anchor.center().y - 6.0 - selected as f32 * SHEET_ROW - SHEET_ROW / 2.0;
    let top = top.clamp(
        screen.top() + 8.0,
        (screen.bottom() - height - 8.0).max(8.0),
    );
    let left = anchor
        .left()
        .clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(8.0));
    let area = egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(pos2(left, top))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::JF_BG)
                .corner_radius(1.5)
                .shadow(shadow())
                .inner_margin(egui::Margin::symmetric(0, 6))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let mut picked = None;
                    for (index, item) in items.iter().enumerate() {
                        let (row, response) =
                            ui.allocate_exact_size(vec2(width, SHEET_ROW), Sense::click());
                        widgets::describe_button(&response, true, item.label);
                        if response.hovered() || response.has_focus() {
                            ui.painter()
                                .rect_filled(row, 0.0, Color32::from_white_alpha(20));
                        }
                        let icon = if item.checked {
                            Some(Icon::Check)
                        } else {
                            item.icon
                        };
                        if let Some(icon) = icon {
                            icons::paint_centered(
                                ui.painter(),
                                icon,
                                pos2(row.left() + 27.0, row.center().y),
                                20.0,
                                theme::JF_TEXT,
                            );
                        }
                        ui.painter().text(
                            pos2(row.left() + text_left, row.center().y),
                            Align2::LEFT_CENTER,
                            item.label,
                            font.clone(),
                            theme::JF_TEXT,
                        );
                        widgets::pointer_on_hover(&response);
                        if response.clicked() {
                            picked = Some(index);
                        }
                    }
                    picked
                })
                .inner
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

/// A dialog section heading, 22.3 px like jellyfin-web's `h2`.
pub fn heading(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::regular(theme::JF_FONT_SECTION),
        theme::JF_TEXT,
    );
}

/// A radio row: a 16 px ring, filled with the accent when selected.
pub fn radio(ui: &mut Ui, label: &str, selected: bool, id: Id) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 31.5), Sense::click());
    let response = ui.interact(rect, id, Sense::click()).union(response);
    widgets::describe_button(&response, true, label);
    let center = pos2(rect.left() + 8.0, rect.center().y);
    let ring = if selected || response.hovered() {
        theme::JF_ACCENT
    } else {
        theme::JF_TEXT_SECONDARY
    };
    ui.painter()
        .circle_stroke(center, 7.0, egui::Stroke::new(2.0, ring));
    if selected {
        ui.painter().circle_filled(center, 3.5, theme::JF_ACCENT);
    }
    ui.painter().text(
        pos2(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::regular(theme::JF_FONT),
        theme::JF_TEXT,
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// A checkbox row, jellyfin-web's square box with a check when ticked.
pub fn checkbox(ui: &mut Ui, label: &str, checked: bool, id: Id) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 31.5), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    widgets::describe_button(&response, true, label);
    let boxed = Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(18.0, 18.0));
    if checked {
        ui.painter().rect_filled(boxed, 2.0, theme::JF_ACCENT);
        icons::paint_centered(
            ui.painter(),
            Icon::Check,
            boxed.center(),
            16.0,
            Color32::WHITE,
        );
    } else {
        let color = if response.hovered() {
            theme::JF_ACCENT
        } else {
            theme::JF_TEXT_SECONDARY
        };
        ui.painter().rect_stroke(
            boxed.shrink(1.0),
            2.0,
            egui::Stroke::new(2.0, color),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::regular(theme::JF_FONT),
        theme::JF_TEXT,
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// A collapsible section header: the title, a chevron, and a hairline under
/// it. Returns whether it was clicked.
pub fn collapsible(ui: &mut Ui, title: &str, open: bool, id: Id) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    widgets::describe_button(&response, true, title);
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::regular(theme::JF_FONT),
        theme::JF_TEXT,
    );
    let chevron_center = pos2(rect.right() - 10.0, rect.center().y);
    let icon = if open {
        Icon::ExpandLess
    } else {
        Icon::ExpandMore
    };
    icons::paint_centered(ui.painter(), icon, chevron_center, 20.0, theme::JF_TEXT);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.33, Color32::from_white_alpha(34)),
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}
