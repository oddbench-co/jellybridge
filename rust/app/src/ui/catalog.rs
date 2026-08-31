//! Catalog screen: callback wiring, presentation, and artwork loading.

use slint::{Image, Model, ModelRc, SharedString, VecModel};

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
    let ui_sections: Vec<CatalogSection> = current
        .sections
        .iter()
        .map(|section| CatalogSection {
            id: section.id.as_str().into(),
            title: section.title.as_str().into(),
            start_index: i32::try_from(section.start_index).unwrap_or(i32::MAX),
            count: i32::try_from(section.count).unwrap_or(i32::MAX),
        })
        .collect();
    let ui_item_data = current.items.clone();
    let artwork_items = current.items.clone();
    let generation = context.next_generation();
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
        window.set_sort_options(ModelRc::new(VecModel::from(sort_options)));
        window.set_sort_index(sort.index());
        window.set_density_index(density.index());
        window.set_tile_target_width(density.card_width() as f32);
        window.set_account_name(account.into());
        window.set_account_initial(account_initial.into());
        window.set_can_go_back(can_go_back);
        window.set_sections(ModelRc::new(VecModel::from(ui_sections)));
        let items = ui_item_data.into_iter().map(to_ui_item).collect::<Vec<_>>();
        if !patch_rows(&window.get_catalog_items(), &items, |a, b| a.id == b.id) {
            window.set_catalog_items(ModelRc::new(VecModel::from(items)));
        }
    });
    load_artwork(context, generation, artwork_items);
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

fn to_ui_item(item: CatalogItemData) -> CatalogItem {
    let placeholder = item
        .title
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect::<String>();
    CatalogItem {
        id: item.id.into(),
        title: item.title.into(),
        subtitle: item.subtitle.into(),
        kind_label: catalog_kind_label(&item.kind).into(),
        kind: item.kind.into(),
        placeholder: placeholder.into(),
        artwork: item
            .image_tag
            .as_deref()
            .and_then(|tag| cached_image(&catalog_image_key(&item.image_item_id, tag)))
            .unwrap_or_default(),
        watched: item.watched,
        progress: item.progress,
        is_container: item.is_container,
        playable: item.playable,
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

fn load_artwork(context: &AppContext, generation: u64, items: Vec<CatalogItemData>) {
    let Some(api) = context.api() else { return };
    for (index, item) in items.into_iter().enumerate() {
        let Some(tag) = item.image_tag.clone() else {
            continue;
        };
        let ctx = context.clone();
        let api = api.clone();
        context.runtime.spawn(async move {
            let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
                return;
            };
            let path = image_cache_path(
                &ctx.image_cache,
                api.session().server_url.as_str(),
                &item.image_item_id,
                &tag,
            );
            let result = if let Some(path) = existing_cached_image(&path) {
                Ok(path)
            } else {
                match api.fetch_primary_image(&item.image_item_id).await {
                    Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => {
                        write_cached_image(&path, &bytes)
                    }
                    Ok(_) => Err("artwork exceeded 20 MiB".to_owned()),
                    Err(error) => Err(error.to_string()),
                }
            };
            if let Ok(path) = result {
                let item_id = item.id;
                let key = catalog_image_key(&item.image_item_id, &tag);
                let apply_ctx = ctx.clone();
                ctx.ui.update(move |window| {
                    let Ok(image) = Image::load_from_path(&path) else {
                        return;
                    };
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
}
