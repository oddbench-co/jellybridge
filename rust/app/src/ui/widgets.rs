//! Drawing primitives shared by the screens: text that elides or wraps,
//! cover-fit artwork with rounded corners, gradients, shadows, the round icon
//! button, and the horizontal shelf scroller.

use std::sync::Arc;

use egui::epaint::RectShape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, CursorIcon, FontId, Galley, Id, Mesh, Painter, Pos2, Rect, Response,
    Shape, Stroke, StrokeKind, TextFormat, TextureId, Ui, Vec2, pos2, vec2,
};

/// One line of text, cut with an ellipsis at `max_width`.
pub fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// Word-wrapped text at `width`, optionally cut after `max_rows` rows.
pub fn wrapped(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
    max_rows: Option<usize>,
) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: max_rows.unwrap_or(usize::MAX),
        break_anywhere: false,
        overflow_character: max_rows.map(|_| '…'),
    };
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// The width `text` needs on one line.
pub fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, Color32::WHITE))
        .size()
        .x
}

/// Paints a galley with its left edge at `x`, vertically centered on `center_y`.
pub fn galley_v_centered(painter: &Painter, x: f32, center_y: f32, galley: Arc<Galley>) -> Rect {
    let top = snap_text_y(painter, center_y - galley.size().y / 2.0);
    let rect = Rect::from_min_size(pos2(x, top), galley.size());
    painter.galley(rect.min, galley, Color32::WHITE);
    rect
}

/// Paints a galley centered in `rect`.
pub fn galley_centered(painter: &Painter, rect: Rect, galley: Arc<Galley>) {
    let min = rect.center() - galley.size() / 2.0;
    painter.galley(
        pos2(min.x, snap_text_y(painter, min.y)),
        galley,
        Color32::WHITE,
    );
}

/// Distance from a galley's top to its first line's baseline.
pub fn galley_baseline(galley: &Galley) -> f32 {
    galley
        .rows
        .first()
        .and_then(|row| row.row.glyphs.first().map(|glyph| row.pos.y + glyph.pos.y))
        .unwrap_or(galley.size().y * 0.75)
}

/// Paints `galley` with its first baseline at `baseline`, snapped down
/// onto the device-pixel grid as Chrome places text (measured: rounding
/// put one line in three a pixel off). `x` is its left edge.
/// `text` broken into the lines egui would wrap it into at `width`.
pub fn wrap_lines(ui: &Ui, text: &str, font: egui::FontId, width: f32) -> Vec<String> {
    let galley = ui
        .painter()
        .layout(text.to_owned(), font, Color32::WHITE, width);
    galley
        .rows
        .iter()
        .map(|row| {
            row.glyphs
                .iter()
                .map(|glyph| glyph.chr)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

pub fn galley_on_baseline(painter: &Painter, x: f32, baseline: f32, galley: Arc<Galley>) -> Rect {
    let pixels = painter.pixels_per_point();
    let baseline = (baseline * pixels).floor() / pixels;
    let min = pos2(x, baseline - galley_baseline(&galley));
    let rect = Rect::from_min_size(min, galley.size());
    painter.galley(min, galley, Color32::WHITE);
    rect
}

/// Snaps a line of text down onto the device-pixel grid, as Chrome lays
/// text out (egui would round to the nearest pixel, which puts some lines
/// a pixel lower than jellyfin-web's).
pub fn snap_text_y(painter: &Painter, y: f32) -> f32 {
    let pixels = painter.pixels_per_point();
    (y * pixels).floor() / pixels
}

/// Paints `texture` scaled to cover `rect`, cropping the overflow evenly --
/// CSS `object-fit: cover` -- with rounded corners.
pub fn image_cover(
    painter: &Painter,
    texture: TextureId,
    texture_size: [usize; 2],
    rect: Rect,
    radius: impl Into<CornerRadius>,
) {
    // Chrome sizes and centres a `cover` image on the element's exact box,
    // then paints it into that box snapped onto device pixels. Sizing it
    // on the snapped box instead can flip which side is cropped and slide
    // the whole picture a device pixel.
    let pixels = painter.pixels_per_point();
    let exact = Rect::from_min_max(rect.min * pixels, rect.max * pixels);
    let snapped = Rect::from_min_max(exact.min.round(), exact.max.round());
    let uv = cover_uv_snapped(texture_size, exact, snapped);
    let rect = Rect::from_min_max(snapped.min / pixels, snapped.max / pixels);
    painter.add(RectShape::filled(rect, radius, Color32::WHITE).with_texture(texture, uv));
}

/// `cover_uv` as Chrome draws `background-size: cover; background-position:
/// center`: the image is scaled to cover the `exact` box, centred with an
/// offset rounded to whole device pixels, and anchored at the `snapped`
/// box it is painted into. Both are in device pixels.
pub fn cover_uv_snapped(texture_size: [usize; 2], exact: Rect, snapped: Rect) -> Rect {
    let [width, height] = texture_size;
    if width == 0 || height == 0 || exact.width() <= 0.0 || exact.height() <= 0.0 {
        return Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    }
    let (width, height) = (width as f32, height as f32);
    let scale = (exact.width() / width).max(exact.height() / height);
    let offset = vec2(
        ((exact.width() - width * scale) / 2.0).round(),
        ((exact.height() - height * scale) / 2.0).round(),
    );
    let tile = vec2(width * scale, height * scale);
    let start = -offset;
    Rect::from_min_max(
        pos2(start.x / tile.x, start.y / tile.y),
        pos2(
            (start.x + snapped.width()) / tile.x,
            (start.y + snapped.height()) / tile.y,
        ),
    )
}

/// Paints `texture` scaled to fit inside `rect` (`object-fit: contain`).
pub fn image_contain(painter: &Painter, texture: TextureId, texture_size: [usize; 2], rect: Rect) {
    let [width, height] = texture_size;
    if width == 0 || height == 0 {
        return;
    }
    let scale = (rect.width() / width as f32).min(rect.height() / height as f32);
    let size = vec2(width as f32 * scale, height as f32 * scale);
    painter.image(
        texture,
        Rect::from_center_size(rect.center(), size),
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// A rect filled with a top-to-bottom gradient (square corners).
pub fn vertical_gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
}

/// A rounded outline drawn inside `rect`.
pub fn border(painter: &Painter, rect: Rect, radius: f32, width: f32, color: Color32) {
    painter.rect_stroke(rect, radius, Stroke::new(width, color), StrokeKind::Inside);
}

/// Marks a response as a button for assistive technology and gives it a
/// tooltip, which icon-only controls otherwise lack.
pub fn describe_button(response: &Response, enabled: bool, label: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
}

/// Pointer cursor over clickable things, as on the web client.
pub fn pointer_on_hover(response: &Response) {
    if response.hovered() {
        response.ctx.set_cursor_icon(CursorIcon::PointingHand);
    }
}

/// Per-shelf scroll state, kept in egui memory under the shelf's id.
#[derive(Clone, Copy, Debug, Default)]
struct ShelfState {
    /// Where the shelf is headed; drawn through a 120 ms ease-out.
    target: f32,
    /// The gesture's locked axis: 0 none, 1 horizontal, 2 vertical.
    axis: u8,
    last_event: f64,
}

/// A horizontally scrolling strip: wheel and trackpad scrolling with a
/// per-gesture axis lock, paging, keyboard focus following, and clamping to
/// its content every frame (so a resize can never leave it past its end).
pub struct Shelf {
    pub id: Id,
    pub viewport: Rect,
    pub content_width: f32,
    pub minimum_offset: f32,
    state: ShelfState,
}

impl Shelf {
    /// Wheel/trackpad multiplier: a tick or flick covers a useful distance.
    const SPEED: f32 = 3.0;
    /// Scroll events carry no begin/end phase, so a short idle period
    /// defines "gesture ended" and releases the axis lock.
    const AXIS_RELEASE_SECONDS: f64 = 0.2;

    pub fn begin(ui: &Ui, id: Id, viewport: Rect, content_width: f32) -> Self {
        let mut state = ui
            .ctx()
            .data(|data| data.get_temp::<ShelfState>(id))
            .unwrap_or_default();
        let minimum_offset = (viewport.width() - content_width).min(0.0);
        state.target = state.target.clamp(minimum_offset, 0.0);
        Self {
            id,
            viewport,
            content_width,
            minimum_offset,
            state,
        }
    }

    /// Applies this frame's scroll input and returns the drawn offset.
    pub fn scroll(&mut self, ui: &Ui) -> f32 {
        let fits = self.content_width <= self.viewport.width();
        let now = ui.input(|input| input.time);
        if fits {
            // A shelf whose cards already fit can never own a gesture, so it
            // must never hold a lock that would swallow the page's scrolling.
            self.state.axis = 0;
        } else if now - self.state.last_event > Self::AXIS_RELEASE_SECONDS {
            self.state.axis = 0;
        }
        let hovered = ui.rect_contains_pointer(self.viewport);
        let delta = ui.input(|input| input.smooth_scroll_delta);
        if hovered && !fits && delta != Vec2::ZERO {
            if self.state.axis == 0 {
                self.state.axis = if delta.x.abs() > delta.y.abs() { 1 } else { 2 };
            }
            self.state.last_event = now;
            if self.state.axis == 1 {
                self.state.target =
                    (self.state.target + delta.x * Self::SPEED).clamp(self.minimum_offset, 0.0);
                // Swallow the whole event so its vertical component cannot
                // scroll the page mid-gesture.
                ui.ctx()
                    .input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
            }
        }
        ui.ctx()
            .animate_value_with_time(self.id.with("offset"), self.state.target, 0.12)
    }

    /// Pages by 80% of the viewport, as the arrow buttons do.
    pub fn page(&mut self, forward: bool) {
        let step = self.viewport.width() * 0.8;
        self.state.target = if forward {
            (self.state.target - step).max(self.minimum_offset)
        } else {
            (self.state.target + step).min(0.0)
        };
    }

    /// Brings a keyboard-focused card fully into view.
    pub fn reveal(&mut self, card_left: f32, card_width: f32) {
        let visible_left = -self.state.target;
        let visible_right = visible_left + self.viewport.width();
        if card_left < visible_left {
            self.state.target = -card_left;
        } else if card_left + card_width > visible_right {
            self.state.target = -(card_left + card_width - self.viewport.width());
        }
        self.state.target = self.state.target.clamp(self.minimum_offset, 0.0);
    }

    pub fn can_page_back(&self, drawn_offset: f32) -> bool {
        drawn_offset < -1.0
    }

    pub fn can_page_forward(&self, drawn_offset: f32) -> bool {
        self.content_width > self.viewport.width() && drawn_offset > self.minimum_offset + 1.0
    }

    /// Persists the state for the next frame.
    pub fn end(self, ui: &Ui) {
        ui.ctx()
            .data_mut(|data| data.insert_temp(self.id, self.state));
    }
}

/// A child `Ui` whose drawing and hit-testing are clipped to `rect`.
pub fn clipped_child(ui: &mut Ui, rect: Rect) -> Ui {
    let clip = rect.intersect(ui.clip_rect());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(clip);
    child
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_the_long_side_evenly() {
        let slot = |w: f32, h: f32| Rect::from_min_size(Pos2::ZERO, vec2(w, h));
        // A 2:3 poster in a square slot loses a sixth top and bottom.
        let uv = cover_uv_snapped([200, 300], slot(100.0, 100.0), slot(100.0, 100.0));
        assert!((uv.min.y - 1.0 / 6.0).abs() < 1e-4);
        assert!((uv.max.y - 5.0 / 6.0).abs() < 1e-4);
        assert_eq!((uv.min.x, uv.max.x), (0.0, 1.0));

        // A 16:9 banner in a 2:3 slot is cropped at the sides.
        let uv = cover_uv_snapped([1600, 900], slot(200.0, 300.0), slot(200.0, 300.0));
        assert_eq!((uv.min.y, uv.max.y), (0.0, 1.0));
        assert!(uv.min.x > 0.3 && uv.max.x < 0.7);
    }

    #[test]
    fn cover_is_sized_on_the_exact_box_not_the_snapped_one() {
        // jellyfin-web's 16:9 card at 1.5x: sized on the exact box, a
        // 695x391 image is scaled by the width and nothing is cropped off
        // the left. Sized on the snapped 566x319 box it would be scaled by
        // the height and slide a device pixel left.
        let exact = Rect::from_min_size(pos2(125.531, 231.492), vec2(566.648, 318.727));
        let snapped = Rect::from_min_max(exact.min.round(), exact.max.round());
        let uv = cover_uv_snapped([695, 391], exact, snapped);
        assert_eq!(uv.min.x, 0.0);
    }
}
