//! The page scrollbar jellyfin-web shows in desktop Chrome: the classic
//! (non-overlay) bar, made thin and recoloured by jellyfin's
//! `scrollbar-width: thin; scrollbar-color: #3b3b3b #202020`.
//!
//! The browser scrolls the whole document, so the bar runs the full window
//! height beside the fixed header, and it takes its width out of the
//! layout. Our pages scroll an area under the header instead; whatever sits
//! above that area counts as document, so the thumb keeps Chrome's size and
//! position.
//!
//! Geometry measured on jellyfin-web at 1.5x, in CSS px.

use egui::scroll_area::ScrollAreaOutput;
use egui::{Color32, Id, Rect, Sense, Ui, UiBuilder, pos2, vec2};

/// For the page's `ScrollArea`: this module draws its bar.
pub const HIDDEN: egui::scroll_area::ScrollBarVisibility =
    egui::scroll_area::ScrollBarVisibility::AlwaysHidden;
/// The bar's width, taken from the page's layout while the page overflows.
pub const WIDTH: f32 = 10.0;
const TRACK: Color32 = Color32::from_gray(32);
const THUMB: Color32 = Color32::from_gray(59);
/// The arrow button at each end.
const BUTTON: f32 = 12.0;
const THUMB_INSET: f32 = 2.0;
const THUMB_MIN: f32 = 20.0;
/// What an arrow click scrolls (Chrome's line step).
const LINE: f32 = 40.0;
/// Chrome's page step: the viewport less an eighth.
const PAGE_FRACTION: f32 = 0.875;
/// Held arrows and track repeat after this delay, at this interval.
const REPEAT_DELAY: f64 = 0.25;
const REPEAT_EVERY: f64 = 0.05;

/// What the press that started on the bar is doing.
#[derive(Clone, Copy)]
enum Press {
    /// Dragging the thumb: where the pointer and the offset started.
    Thumb { pointer: f32, offset: f32 },
    /// Holding an arrow or the track: the step and when it next repeats.
    Repeat { step: Step, next: f64 },
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Line(f32),
    /// Pages toward the pointer until the thumb reaches it.
    Page(f32),
}

/// Space the page gives up to the bar: the bar's width while the page
/// overflowed last frame (the bar itself decides, after laying it out).
pub fn reserved(ui: &Ui) -> f32 {
    let shown = ui.ctx().data(|data| data.get_temp::<bool>(shown_id()));
    if shown.unwrap_or(false) { WIDTH } else { 0.0 }
}

fn shown_id() -> Id {
    Id::new("page-scrollbar-shown")
}

/// Draws the bar for a page's scroll area at the window's right edge and
/// applies presses on it to the area's offset.
pub fn draw<R>(ui: &mut Ui, output: &ScrollAreaOutput<R>) {
    let ctx = ui.ctx().clone();
    let window = ctx.content_rect();
    let above = (output.inner_rect.top() - window.top()).max(0.0);
    let viewport = output.inner_rect.height();
    let content = output.content_size.y;
    let overflows = content > viewport + 0.5;
    let was_shown = ctx.data_mut(|data| data.get_temp::<bool>(shown_id()));
    if was_shown != Some(overflows) {
        ctx.data_mut(|data| data.insert_temp(shown_id(), overflows));
        // The page relays out without (or with) the bar's width.
        ctx.request_repaint();
    }
    if !overflows {
        return;
    }
    let strip = Rect::from_min_max(pos2(window.right() - WIDTH, window.top()), window.max);
    // The page's own Ui is clipped to the page, which ends where the bar
    // starts.
    let mut bar = ui.new_child(UiBuilder::new().max_rect(strip));
    bar.set_clip_rect(strip);
    let ui = &bar;
    let painter = ui.painter();
    painter.rect_filled(strip, 0.0, TRACK);
    paint_arrow(ui, strip, true);
    paint_arrow(ui, strip, false);

    let range = content - viewport;
    let track = Rect::from_min_max(
        pos2(strip.left(), strip.top() + BUTTON),
        pos2(strip.right(), strip.bottom() - BUTTON),
    );
    // The document's numbers: the header and anything else above the
    // area scroll with the page in the browser.
    let visible = viewport + above;
    let total = content + above;
    let thumb_length = (track.height() * visible / total)
        .max(THUMB_MIN)
        .min(track.height());
    let travel = (track.height() - thumb_length).max(1.0);
    let offset = output.state.offset.y.clamp(0.0, range);
    let thumb_top = track.top() + travel * offset / range;
    let thumb = Rect::from_min_max(
        pos2(strip.left() + THUMB_INSET, thumb_top),
        pos2(strip.right() - THUMB_INSET, thumb_top + thumb_length),
    );
    painter.rect_filled(thumb, (WIDTH - 2.0 * THUMB_INSET) / 2.0, THUMB);

    let response = ui.interact(
        strip,
        output.id.with("page-scrollbar"),
        Sense::click_and_drag(),
    );
    let press_id = output.id.with("page-scrollbar-press");
    let now = ui.input(|input| input.time);
    let pointer = response.interact_pointer_pos();
    let mut press = ctx.data(|data| data.get_temp::<Press>(press_id));
    if response.is_pointer_button_down_on() {
        if let (None, Some(at)) = (press, pointer) {
            press = Some(if thumb.contains(at) {
                Press::Thumb {
                    pointer: at.y,
                    offset,
                }
            } else {
                let step = if at.y < track.top() {
                    Step::Line(-LINE)
                } else if at.y > track.bottom() {
                    Step::Line(LINE)
                } else if at.y < thumb.top() {
                    Step::Page(-viewport * PAGE_FRACTION)
                } else {
                    Step::Page(viewport * PAGE_FRACTION)
                };
                // The first step lands at once; holding repeats it.
                scroll_to(ui, output, offset + step.amount(), range);
                Press::Repeat {
                    step,
                    next: now + REPEAT_DELAY,
                }
            });
        }
    } else {
        press = None;
    }
    match press {
        Some(Press::Thumb {
            pointer: start,
            offset: from,
        }) => {
            if let Some(at) = pointer {
                scroll_to(ui, output, from + (at.y - start) * range / travel, range);
            }
        }
        Some(Press::Repeat { step, next }) => {
            let reached = match (step, pointer) {
                // Paging stops once the thumb is under the pointer.
                (Step::Page(amount), Some(at)) => {
                    if amount < 0.0 {
                        at.y >= thumb.top()
                    } else {
                        at.y <= thumb.bottom()
                    }
                }
                _ => false,
            };
            if !reached {
                if now >= next {
                    scroll_to(ui, output, offset + step.amount(), range);
                    press = Some(Press::Repeat {
                        step,
                        next: now + REPEAT_EVERY,
                    });
                }
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(REPEAT_EVERY));
            }
        }
        None => {}
    }
    ctx.data_mut(|data| match press {
        Some(press) => {
            data.insert_temp(press_id, press);
        }
        None => data.remove::<Press>(press_id),
    });
}

impl Step {
    fn amount(self) -> f32 {
        match self {
            Step::Line(amount) | Step::Page(amount) => amount,
        }
    }
}

fn scroll_to<R>(ui: &Ui, output: &ScrollAreaOutput<R>, offset: f32, range: f32) {
    let mut state = output.state;
    state.offset.y = offset.clamp(0.0, range);
    state.store(ui.ctx(), output.id);
    ui.ctx().request_repaint();
}

/// Chrome's arrow: an unantialiased triangle, filled where device-pixel
/// centres fall inside it, so it is built from whole device-pixel rows
/// (as a mesh: a painted rect this thin would be feathered away).
fn paint_arrow(ui: &Ui, strip: Rect, up: bool) {
    let pixels = ui.ctx().pixels_per_point();
    // In CSS px from the bar's left edge and from its end.
    let (apex, base, half, centre) = (3.0, 22.0 / 3.0, 3.0, WIDTH / 2.0);
    let left = (strip.left() * pixels).round();
    let centre_x = left + centre * pixels;
    let mut mesh = egui::Mesh::default();
    let rows = ((apex * pixels).floor() as i32)..((base * pixels).ceil() as i32);
    for row in rows {
        // Distance of this row's pixel centres from the arrow's tip.
        let depth = (row as f32 + 0.5) / pixels - apex;
        if depth <= 0.0 {
            continue;
        }
        let half_width = half * depth / (base - apex) * pixels;
        let first = (centre_x - half_width - 0.5).ceil();
        let last = (centre_x + half_width - 0.5).floor();
        if last < first {
            continue;
        }
        let y = if up {
            (strip.top() * pixels).round() + row as f32
        } else {
            (strip.bottom() * pixels).round() - 1.0 - row as f32
        };
        let rect = Rect::from_min_size(
            pos2(first / pixels, y / pixels),
            vec2((last - first + 1.0) / pixels, 1.0 / pixels),
        );
        mesh.add_colored_rect(rect, THUMB);
    }
    ui.painter().add(mesh);
}
