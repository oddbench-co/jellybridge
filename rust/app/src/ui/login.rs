//! Sign-in, as jellyfin-web asks for it: "Connect to Server" (the host),
//! then "Please sign in" (user, password, Remember Me, Quick Connect,
//! Forgot Password), and the password reset pages.

use egui::{Color32, Id, Rect, Sense, TextEdit, Ui, pos2, vec2};

use super::UiState;
use super::app::UiAction;
use super::dialogs;
use super::icons::{self, Icon};
use super::jf::{self, EM};
use super::theme;
use super::widgets;

/// Which sign-in page is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginStep {
    #[default]
    AddServer,
    SignIn,
    ForgotPassword,
    /// The PIN from the file the server wrote.
    ForgotPin,
}

/// What the user has typed. Ephemeral UI state; it never leaves the window
/// except through the sign-in actions.
pub struct LoginForm {
    pub server: String,
    pub username: String,
    pub password: String,
    pub remember: bool,
    pub step: LoginStep,
    pub forgot_user: String,
    pub pin: String,
    /// The Quick Connect code to enter on a signed-in device, while waiting.
    pub quick_code: Option<String>,
    /// jellyfin-web's alert dialog: title and text.
    pub alert: Option<(String, String)>,
    /// The server "Change Server" left, which Cancel goes back to.
    pub previous_server: Option<String>,
}

/// Leaves "Connect to Server" for the server it was opened from.
fn cancel_add_server(form: &mut LoginForm) {
    if let Some(server) = form.previous_server.take() {
        form.server = server;
        form.step = LoginStep::SignIn;
    }
}

impl Default for LoginForm {
    fn default() -> Self {
        Self {
            server: String::new(),
            username: String::new(),
            password: String::new(),
            remember: true,
            step: LoginStep::AddServer,
            forgot_user: String::new(),
            pin: String::new(),
            quick_code: None,
            alert: None,
            previous_server: None,
        }
    }
}

// jellyfin-web's sign-in metrics: a 54em column, 1.8em titles, 39px
// inputs, 46.86px buttons.
const COLUMN: f32 = 54.0 * EM;
const INPUT_HEIGHT: f32 = 39.09;
const BUTTON: f32 = 46.86;
const INPUT_FONT: f32 = 1.1 * EM;
/// The header logo: 1.7em of the header's 17.41 px font.
const LOGO_HEIGHT: f32 = 1.7 * 17.41;

fn lu(value: f32) -> f32 {
    jf::layout_unit(value)
}

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);
    // The header: just the Jellyfin logo.
    let metrics = jf::header_metrics(screen.width());
    // Without the back button the header is only as tall as the logo and
    // its 0.8em padding, shorter than the library pages' header.
    let back = match state.login.step {
        LoginStep::AddServer => state.login.previous_server.is_some(),
        LoginStep::SignIn => false,
        LoginStep::ForgotPassword | LoginStep::ForgotPin => true,
    };
    let height = if back { metrics.height } else { 2.0 * 0.8 * EM + LOGO_HEIGHT };
    let header = Rect::from_min_size(screen.min, vec2(screen.width(), height));
    // "Please sign in" has a 40% black header over the page; the other
    // sign-in pages the usual one.
    let header_fill = if state.login.step == LoginStep::SignIn {
        Color32::from_black_alpha(102)
    } else {
        theme::JF_HEADER
    };
    ui.painter().rect_filled(header, 0.0, header_fill);
    // Back to the sign-in page when a server is already known.
    if back
        && jf::round_icon_button(
            ui,
            Rect::from_min_size(
                pos2(screen.left() + metrics.inset + jf::HEADER_MARGIN, header.center().y - jf::HEADER_BUTTON / 2.0),
                vec2(jf::HEADER_BUTTON, jf::HEADER_BUTTON),
            ),
            Icon::ArrowBack,
            jf::TEXT_80,
            24.84,
            "Back",
            true,
            Id::new("login-back"),
        )
        .clicked()
    {
        if state.login.step == LoginStep::AddServer {
            cancel_add_server(&mut state.login);
        } else {
            state.login.step = LoginStep::SignIn;
        }
    }
    if let Some(logo) = &state.brand_icon {
        let height = LOGO_HEIGHT;
        let width = height * logo.size()[0] as f32 / logo.size()[1] as f32;
        let rect = Rect::from_min_size(
            pos2(
                screen.left()
                    + metrics.inset
                    + if back { 2.0 * jf::HEADER_MARGIN + jf::HEADER_BUTTON } else { 0.0 }
                    + 0.5 * 17.41,
                header.center().y - height / 2.0,
            ),
            vec2(width, height),
        );
        ui.painter().image(
            logo.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let width = COLUMN.min(screen.width() - 2.0 * jf::page_padding(screen.width()));
    let left = screen.center().x - width / 2.0;
    let busy = state.busy;
    let mut y = screen.top() + 84.89;
    let form = &mut state.login;
    let title = match form.step {
        LoginStep::AddServer => "Connect to Server",
        LoginStep::SignIn => "Please sign in",
        LoginStep::ForgotPassword | LoginStep::ForgotPin => "Forgot Password",
    };
    let h1 = 1.8 * EM;
    let h1_line = lu(1.35 * h1);
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), theme::regular(h1), jf::TEXT_80);
    // "Please sign in" is centred; the other titles start at the column.
    let x = if form.step == LoginStep::SignIn {
        screen.center().x - galley.size().x / 2.0
    } else {
        left
    };
    widgets::galley_on_baseline(ui.painter(), x, jf::baseline(y, h1, h1_line), galley);
    y += h1_line + lu(0.67 * h1);

    match form.step {
        LoginStep::AddServer => {
            let host = input(ui, left, &mut y, width, "Host", &mut form.server, false, busy, "server");
            y += help(ui, left, y, width, "192.168.1.100:8096 or https://myserver.com");
            let enter = host.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            y += 50.57;
            if (button(ui, left, y, width, "Connect", true, "connect") || enter)
                && !form.server.trim().is_empty()
            {
                form.server = normalize_host(&form.server);
                form.step = LoginStep::SignIn;
                form.previous_server = None;
                state.message.clear();
            }
            if form.previous_server.is_some() {
                y += BUTTON + 3.72;
                if button(ui, left, y, width, "Cancel", false, "cancel-server") {
                    cancel_add_server(form);
                }
            }
        }
        LoginStep::SignIn => {
            let user = input(ui, left, &mut y, width, "User", &mut form.username, false, busy, "username");
            y += lu(1.8 * EM);
            let password = input(ui, left, &mut y, width, "Password", &mut form.password, true, busy, "password");
            y += lu(1.8 * EM);
            // "Remember Me".
            let row = Rect::from_min_size(pos2(left, y), vec2(width, 34.95));
            let response = ui.interact(row, Id::new("login-remember"), Sense::click());
            widgets::describe_button(&response, true, "Remember Me");
            let check = Rect::from_min_size(pos2(left, y + 3.0), vec2(27.22, 27.22));
            if form.remember {
                ui.painter().rect_filled(check, 2.08, theme::JF_ACCENT);
                icons::paint_centered(ui.painter(), Icon::Check, check.center(), 23.8, Color32::WHITE);
            } else {
                widgets::border(ui.painter(), check, 2.08, 2.0, jf::TEXT_80);
            }
            let label = ui
                .painter()
                .layout_no_wrap("Remember Me".to_owned(), theme::regular(EM), jf::TEXT_80);
            widgets::galley_on_baseline(ui.painter(), left + lu(2.4 * EM), jf::baseline(y + 7.43, EM, lu(1.35 * EM)), label);
            widgets::pointer_on_hover(&response);
            if response.clicked() {
                form.remember = !form.remember;
            }
            y += 34.95 + lu(1.8 * EM);
            let enter = (password.lost_focus() || user.lost_focus())
                && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let can = !busy && !form.username.trim().is_empty();
            if (button(ui, left, y, width, "Sign In", true, "sign-in") || enter) && can {
                actions.push(UiAction::Login {
                    server: form.server.clone(),
                    username: form.username.trim().to_owned(),
                    password: form.password.clone(),
                    remember: form.remember,
                });
            }
            y += BUTTON + 7.43;
            if button(ui, left, y, width, "Use Quick Connect", false, "quick-connect") && !busy {
                actions.push(UiAction::QuickConnect {
                    server: form.server.clone(),
                    remember: form.remember,
                });
            }
            y += BUTTON + 3.72;
            if button(ui, left, y, width, "Forgot Password", false, "forgot") {
                form.forgot_user = form.username.clone();
                form.step = LoginStep::ForgotPassword;
            }
            y += BUTTON + 3.72;
            // A desktop client of jellyfin-web offers to change the server.
            if button(ui, left, y, width, "Change Server", false, "change-server") {
                form.previous_server = Some(std::mem::take(&mut form.server));
                form.step = LoginStep::AddServer;
                state.message.clear();
            }
        }
        LoginStep::ForgotPassword => {
            let user = input(ui, left, &mut y, width, "User", &mut form.forgot_user, false, busy, "forgot-user");
            y += help(ui, left, y, width, "Enter your username, if you remember it.");
            let enter = user.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            y += lu(1.8 * EM);
            if button(ui, left, y, width, "Submit", false, "forgot-submit") || enter {
                actions.push(UiAction::ForgotPassword {
                    server: form.server.clone(),
                    username: form.forgot_user.trim().to_owned(),
                });
            }
            y += BUTTON + 3.72;
            if button(ui, left, y, width, "Cancel", false, "forgot-cancel") {
                form.step = LoginStep::SignIn;
            }
        }
        LoginStep::ForgotPin => {
            let pin = input(ui, left, &mut y, width, "PIN code", &mut form.pin, false, busy, "pin");
            y += help(ui, left, y, width, "Enter PIN here to finish Password reset");
            let enter = pin.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            y += lu(1.8 * EM);
            if (button(ui, left, y, width, "Submit", false, "pin-submit") || enter)
                && !form.pin.trim().is_empty()
            {
                actions.push(UiAction::ForgotPasswordPin {
                    server: form.server.clone(),
                    pin: std::mem::take(&mut form.pin),
                });
            }
            y += BUTTON + 3.72;
            if button(ui, left, y, width, "Cancel", false, "pin-cancel") {
                form.step = LoginStep::SignIn;
            }
        }
    }
    if !state.message.is_empty() {
        let message = state.message.clone();
        let _ = y;
        super::show_toast(state, &message);
        state.message.clear();
    }
    draw_quick_connect(ui, state, actions);
    draw_alert(ui, state);
}

/// A labelled `emby-input`: the label at 70% (accent while focused), the
/// #292929 box with an accent border while focused.
#[allow(clippy::too_many_arguments)]
fn input(
    ui: &mut Ui,
    left: f32,
    y: &mut f32,
    width: f32,
    label: &str,
    value: &mut String,
    password: bool,
    busy: bool,
    id: &str,
) -> egui::Response {
    let id = Id::new(("login", id));
    let focused = ui.ctx().memory(|memory| memory.has_focus(id));
    let line = lu(1.35 * EM);
    let color = if focused { theme::JF_ACCENT } else { theme::with_alpha(Color32::WHITE, 0.7) };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::regular(EM), color);
    widgets::galley_on_baseline(ui.painter(), left, jf::baseline(*y, EM, line), galley);
    let rect = Rect::from_min_size(pos2(left, *y + line + lu(0.25 * EM)), vec2(width, INPUT_HEIGHT));
    let radius = 0.2 * INPUT_FONT;
    ui.painter().rect_filled(rect, radius, Color32::from_gray(41));
    if focused {
        widgets::border(ui.painter(), rect, radius, 2.0, theme::JF_ACCENT);
    }
    let response = ui.put(
        rect.shrink(2.0),
        TextEdit::singleline(value)
            .id(id)
            .password(password)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 6)))
            .font(theme::regular(INPUT_FONT))
            .text_color(jf::TEXT_80)
            .vertical_align(egui::Align::Center)
            .interactive(!busy)
            .desired_width(f32::INFINITY),
    );
    *y = rect.bottom();
    response
}

/// A `.fieldDescription` under an input; returns its height with its
/// 0.25em gap.
fn help(ui: &Ui, left: f32, top: f32, width: f32, text: &str) -> f32 {
    let top = top + lu(0.25 * EM);
    let height = jf::paragraph(
        ui,
        text,
        theme::regular(EM),
        1.35 * EM,
        top,
        left + 2.232,
        width - 2.232,
        false,
        theme::JF_TEXT_SECONDARY,
    );
    lu(0.25 * EM) + height
}

/// A raised button: accent for the primary action, #303030 otherwise.
fn button(ui: &Ui, left: f32, top: f32, width: f32, label: &str, primary: bool, id: &str) -> bool {
    let rect = Rect::from_min_size(pos2(left, top), vec2(width, BUTTON));
    let response = ui.interact(rect, Id::new(("login-button", id)), Sense::click());
    widgets::describe_button(&response, true, label);
    let (fill, text) = if primary {
        (theme::JF_ACCENT, Color32::WHITE)
    } else {
        (Color32::from_gray(48), theme::with_alpha(Color32::WHITE, 0.87))
    };
    let fill = if response.hovered() { theme::with_alpha(fill, 0.85) } else { fill };
    ui.painter().rect_filled(rect, 0.2 * EM, fill);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::bold(EM), text);
    let x = rect.center().x - galley.size().x / 2.0;
    widgets::galley_on_baseline(ui.painter(), x, jf::baseline(rect.top() + 13.39, EM, 20.0), galley);
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// The Quick Connect wait: jellyfin-web's dialog with the code. "Got It"
/// stops waiting, as it does there.
fn draw_quick_connect(ui: &Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let Some(code) = state.login.quick_code.clone() else { return };
    let text = format!("Enter code {code} to login");
    if dialogs::message_box(ui.ctx(), Id::new("quick-connect-wait"), "Quick Connect", &text, "Got It") {
        state.login.quick_code = None;
        actions.push(UiAction::CancelQuickConnect);
    }
}

/// jellyfin-web's alert: a title, the text, and Got It.
fn draw_alert(ui: &Ui, state: &mut UiState) {
    let Some((title, body)) = state.login.alert.clone() else { return };
    if dialogs::message_box(ui.ctx(), Id::new("login-alert"), &title, &body, "Got It") {
        state.login.alert = None;
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
