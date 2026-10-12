//! jellyfin-web's header: the logo on home; back, home and the page title
//! elsewhere; search and the account menu on the right. Over a detail page's
//! backdrop it turns translucent.

use egui::{Align2, Color32, Id, Rect, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::Icon;
use super::jf;
use super::theme;
use super::widgets;
use super::{Screen, UiState};

/// Header height: the 41 px button row with 8 px above and below.
pub const HEIGHT: f32 = 57.0;

pub fn draw(ui: &mut Ui, state: &UiState, rect: Rect, actions: &mut Vec<UiAction>) {
    let on_detail = state.screen == Screen::Detail;
    let home = state.screen == Screen::Catalog && state.catalog.home && !state.can_go_back;
    let painter = ui.painter().clone();
    painter.rect_filled(
        rect,
        0.0,
        if on_detail {
            Color32::from_black_alpha(102)
        } else {
            theme::JF_HEADER
        },
    );

    let button =
        |x: f32| Rect::from_min_size(pos2(rect.left() + x, rect.top() + 8.0), vec2(41.0, 41.0));
    let white = Color32::WHITE;
    if home {
        // The brand mark where jellyfin-web has its logo.
        let logo = Rect::from_min_size(
            pos2(rect.left() + 16.0, rect.top() + 13.5),
            vec2(30.0, 30.0),
        );
        if let Some(icon) = &state.brand_icon {
            widgets::image_contain(&painter, icon.id(), icon.size(), logo);
        }
        painter.text(
            pos2(logo.right() + 6.0, logo.center().y),
            Align2::LEFT_CENTER,
            "JellyBridge",
            theme::bold(19.5),
            white,
        );
    } else {
        if jf::round_icon_button(
            ui,
            button(4.0),
            Icon::ArrowBack,
            white,
            24.8,
            "Back",
            true,
            Id::new("header-back"),
        )
        .clicked()
        {
            actions.push(UiAction::GoBack);
        }
        if jf::round_icon_button(
            ui,
            button(54.0),
            Icon::Home,
            white,
            24.8,
            "Home",
            true,
            Id::new("header-home"),
        )
        .clicked()
        {
            actions.push(UiAction::GoHome);
        }
        if !on_detail {
            let title = if state.screen == Screen::Detail {
                state.detail.title.as_str()
            } else {
                state.page_title.as_str()
            };
            let galley = widgets::elided(
                ui,
                title,
                theme::regular(theme::JF_FONT_SUBTITLE),
                theme::JF_TEXT,
                rect.width() - 220.0,
            );
            widgets::galley_v_centered(&painter, rect.left() + 109.0, rect.top() + 28.5, galley);
        }
    }

    let search = button(rect.width() - 95.0);
    if jf::round_icon_button(
        ui,
        search,
        Icon::Search,
        white,
        24.8,
        "Search",
        true,
        Id::new("header-search"),
    )
    .clicked()
    {
        actions.push(UiAction::OpenSearch);
    }
    let account = button(rect.width() - 45.0);
    let account_response = jf::round_icon_button(
        ui,
        account,
        Icon::Person,
        white,
        24.8,
        &state.account_name,
        true,
        Id::new("header-account"),
    );
    let menu_id = Id::new("header-account-menu");
    let mut open = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    if account_response.clicked() {
        open = !open;
    }
    if open {
        let menu = egui::Area::new(menu_id.with("area"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(account.right() - 220.0, account.bottom() + 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::JF_HEADER)
                    .corner_radius(theme::JF_RADIUS)
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: Color32::from_black_alpha(160),
                    })
                    .show(ui, |ui| {
                        ui.set_width(220.0);
                        let (name_rect, _) =
                            ui.allocate_exact_size(vec2(220.0, 44.0), Sense::hover());
                        let name = widgets::elided(
                            ui,
                            &state.account_name,
                            theme::bold(theme::JF_FONT),
                            theme::JF_TEXT,
                            190.0,
                        );
                        widgets::galley_v_centered(
                            ui.painter(),
                            name_rect.left() + 15.0,
                            name_rect.center().y,
                            name,
                        );
                        let (item, response) =
                            ui.allocate_exact_size(vec2(220.0, 44.0), Sense::click());
                        widgets::describe_button(&response, true, "Sign Out");
                        if response.hovered() {
                            ui.painter()
                                .rect_filled(item, 0.0, Color32::from_white_alpha(20));
                        }
                        ui.painter().text(
                            pos2(item.left() + 15.0, item.center().y),
                            Align2::LEFT_CENTER,
                            "Sign Out",
                            theme::regular(theme::JF_FONT),
                            theme::JF_TEXT,
                        );
                        widgets::pointer_on_hover(&response);
                        response.clicked()
                    })
                    .inner
            });
        if menu.inner {
            actions.push(UiAction::Logout);
            open = false;
        } else if !account_response.clicked()
            && ui.input(|input| input.pointer.any_click())
            && !menu.response.contains_pointer()
        {
            open = false;
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(menu_id, open));
}
