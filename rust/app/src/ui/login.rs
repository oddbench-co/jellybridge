//! Sign-in screen: brand panel and the server/username/password form.

use egui::{Align2, Color32, Id, Key, Rect, Sense, TextEdit, Ui, pos2, vec2};

use super::UiState;
use super::app::UiAction;
use super::theme;
use super::widgets;

/// What the user has typed. Ephemeral UI state; it never leaves the window
/// except through [`UiAction::Login`].
pub struct LoginForm {
    pub server: String,
    pub username: String,
    pub password: String,
}

impl Default for LoginForm {
    fn default() -> Self {
        Self {
            server: "http://".to_owned(),
            username: String::new(),
            password: String::new(),
        }
    }
}

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    widgets::vertical_gradient(ui.painter(), screen, theme::BG, theme::SURFACE_DIM);

    let card = Rect::from_center_size(
        screen.center(),
        vec2((screen.width() - 48.0).min(860.0), 430.0),
    );
    widgets::drop_shadow(
        ui.painter(),
        card,
        theme::RADIUS_LARGE,
        24.0,
        0.0,
        theme::with_alpha(theme::SCRIM, 0.5),
    );
    ui.painter()
        .rect_filled(card, theme::RADIUS_LARGE, theme::SURFACE);
    widgets::border(ui.painter(), card, theme::RADIUS_LARGE, 1.0, theme::BORDER);

    let inner = card.shrink(38.0);
    let brand_width = inner.width() * 0.42;
    let brand = Rect::from_min_size(inner.min, vec2(brand_width, inner.height()));
    draw_brand(ui, state, brand);

    let divider_x = brand.right() + 40.0;
    ui.painter().rect_filled(
        Rect::from_min_size(pos2(divider_x, inner.top()), vec2(1.0, inner.height())),
        0.0,
        theme::BORDER,
    );
    let form = Rect::from_min_max(pos2(divider_x + 41.0, inner.top()), inner.max);
    draw_form(ui, state, form, actions);
}

fn draw_brand(ui: &mut Ui, state: &UiState, rect: Rect) {
    // Logo, name and tagline, centered as one block.
    let block_height = 62.0 + 12.0 + 42.0 + 12.0 + 17.0;
    let top = rect.center().y - block_height / 2.0;
    let logo = Rect::from_center_size(pos2(rect.center().x, top + 31.0), vec2(62.0, 62.0));
    let painter = ui.painter();
    painter.rect_filled(logo, theme::RADIUS_LARGE, theme::ACCENT_SURFACE);
    if let Some(icon) = &state.brand_icon {
        widgets::image_contain(painter, icon.id(), icon.size(), logo);
    }
    painter.text(
        pos2(rect.center().x, logo.bottom() + 12.0),
        Align2::CENTER_TOP,
        "JellyBridge",
        theme::bold(36.0),
        theme::ACCENT,
    );
    painter.text(
        pos2(rect.center().x, logo.bottom() + 12.0 + 42.0 + 12.0),
        Align2::CENTER_TOP,
        "Native Jellyfin playback with MPV",
        theme::regular(14.0),
        theme::TEXT_MUTED,
    );
}

fn draw_form(ui: &mut Ui, state: &mut UiState, rect: Rect, actions: &mut Vec<UiAction>) {
    let busy = state.busy;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.label(
        egui::RichText::new("Sign in")
            .font(theme::bold(25.0))
            .color(theme::TEXT_PRIMARY),
    );
    ui.add_space(7.0);

    let field =
        |ui: &mut Ui, label: &str, value: &mut String, hint: &str, password: bool, id: &str| {
            ui.label(
                egui::RichText::new(label)
                    .font(theme::regular(theme::FONT_BODY))
                    .color(theme::TEXT_MUTED),
            );
            let response = ui.add_enabled(
                !busy,
                TextEdit::singleline(value)
                    .id(Id::new(("login", id)))
                    .hint_text(hint)
                    .password(password)
                    .font(theme::regular(theme::FONT_SUBTITLE))
                    .margin(vec2(10.0, 8.0))
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(7.0);
            response
        };
    let form = &mut state.login;
    let server = field(
        ui,
        "Server URL",
        &mut form.server,
        "http://jellyfin.local:8096",
        false,
        "server",
    );
    field(
        ui,
        "Username",
        &mut form.username,
        "Jellyfin username",
        false,
        "username",
    );
    let password = field(
        ui,
        "Password",
        &mut form.password,
        "Password",
        true,
        "password",
    );
    // First visit: put the caret where typing should start.
    if !busy && ui.ctx().memory(|memory| memory.focused().is_none()) && form.username.is_empty() {
        let target = if form.server == "http://" {
            server.id
        } else {
            Id::new(("login", "username"))
        };
        ui.ctx().memory_mut(|memory| memory.request_focus(target));
    }

    if !state.message.is_empty() {
        let galley = widgets::wrapped(
            ui,
            &state.message,
            theme::regular(theme::FONT_BODY),
            theme::DANGER_TEXT,
            rect.width(),
            Some(3),
        );
        let (message_rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
        ui.painter()
            .galley(message_rect.min, galley, Color32::WHITE);
    }

    let can_submit = !busy && !form.server.is_empty() && !form.username.is_empty();
    let submitted_by_enter =
        password.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
    let (button_rect, button) = ui.allocate_exact_size(
        vec2(rect.width(), 36.0),
        if can_submit {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    widgets::describe_button(&button, can_submit, "Connect");
    let fill = if !can_submit {
        theme::SURFACE_RAISED
    } else if button.hovered() {
        theme::ACCENT_BRIGHT
    } else {
        theme::ACCENT
    };
    ui.painter()
        .rect_filled(button_rect, theme::RADIUS_SMALL, fill);
    if button.has_focus() {
        widgets::border(
            ui.painter(),
            button_rect.expand(2.0),
            theme::RADIUS_SMALL + 2.0,
            2.0,
            theme::TEXT_BRIGHT,
        );
    }
    ui.painter().text(
        button_rect.center(),
        Align2::CENTER_CENTER,
        if busy { "Connecting…" } else { "Connect" },
        theme::bold(theme::FONT_SUBTITLE),
        if can_submit {
            theme::TEXT_BRIGHT
        } else {
            theme::TEXT_FAINT
        },
    );
    if can_submit {
        widgets::pointer_on_hover(&button);
    }
    if (button.clicked() || submitted_by_enter) && can_submit {
        actions.push(UiAction::Login {
            server: form.server.trim().to_owned(),
            username: form.username.trim().to_owned(),
            password: form.password.clone(),
        });
    }
}
