//! Catalog screens, laid out as jellyfin-web's: the home page (sections of
//! scrolling card shelves), a library (toolbar, poster grid, A–Z picker),
//! and the search page (a search box, results grouped by kind).

use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, TextEdit, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::Icon;
use super::images::{Art, ArtSource, ImageCache};
use super::jf::{self, CardModel, Shape};
use super::top_bar;
use super::widgets::{self, Shelf};
use super::{Screen, UiState, catalog_image_key, detail_art, theme};
use crate::api::CatalogItemData;
use crate::app::AppContext;
use crate::features::catalog::{CardToggle, CatalogSort};

/// What the catalog screen shows. Filled by the presenters below.
#[derive(Default)]
pub struct CatalogVm {
    /// "1-34 of 34", the library toolbar's count.
    pub summary: String,
    pub home: bool,
    pub search_page: bool,
    /// Identifies the page, so each page gets its own scroll position and a
    /// new page opens at the top.
    pub page_key: String,
    pub search_text: String,
    /// When the search box was last edited; a search runs 300 ms after
    /// typing pauses.
    pub search_edited_at: Option<f64>,
    pub sort_index: usize,
    pub sections: Vec<CatalogSection>,
    pub items: Arc<Vec<CatalogItem>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogSection {
    pub id: String,
    pub title: String,
    pub start_index: usize,
    pub count: usize,
    pub shape: Shape,
    /// Text rows under each card: libraries have a title only.
    pub lines: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// For the A–Z picker: the name the library is sorted by.
    pub sort_name: String,
    pub kind: String,
    pub poster: Option<Art>,
    pub thumb: Option<Art>,
    pub watched: bool,
    pub favorite: bool,
    pub unplayed: u32,
    pub progress: f32,
    pub playable: bool,
    /// Libraries and other folders shown as 16:9 banners.
    pub wide: bool,
}

/// Presents the current catalog page (home dashboard, browse results, or
/// search results).
pub fn present(context: &AppContext, message: &str) {
    // A new page outdates whatever the previous one still had in flight.
    context.next_generation();
    present_page(context, Some(message));
}

/// Re-presents the page after a background refresh, under a generation the
/// refresh already claimed. Leaves the screen, busy state, and message alone,
/// so a refresh never pulls the user off the player or clears a status line.
pub fn present_refresh(context: &AppContext, _generation: u64) {
    present_page(context, None);
}

/// `message` is `None` for a background refresh.
fn present_page(context: &AppContext, message: Option<&str>) {
    let (current, account, can_go_back, sort) = match context.state.lock() {
        Ok(state) => (
            state.current.clone(),
            state.account_name.clone(),
            !state.back_stack.is_empty(),
            state.catalog_sort,
        ),
        Err(_) => return,
    };
    let home_dashboard = current.is_home_dashboard();
    let title = if current.search_page {
        "Search".to_owned()
    } else if current.parent.is_none() && !current.search_text.is_empty() {
        "Search results".to_owned()
    } else {
        current.title.clone()
    };
    let shown = current.items.len();
    let summary = if shown == 0 {
        String::new()
    } else {
        format!("1-{shown} of {}", current.total_record_count.max(shown))
    };
    let page_key = format!(
        "{}|{}|{}|{}",
        current
            .parent
            .as_ref()
            .map_or("", |parent| parent.id.as_str()),
        current.search_text,
        current.title,
        current.search_page
    );
    let search_text = current.search_text.clone();
    let account_initial = account
        .trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_owned());
    let message = message.map(str::to_owned);
    let search_page = current.search_page;
    let home_sections = current.sections;
    let items_data = current.items;
    context.ui.update(move |state| {
        if let Some(message) = message {
            state.screen = Screen::Catalog;
            state.busy = false;
            state.message = message;
        }
        state.page_title = title;
        state.account_name = account;
        state.account_initial = account_initial;
        state.can_go_back = can_go_back;
        // Poster pixels for the cards as they are drawn now: the larger of a
        // shelf card and a grid card at this window width.
        let window = state.ctx.content_rect().width().max(760.0);
        let pixels = state.ctx.pixels_per_point();
        let poster_points = jf::overflow_width(window, Shape::Portrait).max(jf::grid_width(
            window,
            window - jf::page_padding(window) - ALPHA_COLUMN,
        ));
        let poster_width = ((poster_points * pixels) / 40.0).ceil() as u32 * 40;
        let items = items_data
            .iter()
            .map(|item| to_ui_item(item, poster_width))
            .collect::<Vec<_>>();
        let (items, sections) = if search_page {
            group_search_results(items)
        } else {
            let sections = home_sections
                .iter()
                .map(|section| {
                    let first = items_data.get(section.start_index);
                    let (shape, lines) = section_shape(&section.id, first);
                    CatalogSection {
                        id: section.id.clone(),
                        title: section.title.clone(),
                        start_index: section.start_index,
                        count: section.count,
                        shape,
                        lines,
                    }
                })
                .collect();
            (items, sections)
        };
        let catalog = &mut state.catalog;
        catalog.summary = summary;
        catalog.home = home_dashboard;
        catalog.search_page = search_page;
        catalog.page_key = page_key;
        // Only write the search box when the value actually changed and the
        // user is not mid-edit: rewriting it would clobber whatever they
        // typed while the request was in flight.
        if catalog.search_edited_at.is_none() && catalog.search_text != search_text {
            catalog.search_text = search_text;
        }
        catalog.sort_index = usize::try_from(sort.index()).unwrap_or(0);
        catalog.sections = sections;
        catalog.items = Arc::new(items);
        tracing::debug!(
            ms = super::startup::elapsed_ms(),
            stage = "catalog_presented",
            "startup_stage"
        );
        super::startup::mark_content_ready("catalog");
    });
}

/// Pushes the persisted sort preference into the window without
/// re-presenting the page.
pub fn present_preferences(context: &AppContext) {
    let sort = match context.state.lock() {
        Ok(state) => state.catalog_sort,
        Err(_) => return,
    };
    context.ui.update(move |state| {
        state.catalog.sort_index = usize::try_from(sort.index()).unwrap_or(0);
    });
}

/// A home section's card shape and text rows, as jellyfin-web's home lays
/// them out.
fn section_shape(id: &str, first: Option<&CatalogItemData>) -> (Shape, usize) {
    match id {
        "libraries" => (Shape::Backdrop, 1),
        "continue" | "next-up" => (Shape::Backdrop, 2),
        _ if first.is_some_and(|item| {
            matches!(item.kind.as_str(), "MusicAlbum" | "Audio" | "MusicArtist")
        }) =>
        {
            (Shape::Square, 2)
        }
        _ => (Shape::Portrait, 2),
    }
}

/// Search results as jellyfin-web shows them: one shelf per kind.
fn group_search_results(items: Vec<CatalogItem>) -> (Vec<CatalogItem>, Vec<CatalogSection>) {
    const GROUPS: [(&str, &str); 6] = [
        ("Movie", "Movies"),
        ("Series", "Shows"),
        ("Episode", "Episodes"),
        ("BoxSet", "Collections"),
        ("Person", "People"),
        ("", "Other"),
    ];
    let group_of = |kind: &str| {
        GROUPS
            .iter()
            .position(|(group, _)| *group == kind)
            .unwrap_or(GROUPS.len() - 1)
    };
    let mut ordered = items;
    // Stable, so the server's relevance order holds within each group.
    ordered.sort_by_key(|item| group_of(&item.kind));
    let mut sections: Vec<CatalogSection> = Vec::new();
    for (index, item) in ordered.iter().enumerate() {
        let title = GROUPS[group_of(&item.kind)].1;
        match sections.last_mut() {
            Some(section) if section.id == title => section.count += 1,
            _ => sections.push(CatalogSection {
                id: title.to_owned(),
                title: title.to_owned(),
                start_index: index,
                count: 1,
                shape: if item.kind == "Episode" {
                    Shape::Backdrop
                } else {
                    Shape::Portrait
                },
                lines: 2,
            }),
        }
    }
    (ordered, sections)
}

fn to_ui_item(item: &CatalogItemData, poster_width: u32) -> CatalogItem {
    let wide = is_wide_tile(&item.kind);
    let poster_width = if wide {
        poster_width * 3 / 2
    } else {
        poster_width
    };
    CatalogItem {
        id: item.id.clone(),
        title: if item.card_title.is_empty() {
            item.title.clone()
        } else {
            item.card_title.clone()
        },
        subtitle: item.card_subtitle.clone(),
        sort_name: item.title.clone(),
        kind: item.kind.clone(),
        poster: item.image_tag.as_ref().map(|tag| Art {
            key: catalog_image_key(&item.image_item_id, tag, poster_width),
            source: ArtSource::Catalog {
                image_item_id: item.image_item_id.clone(),
                tag: tag.clone(),
                width: poster_width,
            },
        }),
        // Shelf thumbnails are at most ~480 points wide.
        thumb: item.thumb.as_ref().map(|image| detail_art(image, 720)),
        watched: item.watched,
        favorite: item.favorite,
        unplayed: item.unplayed_count,
        progress: item.progress,
        playable: item.playable || matches!(item.kind.as_str(), "Series" | "Season" | "BoxSet"),
        wide,
    }
}

/// Folders whose artwork is a 16:9 banner (libraries, playlists).
fn is_wide_tile(kind: &str) -> bool {
    matches!(
        kind,
        "CollectionFolder" | "UserView" | "ManualPlaylistsFolder"
    )
}

impl CatalogItem {
    pub(super) fn model(&self, shape: Shape) -> CardModel<'_> {
        let art = match shape {
            Shape::Backdrop => self.thumb.as_ref().or(self.poster.as_ref()),
            _ => self.poster.as_ref(),
        };
        CardModel {
            title: &self.title,
            subtitle: &self.subtitle,
            art,
            watched: self.watched,
            unplayed: self.unplayed,
            // Progress is drawn for a single video only, as jellyfin-web does.
            progress: if matches!(self.kind.as_str(), "Movie" | "Episode" | "Video") {
                self.progress
            } else {
                0.0
            },
            favorite: self.favorite,
            playable: self.playable,
            actions: !self.wide,
        }
    }
}

const SEARCH_DEBOUNCE_SECONDS: f64 = 0.3;
/// The A–Z picker's column, which the grid leaves free on its right.
const ALPHA_COLUMN: f32 = 94.0;
const ALPHABET: [&str; 27] = [
    "#", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
    "S", "T", "U", "V", "W", "X", "Y", "Z",
];

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);
    let bar = Rect::from_min_size(screen.min, vec2(screen.width(), top_bar::HEIGHT));
    let body = Rect::from_min_max(pos2(screen.left(), bar.bottom()), screen.max);
    let mut body_ui = widgets::clipped_child(ui, body);
    if state.catalog.search_page {
        draw_search(&mut body_ui, state, actions);
    } else if state.busy {
        spinner(&mut body_ui, body);
    } else if state.catalog.items.is_empty() {
        draw_empty(&mut body_ui, state, body, actions);
    } else if state.catalog.home {
        draw_home(&mut body_ui, state, actions);
    } else {
        draw_library(&mut body_ui, state, actions);
    }
    // Drawn last so its account menu sits above the page.
    top_bar::draw(ui, state, bar, actions);
}

fn spinner(ui: &mut Ui, body: Rect) {
    ui.put(
        Rect::from_center_size(body.center(), vec2(56.0, 56.0)),
        egui::Spinner::new().size(56.0).color(theme::JF_ACCENT),
    );
}

fn centered_note(ui: &Ui, rect: Rect, text: &str, color: Color32) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::regular(theme::JF_FONT_SUBTITLE),
        color,
    );
}

fn draw_empty(ui: &mut Ui, state: &UiState, body: Rect, actions: &mut Vec<UiAction>) {
    let (text, color) = if !state.message.is_empty() {
        (state.message.as_str(), theme::DANGER_TEXT)
    } else {
        ("No items found.", theme::JF_TEXT)
    };
    let galley = widgets::wrapped(
        ui,
        text,
        theme::regular(theme::JF_FONT_SUBTITLE),
        color,
        body.width() - 82.0,
        Some(4),
    );
    let top = body.top() + 80.0;
    let text_rect = Rect::from_center_size(
        pos2(body.center().x, top + galley.size().y / 2.0),
        galley.size(),
    );
    ui.painter().galley(text_rect.min, galley, Color32::WHITE);
    if !state.message.is_empty() {
        let button = Rect::from_center_size(
            pos2(body.center().x, text_rect.bottom() + 40.0),
            vec2(160.0, 40.0),
        );
        if raised_button(ui, button, "Retry", Id::new("catalog-retry")) {
            actions.push(UiAction::Retry);
        }
    }
}

/// jellyfin-web's raised button: `#303030`, semibold text.
pub fn raised_button(ui: &mut Ui, rect: Rect, label: &str, id: Id) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    widgets::describe_button(&response, true, label);
    let fill = if response.hovered() {
        theme::SURFACE_HOVER
    } else {
        theme::JF_RAISED
    };
    ui.painter().rect_filled(rect, theme::JF_RADIUS, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::bold(theme::JF_FONT),
        Color32::from_white_alpha(222),
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// Shelves of cards, one per section, as on jellyfin-web's home.
fn draw_home(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let items = state.catalog.items.clone();
    let sections = std::mem::take(&mut state.catalog.sections);
    let width = ui.max_rect().width();
    ScrollArea::vertical()
        .id_salt(("catalog-home", state.catalog.page_key.as_str()))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.allocate_exact_size(vec2(width, 2.0), Sense::hover());
            for section in &sections {
                let rows = items
                    .get(section.start_index..section.start_index + section.count)
                    .unwrap_or_default();
                draw_shelf(ui, &mut state.images, section, rows, width, actions);
            }
            ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
        });
    state.catalog.sections = sections;
}

/// One section: its title row with paging buttons, and a horizontally
/// scrolling row of cards.
fn draw_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    section: &CatalogSection,
    rows: &[CatalogItem],
    width: f32,
    actions: &mut Vec<UiAction>,
) {
    let card_width = jf::overflow_width(width, section.shape);
    let card_height = jf::card_height(card_width, section.shape, section.lines);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 46.0 + card_height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let padding = jf::page_padding(width);
    let viewport = Rect::from_min_max(pos2(rect.left(), rect.top() + 46.0), rect.max);
    let content_width = padding * 2.0 + rows.len() as f32 * card_width;
    let id = Id::new(("catalog-shelf", section.id.as_str()));
    let mut shelf = Shelf::begin(ui, id, viewport, content_width);
    let offset = shelf.scroll(ui);
    let title_row = Rect::from_min_size(rect.min, vec2(width, 46.0));
    let paging = (content_width > viewport.width())
        .then(|| (shelf.can_page_back(offset), shelf.can_page_forward(offset)));
    match jf::section_title(
        ui,
        title_row,
        &section.title,
        padding,
        paging,
        id.with("paging"),
    ) {
        -1 => shelf.page(false),
        1 => shelf.page(true),
        _ => {}
    }
    let mut strip = widgets::clipped_child(ui, viewport);
    for (index, item) in rows.iter().enumerate() {
        let left = padding + index as f32 * card_width;
        let x = viewport.left() + offset + left;
        if x + card_width < viewport.left() - viewport.width()
            || x > viewport.right() + viewport.width()
        {
            continue;
        }
        let response = jf::card(
            &mut strip,
            images,
            &item.model(section.shape),
            pos2(x, viewport.top()),
            card_width,
            section.shape,
            section.lines,
            id.with(index),
        );
        if response.gained_focus {
            shelf.reveal(left - padding, card_width + padding);
        }
        card_actions(item, &response, actions);
    }
    shelf.end(ui);
}

fn card_actions(item: &CatalogItem, response: &jf::CardResponse, actions: &mut Vec<UiAction>) {
    if response.open {
        actions.push(UiAction::OpenItem(item.id.clone()));
    }
    if response.play {
        actions.push(UiAction::PlayItem(item.id.clone()));
    }
    if response.toggle_played {
        actions.push(UiAction::ToggleCard {
            id: item.id.clone(),
            toggle: CardToggle::Played(!item.watched),
        });
    }
    if response.toggle_favorite {
        actions.push(UiAction::ToggleCard {
            id: item.id.clone(),
            toggle: CardToggle::Favorite(!item.favorite),
        });
    }
}

/// A library: the toolbar, the poster grid, and the A–Z picker.
fn draw_library(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let body = ui.max_rect();
    let window = body.width();
    let padding = jf::page_padding(window);
    let items = state.catalog.items.clone();
    let wide = items.iter().all(|item| item.wide);
    let shape = if wide {
        Shape::Backdrop
    } else {
        Shape::Portrait
    };
    let grid_left = body.left() + padding;
    let grid_width = window - padding - ALPHA_COLUMN;
    let card_width = if wide {
        backdrop_grid_width(window, grid_width)
    } else {
        jf::grid_width(window, grid_width)
    };
    let columns = ((grid_width / card_width).round() as usize).max(1);
    let row_height = jf::card_height(card_width, shape, 2);
    let rows = items.len().div_ceil(columns);

    // Toolbar: count and sort, centred above the grid.
    let toolbar = Rect::from_min_size(body.min, vec2(window, 68.0));
    let count = widgets::text_width(ui, &state.catalog.summary, theme::regular(theme::JF_FONT));
    let total = count + 5.0 + 41.0;
    let left = toolbar.center().x - total / 2.0;
    ui.painter().text(
        pos2(left, toolbar.top() + 26.0 + 20.5),
        Align2::LEFT_CENTER,
        &state.catalog.summary,
        theme::regular(theme::JF_FONT),
        theme::JF_TEXT,
    );
    let sort_button = Rect::from_min_size(
        pos2(left + count + 5.0, toolbar.top() + 26.0),
        vec2(41.0, 41.0),
    );

    let grid_area = Rect::from_min_max(pos2(body.left(), toolbar.bottom()), body.max);
    let mut grid_ui = widgets::clipped_child(ui, grid_area);
    let jump_id = Id::new(("catalog-alpha-jump", state.catalog.page_key.as_str()));
    let jump: Option<usize> = ui.ctx().data_mut(|data| data.remove_temp(jump_id));
    let mut scroll = ScrollArea::vertical()
        .id_salt(("catalog-grid", state.catalog.page_key.as_str()))
        .auto_shrink(false);
    if let Some(row) = jump {
        scroll = scroll.vertical_scroll_offset(row as f32 * row_height);
    }
    scroll.show_rows(&mut grid_ui, row_height, rows, |ui, visible| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        for row in visible {
            let (rect, _) = ui.allocate_exact_size(vec2(window, row_height), Sense::hover());
            for column in 0..columns {
                let index = row * columns + column;
                let Some(item) = items.get(index) else { break };
                let origin = pos2(grid_left + column as f32 * card_width, rect.top());
                let id = Id::new(("catalog-grid-card", item.id.as_str()));
                let response = jf::card(
                    ui,
                    &mut state.images,
                    &item.model(shape),
                    origin,
                    card_width,
                    shape,
                    2,
                    id,
                );
                card_actions(item, &response, actions);
            }
        }
    });

    // A–Z picker: jumps the grid to the first title starting with a letter.
    let letter_gap = 20.7;
    let column_height = letter_gap * ALPHABET.len() as f32;
    let top = grid_area.center().y - column_height / 2.0;
    let x = body.right() - 37.0;
    for (index, letter) in ALPHABET.iter().enumerate() {
        let cell = Rect::from_center_size(
            pos2(x, top + index as f32 * letter_gap + letter_gap / 2.0),
            vec2(26.0, letter_gap),
        );
        let response = ui.interact(cell, Id::new(("alpha", *letter)), Sense::click());
        widgets::describe_button(&response, true, letter);
        let color = if response.hovered() {
            theme::JF_TEXT
        } else {
            theme::JF_TEXT_SECONDARY
        };
        ui.painter().text(
            cell.center(),
            Align2::CENTER_CENTER,
            *letter,
            theme::regular(theme::JF_FONT),
            color,
        );
        widgets::pointer_on_hover(&response);
        if response.clicked()
            && let Some(position) = items
                .iter()
                .position(|item| starts_with_letter(&item.sort_name, letter))
        {
            ui.ctx()
                .data_mut(|data| data.insert_temp(jump_id, position / columns));
        }
    }
    // After the grid, so the menu opens above it.
    sort_menu(ui, state, sort_button, actions);
}

/// jellyfin-web's `backdropCard` grid widths.
fn backdrop_grid_width(window: f32, grid: f32) -> f32 {
    let share = match window {
        w if w >= 2500.0 => 1.0 / 6.0,
        w if w >= 1600.0 => 0.20,
        w if w >= 1200.0 => 0.25,
        w if w >= 770.0 => 1.0 / 3.0,
        w if w >= 400.0 => 0.5,
        _ => 1.0,
    };
    grid * share
}

/// Whether `name` belongs under `letter` in the A–Z picker; "#" collects
/// everything that does not start with a letter.
fn starts_with_letter(name: &str, letter: &str) -> bool {
    let first = name
        .trim_start_matches(|character: char| !character.is_alphanumeric())
        .chars()
        .next();
    match first {
        Some(first) if first.is_ascii_alphabetic() => {
            letter.eq_ignore_ascii_case(&first.to_string())
        }
        _ => letter == "#",
    }
}

/// The toolbar's sort button and its menu of orders.
fn sort_menu(ui: &mut Ui, state: &mut UiState, button: Rect, actions: &mut Vec<UiAction>) {
    let busy = state.busy;
    let response = jf::round_icon_button(
        ui,
        button,
        Icon::SortByAlpha,
        Color32::WHITE,
        24.8,
        "Sort",
        !busy,
        Id::new("catalog-sort"),
    );
    let menu_id = Id::new("catalog-sort-menu");
    let mut open = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(menu_id))
        .unwrap_or(false);
    if response.clicked() {
        open = !open;
    }
    if !open {
        ui.ctx().data_mut(|data| data.insert_temp(menu_id, false));
        return;
    }
    let current = state.catalog.sort_index;
    let area = egui::Area::new(menu_id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(button.left(), button.bottom() + 4.0))
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
                .inner_margin(egui::Margin::symmetric(0, 6))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let mut picked = None;
                    for (index, sort) in CatalogSort::ALL.into_iter().enumerate() {
                        let (row, row_response) =
                            ui.allocate_exact_size(vec2(240.0, 40.0), Sense::click());
                        widgets::describe_button(&row_response, true, sort.label());
                        if row_response.hovered() {
                            ui.painter()
                                .rect_filled(row, 0.0, Color32::from_white_alpha(20));
                        }
                        if index == current {
                            super::icons::paint_centered(
                                ui.painter(),
                                Icon::Check,
                                pos2(row.left() + 22.0, row.center().y),
                                20.0,
                                theme::JF_ACCENT,
                            );
                        }
                        ui.painter().text(
                            pos2(row.left() + 44.0, row.center().y),
                            Align2::LEFT_CENTER,
                            sort.label(),
                            theme::regular(theme::JF_FONT),
                            theme::JF_TEXT,
                        );
                        widgets::pointer_on_hover(&row_response);
                        if row_response.clicked() {
                            picked = Some(index);
                        }
                    }
                    picked
                })
                .inner
        });
    if let Some(index) = area.inner {
        open = false;
        if index != current {
            state.catalog.sort_index = index;
            actions.push(UiAction::SortChanged(index));
        }
    } else if !response.clicked()
        && ui.input(|input| input.pointer.any_click())
        && !area.response.contains_pointer()
    {
        open = false;
    }
    ui.ctx().data_mut(|data| data.insert_temp(menu_id, open));
}

/// The search page: a search box, then the results as shelves by kind.
fn draw_search(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let body = ui.max_rect();
    let width = body.width();
    let padding = jf::page_padding(width);
    let field = Rect::from_min_size(
        pos2(body.left() + padding, body.top() + 24.0),
        vec2(width - 2.0 * padding, 41.0),
    );
    ui.painter()
        .rect_filled(field, theme::JF_RADIUS, theme::JF_INPUT);
    super::icons::paint_centered(
        ui.painter(),
        Icon::Search,
        pos2(field.left() + 22.0, field.center().y),
        24.0,
        theme::JF_TEXT_SECONDARY,
    );
    let input = Rect::from_min_max(pos2(field.left() + 44.0, field.top()), field.max);
    let catalog = &mut state.catalog;
    let response = ui.put(
        input,
        TextEdit::singleline(&mut catalog.search_text)
            .id(Id::new("catalog-search"))
            .hint_text("Search")
            .font(theme::regular(16.37))
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 10)))
            .desired_width(f32::INFINITY),
    );
    if response.has_focus() {
        widgets::border(ui.painter(), field, theme::JF_RADIUS, 2.0, theme::JF_ACCENT);
    }
    let now = ui.input(|input| input.time);
    if response.changed() {
        catalog.search_edited_at = Some(now);
    }
    let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    // The page exists to search: put the caret in the box on arrival. After
    // the Enter check, which focusing would hide (`lost_focus` is live).
    if !enter && ui.ctx().memory(|memory| memory.focused().is_none()) && catalog.items.is_empty() {
        response.request_focus();
    }
    if enter {
        catalog.search_edited_at = None;
        actions.push(UiAction::Search(catalog.search_text.clone()));
    } else if let Some(edited) = catalog.search_edited_at {
        let waited = now - edited;
        if waited >= SEARCH_DEBOUNCE_SECONDS {
            catalog.search_edited_at = None;
            actions.push(UiAction::Search(catalog.search_text.clone()));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    SEARCH_DEBOUNCE_SECONDS - waited,
                ));
        }
    }

    let results = Rect::from_min_max(pos2(body.left(), field.bottom() + 16.0), body.max);
    let mut results_ui = widgets::clipped_child(ui, results);
    if state.busy {
        spinner(&mut results_ui, results);
        return;
    }
    if state.catalog.items.is_empty() {
        if !state.catalog.search_text.trim().is_empty() && state.catalog.search_edited_at.is_none()
        {
            let note = Rect::from_min_size(results.min, vec2(width, 80.0));
            centered_note(&results_ui, note, "No results.", theme::JF_TEXT_SECONDARY);
        }
        return;
    }
    let items = state.catalog.items.clone();
    let sections = std::mem::take(&mut state.catalog.sections);
    ScrollArea::vertical()
        .id_salt(("catalog-search-results", state.catalog.page_key.as_str()))
        .auto_shrink(false)
        .show(&mut results_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for section in &sections {
                let rows = items
                    .get(section.start_index..section.start_index + section.count)
                    .unwrap_or_default();
                draw_shelf(ui, &mut state.images, section, rows, width, actions);
            }
        });
    state.catalog.sections = sections;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, name: &str) -> CatalogItem {
        CatalogItem {
            id: name.to_owned(),
            title: name.to_owned(),
            subtitle: String::new(),
            sort_name: name.to_owned(),
            kind: kind.to_owned(),
            poster: None,
            thumb: None,
            watched: false,
            favorite: false,
            unplayed: 0,
            progress: 0.0,
            playable: true,
            wide: false,
        }
    }

    #[test]
    fn search_results_are_grouped_by_kind_in_jellyfin_order() {
        let (items, sections) = group_search_results(vec![
            item("Episode", "e1"),
            item("Movie", "m1"),
            item("Series", "s1"),
            item("Movie", "m2"),
        ]);
        let titles = sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, ["Movies", "Shows", "Episodes"]);
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["m1", "m2", "s1", "e1"]
        );
        assert_eq!(sections[0].count, 2);
        assert_eq!(sections[2].shape, Shape::Backdrop);
    }

    #[test]
    fn the_alpha_picker_files_names_under_their_first_letter() {
        assert!(starts_with_letter("The Iron Giant", "T"));
        assert!(starts_with_letter("alita", "A"));
        assert!(starts_with_letter("1670", "#"));
        assert!(starts_with_letter("¡Three Amigos!", "T"));
        assert!(!starts_with_letter("Tenet", "#"));
    }

    #[test]
    fn home_sections_take_jellyfin_card_shapes() {
        assert_eq!(section_shape("libraries", None), (Shape::Backdrop, 1));
        assert_eq!(section_shape("continue", None), (Shape::Backdrop, 2));
        assert_eq!(section_shape("latest-abc", None), (Shape::Portrait, 2));
    }
}
