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

/// The header row's height on a narrow window, for screens that place
/// content under it without asking for the window's metrics.
pub const HEIGHT: f32 = 57.58;
const DRAWER_WIDTH: f32 = 320.0;
const DRAWER_ROW: f32 = 47.0;
/// The drawer's slide, as jellyfin-web's `transform 0.24s ease-out`.
const DRAWER_SECONDS: f32 = 0.24;

/// The header's full height on the current screen at `width`, the tab
/// strip included when it sits under the row.
pub fn height(state: &UiState, width: f32) -> f32 {
    let metrics = jf::header_metrics(width);
    if state.screen == Screen::Catalog && !state.catalog.tabs.is_empty() && !metrics.wide {
        metrics.height + metrics.tab_height
    } else {
        metrics.height
    }
}

pub fn draw(ui: &mut Ui, state: &mut UiState, rect: Rect, actions: &mut Vec<UiAction>) {
    let on_detail = state.screen == Screen::Detail;
    let home = state.screen == Screen::Catalog && state.catalog.home;
    // Media queries see the whole window; the header itself stops at the
    // page scrollbar.
    let window = jf::window_width(ui);
    let metrics = jf::header_metrics(window);
    let full = Rect::from_min_size(rect.min, vec2(rect.width(), height(state, window)));
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

    let left = rect.left() + metrics.inset + jf::HEADER_MARGIN;
    let button = |index: f32| {
        Rect::from_min_size(
            pos2(
                left + index * jf::HEADER_STEP,
                rect.top() + metrics.button_top,
            ),
            vec2(jf::HEADER_BUTTON, jf::HEADER_BUTTON),
        )
    };
    let buttons_before_title = if home {
        0.0
    } else {
        if header_button(ui, button(0.0), Icon::ArrowBack, "Back", "header-back") {
            actions.push(UiAction::GoBack);
        }
        if header_button(ui, button(1.0), Icon::Home, "Home", "header-home") {
            actions.push(UiAction::GoHome);
        }
        2.0
    };
    if header_button(
        ui,
        button(buttons_before_title),
        Icon::Menu,
        "Menu",
        "header-menu",
    ) {
        state.drawer_open = true;
    }
    // The title (or logo) box: 0.5em of its 17.4 px font after the last
    // button's margin.
    let title_left = button(buttons_before_title).right() + jf::HEADER_MARGIN + 0.5 * 17.41;
    let center_y = rect.top() + metrics.height / 2.0;
    if home {
        // jellyfin-web's logo: `background-size: contain` in a 13.2em by
        // 1.7em box, against its left edge.
        if let Some(logo) = &state.brand_icon {
            let height = 1.7 * 17.41;
            let [w, h] = logo.size();
            let width = height * w as f32 / h as f32;
            let rect = Rect::from_min_size(
                pos2(title_left, center_y - height / 2.0),
                vec2(width, height),
            );
            painter.image(
                logo.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    } else if !on_detail {
        let galley = widgets::elided(
            ui,
            &state.page_title,
            theme::regular(theme::JF_FONT_SUBTITLE),
            theme::JF_TEXT,
            rect.width() * 0.4,
        );
        widgets::galley_v_centered(&painter, title_left, center_y, galley);
    }

    // Right side: search and the account button, from the right edge.
    let right = rect.right() - metrics.inset - jf::HEADER_MARGIN;
    let from_right = |index: f32| {
        Rect::from_min_size(
            pos2(
                right - jf::HEADER_BUTTON - index * jf::HEADER_STEP,
                rect.top() + metrics.button_top,
            ),
            vec2(jf::HEADER_BUTTON, jf::HEADER_BUTTON),
        )
    };
    if header_button(ui, from_right(1.0), Icon::Search, "Search", "header-search") {
        actions.push(UiAction::OpenSearch);
    }
    account_menu(ui, state, from_right(0.0), actions);

    if state.screen == Screen::Catalog && !state.catalog.tabs.is_empty() {
        let strip = Rect::from_min_size(
            pos2(rect.left(), rect.top() + metrics.tabs_top),
            vec2(rect.width(), metrics.tab_height),
        );
        if let Some(index) = tabs(
            ui,
            strip,
            &state.catalog.tabs,
            state.catalog.active_tab,
            metrics.tab_font,
        ) {
            actions.push(UiAction::SwitchTab(index));
        }
    }
    drawer(ui, state, actions);
}

fn header_button(ui: &mut Ui, rect: Rect, icon: Icon, label: &str, id: &str) -> bool {
    jf::round_icon_button(
        ui,
        rect,
        icon,
        theme::JF_TEXT,
        24.84,
        label,
        true,
        Id::new(id),
    )
    .clicked()
}

/// The tab strip: centred labels, the active one white, hovered ones in the
/// accent. Returns a clicked inactive tab.
fn tabs(
    ui: &mut Ui,
    strip: Rect,
    labels: &[&'static str],
    active: usize,
    size: f32,
) -> Option<usize> {
    let font = theme::bold(size);
    // `.emby-tab-button`: 1.5em of padding either side of the label.
    let widths = labels
        .iter()
        .map(|label| widgets::text_width(ui, label, font.clone()) + 3.0 * size)
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
        // `.emby-tab-button`: 1.5em of padding over a 1.25 line; Chrome puts
        // the baseline half the leading plus the ascent (1.069em) below.
        let galley = ui
            .painter()
            .layout_no_wrap((*label).to_owned(), font.clone(), color);
        // (+0.09em: egui's glyphs sit that much higher on the same baseline.)
        let baseline =
            rect.top() + 1.5 * size + (1.25 - 1.362) / 2.0 * size + (1.069 + 0.09) * size;
        let left = rect.center().x - galley.size().x / 2.0;
        widgets::galley_on_baseline(ui.painter(), left, baseline, galley);
        widgets::pointer_on_hover(&response);
        if response.clicked() && index != active {
            clicked = Some(index);
        }
    }
    clicked
}

/// The account button: jellyfin-web's opens the user's settings.
fn account_menu(ui: &mut Ui, state: &UiState, account: Rect, actions: &mut Vec<UiAction>) {
    let account_response = jf::round_icon_button(
        ui,
        account,
        Icon::Person,
        theme::JF_TEXT,
        24.84,
        &state.account_name,
        true,
        Id::new("header-account"),
    );
    if account_response.clicked() {
        actions.push(UiAction::OpenSettings(
            crate::features::settings::SettingsPage::Menu,
        ));
    }
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
                let rect =
                    Rect::from_min_size(pos2(panel.left(), *y), vec2(DRAWER_WIDTH, DRAWER_ROW));
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
            row(
                ui,
                &mut y,
                Icon::Settings,
                "Settings",
                UiAction::OpenSettings(crate::features::settings::SettingsPage::Menu),
            );
            row(ui, &mut y, Icon::ExitToApp, "Sign Out", UiAction::Logout);
            picked
        })
        .inner;
    if let Some(action) = picked {
        state.drawer_open = false;
        actions.push(action);
    }
}
