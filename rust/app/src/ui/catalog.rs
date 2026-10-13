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
use super::scrollbar;
use super::top_bar;
use super::widgets::{self, Shelf as ShelfScroll};
use super::{Screen, UiState, theme};
use crate::api::CatalogItemData;
use crate::app::AppContext;
use crate::features::catalog::CardToggle;
use crate::features::pages::{
    CardShape, Feature, FilterOptions, GridKind, GridSettings, PAGE_SIZE, PageContent,
    SeriesStatus, StatusFilter, TitleStyle, VideoType, ViewMode,
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
    /// The page's route, for Play All and Shuffle.
    pub route: crate::features::pages::PageRoute,
    /// Where a tabbed page's first content starts (window coordinates),
    /// or 0 to start right under the header.
    pub content_top: f32,
}

#[derive(Default)]
pub enum CatalogBody {
    #[default]
    Loading,
    Failed(String),
    Shelves(Vec<ShelfVm>),
    Grid(Box<GridVm>),
    Search(SearchVm),
}

pub struct ShelfVm {
    pub id: String,
    pub title: String,
    pub link: bool,
    pub shape: Shape,
    pub lines: usize,
    /// The tallest text among the shelf's cards, which sets its height.
    pub text: jf::CardText,
    pub wrap: bool,
    pub title_style: TitleStyle,
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
    pub square: Option<Art>,
    pub thumb: Option<Art>,
    pub banner: Option<Art>,
    pub watched: bool,
    pub favorite: bool,
    pub unplayed: u32,
    pub progress: f32,
    pub playable: bool,
    /// A library's type icon, shown when it has no artwork.
    pub icon: Option<Icon>,
    pub series_id: Option<String>,
    pub season_id: Option<String>,
    pub title_link: bool,
    pub subtitle_link: bool,
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
    context
        .ui
        .update(move |state| state.catalog.filter_options = options);
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
    let route = page.route.clone();
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
        catalog.route = route;
        let shelf_sizes = ArtSizes::new(&state.ctx, true);
        let grid_sizes = ArtSizes::new(&state.ctx, false);
        let _ = (window, pixels);
        catalog.body = match page.content {
            PageContent::Loading => CatalogBody::Loading,
            PageContent::Failed(error) => CatalogBody::Failed(error),
            PageContent::Shelves(shelves) => CatalogBody::Shelves(
                shelves
                    .iter()
                    .map(|shelf| shelf_vm(shelf, shelf_sizes))
                    .collect(),
            ),
            PageContent::Grid(grid) => CatalogBody::Grid(Box::new(GridVm {
                kind: grid_kind.unwrap_or(GridKind::Folder),
                settings,
                items: Arc::new(
                    grid.items
                        .iter()
                        .map(|item| {
                            // The Episodes tab turns off `inheritThumb`, so
                            // each episode shows its own still.
                            if grid_kind == Some(GridKind::Episodes) {
                                let mut images = item.images.clone();
                                images.series_thumb = None;
                                images.parent_thumb = None;
                                images.parent_backdrop = None;
                                let mut ui_item = to_ui_item(item, grid_sizes);
                                ui_item.thumb = grid_sizes.art(&images, Shape::Backdrop);
                                ui_item
                            } else {
                                to_ui_item(item, grid_sizes)
                            }
                        })
                        .collect(),
                ),
                total: grid.total,
                start: grid.start,
            })),
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
                        .map(|shelf| shelf_vm(shelf, shelf_sizes))
                        .collect(),
                })
            }
        };
        super::startup::mark_content_ready("catalog");
    });
}

/// What sets the artwork sizes a page asks for: jellyfin-web sizes card
/// images from the window, the screen and whether the cards scroll
/// sideways (shelves) or wrap (grids).
#[derive(Clone, Copy)]
struct ArtSizes {
    window: egui::Vec2,
    screen_width: f32,
    pixels: f32,
    overflow: bool,
}

impl ArtSizes {
    fn new(ctx: &egui::Context, overflow: bool) -> Self {
        let window = ctx.content_rect().size();
        let screen_width = ctx
            .input(|input| input.viewport().monitor_size)
            .map_or(window.x, |size| size.x);
        Self {
            window,
            screen_width,
            pixels: ctx.pixels_per_point(),
            overflow,
        }
    }

    /// The art for `item` on a card of `shape`, as jellyfin-web picks and
    /// sizes it.
    fn art(self, images: &crate::api::CardImages, shape: Shape) -> Option<Art> {
        let picked = images.pick(shape == Shape::Backdrop, shape == Shape::Banner)?;
        let image_shape = jf::ImageShape::for_card(shape, self.overflow);
        let (fill_width, fill_height) = jf::card_fill_size(
            image_shape,
            self.window,
            self.screen_width,
            self.pixels,
            picked.aspect,
        );
        let image = picked.image;
        Some(Art {
            key: format!(
                "{}:{}:{}:{}@{}x{}",
                image.item_id,
                image.image_type.jellyfin_name(),
                image.index.unwrap_or(0),
                image.tag,
                fill_width,
                fill_height
            ),
            source: ArtSource::Card {
                image,
                fill_width,
                fill_height,
            },
        })
    }
}

fn shelf_vm(shelf: &crate::features::pages::Shelf, sizes: ArtSizes) -> ShelfVm {
    let shape = match shelf.shape {
        CardShape::Portrait => Shape::Portrait,
        CardShape::Backdrop => Shape::Backdrop,
        CardShape::Square => Shape::Square,
        CardShape::Auto => auto_shape(&shelf.items),
    };
    let text = jf::CardText {
        lines: shelf.lines,
        title_link: shelf.items.iter().any(|item| item.title_link),
        subtitle_link: shelf
            .items
            .iter()
            .any(|item| item.subtitle_link && !item.card_subtitle.is_empty()),
    };
    ShelfVm {
        id: shelf.id.clone(),
        title: shelf.title.clone(),
        link: shelf.link.is_some(),
        shape,
        lines: shelf.lines,
        text,
        wrap: shelf.wrap,
        title_style: shelf.title_style,
        items: shelf
            .items
            .iter()
            .map(|item| to_ui_item(item, sizes))
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

fn to_ui_item(item: &CatalogItemData, sizes: ArtSizes) -> CatalogItem {
    CatalogItem {
        id: item.id.clone(),
        title: if item.card_title.is_empty() {
            item.title.clone()
        } else {
            item.card_title.clone()
        },
        subtitle: item.card_subtitle.clone(),
        kind: item.kind.clone(),
        poster: sizes.art(&item.images, Shape::Portrait),
        square: sizes.art(&item.images, Shape::Square),
        thumb: sizes.art(&item.images, Shape::Backdrop),
        banner: sizes.art(&item.images, Shape::Banner),
        watched: item.watched,
        favorite: item.favorite,
        unplayed: item.unplayed_count,
        progress: item.progress,
        playable: item.playable || matches!(item.kind.as_str(), "Series" | "Season" | "BoxSet"),
        icon: matches!(
            item.kind.as_str(),
            "CollectionFolder" | "UserView" | "ManualPlaylistsFolder"
        )
        .then(|| top_bar::collection_icon(&item.collection_type)),
        series_id: item.series_id.clone(),
        season_id: item.season_id.clone(),
        title_link: item.title_link,
        subtitle_link: item.subtitle_link,
    }
}

impl ShelfVm {
    fn title_height(&self) -> f32 {
        if self.title_style == TitleStyle::Heading {
            jf::SECTION_HEADING
        } else if self.link || self.title_style == TitleStyle::Tall {
            jf::SECTION_TITLE_LINKED
        } else {
            SECTION_TITLE
        }
    }
}

impl CatalogItem {
    /// The card's text rows, whose heights depend on which are links.
    fn text(&self, lines: usize) -> jf::CardText {
        jf::CardText {
            lines,
            title_link: self.title_link,
            subtitle_link: self.subtitle_link,
        }
    }

    fn model(&self, shape: Shape) -> CardModel<'_> {
        let art = match shape {
            Shape::Backdrop => self.thumb.as_ref(),
            Shape::Banner => self.banner.as_ref(),
            Shape::Square => self.square.as_ref(),
            Shape::Portrait => self.poster.as_ref(),
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
            // Progress is drawn for single playable items, not folders.
            progress: if matches!(
                self.kind.as_str(),
                "Movie" | "Episode" | "Video" | "Audio" | "AudioBook" | "MusicVideo"
            ) {
                self.progress
            } else {
                0.0
            },
            favorite: self.favorite,
            playable: self.playable,
            actions: !folder,
            placeholder_icon: self.icon,
            more: !folder,
            title_link: self.title_link,
            subtitle_link: self.subtitle_link,
        }
    }
}

const SEARCH_DEBOUNCE_SECONDS: f64 = 0.3;
/// The A–Z picker's column, which the grid leaves free on its right.
const ALPHABET: [&str; 27] = [
    "#", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
    "S", "T", "U", "V", "W", "X", "Y", "Z",
];
const NOTHING_HERE: &str = "Nothing here.";
const NO_COLLECTIONS: &str = "Collections allow you to enjoy personalized groupings of Movies, \
     Series, and Albums. Click the '+' button to start creating collections.";

/// A list view row.
const LIST_ROW: f32 = 84.0;
const SECTION_TITLE: f32 = jf::SECTION_TITLE;

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);
    let bar_height = top_bar::height(state, screen.width());
    let metrics = jf::header_metrics(screen.width());
    // The page scrollbar takes its width from the header and the page,
    // not from what media queries and `vw` lengths measure.
    let page_width = screen.width() - scrollbar::reserved(ui);
    let bar = Rect::from_min_size(screen.min, vec2(page_width, metrics.height));
    // Pages with tabs start their content at jellyfin-web's padding-top,
    // a little under the header (more on wide windows, where the tabs sit
    // in the header row).
    state.catalog.content_top = if state.catalog.tabs.is_empty() {
        0.0
    } else {
        screen.top() + metrics.tabbed_content_top
    };
    let body = Rect::from_min_max(
        pos2(screen.left(), screen.top() + bar_height),
        pos2(screen.left() + page_width, screen.bottom()),
    );
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
fn draw_shelves(
    ui: &mut Ui,
    state: &mut UiState,
    shelves: &[ShelfVm],
    actions: &mut Vec<UiAction>,
) {
    let width = ui.max_rect().width();
    let content_top = state.catalog.content_top;
    let output = ScrollArea::vertical()
        .id_salt(("catalog-shelves", state.catalog.page_key.as_str()))
        .scroll_bar_visibility(scrollbar::HIDDEN)
        .auto_shrink(false)
        .show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            // Measured from where the scroll content really starts: the
            // scroll area snaps it to a device pixel, off the header's height.
            let origin = ui.max_rect().top() + viewport.top();
            let lead = (content_top - origin).max(0.0);
            ui.allocate_exact_size(vec2(width, lead), Sense::hover());
            for shelf in shelves {
                if shelf.wrap {
                    draw_wrapped_shelf(ui, &mut state.images, shelf, width, actions);
                } else {
                    draw_shelf(ui, &mut state.images, shelf, width, actions);
                }
            }
            ui.allocate_exact_size(vec2(width, jf::PAGE_BOTTOM), Sense::hover());
        });
    scrollbar::draw(ui, &output);
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
        icons::paint_centered(
            ui.painter(),
            Icon::ChevronRight,
            pos2(
                link.left() + text_width + jf::SECTION_CHEVRON / 2.0,
                row.top() + jf::SECTION_CHEVRON_CENTER,
            ),
            jf::SECTION_CHEVRON,
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
    let window = jf::window_width(ui);
    let card_width = jf::overflow_width(window, shelf.shape);
    let card_height = jf::card_height_for(card_width, shelf.shape, shelf.text);
    let title_height = shelf.title_height();
    let (rect, _) = ui.allocate_exact_size(vec2(width, title_height + card_height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let padding = jf::page_padding(width);
    let viewport = Rect::from_min_max(pos2(rect.left(), rect.top() + title_height), rect.max);
    let content_width = padding * 2.0 + shelf.items.len() as f32 * card_width;
    let id = Id::new(("catalog-shelf", shelf.id.as_str()));
    let mut scroll = ShelfScroll::begin(ui, id, viewport, content_width);
    let offset = scroll.scroll(ui);
    let title_row = Rect::from_min_size(rect.min, vec2(width, title_height));
    let paging = (content_width > viewport.width()).then(|| {
        (
            scroll.can_page_back(offset),
            scroll.can_page_forward(offset),
        )
    });
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
        card_actions(&shelf.items[index..], &response, actions);
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
    let window = jf::window_width(ui);
    let padding = jf::page_padding(width);
    let grid = width - 2.0 * padding;
    let card_width = jf::grid_card_width(window, grid, shelf.shape);
    let columns = ((grid / card_width).round() as usize).max(1);
    let card_height = jf::card_height_for(card_width, shelf.shape, shelf.text);
    let rows = shelf.items.len().div_ceil(columns);
    let title_height = shelf.title_height();
    let (rect, _) = ui.allocate_exact_size(
        vec2(width, title_height + rows as f32 * card_height),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let title_row = Rect::from_min_size(rect.min, vec2(width, title_height));
    shelf_title(ui, title_row, shelf, padding, None, actions);
    let id = Id::new(("catalog-shelf", shelf.id.as_str()));
    for (index, item) in shelf.items.iter().enumerate() {
        let origin = pos2(
            rect.left() + padding + (index % columns) as f32 * card_width,
            rect.top() + title_height + (index / columns) as f32 * card_height,
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
        card_actions(&shelf.items[index..], &response, actions);
    }
}

fn card_actions(rest: &[CatalogItem], response: &jf::CardResponse, actions: &mut Vec<UiAction>) {
    let Some(item) = rest.first() else { return };
    if let Some(anchor) = response.more {
        // "Play all from here": the videos from this card on.
        let queue = rest
            .iter()
            .filter(|item| matches!(item.kind.as_str(), "Movie" | "Episode" | "Video"))
            .map(|item| item.id.clone())
            .collect();
        actions.push(UiAction::OpenMenu(Box::new(super::menu::MenuRequest {
            item_id: item.id.clone(),
            name: item.title.clone(),
            kind: item.kind.clone(),
            series_id: item.series_id.clone(),
            season_id: item.season_id.clone(),
            anchor,
            origin: super::menu::MenuOrigin::Card { queue },
        })));
    }
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

/// A library grid as jellyfin-web lays it out: one scrolling page holding
/// the toolbar, the cards (or list rows) and a second paging line, with the
/// A–Z picker fixed at the right; plus whichever dialog the toolbar opened.
fn draw_grid(ui: &mut Ui, state: &mut UiState, grid: &mut GridVm, actions: &mut Vec<UiAction>) {
    let body = ui.max_rect();
    let window = jf::window_width(ui);
    let window_height = ui.ctx().content_rect().height();
    let page = body.width();
    let alpha = (grid.kind.alphabet() && grid.settings.view != ViewMode::List)
        .then(|| jf::alpha_picker(window, window_height))
        .flatten();
    let padding = jf::page_padding(page);
    // `.padded-right-withalphapicker`: 7.5% of the page.
    let right = if alpha.is_some() {
        jf::layout_unit(page * 0.075)
    } else {
        padding
    };
    let grid_width = page - padding - right;
    let items = grid.items.clone();
    let scroll_id = ("catalog-grid", state.catalog.page_key.clone(), grid.start);
    let shape = grid_shape(grid.settings.view, &items);
    // Banners carry no text; networks just their name.
    let lines = match (shape, grid.kind) {
        (Some(Shape::Banner), _) => 0,
        (_, GridKind::Studios) => 1,
        _ => 2,
    };
    let (card_width, columns, row_height) = match shape {
        Some(shape) => {
            let card_width = jf::layout_unit(jf::grid_card_width(window, grid_width, shape));
            let columns = ((grid_width / card_width).round() as usize).max(1);
            // A row is as tall as its tallest card; one page holds one kind
            // of item, so that is the tallest text any of them has.
            let row_height = items
                .iter()
                .map(|item| jf::card_height_for(card_width, shape, item.text(lines)))
                .fold(0.0, f32::max);
            (card_width, columns, row_height)
        }
        None => (0.0, 1, LIST_ROW),
    };
    let rows = items.len().div_ceil(columns);
    let summary = grid_summary(grid);
    let content_top = state.catalog.content_top;
    // The cards' box, inside the page padding.
    let container = Rect::from_min_max(
        pos2(body.left() + padding, body.top()),
        pos2(body.right() - right, body.bottom()),
    );
    let collections = grid.kind == GridKind::Collections;
    // The Networks tab is just the cards: no toolbar or paging lines, 1em
    // of padding on top, rows centred.
    let bare = grid.kind == GridKind::Studios;
    let toolbar_height = if bare {
        jf::TOOLBAR_PADDING
    } else {
        jf::TOOLBAR_HEIGHT
    };
    let paging_height = if bare { 0.0 } else { jf::PAGING_BAR_HEIGHT };
    let centered = collections || bare;
    let empty_height = if !items.is_empty() {
        0.0
    } else if collections {
        jf::center_message(ui, container, 0.0, NOTHING_HERE, NO_COLLECTIONS, false)
    } else {
        0.0
    };
    let mut buttons = ToolbarButtons::default();
    let output = ScrollArea::vertical()
        .id_salt(scroll_id)
        .scroll_bar_visibility(scrollbar::HIDDEN)
        .auto_shrink(false)
        .show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            // Page coordinates start at the window top; the scroll area
            // starts under the header (snapped to a device pixel).
            let top = ui.max_rect().top();
            let lead = (content_top - (top + viewport.top())).max(0.0);
            let grid_top = lead + toolbar_height;
            let grid_height = if items.is_empty() {
                empty_height
            } else {
                rows as f32 * row_height
            };
            let total = grid_top + grid_height + paging_height + jf::PAGE_BOTTOM;
            ui.allocate_exact_size(vec2(page, total), Sense::hover());
            let toolbar = Rect::from_min_size(
                pos2(body.left(), top + lead),
                vec2(page, jf::TOOLBAR_HEIGHT),
            );
            if !bare {
                buttons = draw_toolbar(ui, toolbar, grid, &summary, &state.catalog.route, actions);
            }
            let bar = Rect::from_min_size(
                pos2(body.left(), top + grid_top + grid_height),
                vec2(page, jf::PAGING_BAR_HEIGHT),
            );
            if !bare {
                paging_text(ui, bar, bar.top() + jf::TOOLBAR_PADDING, &summary);
            }
            if items.is_empty() {
                if collections {
                    let at = top + grid_top;
                    jf::center_message(ui, container, at, NOTHING_HERE, NO_COLLECTIONS, true);
                }
                return;
            }
            // Only the rows in view are laid out and asked for artwork.
            let first = ((viewport.top() - grid_top) / row_height).floor().max(0.0) as usize;
            let last = (((viewport.bottom() - grid_top) / row_height)
                .ceil()
                .max(0.0) as usize)
                .min(rows);
            for row in first..last {
                let row_top = top + grid_top + row as f32 * row_height;
                match shape {
                    Some(shape) => {
                        // Collections centre a short row (`.centered`).
                        let in_row = (items.len() - row * columns).min(columns);
                        let indent = if centered {
                            (grid_width - in_row as f32 * card_width) / 2.0
                        } else {
                            0.0
                        };
                        for column in 0..columns {
                            let index = row * columns + column;
                            let Some(item) = items.get(index) else { break };
                            let origin = pos2(
                                body.left() + padding + indent + column as f32 * card_width,
                                row_top,
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
                            card_actions(&items[index..], &response, actions);
                        }
                    }
                    None => {
                        let rect =
                            Rect::from_min_size(pos2(body.left(), row_top), vec2(page, LIST_ROW));
                        list_row(ui, &mut state.images, &items, row, rect, padding, actions);
                    }
                }
            }
        });
    scrollbar::draw(ui, &output);
    if let Some(alpha) = alpha {
        draw_alphabet(ui, body, &alpha, &grid.settings, actions);
    }
    draw_dialog(ui, state, grid, &buttons, actions);
}

/// "1-35 of 35", or nothing for an empty page.
fn grid_summary(grid: &GridVm) -> String {
    if grid.items.is_empty() {
        return "0-0 of 0".to_owned();
    }
    let shown_end = (grid.start + grid.items.len()).min(grid.total);
    format!("{}-{} of {}", grid.start + 1, shown_end, grid.total)
}

/// The paging line centred in `row`, its 22px block starting at
/// `block_top`; returns its width.
fn paging_text(ui: &Ui, row: Rect, block_top: f32, summary: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        summary.to_owned(),
        theme::regular(theme::JF_FONT),
        jf::TEXT_80,
    );
    let width = galley.size().x;
    widgets::galley_on_baseline(
        ui.painter(),
        row.center().x - width / 2.0,
        block_top + jf::PAGING_BASELINE,
        galley,
    );
    width
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
    summary: &str,
    route: &crate::features::pages::PageRoute,
    actions: &mut Vec<UiAction>,
) -> ToolbarButtons {
    let shown_end = (grid.start + grid.items.len()).min(grid.total);
    let paged = grid.total > PAGE_SIZE;
    let mut icons_row: Vec<(Icon, &str, &str, bool)> = Vec::new();
    if paged {
        icons_row.push((Icon::ArrowBack, "Previous page", "prev", grid.start > 0));
        icons_row.push((
            Icon::ArrowForward,
            "Next page",
            "next",
            shown_end < grid.total,
        ));
    }
    if grid.kind.plays() && !grid.items.is_empty() {
        icons_row.push((Icon::Play, "Play All", "play", true));
    }
    if grid.kind.shuffles() && !grid.items.is_empty() {
        icons_row.push((Icon::Shuffle, "Shuffle", "shuffle", true));
    }
    icons_row.push((Icon::ViewComfy, "Select view", "view", true));
    if !grid.kind.sorts().is_empty() {
        icons_row.push((Icon::SortByAlpha, "Sort", "sort", true));
    }
    if grid.kind.filters() {
        icons_row.push((Icon::FilterAlt, "Filter", "filter", true));
    }
    match grid.kind {
        GridKind::Collections => {
            icons_row.push((Icon::Add, "New Collection", "new-collection", true))
        }
        GridKind::Playlists => icons_row.push((Icon::Add, "New Playlist", "new-playlist", true)),
        _ => {}
    }
    let mut out = ToolbarButtons::default();
    if !ui.is_rect_visible(toolbar) {
        return out;
    }
    // A centred flex row: the paging text, then buttons with their margins.
    let summary_width = widgets::text_width(ui, summary, theme::regular(theme::JF_FONT));
    let total = summary_width + icons_row.len() as f32 * jf::TOOLBAR_STEP;
    let left = toolbar.center().x - total / 2.0;
    let button_top = toolbar.top() + jf::TOOLBAR_PADDING;
    let row = Rect::from_min_size(pos2(left, toolbar.top()), vec2(summary_width, 0.0));
    if !summary.is_empty() {
        let block_top = button_top + (jf::TOOLBAR_BUTTON - jf::PAGING_BLOCK) / 2.0;
        paging_text(ui, row, block_top, summary);
    }
    let filtered = grid.settings.filters.count() > 0;
    for (index, (icon, label, name, enabled)) in icons_row.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(
                left + summary_width + jf::TOOLBAR_MARGIN + jf::TOOLBAR_STEP * index as f32,
                button_top,
            ),
            vec2(jf::TOOLBAR_BUTTON, jf::TOOLBAR_BUTTON),
        );
        let color = if name == "filter" && filtered {
            theme::JF_ACCENT
        } else {
            jf::TEXT_80
        };
        let clicked = jf::round_icon_button(
            ui,
            rect,
            icon,
            color,
            jf::TOOLBAR_ICON,
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
            "play" | "shuffle" => actions.push(UiAction::PlayQueue {
                source: crate::api::QueueSource::Grid(
                    Box::new(route.clone()),
                    Box::new(grid.settings.clone()),
                ),
                shuffle: name == "shuffle",
            }),
            "new-collection" => actions.push(UiAction::NewContainer { collection: true }),
            "new-playlist" => actions.push(UiAction::NewContainer { collection: false }),
            "prev" => actions.push(UiAction::GridPage(grid.start.saturating_sub(PAGE_SIZE))),
            "next" => actions.push(UiAction::GridPage(grid.start + PAGE_SIZE)),
            "view" => actions.push(UiAction::GridDialog(Some(GridDialog::View))),
            "sort" => actions.push(UiAction::GridDialog(Some(GridDialog::Sort))),
            _ => {
                actions.push(UiAction::GridDialog(Some(GridDialog::Filter(vec![
                    "Filters",
                ]))));
                actions.push(UiAction::LoadFilterOptions);
            }
        }
    }
    out
}

/// One row of the list view.
fn list_row(
    ui: &mut Ui,
    images: &mut ImageCache,
    items: &[CatalogItem],
    index: usize,
    row: Rect,
    padding: f32,
    actions: &mut Vec<UiAction>,
) {
    let item = &items[index];
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
    let texture = art.and_then(|art| images.texture(art).zip(images.size(&art.key)));
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
        widgets::galley_v_centered(ui.painter(), text_left, inner.center().y + 11.0, subtitle);
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

/// The A–Z picker: filters the grid to names starting with a letter; the
/// active letter is in the accent, and clicking it again clears it.
fn draw_alphabet(
    ui: &mut Ui,
    page: Rect,
    alpha: &jf::AlphaPicker,
    settings: &GridSettings,
    actions: &mut Vec<UiAction>,
) {
    let window = ui.ctx().content_rect();
    let bottom = window.bottom() - alpha.bottom;
    let top = bottom - alpha.row * ALPHABET.len() as f32;
    let left = page.right() - alpha.right - alpha.width;
    let active = settings.filters.starts_with.as_deref();
    let font = theme::regular(alpha.font);
    for (index, letter) in ALPHABET.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(left, top + index as f32 * alpha.row),
            vec2(alpha.width, alpha.row),
        );
        let response = ui.interact(cell, Id::new(("alpha", *letter)), Sense::click());
        widgets::describe_button(&response, true, letter);
        let selected = active == Some(*letter);
        // `.alphaPickerButton` is 50% white, `-selected` white.
        let color = if selected {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(128)
        };
        let galley = ui
            .painter()
            .layout_no_wrap((*letter).to_owned(), font.clone(), color);
        let x = cell.center().x - galley.size().x / 2.0;
        widgets::galley_on_baseline(
            ui.painter(),
            x,
            cell.top() + alpha.pad + alpha.ascent,
            galley,
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
                            let selected = sort.value == grid.settings.sort;
                            if dialogs::radio(
                                ui,
                                sort.label,
                                selected,
                                Id::new(("sort-by", sort.value)),
                            ) && !selected
                            {
                                let mut next = grid.settings.clone();
                                next.sort = sort.value.to_owned();
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
            let (toggled, close) =
                dialogs::modal(&ctx, Id::new("grid-filter"), 300.0, height, |ui| {
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
        GridKind::Shows | GridKind::Collections => &[
            StatusFilter::Played,
            StatusFilter::Unplayed,
            StatusFilter::Favorites,
        ],
        _ => &StatusFilter::ALL,
    };
    if section_header(ui, "Filters", expanded, &mut toggled) {
        for status in statuses {
            if checkbox_row(
                ui,
                "status",
                status.label(),
                filters.status.contains(status),
            ) {
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
            if checkbox_row(
                ui,
                "feature",
                feature.label(),
                filters.features.contains(&feature),
            ) {
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
    if matches!(
        grid.kind,
        GridKind::Movies | GridKind::Episodes | GridKind::Folder
    ) && section_header(ui, "Video Types", expanded, &mut toggled)
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
            let text = format!(
                "Sorry! No results found for \u{201c}{}\u{201d}",
                search.term
            );
            centered_note(&results_ui, note, &text, theme::JF_TEXT_SECONDARY);
        }
        return;
    }
    let output = ScrollArea::vertical()
        .id_salt(("catalog-search-results", search.term.as_str()))
        .scroll_bar_visibility(scrollbar::HIDDEN)
        .auto_shrink(false)
        .show(&mut results_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for shelf in &search.results {
                draw_shelf(ui, &mut state.images, shelf, width, actions);
            }
        });
    scrollbar::draw(&mut results_ui, &output);
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
        let rect = Rect::from_center_size(
            pos2(area.center().x, y + 14.0),
            galley.size() + vec2(8.0, 8.0),
        );
        let response = ui.interact(
            rect,
            Id::new(("search-suggestion", id.as_str())),
            Sense::click(),
        );
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
            square: None,
            thumb: None,
            banner: None,
            watched: false,
            favorite: false,
            unplayed: 0,
            progress: 0.0,
            playable: true,
            icon: None,
            series_id: None,
            season_id: None,
            title_link: true,
            subtitle_link: false,
        };
        let items = vec![movie];
        assert_eq!(grid_shape(ViewMode::Poster, &items), Some(Shape::Portrait));
        assert_eq!(
            grid_shape(ViewMode::ThumbCard, &items),
            Some(Shape::Backdrop)
        );
        assert_eq!(grid_shape(ViewMode::Banner, &items), Some(Shape::Banner));
        assert_eq!(grid_shape(ViewMode::List, &items), None);
    }
}
