//! Drawing primitives shared by the screens: text that elides or wraps,
//! cover-fit artwork with rounded corners, gradients, shadows, the round icon
//! button, and the horizontal shelf scroller.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::{RectShape, Shadow};
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, ColorImage, CornerRadius, CursorIcon, FontId, Galley, Id, Mesh, Painter, Pos2,
    Rect, Response, Sense, Shape, Stroke, StrokeKind, TextFormat, TextureHandle, TextureId,
    TextureOptions, Ui, Vec2, pos2, vec2,
};

use super::icons::{self, Icon};
use super::theme;

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
    let rect = Rect::from_min_size(pos2(x, center_y - galley.size().y / 2.0), galley.size());
    painter.galley(rect.min, galley, Color32::WHITE);
    rect
}

/// Paints a galley centered in `rect`.
pub fn galley_centered(painter: &Painter, rect: Rect, galley: Arc<Galley>) {
    let min = rect.center() - galley.size() / 2.0;
    painter.galley(min, galley, Color32::WHITE);
}

/// Paints a galley at `rect.min` (top-left aligned).
pub fn galley_at(painter: &Painter, min: Pos2, galley: Arc<Galley>) -> Rect {
    let rect = Rect::from_min_size(min, galley.size());
    painter.galley(min, galley, Color32::WHITE);
    rect
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
    let uv = cover_uv(texture_size, rect.size());
    painter.add(RectShape::filled(rect, radius, Color32::WHITE).with_texture(texture, uv));
}

/// The texture region that covers `target` without distortion.
pub fn cover_uv(texture_size: [usize; 2], target: Vec2) -> Rect {
    let [width, height] = texture_size;
    if width == 0 || height == 0 || target.x <= 0.0 || target.y <= 0.0 {
        return Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    }
    let texture_aspect = width as f32 / height as f32;
    let target_aspect = target.x / target.y;
    if texture_aspect > target_aspect {
        // Wider than the slot: crop the sides.
        let visible = target_aspect / texture_aspect;
        let margin = (1.0 - visible) / 2.0;
        Rect::from_min_max(pos2(margin, 0.0), pos2(1.0 - margin, 1.0))
    } else {
        let visible = texture_aspect / target_aspect;
        let margin = (1.0 - visible) / 2.0;
        Rect::from_min_max(pos2(0.0, margin), pos2(1.0, 1.0 - margin))
    }
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

/// A soft drop shadow under a rounded rect, like `drop-shadow-*` in CSS.
pub fn drop_shadow(
    painter: &Painter,
    rect: Rect,
    radius: f32,
    blur: f32,
    offset_y: f32,
    color: Color32,
) {
    let shadow = Shadow {
        offset: [0, offset_y.round().clamp(-127.0, 127.0) as i8],
        blur: blur.round().clamp(0.0, 255.0) as u8,
        spread: 0,
        color,
    };
    painter.add(shadow.as_shape(rect, radius));
}

/// A rounded rect filled with a 145° linear gradient from `from` to `to`.
///
/// The gradient is a tiny cached texture stretched over the rect, which keeps
/// the rounded corners (a vertex-colored mesh would not have them).
pub fn diagonal_gradient(painter: &Painter, rect: Rect, radius: f32, from: Color32, to: Color32) {
    let texture = gradient_texture(painter.ctx(), from, to);
    painter.add(
        RectShape::filled(rect, radius, Color32::WHITE)
            .with_texture(texture, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
    );
}

#[derive(Clone, Default)]
struct GradientTextures(HashMap<([u8; 4], [u8; 4]), TextureHandle>);

fn gradient_texture(ctx: &egui::Context, from: Color32, to: Color32) -> TextureId {
    let key = (from.to_array(), to.to_array());
    let id = Id::new("jellybridge-gradient-textures");
    let cached = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<GradientTextures>(id)
            .0
            .get(&key)
            .map(TextureHandle::id)
    });
    if let Some(texture) = cached {
        return texture;
    }
    const SIZE: usize = 32;
    // CSS 145deg: the gradient runs toward the lower right, mostly down.
    let angle = 145_f32.to_radians();
    let direction = vec2(angle.sin(), -angle.cos());
    let span = direction.x.abs() + direction.y.abs();
    let mut pixels = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let point = vec2(
                x as f32 / (SIZE - 1) as f32 - 0.5,
                y as f32 / (SIZE - 1) as f32 - 0.5,
            );
            let t = (point.dot(direction) / span + 0.5).clamp(0.0, 1.0);
            pixels.push(lerp_color(from, to, t));
        }
    }
    let image = ColorImage {
        size: [SIZE, SIZE],
        source_size: vec2(SIZE as f32, SIZE as f32),
        pixels,
    };
    let handle = ctx.load_texture("gradient", image, TextureOptions::LINEAR);
    let texture = handle.id();
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<GradientTextures>(id)
            .0
            .insert(key, handle);
    });
    texture
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let a = from.to_srgba_unmultiplied();
    let b = to.to_srgba_unmultiplied();
    let mix = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8;
    Color32::from_rgba_unmultiplied(mix(0), mix(1), mix(2), mix(3))
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

/// A thin progress bar along the bottom of a card: overlay rail, accent fill
/// anchored to the left edge.
pub fn card_progress(painter: &Painter, card: Rect, height: f32, progress: f32) {
    if !(progress > 0.0 && progress < 1.0) {
        return;
    }
    let rail = Rect::from_min_max(
        pos2(card.left(), card.bottom() - height),
        card.right_bottom(),
    );
    painter.rect_filled(rail, 0.0, theme::OVERLAY);
    let fill = Rect::from_min_size(rail.min, vec2(rail.width() * progress, height));
    painter.rect_filled(fill, 0.0, theme::ACCENT);
}

/// The round accent "watched" badge with a check mark.
pub fn watched_badge(painter: &Painter, rect: Rect, check_size: f32) {
    painter.rect_filled(rect, theme::RADIUS_LARGE, theme::ACCENT);
    icons::paint_centered(
        painter,
        Icon::Check,
        rect.center(),
        check_size,
        theme::TEXT_BRIGHT,
    );
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

/// The detail page's round icon toggle: played, favorite, play from start.
pub struct RoundIconButton<'a> {
    pub icon: Icon,
    pub label: &'a str,
    pub size: f32,
    pub active: bool,
    pub active_color: Color32,
    pub enabled: bool,
}

impl RoundIconButton<'_> {
    pub fn show(self, ui: &mut Ui, rect: Rect, id: Id) -> Response {
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, id, sense);
        describe_button(&response, self.enabled, self.label);
        let hovered = response.hovered() && self.enabled;
        let painter = ui.painter();
        let opacity = if self.enabled { 1.0 } else { 0.45 };
        let fill = if self.active {
            theme::ACCENT_DEEP
        } else if hovered {
            theme::SURFACE_HOVER
        } else {
            theme::SURFACE_RAISED
        };
        let radius = rect.width() / 2.0;
        painter.rect_filled(rect, radius, fill.gamma_multiply(opacity));
        if !self.active {
            border(
                painter,
                rect,
                radius,
                1.0,
                theme::BORDER.gamma_multiply(opacity),
            );
        }
        if response.has_focus() {
            border(
                painter,
                rect.expand(2.0),
                radius + 2.0,
                2.0,
                theme::ACCENT_BRIGHT,
            );
        }
        let color = if self.active {
            self.active_color
        } else {
            theme::TEXT_SECONDARY
        };
        icons::paint_centered(
            painter,
            self.icon,
            rect.center(),
            self.size * 0.5,
            color.gamma_multiply(opacity),
        );
        if self.enabled {
            pointer_on_hover(&response);
        }
        response.on_hover_text(self.label)
    }
}

/// A round ‹ or › paging button over a shelf.
pub fn shelf_arrow(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    left: bool,
    fill: Color32,
    hover_fill: Color32,
    outlined: bool,
) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    describe_button(
        &response,
        true,
        if left { "Scroll left" } else { "Scroll right" },
    );
    let painter = ui.painter();
    let radius = rect.width() / 2.0;
    painter.rect_filled(
        rect,
        radius,
        if response.hovered() { hover_fill } else { fill },
    );
    if outlined {
        border(painter, rect, radius, 1.0, theme::BORDER);
    }
    painter.text(
        rect.center() - vec2(0.0, 2.0),
        Align2::CENTER_CENTER,
        if left { "‹" } else { "›" },
        theme::regular(28.0),
        theme::TEXT_BRIGHT,
    );
    pointer_on_hover(&response);
    response.clicked()
}

/// Per-shelf scroll state, kept in egui memory under the shelf's id.
#[derive(Clone, Copy, Debug, Default)]
struct ShelfState {
    /// Where the shelf is headed; drawn through a 120 ms ease-out.
    target: f32,
    /// The gesture's locked axis: 0 none, 1 horizontal, 2 vertical.
    axis: u8,
    last_event: f64,
    /// What `center_on` last centered, so it re-centers only on change.
    centered: Option<(usize, u32)>,
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

    /// Opens scrolled so item `index` (of width `card_width`, `gap` apart)
    /// is centered, and re-centers whenever `index` or the content changes.
    pub fn center_on(&mut self, index: usize, card_width: f32, gap: f32) {
        let key = (index, self.content_width.round() as u32);
        if self.state.centered == Some(key) {
            return;
        }
        self.state.centered = Some(key);
        let center = index as f32 * (card_width + gap) + card_width / 2.0;
        self.state.target = (self.viewport.width() / 2.0 - center).clamp(self.minimum_offset, 0.0);
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
        // A 2:3 poster in a square slot loses a sixth top and bottom.
        let uv = cover_uv([200, 300], vec2(100.0, 100.0));
        assert!((uv.min.y - 1.0 / 6.0).abs() < 1e-4);
        assert!((uv.max.y - 5.0 / 6.0).abs() < 1e-4);
        assert_eq!((uv.min.x, uv.max.x), (0.0, 1.0));

        // A 16:9 banner in a 2:3 slot is cropped at the sides.
        let uv = cover_uv([1600, 900], vec2(200.0, 300.0));
        assert_eq!((uv.min.y, uv.max.y), (0.0, 1.0));
        assert!(uv.min.x > 0.3 && uv.max.x < 0.7);
    }
}
