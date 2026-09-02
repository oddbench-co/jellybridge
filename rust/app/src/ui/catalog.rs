//! Catalog screen: callback wiring, presentation, and artwork loading.

use std::sync::Arc;

use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use super::{
    CatalogItem, CatalogSection, JellyBridgeWindow, cached_image, catalog_image_key, patch_rows,
    store_cached_image,
};
use crate::api::CatalogItemData;
use crate::app::AppContext;
use crate::features;
use crate::features::catalog::CatalogSort;
use crate::services::artwork::{existing_cached_image, image_cache_path, write_cached_image};

pub fn connect(window: &JellyBridgeWindow, context: &AppContext) {
    let ctx = context.clone();
    window.on_open_item(move |id, _kind, _container, _playable| {
        features::catalog::open_item(&ctx, id.as_str());
    });

    let ctx = context.clone();
    window.on_search(move |term| features::catalog::start_search(&ctx, term.as_str().to_owned()));

    let ctx = context.clone();
    window.on_sort_changed(move |index| features::catalog::change_catalog_sort(&ctx, index));

    let ctx = context.clone();
    window.on_density_changed(move |index| features::catalog::change_catalog_density(&ctx, index));

    let ctx = context.clone();
    window.on_go_home(move || features::catalog::show_home(&ctx, true));

    let ctx = context.clone();
    window.on_go_back(move || features::catalog::go_back(&ctx));

    let ctx = context.clone();
    window.on_retry(move || features::catalog::retry(&ctx));
}

/// Presents the current catalog page (home dashboard, browse results, or
/// search results) and kicks off artwork loading for its items.
pub fn present(context: &AppContext, message: &str) {
    let (mut current, account, can_go_back, sort, density) = match context.state.lock() {
        Ok(state) => (
            state.current.clone(),
            state.account_name.clone(),
            !state.back_stack.is_empty(),
            state.catalog_sort,
            state.catalog_density,
        ),
        Err(_) => return,
    };
    let ui_sections: Vec<CatalogSection> = current
        .sections
        .iter()
        .map(|section| CatalogSection {
            id: section.id.as_str().into(),
            title: section.title.as_str().into(),
            start_index: i32::try_from(section.start_index).unwrap_or(i32::MAX),
            count: i32::try_from(section.count).unwrap_or(i32::MAX),
            // Taken from the rows themselves rather than the section id, so a
            // folder of libraries is shaped correctly too. A shelf holds one
            // kind of row, so the first one decides.
            wide: current
                .items
                .get(section.start_index)
                .is_some_and(|item| is_wide_tile(&item.kind)),
        })
        .collect();
    // `present` used to deep-clone this vector twice more on top of the
    // `current` clone above -- once for the UI mapping and once for the
    // artwork loader. One shared copy serves both.
    let items_data = Arc::new(std::mem::take(&mut current.items));
    let generation = context.next_generation();
    let artwork_ctx = context.clone();
    let artwork_items = items_data.clone();
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
            items_data.len()
        )
    } else if current.total_record_count > items_data.len() {
        format!(
            "Showing first {} of {}",
            items_data.len(),
            current.total_record_count
        )
    } else {
        let count = current.total_record_count;
        format!("{count} {}", if count == 1 { "item" } else { "items" })
    };
    let search_text = current.search_text.clone();
    let account_initial = account
        .trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_owned());
    let sort_options = CatalogSort::ALL
        .into_iter()
        .map(|sort| SharedString::from(sort.label()))
        .collect::<Vec<_>>();
    let message = message.to_owned();
    let ui_items = items_data.clone();
    context.ui.update(move |window| {
        window.set_screen(super::Screen::Catalog);
        window.set_busy(false);
        window.set_message(message.into());
        window.set_page_title(title.into());
        window.set_catalog_summary(summary.into());
        window.set_catalog_home(home_dashboard);
        // Only write the search box when the value actually changed:
        // rewriting it on every present resets the caret and clobbers
        // whatever the user typed while the request was in flight.
        if window.get_search_text().as_str() != search_text {
            window.set_search_text(search_text.into());
        }
        // Replacing a model tears down and rebuilds everything bound to it.
        // These options never change, so only install them once.
        if !patch_rows(&window.get_sort_options(), &sort_options, |a, b| a == b) {
            window.set_sort_options(ModelRc::new(VecModel::from(sort_options)));
        }
        window.set_sort_index(sort.index());
        window.set_density_index(density.index());
        window.set_tile_target_width(density.card_width() as f32);
        window.set_account_name(account.into());
        window.set_account_initial(account_initial.into());
        window.set_can_go_back(can_go_back);
        // `catalog-screen.slint` iterates `sections` to build every
        // CatalogShelf, and each shelf builds its MediaTiles. Replacing the
        // model destroyed and rebuilt all of them on every present, which
        // also threw away the `patch_rows` on `catalog_items` below.
        if !patch_rows(&window.get_sections(), &ui_sections, |a, b| a.id == b.id) {
            window.set_sections(ModelRc::new(VecModel::from(ui_sections)));
        }
        // The scale factor is only knowable here, with a real window, and it
        // decides how large the artwork has to be: a card is
        // `card_width * scale_factor` physical pixels.
        let artwork_width = density.artwork_width(window.window().scale_factor());
        let items = ui_items
            .iter()
            .map(|item| to_ui_item(item, artwork_width))
            .collect::<Vec<_>>();
        if !patch_rows(&window.get_catalog_items(), &items, |a, b| a.id == b.id) {
            window.set_catalog_items(ModelRc::new(VecModel::from(items)));
        }
        tracing::debug!(
            ms = super::startup::elapsed_ms(),
            stage = "catalog_presented",
            "startup_stage"
        );
        super::startup::mark_content_ready("catalog");
        // Deliberately here rather than after `ui.update` returns. The
        // resident-image check reads a thread-local cache that only the UI
        // thread ever writes, so running it on whichever thread called
        // `present` -- a tokio worker, for everything driven by the network --
        // read an empty map and re-fetched every row it should have skipped.
        load_artwork(&artwork_ctx, generation, &artwork_items, artwork_width);
    });
}

/// Pushes the persisted sort/density preferences into the window without
/// re-presenting the page.
pub fn present_preferences(context: &AppContext) {
    let (density, sort) = match context.state.lock() {
        Ok(state) => (state.catalog_density, state.catalog_sort),
        Err(_) => return,
    };
    context.ui.update(move |window| {
        window.set_density_index(density.index());
        window.set_tile_target_width(density.card_width() as f32);
        window.set_sort_index(sort.index());
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
        id: item.id.as_str().into(),
        title: item.title.as_str().into(),
        subtitle: item.subtitle.as_str().into(),
        kind_label: catalog_kind_label(&item.kind).into(),
        kind: item.kind.as_str().into(),
        placeholder: placeholder.into(),
        artwork: item
            .image_tag
            .as_deref()
            .and_then(|tag| {
                cached_image(&catalog_image_key(&item.image_item_id, tag, artwork_width))
            })
            .unwrap_or_default(),
        watched: item.watched,
        progress: item.progress,
        is_container: item.is_container,
        playable: item.playable,
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

fn load_artwork(
    context: &AppContext,
    generation: u64,
    items: &[CatalogItemData],
    base_width: u32,
) {
    let Some(api) = context.api() else { return };
    let mut spawned = 0_usize;
    let mut resident = 0_usize;
    for (index, item) in items.iter().enumerate() {
        let Some(tag) = item.image_tag.clone() else {
            continue;
        };
        let artwork_width = item_artwork_width(&item.kind, base_width);
        // `to_ui_item` already filled the row from this cache, so a hit means
        // the whole fetch-probe-decode round trip below would produce an image
        // that is already on screen. Without this, every re-present -- going
        // back, toggling watched, changing density -- re-ran it for every row.
        if cached_image(&catalog_image_key(&item.image_item_id, &tag, artwork_width)).is_some() {
            resident += 1;
            continue;
        }
        spawned += 1;
        let ctx = context.clone();
        let api = api.clone();
        // Two ids, rather than a clone of the whole row per spawned task.
        let image_item_id = item.image_item_id.clone();
        let item_id = item.id.clone();
        context.runtime.spawn(async move {
            let path = image_cache_path(
                &ctx.image_cache,
                api.session().server_url.as_str(),
                &image_item_id,
                &tag,
                artwork_width,
            );
            // `existing_cached_image` and `write_cached_image` are blocking
            // std::fs, and the write fsyncs; running them directly on a
            // runtime worker stalls every other task sharing that thread.
            let probe = path.clone();
            let cached = tokio::task::spawn_blocking(move || existing_cached_image(&probe))
                .await
                .ok()
                .flatten();
            let result = if let Some(path) = cached {
                Ok(path)
            } else {
                // The permit bounds concurrent downloads, so it is taken here
                // rather than at the top of the task: a poster already on disk
                // used to wait behind six in-flight fetches for a permit it
                // then did not use.
                let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
                    return;
                };
                match api.fetch_primary_image(&image_item_id, artwork_width).await {
                    Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => {
                        tokio::task::spawn_blocking(move || write_cached_image(&path, &bytes))
                            .await
                            .unwrap_or_else(|error| Err(error.to_string()))
                    }
                    Ok(_) => Err("artwork exceeded 20 MiB".to_owned()),
                    Err(error) => Err(error.to_string()),
                }
            };
            if let Ok(path) = result {
                let key = catalog_image_key(&image_item_id, &tag, artwork_width);
                let apply_ctx = ctx.clone();
                ctx.ui.update(move |window| {
                    // Decoded here, on the event loop, deliberately. Decoding on
                    // the blocking pool and shipping a `SharedPixelBuffer` does
                    // take 8.6% off the UI thread, but it also costs 17 MiB of
                    // settled RSS because Slint's own loader stores the pixels
                    // more compactly than `Image::from_rgb8` does. Measured both
                    // ways; see docs/rust-port/performance-protocol.md.
                    let Ok(image) = Image::load_from_path(&path) else {
                        return;
                    };
                    // Cached before the generation check on purpose: a fetched
                    // image stays valid however the user navigated meanwhile.
                    store_cached_image(key, image.clone());
                    if !apply_ctx.is_current(generation) {
                        return;
                    }
                    let model = window.get_catalog_items();
                    if let Some(mut row) = model.row_data(index)
                        && row.id.as_str() == item_id
                    {
                        row.artwork = image;
                        model.set_row_data(index, row);
                    }
                });
            }
        });
    }
    tracing::debug!(
        spawned,
        resident,
        thread = std::thread::current().name().unwrap_or("?"),
        "catalog artwork pass"
    );
}
