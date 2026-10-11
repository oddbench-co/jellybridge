//! Catalog screen: presentation into the view model, and drawing of the home
//! dashboard shelves and the library grid.

use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, TextEdit, Ui, pos2, vec2};

use super::app::UiAction;
use super::images::{Art, ArtSource, ImageCache};
use super::top_bar;
use super::widgets::{self, Shelf};
use super::{Screen, UiState, catalog_image_key, theme};
use crate::api::CatalogItemData;
use crate::app::AppContext;
use crate::features::catalog::CatalogSort;

/// What the catalog screen shows. Filled by the presenters below.
#[derive(Default)]
pub struct CatalogVm {
    pub summary: String,
    pub home: bool,
    /// Identifies the page, so each page gets its own scroll position and a
    /// new page opens at the top.
    pub page_key: String,
    pub search_text: String,
    /// When the search box was last edited; a search runs 300 ms after
    /// typing pauses.
    pub search_edited_at: Option<f64>,
    pub sort_index: usize,
    pub density_index: usize,
    pub tile_target_width: f32,
    pub sections: Vec<CatalogSection>,
    pub items: Arc<Vec<CatalogItem>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogSection {
    pub id: String,
    pub title: String,
    pub start_index: usize,
    pub count: usize,
    /// Libraries and playlists are 16:9 banner art, not 2:3 posters. Cropping
    /// one into the other threw away 62% of the width and upscaled the rest.
    pub wide: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind_label: String,
    pub kind: String,
    pub placeholder: String,
    pub art: Option<Art>,
    pub watched: bool,
    pub progress: f32,
}

/// Presents the current catalog page (home dashboard, browse results, or
/// search results).
pub fn present(context: &AppContext, message: &str) {
    // A new page outdates whatever the previous one still had in flight.
    context.next_generation();
    present_page(context, Some(message));
}

/// Re-presents the page after a background refresh, under a generation the
/// refresh already claimed -- bumping it again would outdate a navigation the
/// user started meanwhile. Leaves the screen, busy state, and message alone,
/// so a refresh never pulls the user off the player or clears a status line.
pub fn present_refresh(context: &AppContext, _generation: u64) {
    present_page(context, None);
}

/// `message` is `None` for a background refresh.
fn present_page(context: &AppContext, message: Option<&str>) {
    let (current, account, can_go_back, sort, density) = match context.state.lock() {
        Ok(state) => (
            state.current.clone(),
            state.account_name.clone(),
            !state.back_stack.is_empty(),
            state.catalog_sort,
            state.catalog_density,
        ),
        Err(_) => return,
    };
    let sections: Vec<CatalogSection> = current
        .sections
        .iter()
        .map(|section| CatalogSection {
            id: section.id.clone(),
            title: section.title.clone(),
            start_index: section.start_index,
            count: section.count,
            // Taken from the rows themselves rather than the section id, so a
            // folder of libraries is shaped correctly too. A shelf holds one
            // kind of row, so the first one decides.
            wide: current
                .items
                .get(section.start_index)
                .is_some_and(|item| is_wide_tile(&item.kind)),
        })
        .collect();
    let home_dashboard = current.is_home_dashboard();
    let title = if current.parent.is_none() && !current.search_text.is_empty() {
        "Search results".to_owned()
    } else {
        current.title.clone()
    };
    let summary = if home_dashboard {
        format!(
            "{} sections • {} items",
            current.sections.len(),
            current.items.len()
        )
    } else if current.total_record_count > current.items.len() {
        format!(
            "Showing first {} of {}",
            current.items.len(),
            current.total_record_count
        )
    } else {
        let count = current.total_record_count;
        format!("{count} {}", if count == 1 { "item" } else { "items" })
    };
    let page_key = format!(
        "{}|{}|{}",
        current
            .parent
            .as_ref()
            .map_or("", |parent| parent.id.as_str()),
        current.search_text,
        current.title
    );
    let search_text = current.search_text.clone();
    let account_initial = account
        .trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_owned());
    let message = message.map(str::to_owned);
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
        let catalog = &mut state.catalog;
        catalog.summary = summary;
        catalog.home = home_dashboard;
        catalog.page_key = page_key;
        // Only write the search box when the value actually changed and the
        // user is not mid-edit: rewriting it would clobber whatever they
        // typed while the request was in flight.
        if catalog.search_edited_at.is_none() && catalog.search_text != search_text {
            catalog.search_text = search_text;
        }
        catalog.sort_index = usize::try_from(sort.index()).unwrap_or(0);
        catalog.density_index = usize::try_from(density.index()).unwrap_or(1);
        catalog.tile_target_width = density.card_width() as f32;
        catalog.sections = sections;
        // The scale factor decides how large the artwork has to be: a card
        // is `card_width * scale_factor` physical pixels.
        let artwork_width = density.artwork_width(state.ctx.pixels_per_point());
        catalog.items = Arc::new(
            items_data
                .iter()
                .map(|item| to_ui_item(item, artwork_width))
                .collect(),
        );
        tracing::debug!(
            ms = super::startup::elapsed_ms(),
            stage = "catalog_presented",
            "startup_stage"
        );
        super::startup::mark_content_ready("catalog");
    });
}

/// Pushes the persisted sort/density preferences into the window without
/// re-presenting the page.
pub fn present_preferences(context: &AppContext) {
    let (density, sort) = match context.state.lock() {
        Ok(state) => (state.catalog_density, state.catalog_sort),
        Err(_) => return,
    };
    context.ui.update(move |state| {
        state.catalog.density_index = usize::try_from(density.index()).unwrap_or(1);
        state.catalog.tile_target_width = density.card_width() as f32;
        state.catalog.sort_index = usize::try_from(sort.index()).unwrap_or(0);
    });
}

fn to_ui_item(item: &CatalogItemData, base_width: u32) -> CatalogItem {
    let artwork_width = item_artwork_width(&item.kind, base_width);
    let placeholder = item
        .title
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect::<String>();
    CatalogItem {
        id: item.id.clone(),
        title: item.title.clone(),
        subtitle: item.subtitle.clone(),
        kind_label: catalog_kind_label(&item.kind).to_owned(),
        kind: item.kind.clone(),
        placeholder,
        art: item.image_tag.as_ref().map(|tag| Art {
            key: catalog_image_key(&item.image_item_id, tag, artwork_width),
            source: ArtSource::Catalog {
                image_item_id: item.image_item_id.clone(),
                tag: tag.clone(),
                width: artwork_width,
            },
        }),
        watched: item.watched,
        progress: item.progress,
    }
}

/// Whether a row shows the server's 16:9 banner art rather than poster art.
///
/// Jellyfin gives collection folders and playlist folders a wide image with
/// the library name rendered into it -- 960x540 here. Fitting one into a 2:3
/// poster tile cropped away 62% of the width, name included, and upscaled what
/// was left 2.6x.
fn is_wide_tile(kind: &str) -> bool {
    matches!(
        kind,
        "CollectionFolder" | "UserView" | "ManualPlaylistsFolder"
    )
}

/// The artwork width for one row. Wide tiles are drawn half again as wide as
/// poster tiles, so they need correspondingly more pixels.
fn item_artwork_width(kind: &str, base_width: u32) -> u32 {
    if is_wide_tile(kind) {
        (base_width * 3 / 2).div_ceil(40) * 40
    } else {
        base_width
    }
}

fn catalog_kind_label(kind: &str) -> &str {
    match kind {
        "CollectionFolder" | "UserView" => "Library",
        "ManualPlaylistsFolder" => "Playlists",
        "BoxSet" => "Collection",
        other => other,
    }
}

const DENSITY_LABELS: [&str; 3] = ["Compact", "Normal", "Large"];
const SEARCH_DEBOUNCE_SECONDS: f64 = 0.3;
const GRID_PADDING: f32 = 24.0;
const GRID_GAP: f32 = 18.0;
const SHELF_GAP: f32 = 16.0;

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    let bar = Rect::from_min_size(screen.min, vec2(screen.width(), top_bar::HEIGHT));
    top_bar::draw(ui, state, bar, actions);

    let narrow = screen.width() < 980.0;
    let header = Rect::from_min_size(
        pos2(screen.left(), bar.bottom()),
        vec2(screen.width(), if narrow { 142.0 } else { 92.0 }),
    );
    draw_header(ui, state, header, narrow, actions);
    ui.painter().rect_filled(
        Rect::from_min_size(header.left_bottom(), vec2(screen.width(), 1.0)),
        0.0,
        theme::SURFACE_RAISED,
    );

    let banner = !state.message.is_empty() && !state.catalog.items.is_empty();
    let body = Rect::from_min_max(
        pos2(screen.left(), header.bottom() + 1.0),
        pos2(
            screen.right(),
            screen.bottom() - if banner { 38.0 } else { 0.0 },
        ),
    );
    ui.painter().rect_filled(body, 0.0, theme::BG);
    let mut body_ui = widgets::clipped_child(ui, body);
    if state.busy {
        centered_note(&body_ui, body, "Loading your library…", theme::TEXT_MUTED);
    } else if state.catalog.items.is_empty() {
        draw_empty(&mut body_ui, state, body, actions);
    } else if state.catalog.home {
        draw_home(&mut body_ui, state, actions);
    } else {
        draw_grid(&mut body_ui, state, actions);
    }

    if banner {
        let rect = Rect::from_min_max(pos2(screen.left(), body.bottom()), screen.max);
        ui.painter().rect_filled(rect, 0.0, theme::DANGER_SURFACE);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            &state.message,
            theme::regular(12.0),
            theme::DANGER_TEXT,
        );
    }
}

fn draw_header(
    ui: &mut Ui,
    state: &mut UiState,
    header: Rect,
    narrow: bool,
    actions: &mut Vec<UiAction>,
) {
    let painter = ui.painter().clone();
    painter.rect_filled(header, 0.0, theme::BG);
    let controls_y = header.top() + if narrow { 91.0 } else { 26.0 };
    let search = if narrow {
        Rect::from_min_size(
            pos2(header.left() + 24.0, controls_y),
            vec2((header.width() - 482.0).max(120.0), 32.0),
        )
    } else {
        Rect::from_min_size(pos2(header.right() - 696.0, controls_y), vec2(300.0, 32.0))
    };
    let title_width = if narrow {
        header.width() - 48.0
    } else {
        search.left() - header.left() - 48.0
    };
    let title = widgets::elided(
        ui,
        &state.page_title,
        theme::bold(27.0),
        theme::TEXT_PRIMARY,
        title_width,
    );
    widgets::galley_at(&painter, header.min + vec2(24.0, 14.0), title);
    painter.text(
        header.min + vec2(24.0, 52.0),
        Align2::LEFT_TOP,
        &state.catalog.summary,
        theme::regular(12.0),
        theme::TEXT_FAINT,
    );

    let busy = state.busy;
    let catalog = &mut state.catalog;
    let hint = if catalog.home {
        "Search all media"
    } else {
        "Search this library"
    };
    let response = ui.put(
        search,
        TextEdit::singleline(&mut catalog.search_text)
            .id(Id::new("catalog-search"))
            .hint_text(hint)
            .font(theme::regular(theme::FONT_SUBTITLE))
            .margin(vec2(10.0, 7.0))
            .interactive(!busy),
    );
    let now = ui.input(|input| input.time);
    if response.changed() {
        catalog.search_edited_at = Some(now);
    }
    if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
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

    let sort = Rect::from_min_size(
        pos2(
            header.right() - if narrow { 440.0 } else { 380.0 },
            controls_y,
        ),
        vec2(222.0, 32.0),
    );
    let mut sort_index = catalog.sort_index;
    let mut sort_ui = ui.new_child(egui::UiBuilder::new().max_rect(sort));
    sort_ui.add_enabled_ui(!busy, |ui| {
        ui.spacing_mut().interact_size.y = 32.0;
        egui::ComboBox::from_id_salt("catalog-sort")
            .width(222.0)
            .selected_text(CatalogSort::ALL[sort_index.min(CatalogSort::ALL.len() - 1)].label())
            .show_ui(ui, |ui| {
                for (index, option) in CatalogSort::ALL.into_iter().enumerate() {
                    ui.selectable_value(&mut sort_index, index, option.label());
                }
            });
    });
    if sort_index != catalog.sort_index {
        catalog.sort_index = sort_index;
        actions.push(UiAction::SortChanged(sort_index));
    }

    let density = Rect::from_min_size(pos2(header.right() - 204.0, controls_y), vec2(180.0, 32.0));
    painter.rect_filled(density, theme::RADIUS_SMALL, theme::SURFACE);
    widgets::border(&painter, density, theme::RADIUS_SMALL, 1.0, theme::BORDER);
    let inner = density.shrink(3.0);
    let segment_width = (inner.width() - 4.0) / 3.0;
    for (index, label) in DENSITY_LABELS.into_iter().enumerate() {
        let segment = Rect::from_min_size(
            pos2(
                inner.left() + index as f32 * (segment_width + 2.0),
                inner.top(),
            ),
            vec2(segment_width, inner.height()),
        );
        let response = ui.interact(segment, Id::new(("catalog-density", index)), Sense::click());
        widgets::describe_button(&response, true, label);
        let selected = catalog.density_index == index;
        let fill = if selected {
            theme::ACCENT_DEEP
        } else if response.hovered() {
            theme::SURFACE_HOVER
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(segment, theme::RADIUS_SMALL, fill);
        painter.text(
            segment.center(),
            Align2::CENTER_CENTER,
            label,
            if selected {
                theme::bold(10.0)
            } else {
                theme::regular(10.0)
            },
            if selected {
                theme::TEXT_BRIGHT
            } else {
                theme::TEXT_MUTED
            },
        );
        widgets::pointer_on_hover(&response);
        if response.clicked() && !selected {
            catalog.density_index = index;
            actions.push(UiAction::DensityChanged(index));
        }
    }
}

fn centered_note(ui: &Ui, rect: Rect, text: &str, color: Color32) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::regular(16.0),
        color,
    );
}

fn draw_empty(ui: &mut Ui, state: &UiState, body: Rect, actions: &mut Vec<UiAction>) {
    let (text, color) = if !state.message.is_empty() {
        (state.message.as_str(), theme::DANGER_TEXT)
    } else if state.catalog.search_text.is_empty() {
        ("Nothing to show", theme::TEXT_MUTED)
    } else {
        ("No matching media", theme::TEXT_MUTED)
    };
    let galley = widgets::wrapped(
        ui,
        text,
        theme::regular(16.0),
        color,
        body.width() - 48.0,
        Some(4),
    );
    let block = galley.size().y
        + if state.message.is_empty() {
            0.0
        } else {
            10.0 + 32.0
        };
    let top = body.center().y - block / 2.0;
    let text_rect = Rect::from_center_size(
        pos2(body.center().x, top + galley.size().y / 2.0),
        galley.size(),
    );
    ui.painter().galley(text_rect.min, galley, Color32::WHITE);
    if !state.message.is_empty() {
        let button = Rect::from_center_size(
            pos2(body.center().x, text_rect.bottom() + 26.0),
            vec2(96.0, 32.0),
        );
        let response = ui.put(button, egui::Button::new("Retry"));
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            actions.push(UiAction::Retry);
        }
    }
}

fn draw_home(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let card_width = state.catalog.tile_target_width;
    let items = state.catalog.items.clone();
    let sections = std::mem::take(&mut state.catalog.sections);
    let width = ui.max_rect().width();
    ScrollArea::vertical()
        .id_salt(("catalog-home", state.catalog.page_key.as_str()))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for section in &sections {
                let aspect = if section.wide { 0.5625 } else { 1.5 };
                let tile_width = if section.wide {
                    card_width * 1.5
                } else {
                    card_width
                };
                let height = tile_width * aspect + 112.0;
                let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let rows = items
                    .get(section.start_index..section.start_index + section.count)
                    .unwrap_or_default();
                draw_shelf(
                    ui,
                    &mut state.images,
                    section,
                    rows,
                    rect,
                    tile_width,
                    aspect,
                    actions,
                );
            }
        });
    state.catalog.sections = sections;
}

#[allow(clippy::too_many_arguments)]
fn draw_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    section: &CatalogSection,
    rows: &[CatalogItem],
    rect: Rect,
    tile_width: f32,
    aspect: f32,
    actions: &mut Vec<UiAction>,
) {
    ui.painter().text(
        rect.min + vec2(24.0, 7.0),
        Align2::LEFT_TOP,
        &section.title,
        theme::bold(20.0),
        theme::TEXT_PRIMARY,
    );
    let poster_height = tile_width * aspect;
    let viewport = Rect::from_min_size(
        rect.min + vec2(24.0, 43.0),
        vec2(rect.width() - 48.0, poster_height + 62.0),
    );
    let count = rows.len() as f32;
    let content_width = (count * (tile_width + SHELF_GAP) - SHELF_GAP).max(0.0);
    let id = Id::new(("catalog-shelf", section.id.as_str()));
    let mut shelf = Shelf::begin(ui, id, viewport, content_width);
    let offset = shelf.scroll(ui);
    {
        let mut strip = widgets::clipped_child(ui, viewport);
        for (index, item) in rows.iter().enumerate() {
            let left = index as f32 * (tile_width + SHELF_GAP);
            let tile = Rect::from_min_size(
                pos2(viewport.left() + offset + left, viewport.top()),
                vec2(tile_width, poster_height + 58.0),
            );
            // Drawn a screen ahead on either side so a page flip lands on
            // artwork that is already loading.
            if tile.right() < viewport.left() - viewport.width()
                || tile.left() > viewport.right() + viewport.width()
            {
                continue;
            }
            let response = media_tile(&mut strip, images, item, tile, aspect, id.with(index));
            if response.gained_focus() {
                shelf.reveal(left, tile_width);
            }
            if response.clicked() {
                actions.push(UiAction::OpenItem(item.id.clone()));
            }
        }
    }
    let arrow_y = viewport.top() + poster_height * 0.41;
    if shelf.can_page_back(offset) {
        let arrow = Rect::from_min_size(pos2(rect.left() + 4.0, arrow_y), vec2(38.0, 58.0));
        if widgets::shelf_arrow(
            ui,
            arrow,
            id.with("left"),
            true,
            theme::OVERLAY_DIM,
            theme::OVERLAY_RAISED,
            false,
        ) {
            shelf.page(false);
        }
    }
    if shelf.can_page_forward(offset) {
        let arrow = Rect::from_min_size(pos2(rect.right() - 42.0, arrow_y), vec2(38.0, 58.0));
        if widgets::shelf_arrow(
            ui,
            arrow,
            id.with("right"),
            false,
            theme::OVERLAY_DIM,
            theme::OVERLAY_RAISED,
            false,
        ) {
            shelf.page(true);
        }
    }
    shelf.end(ui);
}

fn draw_grid(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let width = ui.max_rect().width();
    let target = state.catalog.tile_target_width.max(1.0);
    let columns =
        (((width - 2.0 * GRID_PADDING + GRID_GAP) / (target + GRID_GAP)).floor() as usize).max(1);
    let card_width =
        (width - 2.0 * GRID_PADDING - (columns - 1) as f32 * GRID_GAP) / columns as f32;
    let row_height = card_width * 1.5 + 82.0;
    let items = state.catalog.items.clone();
    let rows = items.len().div_ceil(columns);
    ScrollArea::vertical()
        .id_salt(("catalog-grid", state.catalog.page_key.as_str()))
        .auto_shrink(false)
        .show_rows(ui, row_height, rows, |ui, visible| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            for row in visible {
                let (rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                for column in 0..columns {
                    let index = row * columns + column;
                    let Some(item) = items.get(index) else { break };
                    let tile = Rect::from_min_size(
                        pos2(
                            rect.left() + GRID_PADDING + column as f32 * (card_width + GRID_GAP),
                            rect.top(),
                        ),
                        vec2(card_width, card_width * 1.5 + 58.0),
                    );
                    let id = Id::new(("catalog-grid-tile", item.id.as_str()));
                    if media_tile(ui, &mut state.images, item, tile, 1.5, id).clicked() {
                        actions.push(UiAction::OpenItem(item.id.clone()));
                    }
                }
            }
        });
}

/// A poster card: artwork (or initials while it loads), kind badge, watched
/// badge, progress, and the title lines. Clicking the poster, or Enter/Space
/// while it has keyboard focus, opens the item.
fn media_tile(
    ui: &mut Ui,
    images: &mut ImageCache,
    item: &CatalogItem,
    tile: Rect,
    aspect: f32,
    id: Id,
) -> egui::Response {
    let poster = Rect::from_min_size(tile.min, vec2(tile.width(), tile.width() * aspect));
    let response = ui.interact(poster, id, Sense::click());
    widgets::describe_button(&response, true, &item.title);
    let hovered = response.hovered();
    let focused = response.has_focus();
    let painter = ui.painter().clone();
    let radius = theme::RADIUS_MEDIUM;
    if ui.is_rect_visible(poster) {
        if hovered {
            widgets::drop_shadow(
                &painter,
                poster,
                radius,
                15.0,
                5.0,
                theme::with_alpha(theme::SCRIM, 0.66),
            );
        } else {
            widgets::drop_shadow(
                &painter,
                poster,
                radius,
                8.0,
                3.0,
                theme::with_alpha(theme::SCRIM, 0.44),
            );
        }
        let texture = item
            .art
            .as_ref()
            .and_then(|art| images.texture(art).zip(images.size(&art.key)));
        match texture {
            Some((texture, size)) => widgets::image_cover(&painter, texture, size, poster, radius),
            None => {
                widgets::diagonal_gradient(
                    &painter,
                    poster,
                    radius,
                    theme::ACCENT_SURFACE,
                    theme::BG,
                );
                painter.text(
                    poster.center(),
                    Align2::CENTER_CENTER,
                    &item.placeholder,
                    theme::bold((poster.width() / 3.0).min(42.0)),
                    theme::BORDER_STRONG,
                );
            }
        }
        let badge = widgets::elided(
            ui,
            &item.kind_label,
            theme::bold(10.0),
            theme::TEXT_PRIMARY,
            poster.width() - 30.0,
        );
        let badge_rect = Rect::from_min_size(
            poster.min + vec2(8.0, 8.0),
            vec2(badge.size().x + 14.0, 24.0),
        );
        painter.rect_filled(
            badge_rect,
            theme::RADIUS_LARGE,
            theme::with_alpha(theme::BG, 0.86),
        );
        widgets::galley_centered(&painter, badge_rect, badge);
        if item.watched {
            let rect = Rect::from_min_size(
                pos2(poster.right() - 34.0, poster.top() + 8.0),
                vec2(26.0, 26.0),
            );
            widgets::watched_badge(&painter, rect, 17.0);
        }
        if item.progress > 0.0 && item.progress < 1.0 {
            // Inset by the border so the bar stays inside the rounded corners.
            widgets::card_progress(&painter, poster.shrink2(vec2(1.0, 1.0)), 6.0, item.progress);
        }
        let (width, color) = if focused {
            (3.0, theme::ACCENT_BRIGHT)
        } else if hovered {
            (2.0, theme::TEXT_FAINT)
        } else {
            (1.0, theme::SURFACE_HOVER)
        };
        widgets::border(&painter, poster, radius, width, color);
    }
    let title = widgets::elided(
        ui,
        &item.title,
        theme::bold(14.0),
        theme::TEXT_PRIMARY,
        tile.width(),
    );
    let title_rect = widgets::galley_at(&painter, pos2(tile.left(), poster.bottom() + 8.0), title);
    let subtitle = if item.subtitle.is_empty() {
        &item.kind
    } else {
        &item.subtitle
    };
    let subtitle = widgets::elided(
        ui,
        subtitle,
        theme::regular(12.0),
        theme::TEXT_FAINT,
        tile.width(),
    );
    widgets::galley_at(
        &painter,
        pos2(tile.left(), title_rect.bottom() + 13.0),
        subtitle,
    );
    widgets::pointer_on_hover(&response);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_tiles_ask_for_proportionally_wider_artwork() {
        assert_eq!(item_artwork_width("Movie", 320), 320);
        assert_eq!(item_artwork_width("CollectionFolder", 320), 480);
        assert_eq!(catalog_kind_label("UserView"), "Library");
        assert_eq!(catalog_kind_label("Series"), "Series");
    }
}
