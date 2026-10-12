//! Sign-in, as jellyfin-web asks for it: first "Connect to Server" (the
//! host), then "Please sign in" (user and password).

use egui::{Align2, Color32, Id, Key, Rect, Sense, TextEdit, Ui, pos2, vec2};

use super::UiState;
use super::app::UiAction;
use super::icons::Icon;
use super::jf;
use super::theme;
use super::widgets;

/// What the user has typed. Ephemeral UI state; it never leaves the window
/// except through [`UiAction::Login`].
#[derive(Default)]
pub struct LoginForm {
    pub server: String,
    pub username: String,
    pub password: String,
    /// Past the "Connect to Server" step.
    pub server_chosen: bool,
}

/// The form's width: 64vw, at most 804 points, as on jellyfin-web.
fn form_width(window: f32) -> f32 {
    (window * 0.64).clamp(280.0, 804.0)
}

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);

    // A slim header with the brand, as on the sign-in page.
    let header = Rect::from_min_size(screen.min, vec2(screen.width(), 57.0));
    ui.painter()
        .rect_filled(header, 0.0, Color32::from_black_alpha(102));
    if state.login.server_chosen
        && jf::round_icon_button(
            ui,
            Rect::from_min_size(
                pos2(header.left() + 4.0, header.top() + 8.0),
                vec2(41.0, 41.0),
            ),
            Icon::ArrowBack,
            Color32::WHITE,
            24.8,
            "Change server",
            !state.busy,
            Id::new("login-back"),
        )
        .clicked()
    {
        state.login.server_chosen = false;
        state.message.clear();
    }
    let logo = Rect::from_min_size(
        pos2(header.left() + 54.0, header.top() + 13.5),
        vec2(30.0, 30.0),
    );
    if let Some(icon) = &state.brand_icon {
        widgets::image_contain(ui.painter(), icon.id(), icon.size(), logo);
    }
    ui.painter().text(
        pos2(logo.right() + 6.0, logo.center().y),
        Align2::LEFT_CENTER,
        "JellyBridge",
        theme::bold(19.5),
        Color32::WHITE,
    );

    let width = form_width(screen.width());
    let left = screen.center().x - width / 2.0;
    let mut y = header.bottom() + 28.0;
    let title = if state.login.server_chosen {
        "Please sign in"
    } else {
        "Connect to Server"
    };
    ui.painter().text(
        pos2(screen.center().x, y + 18.0),
        Align2::CENTER_CENTER,
        title,
        theme::regular(theme::JF_FONT_TITLE),
        theme::JF_TEXT,
    );
    y += 54.0;

    let busy = state.busy;
    let form = &mut state.login;
    let mut submit = false;
    if !form.server_chosen {
        let host = field(
            ui,
            left,
            &mut y,
            width,
            "Host",
            &mut form.server,
            "http://jellyfin.local:8096",
            false,
            busy,
            "server",
        );
        ui.painter().text(
            pos2(left, y - 18.0),
            Align2::LEFT_TOP,
            "The address of your Jellyfin server, for example http://192.168.1.10:8096",
            theme::regular(theme::JF_FONT_SECONDARY),
            theme::JF_TEXT_SECONDARY,
        );
        y += 14.0;
        // Read Enter before re-focusing: `lost_focus` is evaluated live, and
        // focusing the field again would hide the Enter that just left it.
        let enter = host.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        if !enter && ui.ctx().memory(|memory| memory.focused().is_none()) && !busy {
            host.request_focus();
        }
        let can = !form.server.trim().is_empty();
        if (primary_button(ui, left, &mut y, width, "Connect", can) || enter) && can {
            form.server = normalize_host(&form.server);
            form.server_chosen = true;
            state.message.clear();
            ui.ctx()
                .memory_mut(|memory| memory.request_focus(Id::new(("login", "username"))));
        }
    } else {
        let user = field(
            ui,
            left,
            &mut y,
            width,
            "User",
            &mut form.username,
            "",
            false,
            busy,
            "username",
        );
        let password = field(
            ui,
            left,
            &mut y,
            width,
            "Password",
            &mut form.password,
            "",
            true,
            busy,
            "password",
        );
        let enter = (password.lost_focus() || user.lost_focus())
            && ui.input(|input| input.key_pressed(Key::Enter));
        if !enter
            && ui.ctx().memory(|memory| memory.focused().is_none())
            && !busy
            && form.username.is_empty()
        {
            user.request_focus();
        }
        y += 8.0;
        let can = !busy && !form.username.trim().is_empty();
        let label = if busy { "Signing In…" } else { "Sign In" };
        if (primary_button(ui, left, &mut y, width, label, can) || enter) && can {
            submit = true;
        }
    }
    if !state.message.is_empty() {
        let galley = widgets::wrapped(
            ui,
            &state.message,
            theme::regular(theme::JF_FONT),
            theme::DANGER_TEXT,
            width,
            Some(4),
        );
        ui.painter()
            .galley(pos2(left, y + 8.0), galley, Color32::WHITE);
    }
    if submit {
        let form = &state.login;
        actions.push(UiAction::Login {
            server: form.server.clone(),
            username: form.username.trim().to_owned(),
            password: form.password.clone(),
        });
    }
}

/// "jellyfin.local:8096" → "http://jellyfin.local:8096", as jellyfin-web
/// assumes plain HTTP when no scheme is typed.
fn normalize_host(host: &str) -> String {
    let host = host.trim().trim_end_matches('/');
    if host.contains("://") {
        host.to_owned()
    } else {
        format!("http://{host}")
    }
}

/// A labelled input: label above (accent while focused), a `#292929` field
/// with a 2 px border that turns accent while focused.
#[allow(clippy::too_many_arguments)]
fn field(
    ui: &mut Ui,
    left: f32,
    y: &mut f32,
    width: f32,
    label: &str,
    value: &mut String,
    hint: &str,
    password: bool,
    busy: bool,
    id: &str,
) -> egui::Response {
    let id = Id::new(("login", id));
    let focused = ui.ctx().memory(|memory| memory.has_focus(id));
    ui.painter().text(
        pos2(left, *y),
        Align2::LEFT_TOP,
        label,
        theme::regular(theme::JF_FONT),
        if focused {
            theme::JF_ACCENT
        } else {
            theme::JF_TEXT_LABEL
        },
    );
    let rect = Rect::from_min_size(pos2(left, *y + 24.0), vec2(width, 39.0));
    ui.painter()
        .rect_filled(rect, theme::JF_RADIUS, theme::JF_INPUT);
    widgets::border(
        ui.painter(),
        rect,
        theme::JF_RADIUS,
        2.0,
        if focused {
            theme::JF_ACCENT
        } else {
            theme::JF_INPUT
        },
    );
    let response = ui.put(
        rect.shrink2(vec2(6.0, 0.0)),
        TextEdit::singleline(value)
            .id(id)
            .hint_text(hint)
            .password(password)
            // A frameless field takes its padding from the frame, not
            // `margin`: centres the text in the 39 pt field.
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(2, 9)))
            .font(theme::regular(16.37))
            .text_color(theme::JF_TEXT)
            .interactive(!busy)
            .desired_width(f32::INFINITY),
    );
    *y = rect.bottom() + 26.0;
    response
}

/// The accent "Sign In"/"Connect" button, 47 points tall.
fn primary_button(
    ui: &mut Ui,
    left: f32,
    y: &mut f32,
    width: f32,
    label: &str,
    enabled: bool,
) -> bool {
    let rect = Rect::from_min_size(pos2(left, *y), vec2(width, 47.0));
    *y = rect.bottom() + 7.0;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::new(("login-button", label)), sense);
    widgets::describe_button(&response, enabled, label);
    let fill = if !enabled {
        theme::JF_RAISED
    } else if response.hovered() {
        theme::ACCENT_BRIGHT
    } else {
        theme::JF_ACCENT
    };
    ui.painter().rect_filled(rect, theme::JF_RADIUS, fill);
    if response.has_focus() {
        widgets::border(
            ui.painter(),
            rect.expand(2.0),
            theme::JF_RADIUS,
            2.0,
            Color32::WHITE,
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::bold(theme::JF_FONT),
        if enabled {
            Color32::WHITE
        } else {
            theme::JF_TEXT_SECONDARY
        },
    );
    if enabled {
        widgets::pointer_on_hover(&response);
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_gets_a_scheme() {
        assert_eq!(normalize_host("jellyfin.local"), "http://jellyfin.local");
        assert_eq!(normalize_host(" https://jellyfin.local/ "), "https://jellyfin.local");
        assert_eq!(
            normalize_host("192.168.1.10:8096"),
            "http://192.168.1.10:8096"
        );
    }
}
