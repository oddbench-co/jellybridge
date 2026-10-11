//! Shared chrome above the catalog and detail screens: back, brand/home,
//! current title, account, and sign-out.

use egui::{Align2, Color32, Id, Rect, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::theme;
use super::widgets;
use super::{Screen, UiState};

pub const HEIGHT: f32 = 64.0;

pub fn draw(ui: &mut Ui, state: &UiState, rect: Rect, actions: &mut Vec<UiAction>) {
    let show_back = state.can_go_back || state.screen == Screen::Detail;
    let home_active = state.catalog.home && state.screen == Screen::Catalog;
    let title = if state.screen == Screen::Detail {
        state.detail.title.as_str()
    } else {
        state.page_title.as_str()
    };

    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, theme::with_alpha(theme::BG, 0.96));
    painter.rect_filled(
        Rect::from_min_max(pos2(rect.left(), rect.bottom() - 1.0), rect.max),
        0.0,
        theme::SURFACE_HOVER,
    );

    let top = rect.top() + 10.0;
    let height = HEIGHT - 10.0 - 11.0;
    let item = |x: f32, width: f32| Rect::from_min_size(pos2(x, top), vec2(width, height));
    let mut x = rect.left() + 18.0;

    if show_back {
        let back = item(x, 42.0);
        let response = ui.interact(back, Id::new("top-bar-back"), Sense::click());
        widgets::describe_button(&response, true, "Back");
        painter.rect_filled(
            back,
            21.0,
            if response.hovered() {
                theme::SURFACE_HOVER
            } else {
                theme::SURFACE
            },
        );
        widgets::border(&painter, back, 21.0, 1.0, theme::BORDER);
        painter.text(
            back.center() - vec2(0.0, 2.0),
            Align2::CENTER_CENTER,
            "‹",
            theme::regular(27.0),
            theme::TEXT_PRIMARY,
        );
        widgets::pointer_on_hover(&response);
        if response.on_hover_text("Back").clicked() {
            actions.push(UiAction::GoBack);
        }
        x = back.right() + 10.0;
    }

    let brand = item(x, 172.0);
    let response = ui.interact(brand, Id::new("top-bar-brand"), Sense::click());
    widgets::describe_button(&response, true, "Home");
    let logo = Rect::from_min_size(pos2(brand.left(), brand.top() + 2.0), vec2(38.0, 38.0));
    widgets::diagonal_gradient(
        &painter,
        logo,
        theme::RADIUS_LARGE,
        theme::ACCENT,
        theme::ACCENT_DEEP,
    );
    if let Some(icon) = &state.brand_icon {
        widgets::image_contain(&painter, icon.id(), icon.size(), logo);
    }
    painter.text(
        pos2(brand.left() + 49.0, brand.center().y),
        Align2::LEFT_CENTER,
        "JellyBridge",
        theme::bold(19.0),
        theme::TEXT_PRIMARY,
    );
    widgets::pointer_on_hover(&response);
    if response.clicked() {
        actions.push(UiAction::GoHome);
    }
    x = brand.right() + 10.0;

    painter.rect_filled(item(x, 1.0), 0.0, theme::BORDER);
    x += 11.0;

    let home = item(x, 70.0);
    let response = ui.interact(home, Id::new("top-bar-home"), Sense::click());
    widgets::describe_button(&response, true, "Home");
    let fill = if home_active {
        theme::ACCENT_SURFACE
    } else if response.hovered() {
        theme::SURFACE_RAISED
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(home, theme::RADIUS_MEDIUM, fill);
    painter.text(
        home.center(),
        Align2::CENTER_CENTER,
        "Home",
        theme::bold(13.0),
        if home_active {
            theme::ACCENT_BRIGHT
        } else {
            theme::TEXT_SECONDARY
        },
    );
    widgets::pointer_on_hover(&response);
    if response.clicked() {
        actions.push(UiAction::GoHome);
    }
    x = home.right() + 10.0;

    // Right-aligned: sign-out, then the account pill.
    let sign_out = item(rect.right() - 18.0 - 82.0, 82.0);
    let account = item(sign_out.left() - 10.0 - 174.0, 174.0);

    let title_rect = Rect::from_min_max(pos2(x, top), pos2(account.left() - 10.0, top + height));
    if title_rect.width() > 0.0 {
        let galley = widgets::elided(
            ui,
            title,
            theme::regular(12.0),
            theme::TEXT_FAINT,
            title_rect.width(),
        );
        widgets::galley_centered(&painter, title_rect, galley);
    }

    painter.rect_filled(account, 21.0, theme::SURFACE);
    widgets::border(&painter, account, 21.0, 1.0, theme::SURFACE_HOVER);
    let avatar = Rect::from_min_size(account.min + vec2(5.0, 4.0), vec2(34.0, 34.0));
    painter.rect_filled(avatar, 17.0, theme::ACCENT_DEEP);
    painter.text(
        avatar.center(),
        Align2::CENTER_CENTER,
        &state.account_initial,
        theme::bold(15.0),
        Color32::WHITE,
    );
    let name = widgets::elided(
        ui,
        &state.account_name,
        theme::bold(12.0),
        theme::TEXT_SECONDARY,
        account.width() - 58.0,
    );
    widgets::galley_v_centered(&painter, account.left() + 48.0, account.center().y, name);

    let response = ui.interact(sign_out, Id::new("top-bar-sign-out"), Sense::click());
    widgets::describe_button(&response, true, "Sign out");
    let hovered = response.hovered();
    painter.rect_filled(
        sign_out,
        theme::RADIUS_MEDIUM,
        if hovered {
            theme::DANGER_SURFACE
        } else {
            Color32::TRANSPARENT
        },
    );
    widgets::border(
        &painter,
        sign_out,
        theme::RADIUS_MEDIUM,
        1.0,
        if hovered {
            theme::DANGER_BORDER
        } else {
            theme::BORDER
        },
    );
    painter.text(
        sign_out.center(),
        Align2::CENTER_CENTER,
        "Sign out",
        theme::bold(12.0),
        if hovered {
            theme::DANGER_TEXT
        } else {
            theme::TEXT_MUTED
        },
    );
    widgets::pointer_on_hover(&response);
    if response.clicked() {
        actions.push(UiAction::Logout);
    }
}
