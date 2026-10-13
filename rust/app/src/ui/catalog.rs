//! Every list page, laid out as jellyfin-web's: pages of shelves (home,
//! suggestions, genres, upcoming, favorites), library grids (toolbar,
//! paging, sort/view/filter dialogs, A–Z picker) and the search page.
//!
//! The page arrives as data from `features::catalog`; this module only turns
//! it into cards and turns clicks into [`UiAction`]s.

use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, TextEdit, Ui, pos2, vec2};

use super::app::UiAction;
use super::dialogs;
use super::icons::{self, Icon};
use super::images::{Art, ArtSource, ImageCache};
use super::jf::{self, CardModel, Shape};
use super::top_bar;
use super::widgets::{self, Shelf as ShelfScroll};
use super::{Screen, UiState, catalog_image_key, detail_art, theme};
use crate::api::CatalogItemData;
use crate::app::AppContext;
use crate::features::catalog::CardToggle;
use crate::features::pages::{
    CardShape, Feature, FilterOptions, GridKind, GridSettings, PAGE_SIZE, PageContent,
    SeriesStatus, StatusFilter, VideoType, ViewMode,
};

/// What the catalog screen shows. Filled by the presenters below.
#[derive(Default)]
pub struct CatalogVm {
    /// Identifies the page, so each page keeps its own scroll position.
    pub page_key: String,
    /// The home page, whose header shows the logo instead of back/home.
    pub home: bool,
    pub tabs: Vec<&'static str>,
    pub active_tab: usize,
    pub body: CatalogBody,
    pub search_text: String,
    /// When the search box was last edited; a search runs 300 ms after
    /// typing pauses.
    pub search_edited_at: Option<f64>,
    pub dialog: Option<GridDialog>,
    pub filter_options: Option<FilterOptions>,
}

#[derive(Default)]
pub enum CatalogBody {
    #[default]
    Loading,
    Failed(String),
    Shelves(Vec<ShelfVm>),
    Grid(GridVm),
    Search(SearchVm),
}

pub struct ShelfVm {
    pub id: String,
    pub title: String,
    pub link: bool,
    pub shape: Shape,
    pub lines: usize,
    pub wrap: bool,
    pub items: Vec<CatalogItem>,
}

pub struct GridVm {
    pub kind: GridKind,
    pub settings: GridSettings,
    pub items: Arc<Vec<CatalogItem>>,
    pub total: usize,
    pub start: usize,
}

pub struct SearchVm {
    pub suggestions: Vec<(String, String)>,
    pub term: String,
    pub results: Vec<ShelfVm>,
}

/// Which grid dialog is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridDialog {
    Sort,
    View,
    /// The filter dialog and which of its sections are expanded.
    Filter(Vec<&'static str>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: String,
    pub poster: Option<Art>,
    pub thumb: Option<Art>,
    pub banner: Option<Art>,
    pub watched: bool,
    pub favorite: bool,
    pub unplayed: u32,
    pub progress: f32,
    pub playable: bool,
    /// A library's type icon, shown when it has no artwork.
    pub icon: Option<Icon>,
}

/// Presents the page on screen (and switches to it).
pub fn present(context: &AppContext) {
    present_page(context, true);
}

/// Re-presents the page after a background refresh, without switching
/// screens, so a refresh never pulls the user off the player.
pub fn present_refresh(context: &AppContext) {
    present_page(context, false);
}

/// Hands the filter dialog the options that just arrived.
pub fn present_filter_options(context: &AppContext) {
    let options = context.state.lock().ok().and_then(|state| {
        let key = state.page.route.settings_key();
        state
            .filter_options
            .as_ref()
            .filter(|(cached, _)| *cached == key)
            .map(|(_, options)| options.clone())
    });
    context.ui.update(move |state| state.catalog.filter_options = options);
}

fn present_page(context: &AppContext, show: bool) {
    let Ok(state) = context.state.lock() else {
        return;
    };
    let page = state.page.clone();
    let account = state.account_name.clone();
    let libraries = state.libraries.clone();
    let can_go_back = !state.history.is_empty();
    let settings = state.grid_settings_for(&page.route);
    let key = page.route.settings_key();
    let filter_options = state
        .filter_options
        .as_ref()
        .filter(|(cached, _)| *cached == key)
        .map(|(_, options)| options.clone());
    drop(state);
    let title = page.route.title();
    let (tabs, active_tab) = page.route.tabs();
    let home = page.route.is_home();
    let grid_kind = page.route.grid();
    let page_key = format!("{:?}", page.route);
    context.ui.update(move |state| {
        if show {
            state.screen = Screen::Catalog;
            state.busy = page.is_loading();
            state.message.clear();
        }
        state.page_title = title;
        state.account_name = account;
        state.libraries = libraries;
        state.can_go_back = can_go_back;
        let pixels = state.ctx.pixels_per_point();
        let window = state.ctx.content_rect().width().max(760.0);
        let catalog = &mut state.catalog;
        if catalog.page_key != page_key {
            catalog.dialog = None;
            catalog.search_edited_at = None;
        }
        catalog.page_key = page_key;
        catalog.home = home;
        catalog.tabs = tabs;
        catalog.active_tab = active_tab;
        catalog.filter_options = filter_options;
        let widths = ArtWidths::new(window, pixels);
        catalog.body = match page.content {
            PageContent::Loading => CatalogBody::Loading,
            PageContent::Failed(error) => CatalogBody::Failed(error),
            PageContent::Shelves(shelves) => CatalogBody::Shelves(
                shelves
                    .iter()
                    .map(|shelf| shelf_vm(shelf, &widths))
                    .collect(),
            ),
            PageContent::Grid(grid) => CatalogBody::Grid(GridVm {
                kind: grid_kind.unwrap_or(GridKind::Folder),
                settings,
                items: Arc::new(
                    grid.items
                        .iter()
                        .map(|item| {
                            let mut ui_item = to_ui_item(item, &widths);
                            // The Episodes tab shows each episode's own still.
                            if grid_kind == Some(GridKind::Episodes)
                                && let Some(still) = &item.still
                            {
                                ui_item.thumb = Some(detail_art(still, widths.wide.max(480)));
                            }
                            ui_item
                        })
                        .collect(),
                ),
                total: grid.total,
                start: grid.start,
            }),
            PageContent::Search {
                suggestions,
                term,
                results,
            } => {
                // Only write the box when the user is not mid-edit: rewriting
                // it would clobber what they typed while the request ran.
                if catalog.search_edited_at.is_none() && catalog.search_text != term {
                    catalog.search_text.clone_from(&term);
                }
                CatalogBody::Search(SearchVm {
                    suggestions: suggestions
                        .into_iter()
                        .map(|item| (item.id, item.title))
                        .collect(),
                    term,
                    results: results
                        .iter()
                        .map(|shelf| shelf_vm(shelf, &widths))
                        .collect(),
                })
            }
        };
        super::startup::mark_content_ready("catalog");
    });
}

/// Pixel widths to request artwork at, for the window as it is now.
struct ArtWidths {
    poster: u32,
    wide: u32,
}

impl ArtWidths {
    fn new(window: f32, pixels: f32) -> Self {
        let round = |points: f32| ((points * pixels) / 40.0).ceil() as u32 * 40;
        let poster = jf::overflow_width(window, Shape::Portrait)
            .max(jf::grid_width(window, window - jf::page_padding(window) - ALPHA_COLUMN));
        let wide = jf::overflow_width(window, Shape::Backdrop);
        Self {
            poster: round(poster),
            wide: round(wide),
        }
    }
}

fn shelf_vm(shelf: &crate::features::pages::Shelf, widths: &ArtWidths) -> ShelfVm {
    let shape = match shelf.shape {
        CardShape::Portrait => Shape::Portrait,
        CardShape::Backdrop => Shape::Backdrop,
        CardShape::Square => Shape::Square,
        CardShape::Auto => auto_shape(&shelf.items),
    };
    ShelfVm {
        id: shelf.id.clone(),
        title: shelf.title.clone(),
        link: shelf.link.is_some(),
        shape,
        lines: shelf.lines,
        wrap: shelf.wrap,
        items: shelf
            .items
            .iter()
            .map(|item| to_ui_item(item, widths))
            .collect(),
    }
}

/// Square cards for music, posters for everything else.
fn auto_shape(items: &[CatalogItemData]) -> Shape {
    if items.first().is_some_and(|item| {
        matches!(
            item.kind.as_str(),
            "MusicAlbum" | "Audio" | "MusicArtist" | "Playlist"
        )
    }) {
        Shape::Square
    } else {
        Shape::Portrait
    }
}

fn to_ui_item(item: &CatalogItemData, widths: &ArtWidths) -> CatalogItem {
    CatalogItem {
        id: item.id.clone(),
        title: if item.card_title.is_empty() {
            item.title.clone()
        } else {
            item.card_title.clone()
        },
        subtitle: item.card_subtitle.clone(),
        kind: item.kind.clone(),
        poster: item.image_tag.as_ref().map(|tag| Art {
            key: catalog_image_key(&item.image_item_id, tag, widths.poster),
            source: ArtSource::Catalog {
                image_item_id: item.image_item_id.clone(),
                tag: tag.clone(),
                width: widths.poster,
            },
        }),
        thumb: item
            .thumb
            .as_ref()
            .map(|image| detail_art(image, widths.wide.max(480))),
        banner: item
            .banner
            .as_ref()
            .map(|image| detail_art(image, widths.wide.max(480) * 2)),
        watched: item.watched,
        favorite: item.favorite,
        unplayed: item.unplayed_count,
        progress: item.progress,
        playable: item.playable || matches!(item.kind.as_str(), "Series" | "Season" | "BoxSet"),
        icon: matches!(item.kind.as_str(), "CollectionFolder" | "UserView" | "ManualPlaylistsFolder")
            .then(|| top_bar::collection_icon(&item.collection_type)),
    }
}

impl CatalogItem {
    fn model(&self, shape: Shape) -> CardModel<'_> {
        let art = match shape {
            Shape::Backdrop => self.thumb.as_ref().or(self.poster.as_ref()),
            Shape::Banner => self.banner.as_ref(),
            Shape::Portrait | Shape::Square => self.poster.as_ref(),
        };
        let folder = matches!(
            self.kind.as_str(),
            "CollectionFolder" | "UserView" | "Studio" | "Genre" | "Person" | "Folder"
        );
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
            actions: !folder,
            placeholder_icon: self.icon,
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
/// The toolbar row above a grid.
const TOOLBAR_HEIGHT: f32 = 68.0;
const SECTION_TITLE: f32 = 46.0;

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);
    let bar_height = top_bar::height(state);
    let bar = Rect::from_min_size(screen.min, vec2(screen.width(), top_bar::HEIGHT));
    let body = Rect::from_min_max(pos2(screen.left(), screen.top() + bar_height), screen.max);
    let mut body_ui = widgets::clipped_child(ui, body);
    let mut catalog_body = std::mem::take(&mut state.catalog.body);
    match &mut catalog_body {
        CatalogBody::Loading => spinner(&mut body_ui, body),
        CatalogBody::Failed(error) => draw_failed(&mut body_ui, body, error, actions),
        CatalogBody::Shelves(shelves) => {
            if shelves.is_empty() {
                let note = Rect::from_min_size(body.min, vec2(body.width(), 120.0));
                centered_note(&body_ui, note, "No items found.", theme::JF_TEXT_SECONDARY);
            } else {
                draw_shelves(&mut body_ui, state, shelves, actions);
            }
        }
        CatalogBody::Grid(grid) => draw_grid(&mut body_ui, state, grid, actions),
        CatalogBody::Search(search) => draw_search(&mut body_ui, state, search, actions),
    }
    state.catalog.body = catalog_body;
    // Drawn last so its menus and drawer sit above the page.
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

fn draw_failed(ui: &mut Ui, body: Rect, error: &str, actions: &mut Vec<UiAction>) {
    let galley = widgets::wrapped(
        ui,
        error,
        theme::regular(theme::JF_FONT_SUBTITLE),
        theme::DANGER_TEXT,
        body.width() - 82.0,
        Some(4),
    );
    let top = body.top() + 80.0;
    let text_rect = Rect::from_center_size(
        pos2(body.center().x, top + galley.size().y / 2.0),
        galley.size(),
    );
    ui.painter().galley(text_rect.min, galley, Color32::WHITE);
    let button = Rect::from_center_size(
        pos2(body.center().x, text_rect.bottom() + 40.0),
        vec2(160.0, 40.0),
    );
    if raised_button(ui, button, "Retry", Id::new("catalog-retry")) {
        actions.push(UiAction::Retry);
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

/// A page of shelves, one section each.
fn draw_shelves(ui: &mut Ui, state: &mut UiState, shelves: &[ShelfVm], actions: &mut Vec<UiAction>) {
    let width = ui.max_rect().width();
    ScrollArea::vertical()
        .id_salt(("catalog-shelves", state.catalog.page_key.as_str()))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.allocate_exact_size(vec2(width, 2.0), Sense::hover());
            for shelf in shelves {
                if shelf.wrap {
                    draw_wrapped_shelf(ui, &mut state.images, shelf, width, actions);
                } else {
                    draw_shelf(ui, &mut state.images, shelf, width, actions);
                }
            }
            ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
        });
}

/// A section title, with a "›" link when the section opens a page.
fn shelf_title(
    ui: &mut Ui,
    row: Rect,
    shelf: &ShelfVm,
    padding: f32,
    paging: Option<(bool, bool)>,
    actions: &mut Vec<UiAction>,
) -> i32 {
    let id = Id::new(("catalog-shelf", shelf.id.as_str()));
    if shelf.link {
        let font = theme::regular(theme::JF_FONT_SECTION);
        let text_width = widgets::text_width(ui, &shelf.title, font);
        let link = Rect::from_min_size(
            pos2(row.left() + padding, row.top()),
            vec2(text_width + 30.0, row.height()),
        );
        let response = ui.interact(link, id.with("link"), Sense::click());
        widgets::describe_button(&response, true, &shelf.title);
        if response.hovered() {
            ui.painter().hline(
                link.left()..=link.left() + text_width,
                row.center().y + 13.0,
                egui::Stroke::new(1.0, theme::JF_TEXT),
            );
        }
        icons::paint_centered(
            ui.painter(),
            Icon::ChevronRight,
            pos2(link.left() + text_width + 14.0, row.center().y + 1.0),
            22.0,
            theme::JF_TEXT,
        );
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            actions.push(UiAction::OpenShelfLink(shelf.id.clone()));
        }
    }
    jf::section_title(ui, row, &shelf.title, padding, paging, id.with("paging"))
}

/// One section: its title row with paging buttons, and a horizontally
/// scrolling row of cards.
fn draw_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    shelf: &ShelfVm,
    width: f32,
    actions: &mut Vec<UiAction>,
) {
    let card_width = jf::overflow_width(width, shelf.shape);
    let card_height = jf::card_height(card_width, shelf.shape, shelf.lines);
    let (rect, _) = ui.allocate_exact_size(vec2(width, SECTION_TITLE + card_height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let padding = jf::page_padding(width);
    let viewport = Rect::from_min_max(pos2(rect.left(), rect.top() + SECTION_TITLE), rect.max);
    let content_width = padding * 2.0 + shelf.items.len() as f32 * card_width;
    let id = Id::new(("catalog-shelf", shelf.id.as_str()));
    let mut scroll = ShelfScroll::begin(ui, id, viewport, content_width);
    let offset = scroll.scroll(ui);
    let title_row = Rect::from_min_size(rect.min, vec2(width, SECTION_TITLE));
    let paging = (content_width > viewport.width())
        .then(|| (scroll.can_page_back(offset), scroll.can_page_forward(offset)));
    match shelf_title(ui, title_row, shelf, padding, paging, actions) {
        -1 => scroll.page(false),
        1 => scroll.page(true),
        _ => {}
    }
    let mut strip = widgets::clipped_child(ui, viewport);
    for (index, item) in shelf.items.iter().enumerate() {
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
            &item.model(shelf.shape),
            pos2(x, viewport.top()),
            card_width,
            shelf.shape,
            shelf.lines,
            id.with(index),
        );
        if response.gained_focus {
            scroll.reveal(left - padding, card_width + padding);
        }
        card_actions(item, &response, actions);
    }
    scroll.end(ui);
}

/// A section whose cards wrap into rows, as on the Genres and Upcoming tabs.
fn draw_wrapped_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    shelf: &ShelfVm,
    width: f32,
    actions: &mut Vec<UiAction>,
) {
    let padding = jf::page_padding(width);
    let grid = width - 2.0 * padding;
    let card_width = jf::grid_card_width(width, grid, shelf.shape);
    let columns = ((grid / card_width).round() as usize).max(1);
    let card_height = jf::card_height(card_width, shelf.shape, shelf.lines);
    let rows = shelf.items.len().div_ceil(columns);
    let (rect, _) = ui.allocate_exact_size(
        vec2(width, SECTION_TITLE + rows as f32 * card_height),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let title_row = Rect::from_min_size(rect.min, vec2(width, SECTION_TITLE));
    shelf_title(ui, title_row, shelf, padding, None, actions);
    let id = Id::new(("catalog-shelf", shelf.id.as_str()));
    for (index, item) in shelf.items.iter().enumerate() {
        let origin = pos2(
            rect.left() + padding + (index % columns) as f32 * card_width,
            rect.top() + SECTION_TITLE + (index / columns) as f32 * card_height,
        );
        let card_rect = Rect::from_min_size(origin, vec2(card_width, card_height));
        if !ui.is_rect_visible(card_rect) {
            continue;
        }
        let response = jf::card(
            ui,
            images,
            &item.model(shelf.shape),
            origin,
            card_width,
            shelf.shape,
            shelf.lines,
            id.with(index),
        );
        card_actions(item, &response, actions);
    }
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

/// The card shape a grid draws in `view`.
fn grid_shape(view: ViewMode, items: &[CatalogItem]) -> Option<Shape> {
    match view {
        ViewMode::Poster | ViewMode::PosterCard => Some(
            if items
                .first()
                .is_some_and(|item| matches!(item.kind.as_str(), "Playlist" | "MusicAlbum"))
            {
                Shape::Square
            } else {
                Shape::Portrait
            },
        ),
        ViewMode::Thumb | ViewMode::ThumbCard => Some(Shape::Backdrop),
        ViewMode::Banner => Some(Shape::Banner),
        ViewMode::List => None,
    }
}

/// A library grid: the toolbar, the cards (or list rows), the A–Z picker,
/// and whichever dialog the toolbar opened.
fn draw_grid(ui: &mut Ui, state: &mut UiState, grid: &mut GridVm, actions: &mut Vec<UiAction>) {
    let body = ui.max_rect();
    let window = body.width();
    let padding = jf::page_padding(window);
    let alphabet = grid.kind.alphabet() && grid.settings.view != ViewMode::List;
    let right = if alphabet { ALPHA_COLUMN } else { padding };
    let grid_width = window - padding - right;

    let toolbar = Rect::from_min_size(body.min, vec2(window, TOOLBAR_HEIGHT));
    let buttons = draw_toolbar(ui, toolbar, grid, actions);

    let area = Rect::from_min_max(pos2(body.left(), toolbar.bottom()), body.max);
    let mut grid_ui = widgets::clipped_child(ui, area);
    if grid.items.is_empty() {
        let note = Rect::from_min_size(area.min, vec2(window, 80.0));
        centered_note(&grid_ui, note, "No items found.", theme::JF_TEXT_SECONDARY);
    } else {
        let items = grid.items.clone();
        let page_key = state.catalog.page_key.clone();
        let scroll_id = ("catalog-grid", page_key.as_str(), grid.start);
        match grid_shape(grid.settings.view, &items) {
            Some(shape) => {
                let lines = if shape == Shape::Banner { 0 } else { 2 };
                let card_width = jf::grid_card_width(window, grid_width, shape);
                let columns = ((grid_width / card_width).round() as usize).max(1);
                let row_height = jf::card_height(card_width, shape, lines);
                let rows = items.len().div_ceil(columns);
                ScrollArea::vertical()
                    .id_salt(scroll_id)
                    .auto_shrink(false)
                    .show_rows(&mut grid_ui, row_height, rows, |ui, visible| {
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        for row in visible {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(window, row_height), Sense::hover());
                            for column in 0..columns {
                                let index = row * columns + column;
                                let Some(item) = items.get(index) else { break };
                                let origin = pos2(
                                    body.left() + padding + column as f32 * card_width,
                                    rect.top(),
                                );
                                let response = jf::card(
                                    ui,
                                    &mut state.images,
                                    &item.model(shape),
                                    origin,
                                    card_width,
                                    shape,
                                    lines,
                                    Id::new(("catalog-grid-card", item.id.as_str())),
                                );
                                card_actions(item, &response, actions);
                            }
                        }
                    });
            }
            None => draw_list(&mut grid_ui, state, &items, scroll_id, padding, actions),
        }
    }
    if alphabet {
        draw_alphabet(ui, area, &grid.settings, actions);
    }
    draw_dialog(ui, state, grid, &buttons, actions);
}

/// Where the toolbar's dialog buttons sit, for anchoring their menus.
#[derive(Default)]
struct ToolbarButtons {
    view: Option<Rect>,
}

fn draw_toolbar(
    ui: &mut Ui,
    toolbar: Rect,
    grid: &GridVm,
    actions: &mut Vec<UiAction>,
) -> ToolbarButtons {
    let shown_end = (grid.start + grid.items.len()).min(grid.total);
    let summary = if grid.items.is_empty() {
        String::new()
    } else {
        format!("{}-{} of {}", grid.start + 1, shown_end, grid.total)
    };
    let paged = grid.total > PAGE_SIZE;
    let mut icons_row: Vec<(Icon, &str, &str, bool)> = Vec::new();
    if paged {
        icons_row.push((Icon::ArrowBack, "Previous page", "prev", grid.start > 0));
        icons_row.push((Icon::ArrowForward, "Next page", "next", shown_end < grid.total));
    }
    icons_row.push((Icon::ViewComfy, "Select view", "view", true));
    if !grid.kind.sorts().is_empty() {
        icons_row.push((Icon::SortByAlpha, "Sort", "sort", true));
    }
    if grid.kind.filters() {
        icons_row.push((Icon::FilterAlt, "Filter", "filter", true));
    }
    const STEP: f32 = 50.0;
    let count = widgets::text_width(ui, &summary, theme::regular(theme::JF_FONT));
    let total = count + 9.0 + icons_row.len() as f32 * STEP;
    let left = toolbar.center().x - total / 2.0;
    let center_y = toolbar.top() + 26.0 + 20.5;
    ui.painter().text(
        pos2(left, center_y),
        Align2::LEFT_CENTER,
        &summary,
        theme::regular(theme::JF_FONT),
        theme::JF_TEXT,
    );
    let mut out = ToolbarButtons::default();
    let filtered = grid.settings.filters.count() > 0;
    for (index, (icon, label, name, enabled)) in icons_row.into_iter().enumerate() {
        let rect = Rect::from_center_size(
            pos2(left + count + 9.0 + STEP * index as f32 + STEP / 2.0, center_y),
            vec2(41.0, 41.0),
        );
        let color = if name == "filter" && filtered {
            theme::JF_ACCENT
        } else {
            Color32::WHITE
        };
        let clicked = jf::round_icon_button(
            ui,
            rect,
            icon,
            color,
            24.8,
            label,
            enabled,
            Id::new(("grid-toolbar", name)),
        )
        .clicked();
        if name == "view" {
            out.view = Some(rect);
        }
        if !clicked {
            continue;
        }
        match name {
            "prev" => actions.push(UiAction::GridPage(grid.start.saturating_sub(PAGE_SIZE))),
            "next" => actions.push(UiAction::GridPage(grid.start + PAGE_SIZE)),
            "view" => actions.push(UiAction::GridDialog(Some(GridDialog::View))),
            "sort" => actions.push(UiAction::GridDialog(Some(GridDialog::Sort))),
            _ => {
                actions.push(UiAction::GridDialog(Some(GridDialog::Filter(vec!["Filters"]))));
                actions.push(UiAction::LoadFilterOptions);
            }
        }
    }
    out
}

/// jellyfin-web's list view: a row per item with a small thumbnail.
fn draw_list(
    ui: &mut Ui,
    state: &mut UiState,
    items: &[CatalogItem],
    scroll_id: (&str, &str, usize),
    padding: f32,
    actions: &mut Vec<UiAction>,
) {
    const ROW: f32 = 84.0;
    let width = ui.max_rect().width();
    ScrollArea::vertical()
        .id_salt(scroll_id)
        .auto_shrink(false)
        .show_rows(ui, ROW, items.len(), |ui, visible| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for index in visible {
                let item = &items[index];
                let (row, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
                let inner = Rect::from_min_max(
                    pos2(row.left() + padding, row.top()),
                    pos2(row.right() - padding, row.bottom()),
                );
                let id = Id::new(("catalog-list-row", item.id.as_str()));
                let response = ui.interact(inner, id, Sense::click());
                widgets::describe_button(&response, true, &item.title);
                if response.hovered() || response.has_focus() {
                    ui.painter()
                        .rect_filled(inner, 0.0, Color32::from_white_alpha(13));
                }
                let image = Rect::from_min_size(
                    pos2(inner.left() + 8.0, inner.top() + 8.0),
                    vec2(122.0, 68.0),
                );
                let art = item.thumb.as_ref().or(item.poster.as_ref());
                let texture = art.and_then(|art| {
                    state
                        .images
                        .texture(art)
                        .zip(state.images.size(&art.key))
                });
                match texture {
                    Some((texture, size)) => {
                        widgets::image_cover(ui.painter(), texture, size, image, theme::JF_RADIUS);
                    }
                    None => {
                        let color = jf::default_background(&item.title);
                        ui.painter().rect_filled(image, theme::JF_RADIUS, color);
                    }
                }
                let text_left = image.right() + 16.0;
                let text_width = inner.right() - text_left - 16.0;
                let title = widgets::elided(
                    ui,
                    &item.title,
                    theme::regular(theme::JF_FONT),
                    theme::JF_TEXT,
                    text_width,
                );
                widgets::galley_v_centered(ui.painter(), text_left, inner.center().y - 10.0, title);
                if !item.subtitle.is_empty() {
                    let subtitle = widgets::elided(
                        ui,
                        &item.subtitle,
                        theme::regular(theme::JF_FONT_SECONDARY),
                        theme::JF_TEXT_SECONDARY,
                        text_width,
                    );
                    widgets::galley_v_centered(
                        ui.painter(),
                        text_left,
                        inner.center().y + 11.0,
                        subtitle,
                    );
                }
                if item.watched {
                    icons::paint_centered(
                        ui.painter(),
                        Icon::Check,
                        pos2(inner.right() - 24.0, inner.center().y),
                        22.0,
                        theme::JF_PLAYED_RED,
                    );
                }
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    actions.push(UiAction::OpenItem(item.id.clone()));
                }
            }
        });
}

/// The A–Z picker: filters the grid to names starting with a letter; the
/// active letter is in the accent, and clicking it again clears it.
fn draw_alphabet(ui: &mut Ui, area: Rect, settings: &GridSettings, actions: &mut Vec<UiAction>) {
    let letter_gap = 20.7;
    let column_height = letter_gap * ALPHABET.len() as f32;
    let top = area.center().y - column_height / 2.0;
    let x = area.right() - 37.0;
    let active = settings.filters.starts_with.as_deref();
    for (index, letter) in ALPHABET.iter().enumerate() {
        let cell = Rect::from_center_size(
            pos2(x, top + index as f32 * letter_gap + letter_gap / 2.0),
            vec2(26.0, letter_gap),
        );
        let response = ui.interact(cell, Id::new(("alpha", *letter)), Sense::click());
        widgets::describe_button(&response, true, letter);
        let selected = active == Some(*letter);
        let color = if selected {
            theme::JF_ACCENT
        } else if response.hovered() {
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
        if response.clicked() {
            let mut next = settings.clone();
            next.filters.starts_with = (!selected).then(|| (*letter).to_owned());
            actions.push(UiAction::GridSettings(next));
        }
    }
}

fn draw_dialog(
    ui: &mut Ui,
    state: &mut UiState,
    grid: &GridVm,
    buttons: &ToolbarButtons,
    actions: &mut Vec<UiAction>,
) {
    let Some(dialog) = state.catalog.dialog.clone() else {
        return;
    };
    let ctx = ui.ctx().clone();
    match dialog {
        GridDialog::View => {
            let items = ViewMode::ALL
                .iter()
                .map(|view| dialogs::SheetItem {
                    label: view.label(),
                    icon: None,
                    checked: *view == grid.settings.view,
                })
                .collect::<Vec<_>>();
            let selected = ViewMode::ALL
                .iter()
                .position(|view| *view == grid.settings.view)
                .unwrap_or(0);
            let anchor = buttons.view.unwrap_or(Rect::from_min_size(
                ui.max_rect().center(),
                vec2(41.0, 41.0),
            ));
            let (picked, close) =
                dialogs::action_sheet(&ctx, Id::new("grid-view-sheet"), anchor, &items, selected);
            if let Some(index) = picked {
                let mut next = grid.settings.clone();
                next.view = ViewMode::ALL[index];
                actions.push(UiAction::GridSettings(next));
            }
            if close {
                state.catalog.dialog = None;
            }
        }
        GridDialog::Sort => {
            let ((), close) = dialogs::modal(&ctx, Id::new("grid-sort"), 199.0, None, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(22, 12))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        dialogs::heading(ui, "Sort By");
                        for sort in grid.kind.sorts() {
                            let selected = *sort == grid.settings.sort;
                            if dialogs::radio(ui, sort.label(), selected, Id::new(("sort-by", sort.key())))
                                && !selected
                            {
                                let mut next = grid.settings.clone();
                                next.sort = *sort;
                                actions.push(UiAction::GridSettings(next));
                            }
                        }
                        ui.add_space(12.0);
                        dialogs::heading(ui, "Sort Order");
                        for (label, descending) in [("Ascending", false), ("Descending", true)] {
                            let selected = grid.settings.descending == descending;
                            if dialogs::radio(ui, label, selected, Id::new(("sort-order", label)))
                                && !selected
                            {
                                let mut next = grid.settings.clone();
                                next.descending = descending;
                                actions.push(UiAction::GridSettings(next));
                            }
                        }
                    });
            });
            if close {
                state.catalog.dialog = None;
            }
        }
        GridDialog::Filter(expanded) => {
            let options = state.catalog.filter_options.clone().unwrap_or_default();
            // jellyfin-web's filter dialog keeps one height as sections open.
            let height = Some(ui.ctx().content_rect().height() * 0.65);
            let (toggled, close) = dialogs::modal(&ctx, Id::new("grid-filter"), 300.0, height, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(29, 16))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        filter_sections(ui, grid, &options, &expanded, actions)
                    })
                    .inner
            });
            if close {
                state.catalog.dialog = None;
            } else if let Some(section) = toggled {
                let mut expanded = expanded;
                if let Some(at) = expanded.iter().position(|open| *open == section) {
                    expanded.remove(at);
                } else {
                    expanded.push(section);
                }
                state.catalog.dialog = Some(GridDialog::Filter(expanded));
            }
        }
    }
}

/// The filter dialog's collapsible sections. Each change applies at once.
/// Returns a section whose header was clicked.
fn filter_sections(
    ui: &mut Ui,
    grid: &GridVm,
    options: &FilterOptions,
    expanded: &[&'static str],
    actions: &mut Vec<UiAction>,
) -> Option<&'static str> {
    let filters = &grid.settings.filters;
    let mut next = grid.settings.clone();
    let mut toggled = None;
    let statuses: &[StatusFilter] = match grid.kind {
        GridKind::Shows | GridKind::Collections => {
            &[StatusFilter::Played, StatusFilter::Unplayed, StatusFilter::Favorites]
        }
        _ => &StatusFilter::ALL,
    };
    if section_header(ui, "Filters", expanded, &mut toggled) {
        for status in statuses {
            if checkbox_row(ui, "status", status.label(), filters.status.contains(status)) {
                toggle(&mut next.filters.status, *status);
            }
        }
    }
    if grid.kind == GridKind::Shows && section_header(ui, "Status", expanded, &mut toggled) {
        for status in SeriesStatus::ALL {
            let checked = filters.series_status.contains(&status);
            if checkbox_row(ui, "series", status.label(), checked) {
                toggle(&mut next.filters.series_status, status);
            }
        }
    }
    if section_header(ui, "Features", expanded, &mut toggled) {
        for feature in Feature::ALL {
            if checkbox_row(ui, "feature", feature.label(), filters.features.contains(&feature)) {
                toggle(&mut next.filters.features, feature);
            }
        }
    }
    if section_header(ui, "Genres", expanded, &mut toggled) {
        for value in values_or_none(ui, &options.genres) {
            if checkbox_row(ui, "genre", value, filters.genres.contains(value)) {
                toggle(&mut next.filters.genres, value.clone());
            }
        }
    }
    if section_header(ui, "Parental Ratings", expanded, &mut toggled) {
        for value in values_or_none(ui, &options.ratings) {
            if checkbox_row(ui, "rating", value, filters.ratings.contains(value)) {
                toggle(&mut next.filters.ratings, value.clone());
            }
        }
    }
    if section_header(ui, "Tags", expanded, &mut toggled) {
        for value in values_or_none(ui, &options.tags) {
            if checkbox_row(ui, "tag", value, filters.tags.contains(value)) {
                toggle(&mut next.filters.tags, value.clone());
            }
        }
    }
    if matches!(grid.kind, GridKind::Movies | GridKind::Episodes | GridKind::Folder)
        && section_header(ui, "Video Types", expanded, &mut toggled)
    {
        for video_type in VideoType::ALL {
            let checked = filters.video_types.contains(&video_type);
            if checkbox_row(ui, "video", video_type.label(), checked) {
                toggle(&mut next.filters.video_types, video_type);
            }
        }
    }
    if section_header(ui, "Years", expanded, &mut toggled) {
        if options.years.is_empty() {
            values_or_none(ui, &[]);
        }
        for year in &options.years {
            if checkbox_row(ui, "year", &year.to_string(), filters.years.contains(year)) {
                toggle(&mut next.filters.years, *year);
            }
        }
    }
    if next != grid.settings {
        actions.push(UiAction::GridSettings(next));
    }
    toggled
}

/// A filter section's header; returns whether the section is open.
fn section_header(
    ui: &mut Ui,
    title: &'static str,
    expanded: &[&'static str],
    toggled: &mut Option<&'static str>,
) -> bool {
    let open = expanded.contains(&title);
    if dialogs::collapsible(ui, title, open, Id::new(("filter-section", title))) {
        *toggled = Some(title);
    }
    if open {
        ui.add_space(6.0);
    }
    open
}

fn checkbox_row(ui: &mut Ui, group: &str, label: &str, checked: bool) -> bool {
    dialogs::checkbox(ui, label, checked, Id::new(("filter", group, label)))
}

/// The values to list, or a "None" note when the server has none.
fn values_or_none<'a>(ui: &mut Ui, values: &'a [String]) -> &'a [String] {
    if values.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 31.5), Sense::hover());
        ui.painter().text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            "None",
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT_SECONDARY,
        );
    }
    values
}

fn toggle<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if let Some(at) = values.iter().position(|existing| *existing == value) {
        values.remove(at);
    } else {
        values.push(value);
    }
}

/// The search page: a search box, suggestions while it is empty, and then
/// the results as shelves by kind.
fn draw_search(ui: &mut Ui, state: &mut UiState, search: &SearchVm, actions: &mut Vec<UiAction>) {
    let body = ui.max_rect();
    let width = body.width();
    let padding = jf::page_padding(width);
    let field = Rect::from_min_size(
        pos2(body.left() + padding, body.top() + 24.0),
        vec2(width - 2.0 * padding, 41.0),
    );
    ui.painter()
        .rect_filled(field, theme::JF_RADIUS, theme::JF_INPUT);
    icons::paint_centered(
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
    if !enter && ui.ctx().memory(|memory| memory.focused().is_none()) && search.results.is_empty() {
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
    if search.term.is_empty() {
        draw_suggestions(&mut results_ui, results, &search.suggestions, actions);
        return;
    }
    if search.results.is_empty() {
        if state.catalog.search_edited_at.is_none() {
            let note = Rect::from_min_size(results.min, vec2(width, 80.0));
            let text = format!("Sorry! No results found for \u{201c}{}\u{201d}", search.term);
            centered_note(&results_ui, note, &text, theme::JF_TEXT_SECONDARY);
        }
        return;
    }
    ScrollArea::vertical()
        .id_salt(("catalog-search-results", search.term.as_str()))
        .auto_shrink(false)
        .show(&mut results_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for shelf in &search.results {
                draw_shelf(ui, &mut state.images, shelf, width, actions);
            }
        });
}

/// jellyfin-web's "Suggestions": a centred column of title links.
fn draw_suggestions(
    ui: &mut Ui,
    area: Rect,
    suggestions: &[(String, String)],
    actions: &mut Vec<UiAction>,
) {
    if suggestions.is_empty() {
        return;
    }
    let mut y = area.top() + 8.0;
    ui.painter().text(
        pos2(area.center().x, y + 20.0),
        Align2::CENTER_CENTER,
        "Suggestions",
        theme::regular(theme::JF_FONT_SECTION),
        theme::JF_TEXT,
    );
    y += 52.0;
    for (id, title) in suggestions {
        let galley = widgets::elided(
            ui,
            title,
            theme::regular(theme::JF_FONT),
            theme::JF_ACCENT,
            area.width() - 40.0,
        );
        let rect = Rect::from_center_size(pos2(area.center().x, y + 14.0), galley.size() + vec2(8.0, 8.0));
        let response = ui.interact(rect, Id::new(("search-suggestion", id.as_str())), Sense::click());
        widgets::describe_button(&response, true, title);
        let text_rect = Rect::from_center_size(rect.center(), galley.size());
        ui.painter().galley(text_rect.min, galley, theme::JF_ACCENT);
        if response.hovered() {
            ui.painter().hline(
                text_rect.x_range(),
                text_rect.bottom(),
                egui::Stroke::new(1.0, theme::JF_ACCENT),
            );
        }
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            actions.push(UiAction::OpenItem(id.clone()));
        }
        y += 30.0;
        if y > area.bottom() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str) -> CatalogItemData {
        CatalogItemData {
            id: kind.to_owned(),
            title: kind.to_owned(),
            kind: kind.to_owned(),
            ..CatalogItemData::default()
        }
    }

    #[test]
    fn music_shelves_are_square_and_everything_else_is_a_poster() {
        assert_eq!(auto_shape(&[item("MusicAlbum")]), Shape::Square);
        assert_eq!(auto_shape(&[item("Movie")]), Shape::Portrait);
        assert_eq!(auto_shape(&[]), Shape::Portrait);
    }

    #[test]
    fn views_choose_card_shapes() {
        let movie = CatalogItem {
            id: "m".into(),
            title: "m".into(),
            subtitle: String::new(),
            kind: "Movie".into(),
            poster: None,
            thumb: None,
            banner: None,
            watched: false,
            favorite: false,
            unplayed: 0,
            progress: 0.0,
            playable: true,
            icon: None,
        };
        let items = vec![movie];
        assert_eq!(grid_shape(ViewMode::Poster, &items), Some(Shape::Portrait));
        assert_eq!(grid_shape(ViewMode::ThumbCard, &items), Some(Shape::Backdrop));
        assert_eq!(grid_shape(ViewMode::Banner, &items), Some(Shape::Banner));
        assert_eq!(grid_shape(ViewMode::List, &items), None);
    }
}
