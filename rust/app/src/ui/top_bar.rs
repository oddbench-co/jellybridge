//! jellyfin-web's header: menu and logo on home; back, home, menu and the
//! page title elsewhere; search and the account button on the right; the
//! page's tabs in a strip underneath. Over a detail page's backdrop it turns
//! translucent. Also draws the side drawer the menu button opens.

use egui::{Align2, Color32, Id, Rect, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::jf;
use super::theme;
use super::widgets;
use super::{Screen, UiState};
use crate::features::pages::LibraryRef;

/// Header height: the 41 px button row with 8 px above and below.
pub const HEIGHT: f32 = 57.0;
/// The tab strip under the header.
pub const TABS_HEIGHT: f32 = 53.0;
const DRAWER_WIDTH: f32 = 320.0;
const DRAWER_ROW: f32 = 47.0;
/// The drawer's slide, as jellyfin-web's `transform 0.24s ease-out`.
const DRAWER_SECONDS: f32 = 0.24;

/// The header's full height on the current screen, tabs included.
pub fn height(state: &UiState) -> f32 {
    if state.screen == Screen::Catalog && !state.catalog.tabs.is_empty() {
        HEIGHT + TABS_HEIGHT
    } else {
        HEIGHT
    }
}

pub fn draw(ui: &mut Ui, state: &mut UiState, rect: Rect, actions: &mut Vec<UiAction>) {
    let on_detail = state.screen == Screen::Detail;
    let home = state.screen == Screen::Catalog && state.catalog.home;
    let full = Rect::from_min_size(rect.min, vec2(rect.width(), height(state)));
    let painter = ui.painter().clone();
    painter.rect_filled(
        full,
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
    let menu_x = if home {
        4.0
    } else {
        if header_button(ui, button(4.0), Icon::ArrowBack, "Back", "header-back") {
            actions.push(UiAction::GoBack);
        }
        if header_button(ui, button(54.0), Icon::Home, "Home", "header-home") {
            actions.push(UiAction::GoHome);
        }
        104.0
    };
    if header_button(ui, button(menu_x), Icon::Menu, "Menu", "header-menu") {
        state.drawer_open = true;
    }
    if home {
        // The brand mark where jellyfin-web has its logo.
        let logo = Rect::from_min_size(
            pos2(rect.left() + 59.0, rect.top() + 13.5),
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
    } else if !on_detail {
        let galley = widgets::elided(
            ui,
            &state.page_title,
            theme::regular(theme::JF_FONT_SUBTITLE),
            theme::JF_TEXT,
            rect.width() - 280.0,
        );
        widgets::galley_v_centered(&painter, rect.left() + 159.0, rect.top() + 28.5, galley);
    }

    if header_button(
        ui,
        button(rect.width() - 95.0),
        Icon::Search,
        "Search",
        "header-search",
    ) {
        actions.push(UiAction::OpenSearch);
    }
    account_menu(ui, state, button(rect.width() - 45.0), actions);

    if state.screen == Screen::Catalog && !state.catalog.tabs.is_empty() {
        let strip = Rect::from_min_size(
            pos2(rect.left(), rect.top() + HEIGHT),
            vec2(rect.width(), TABS_HEIGHT),
        );
        if let Some(index) = tabs(ui, strip, &state.catalog.tabs, state.catalog.active_tab) {
            actions.push(UiAction::SwitchTab(index));
        }
    }
    drawer(ui, state, actions);
}

fn header_button(ui: &mut Ui, rect: Rect, icon: Icon, label: &str, id: &str) -> bool {
    jf::round_icon_button(ui, rect, icon, Color32::WHITE, 24.8, label, true, Id::new(id)).clicked()
}

/// The tab strip: centred labels, the active one white, hovered ones in the
/// accent. Returns a clicked inactive tab.
fn tabs(ui: &mut Ui, strip: Rect, labels: &[&'static str], active: usize) -> Option<usize> {
    let font = theme::bold(12.42);
    let widths = labels
        .iter()
        .map(|label| widgets::text_width(ui, label, font.clone()) + 36.0)
        .collect::<Vec<_>>();
    let total: f32 = widths.iter().sum();
    let mut x = strip.center().x - total / 2.0;
    let mut clicked = None;
    for (index, (label, width)) in labels.iter().zip(widths).enumerate() {
        let rect = Rect::from_min_size(pos2(x, strip.top()), vec2(width, strip.height()));
        x += width;
        let response = ui.interact(rect, Id::new(("header-tab", index)), Sense::click());
        widgets::describe_button(&response, true, label);
        let color = if index == active {
            Color32::WHITE
        } else if response.hovered() || response.has_focus() {
            theme::JF_ACCENT
        } else {
            Color32::from_gray(0x99)
        };
        ui.painter()
            .text(rect.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
        widgets::pointer_on_hover(&response);
        if response.clicked() && index != active {
            clicked = Some(index);
        }
    }
    clicked
}

/// The account button and its menu.
fn account_menu(ui: &mut Ui, state: &UiState, account: Rect, actions: &mut Vec<UiAction>) {
    let account_response = jf::round_icon_button(
        ui,
        account,
        Icon::Person,
        Color32::WHITE,
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
        let items = [super::dialogs::SheetItem {
            label: "Sign Out",
            icon: Some(Icon::ExitToApp),
            checked: false,
        }];
        let (picked, close) =
            super::dialogs::action_sheet(ui.ctx(), menu_id.with("sheet"), account, &items, 0);
        if picked == Some(0) {
            actions.push(UiAction::Logout);
        }
        if close {
            open = false;
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(menu_id, open));
}

/// The icon jellyfin-web gives a library in the drawer.
fn library_icon(library: &LibraryRef) -> Icon {
    collection_icon(&library.collection_type)
}

/// The icon for a library of `collection_type`.
pub fn collection_icon(collection_type: &str) -> Icon {
    match collection_type {
        "movies" => Icon::Movie,
        "tvshows" => Icon::Tv,
        "music" => Icon::MusicNote,
        "books" => Icon::Book,
        "playlists" => Icon::Queue,
        "homevideos" | "photos" => Icon::Photo,
        _ => Icon::Folder,
    }
}

/// The side drawer: Home, the libraries under "Media", and Sign Out under
/// "User". Slides in from the left over a dimmed page.
fn drawer(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let ctx = ui.ctx().clone();
    let shown = ctx.animate_bool_with_time_and_easing(
        Id::new("drawer-anim"),
        state.drawer_open,
        DRAWER_SECONDS,
        egui::emath::easing::cubic_out,
    );
    if shown <= 0.0 {
        return;
    }
    let screen = ctx.content_rect();
    let backdrop = egui::Area::new(Id::new("drawer-backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(&ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha((128.0 * shown) as u8));
            response.clicked()
        });
    if backdrop.inner || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.drawer_open = false;
    }
    let left = screen.left() - DRAWER_WIDTH * (1.0 - shown);
    let libraries = state.libraries.clone();
    let picked = egui::Area::new(Id::new("drawer"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos2(left, screen.top()))
        .show(&ctx, |ui| {
            let (panel, _) =
                ui.allocate_exact_size(vec2(DRAWER_WIDTH, screen.height()), Sense::click());
            let painter = ui.painter();
            painter.rect_filled(
                panel.translate(vec2(2.0, 0.0)).expand(6.0),
                0.0,
                Color32::from_black_alpha(40),
            );
            painter.rect_filled(panel, 0.0, theme::JF_BG);
            let mut y = panel.top() + 7.0;
            let mut picked = None;
            let mut row = |ui: &mut Ui, y: &mut f32, icon: Icon, label: &str, action: UiAction| {
                let rect = Rect::from_min_size(pos2(panel.left(), *y), vec2(DRAWER_WIDTH, DRAWER_ROW));
                *y += DRAWER_ROW;
                let response = ui.interact(rect, Id::new(("drawer-row", label)), Sense::click());
                widgets::describe_button(&response, true, label);
                if response.hovered() || response.has_focus() {
                    ui.painter()
                        .rect_filled(rect, 0.0, Color32::from_white_alpha(20));
                }
                icons::paint_centered(
                    ui.painter(),
                    icon,
                    pos2(rect.left() + 46.0, rect.center().y),
                    20.24,
                    theme::JF_TEXT,
                );
                ui.painter().text(
                    pos2(rect.left() + 76.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    theme::regular(theme::JF_FONT),
                    theme::JF_TEXT,
                );
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    picked = Some(action);
                }
            };
            let header = |ui: &mut Ui, y: &mut f32, text: &str| {
                *y += 14.88;
                ui.painter().text(
                    pos2(panel.left() + 17.86, *y + 10.0),
                    Align2::LEFT_CENTER,
                    text,
                    theme::regular(theme::JF_FONT),
                    theme::JF_TEXT,
                );
                *y += 20.0 + 7.44;
            };
            row(ui, &mut y, Icon::Home, "Home", UiAction::GoHome);
            if !libraries.is_empty() {
                header(ui, &mut y, "Media");
                for library in &libraries {
                    row(
                        ui,
                        &mut y,
                        library_icon(library),
                        &library.name,
                        UiAction::OpenLibrary(library.clone()),
                    );
                }
            }
            header(ui, &mut y, "User");
            row(ui, &mut y, Icon::ExitToApp, "Sign Out", UiAction::Logout);
            picked
        })
        .inner;
    if let Some(action) = picked {
        state.drawer_open = false;
        actions.push(action);
    }
}
