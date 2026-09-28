//! Detail screen: callback wiring, presentation, formatters, and artwork.

use slint::{Image, Model, ModelRc, SharedString, VecModel};

use super::{
    DetailCard, DetailPerson, JellyBridgeWindow, cached_image, format_playback_time, image_key,
    patch_rows, primary_card_image, store_cached_image,
};
use crate::app::AppContext;
use crate::features;
use crate::features::detail::{
    DetailItemData, DetailKind, DetailMessage, DetailPageData, DetailTarget, ImageRef, ImageType,
    StreamData,
};
use crate::features::playback::clean_preference;
use crate::services::artwork::{existing_cached_image, image_cache_path, write_cached_image};

pub fn connect(window: &JellyBridgeWindow, context: &AppContext) {
    let ctx = context.clone();
    window.on_open_detail(move |id, kind| {
        features::detail::start_detail(
            &ctx,
            DetailTarget::new(id.as_str(), DetailKind::from_jellyfin(kind.as_str())),
        );
    });

    let ctx = context.clone();
    window.on_play_current(move || features::playback::start_playback(&ctx, false));

    let ctx = context.clone();
    window.on_play_current_from_start(move || features::playback::start_playback(&ctx, true));

    window.on_open_external_url(move |url| open_in_browser(url.as_str()));

    let ctx = context.clone();
    window.on_play_detail(move |id| {
        let item = ctx
            .state
            .lock()
            .ok()
            .and_then(|state| state.detail.item_by_id(id.as_str()));
        if let Some(item) = item {
            features::playback::start_playback_item(&ctx, item, false);
        }
    });

    let ctx = context.clone();
    window.on_set_detail_watched(move |id, watched| {
        features::detail::update_detail_mutation(
            &ctx,
            DetailMessage::SetWatched {
                id: id.as_str().to_owned(),
                watched,
            },
        );
    });

    let ctx = context.clone();
    window.on_set_detail_favorite(move |id, favorite| {
        features::detail::update_detail_mutation(
            &ctx,
            DetailMessage::SetFavorite {
                id: id.as_str().to_owned(),
                favorite,
            },
        );
    });

    let ctx = context.clone();
    window.on_detail_audio_selected(move |index| {
        features::detail::select_detail_track(&ctx, index, true);
    });

    let ctx = context.clone();
    window.on_detail_subtitle_selected(move |index| {
        features::detail::select_detail_track(&ctx, index, false);
    });
}

/// Blanks every property the detail page renders.
///
/// This must cover *everything* on the page. Anything left behind is shown
/// against the next item, which is what made opening a second movie flash the
/// first one's poster, tagline and cast for a few hundred milliseconds.
pub(super) fn clear_detail_page(window: &JellyBridgeWindow) {
    window.set_detail_id("".into());
    window.set_detail_title("".into());
    window.set_detail_heading("".into());
    window.set_detail_subheading("".into());
    window.set_detail_ends_at("".into());
    window.set_detail_tags("".into());
    window.set_detail_links(ModelRc::new(VecModel::default()));
    window.set_detail_director("".into());
    window.set_detail_writers("".into());
    window.set_detail_star_rating(0.0);
    window.set_detail_critic_rating(-1);
    window.set_detail_series_id("".into());
    window.set_detail_series_name("".into());
    window.set_detail_season_id("".into());
    window.set_detail_season_name("".into());
    window.set_detail_series_summary("".into());
    window.set_detail_play_context("".into());
    window.set_detail_season_summary("".into());
    window.set_detail_play_label("".into());
    window.set_detail_progress(0.0);
    window.set_detail_progress_label("".into());
    window.set_detail_seasons(ModelRc::new(VecModel::from(Vec::new())));
    window.set_detail_episodes(ModelRc::new(VecModel::from(Vec::new())));
    window.set_detail_next_up(ModelRc::new(VecModel::<DetailCard>::default()));
    window.set_detail_similar(ModelRc::new(VecModel::<DetailCard>::default()));
    window.set_detail_people(ModelRc::new(VecModel::from(Vec::new())));
    window.set_detail_guest_stars(ModelRc::new(VecModel::from(Vec::new())));
    window.set_detail_current_episode_index(-1);
    window.set_detail_unplayed_count(0);
    // The rest were missing from the old reset, and they are the loudest ones
    // on screen: the hero images, the synopsis and the metadata lines.
    window.set_detail_artwork(Image::default());
    window.set_detail_backdrop(Image::default());
    window.set_detail_logo(Image::default());
    window.set_detail_overview("".into());
    window.set_detail_tagline("".into());
    window.set_detail_genres("".into());
    window.set_detail_studios("".into());
    window.set_detail_media_info("".into());
    window.set_detail_subtitle("".into());
    window.set_detail_playable(false);
    window.set_detail_watched(false);
    window.set_detail_favorite(false);
    window.set_detail_audio_options(ModelRc::new(VecModel::<SharedString>::default()));
    window.set_detail_subtitle_options(ModelRc::new(VecModel::<SharedString>::default()));
    window.set_detail_audio_index(-1);
    window.set_detail_subtitle_index(-1);
}

/// Switches to an empty detail page while the real one loads.
///
/// `DetailMessage::Open` already drops the outgoing page from the state
/// machine, but nothing told the window, so it kept rendering the previous
/// item until the fetch landed.
pub fn present_loading(context: &AppContext) {
    let can_go_back = context
        .state
        .lock()
        .map(|state| state.detail.snapshot().can_go_back)
        .unwrap_or(false);
    context.ui.update(move |window| {
        window.set_screen(super::Screen::Detail);
        window.set_busy(true);
        window.set_message("".into());
        window.set_can_go_back(can_go_back);
        clear_detail_page(window);
    });
}

/// Presents the current detail page and kicks off its artwork loading.
pub fn present(context: &AppContext, generation: u64) {
    present_page(context, generation, false);
}

/// Re-presents the page after a background refresh. Leaves the screen, busy
/// state, and message alone, so a refresh never pulls the user off the
/// player or clears a status line.
pub fn present_refresh(context: &AppContext, generation: u64) {
    present_page(context, generation, true);
}

fn present_page(context: &AppContext, generation: u64, background: bool) {
    let (page, error, can_go_back, tracks) = match context.state.lock() {
        Ok(state) => {
            let snapshot = state.detail.snapshot();
            (
                snapshot.page.cloned(),
                snapshot.error.unwrap_or_default().to_owned(),
                snapshot.can_go_back,
                state.detail_tracks.clone(),
            )
        }
        Err(_) => return,
    };
    let Some(page) = page else {
        if background {
            return;
        }
        context.ui.update(move |window| {
            window.set_screen(super::Screen::Detail);
            window.set_busy(false);
            window.set_message(error.into());
            window.set_can_go_back(can_go_back);
            clear_detail_page(window);
        });
        return;
    };
    let detail_id = page.item.id.clone();
    let title = page.item.title.clone();
    let subtitle = detail_info_line(&page);
    let unplayed_count = matches!(page.item.kind, DetailKind::Series | DetailKind::Season)
        .then_some(page.item.unplayed_count)
        .flatten()
        .unwrap_or(0);
    let overview = page.item.overview.clone();
    let kind = to_ui_kind(page.item.kind);
    let tagline = page.item.tagline.clone();
    let genres = page.item.genres.join(" • ");
    let studios = page.item.studios.join(" • ");
    let media_info = page.item.media_summary.clone();
    let series_id = page.item.series_id.clone().unwrap_or_default();
    let series_name = page.item.series_name.clone().unwrap_or_default();
    let season_id = (page.item.kind == DetailKind::Episode)
        .then(|| page.item.season_id.clone())
        .flatten()
        .unwrap_or_default();
    let season_name = (page.item.kind == DetailKind::Episode)
        .then(|| page.item.season_name.clone())
        .flatten()
        .unwrap_or_default();
    let playable = page.item.playable || !page.episodes.is_empty();
    let watched = page.item.watched;
    let favorite = page.item.favorite;
    let series_summary = detail_series_summary(&page);
    let season_summary = detail_season_summary(&page);
    let play_label = page.primary_play_label().unwrap_or_default().to_owned();
    let play_context = detail_series_play_context(&page);
    let progress_item = if page.item.kind == DetailKind::Series {
        page.play_target().unwrap_or(&page.item)
    } else {
        &page.item
    };
    let progress = progress_item.progress;
    let progress_label = detail_progress_label(progress_item);
    let message = page.children_error.clone().unwrap_or(error);
    let seasons = page.seasons.clone();
    let episodes = page.episodes.clone();
    let next_up = (page.item.kind == DetailKind::Series)
        .then(|| page.play_target().cloned())
        .flatten();
    let people = page.item.people.clone();
    let similar = page.similar.clone();
    let guest_stars = page.item.guest_stars.clone();
    let current_episode_index = current_episode_index(&page);
    let audio_options = page
        .item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .enumerate()
        .map(|(index, stream)| detail_stream_label(stream, "Audio", index))
        .collect::<Vec<_>>();
    let subtitle_options = std::iter::once("Off".to_owned())
        .chain(
            page.item
                .streams
                .iter()
                .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
                .enumerate()
                .map(|(index, stream)| detail_stream_label(stream, "Subtitle", index)),
        )
        .collect::<Vec<_>>();
    let audio_index = tracks.audio_index;
    let subtitle_index = tracks.subtitle_index;
    let (heading, subheading) = detail_headings(&page.item);
    let ends_at = ends_at_label(&page.item);
    let tags_line = page.item.tags.join(", ");
    let links = page.item.external_urls.clone();
    let director = page.item.directors.join(", ");
    let writers_line = page.item.writers.join(", ");
    let star_rating = page.item.community_rating.unwrap_or(0.0);
    let critic_rating = page
        .item
        .critic_rating
        .map(|value| value.round() as i32)
        .unwrap_or(-1);
    let hero_poster_key = primary_card_image(&page.item).map(image_key);
    let hero_backdrop_key = page.item.backdrop.as_ref().map(image_key);
    let hero_logo_key = page.item.logo.as_ref().map(image_key);
    context.ui.update(move |window| {
        let hero_image = |key: Option<String>| {
            key.as_deref()
                .and_then(cached_image)
                .unwrap_or_default()
        };
        let seasons = seasons.iter().map(to_detail_card).collect::<Vec<_>>();
        let episodes = episodes.iter().map(to_detail_card).collect::<Vec<_>>();
        let people = people.iter().map(to_detail_person).collect::<Vec<_>>();
        let guest_stars = guest_stars.iter().map(to_detail_person).collect::<Vec<_>>();
        if !background {
            window.set_screen(super::Screen::Detail);
            window.set_busy(false);
            window.set_message(message.into());
        }
        window.set_detail_id(detail_id.into());
        window.set_detail_title(title.into());
        window.set_detail_heading(heading.into());
        window.set_detail_subheading(subheading.into());
        window.set_detail_ends_at(ends_at.into());
        window.set_detail_tags(tags_line.into());
        let links = links
            .into_iter()
            .map(|(name, url)| super::ExternalLink {
                name: name.into(),
                url: url.into(),
            })
            .collect::<Vec<_>>();
        if !patch_rows(&window.get_detail_links(), &links, |a, b| a.name == b.name) {
            window.set_detail_links(ModelRc::new(VecModel::from(links)));
        }
        window.set_detail_director(director.into());
        window.set_detail_writers(writers_line.into());
        window.set_detail_star_rating(star_rating);
        window.set_detail_critic_rating(critic_rating);
        window.set_detail_subtitle(subtitle.into());
        window.set_detail_overview(overview.into());
        window.set_detail_kind(kind);
        window.set_detail_tagline(tagline.into());
        window.set_detail_genres(genres.into());
        window.set_detail_studios(studios.into());
        window.set_detail_media_info(media_info.into());
        window.set_detail_series_id(series_id.into());
        window.set_detail_series_name(series_name.into());
        window.set_detail_season_id(season_id.into());
        window.set_detail_season_name(season_name.into());
        window.set_detail_series_summary(series_summary.into());
        window.set_detail_play_context(play_context.into());
        window.set_detail_season_summary(season_summary.into());
        window.set_detail_play_label(play_label.into());
        window.set_detail_progress(progress);
        window.set_detail_progress_label(progress_label.into());
        window.set_detail_playable(playable);
        window.set_detail_watched(watched);
        window.set_detail_favorite(favorite);
        window.set_detail_artwork(hero_image(hero_poster_key));
        window.set_detail_backdrop(hero_image(hero_backdrop_key));
        window.set_detail_logo(hero_image(hero_logo_key));
        if !patch_rows(&window.get_detail_seasons(), &seasons, |a, b| a.id == b.id) {
            window.set_detail_seasons(ModelRc::new(VecModel::from(seasons)));
        }
        if !patch_rows(&window.get_detail_episodes(), &episodes, |a, b| a.id == b.id) {
            window.set_detail_episodes(ModelRc::new(VecModel::from(episodes)));
        }
        let next_up = next_up.iter().map(to_next_up_card).collect::<Vec<_>>();
        if !patch_rows(&window.get_detail_next_up(), &next_up, |a, b| a.id == b.id) {
            window.set_detail_next_up(ModelRc::new(VecModel::from(next_up)));
        }
        let similar = similar.iter().map(to_detail_card).collect::<Vec<_>>();
        if !patch_rows(&window.get_detail_similar(), &similar, |a, b| a.id == b.id) {
            window.set_detail_similar(ModelRc::new(VecModel::from(similar)));
        }
        if !patch_rows(&window.get_detail_people(), &people, |a, b| {
            a.id == b.id && a.name == b.name
        }) {
            window.set_detail_people(ModelRc::new(VecModel::from(people)));
        }
        if !patch_rows(&window.get_detail_guest_stars(), &guest_stars, |a, b| {
            a.id == b.id && a.name == b.name
        }) {
            window.set_detail_guest_stars(ModelRc::new(VecModel::from(guest_stars)));
        }
        window.set_detail_current_episode_index(current_episode_index);
        window.set_detail_unplayed_count(unplayed_count);
        // These back ComboBoxes. Replacing the model rebuilds the popup and
        // resets what it is showing, so only install a genuinely new list --
        // `update_detail_mutation` re-presents the whole page twice for one
        // watched toggle, and the track lists are identical both times.
        let audio_options = audio_options
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>();
        if !patch_rows(&window.get_detail_audio_options(), &audio_options, |a, b| {
            a == b
        }) {
            window.set_detail_audio_options(ModelRc::new(VecModel::from(audio_options)));
        }
        let subtitle_options = subtitle_options
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>();
        if !patch_rows(
            &window.get_detail_subtitle_options(),
            &subtitle_options,
            |a, b| a == b,
        ) {
            window.set_detail_subtitle_options(ModelRc::new(VecModel::from(subtitle_options)));
        }
        window.set_detail_audio_index(audio_index);
        window.set_detail_subtitle_index(subtitle_index);
        window.set_can_go_back(can_go_back);
    });
    load_detail_artwork(context, generation, &page);
}

fn to_detail_card(item: &DetailItemData) -> DetailCard {
    let index_label = (item.kind == DetailKind::Episode)
        .then_some(item.index_number)
        .flatten()
        .map(|number| format!("{number}."))
        .unwrap_or_default();
    DetailCard {
        id: item.id.as_str().into(),
        title: item.title.as_str().into(),
        subtitle: detail_card_subtitle(item).into(),
        overview: item.overview.as_str().into(),
        kind: detail_kind_name(item.kind).into(),
        artwork: primary_card_image(item)
            .and_then(|image| cached_image(&image_key(image)))
            .unwrap_or_default(),
        watched: item.watched,
        favorite: item.favorite,
        progress: item.progress,
        unplayed_count: item.unplayed_count.unwrap_or(0),
        rating: item.community_rating.unwrap_or(0.0),
        ends_at: ends_at_label(item).into(),
        index_label: index_label.into(),
    }
}

/// The Next Up card names its season too, as Jellyfin's does:
/// "S2:E3 - Title".
fn to_next_up_card(item: &DetailItemData) -> DetailCard {
    let mut card = to_detail_card(item);
    card.index_label = next_up_label(item).into();
    card
}

fn next_up_label(item: &DetailItemData) -> String {
    match (item.parent_index_number, item.index_number) {
        (Some(season), Some(episode)) => format!("S{season}:E{episode} -"),
        (None, Some(episode)) => format!("E{episode} -"),
        _ => String::new(),
    }
}

fn detail_card_subtitle(item: &DetailItemData) -> String {
    let runtime = item.runtime_seconds.map(format_runtime);
    // Episode rows carry the episode number in their title and rating /
    // ends-at as dedicated fields; their subtitle is when it aired and how
    // long it runs, as Jellyfin's episode list shows.
    if item.kind == DetailKind::Episode {
        return [item.premiere_date.map(format_air_date), runtime]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" • ");
    }
    let unplayed = item
        .unplayed_count
        .filter(|count| *count > 0)
        .map(|count| format!("{count} unwatched"));
    [
        item.production_year.map(|year| year.to_string()),
        runtime,
        unplayed,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" • ")
}

fn detail_episode_number(item: &DetailItemData) -> Option<String> {
    match (item.parent_index_number, item.index_number) {
        (Some(season), Some(episode)) if item.kind == DetailKind::Episode => {
            Some(format!("S{season:02}E{episode:02}"))
        }
        (_, Some(episode)) if item.kind == DetailKind::Episode => {
            Some(format!("Episode {episode}"))
        }
        _ => None,
    }
}

fn to_detail_person(person: &crate::features::detail::PersonData) -> DetailPerson {
    DetailPerson {
        id: person.id.as_deref().unwrap_or_default().into(),
        name: person.name.as_str().into(),
        role: person.role.as_str().into(),
        artwork: person
            .image
            .as_ref()
            .and_then(|image| cached_image(&image_key(image)))
            .unwrap_or_default(),
    }
}

/// Where an episode page's own episode sits in its season list, or -1.
fn current_episode_index(page: &DetailPageData) -> i32 {
    if page.item.kind != DetailKind::Episode {
        return -1;
    }
    page.episodes
        .iter()
        .position(|episode| episode.id == page.item.id)
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(-1)
}

fn detail_stream_label(stream: &StreamData, fallback: &str, index: usize) -> String {
    if !stream.label.is_empty() {
        stream.label.clone()
    } else if let Some(codec) = clean_preference(&stream.codec) {
        codec.to_uppercase()
    } else {
        format!("{fallback} {}", index + 1)
    }
}

/// Opens an external metadata link (IMDb, TMDB, ...) in the default browser.
fn open_in_browser(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "macos")]
    let launcher = "open";
    #[cfg(not(target_os = "macos"))]
    let launcher = "xdg-open";
    if let Err(error) = std::process::Command::new(launcher).arg(url).spawn() {
        tracing::warn!(%error, url, "external link could not be opened");
    }
}

/// "Ends at 4:47 PM" for the remaining runtime, in local time. Empty when
/// the item has no known runtime or is not playable.
fn ends_at_label(item: &DetailItemData) -> String {
    let Some(runtime) = item.runtime_seconds.filter(|seconds| *seconds > 0.0) else {
        return String::new();
    };
    if !item.playable {
        return String::new();
    }
    let resume = if item.resume_seconds > 0.0 && item.resume_seconds < runtime && !item.watched {
        item.resume_seconds
    } else {
        0.0
    };
    super::ends_at_from_now(runtime - resume)
}

/// Title strip heading/subheading: episodes and seasons lead with the series
/// name and describe themselves on the second line, like Jellyfin.
fn detail_headings(item: &DetailItemData) -> (String, String) {
    match item.kind {
        DetailKind::Episode => {
            let heading = item
                .series_name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| item.title.clone());
            let episode_part = match item.index_number {
                Some(number) => format!("{number}. {}", item.title),
                None => item.title.clone(),
            };
            let subheading = match item.parent_index_number {
                Some(season) => format!("Season {season} - {episode_part}"),
                None => episode_part,
            };
            (heading, subheading)
        }
        DetailKind::Season => {
            let heading = item
                .series_name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| item.title.clone());
            let subheading = if heading == item.title {
                String::new()
            } else {
                item.title.clone()
            };
            (heading, subheading)
        }
        _ => (item.title.clone(), String::new()),
    }
}

fn to_ui_kind(kind: DetailKind) -> super::DetailKind {
    match kind {
        DetailKind::Movie => super::DetailKind::Movie,
        DetailKind::Series => super::DetailKind::Series,
        DetailKind::Season => super::DetailKind::Season,
        DetailKind::Episode => super::DetailKind::Episode,
        DetailKind::Other => super::DetailKind::Other,
    }
}

fn detail_kind_name(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Movie => "Movie",
        DetailKind::Series => "Series",
        DetailKind::Season => "Season",
        DetailKind::Episode => "Episode",
        DetailKind::Other => "Media",
    }
}

/// "49m" or "1h 5m", as Jellyfin writes runtimes.
fn format_runtime(seconds: f64) -> String {
    let minutes = (seconds / 60.0).round() as u64;
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// An air date the way Jellyfin's en-US locale prints it: "1/31/2025".
fn format_air_date((year, month, day): (i32, u32, u32)) -> String {
    format!("{month}/{day}/{year}")
}

/// The title strip's info row. A season counts its episodes, as Jellyfin's
/// does; every other kind reads its own metadata.
fn detail_info_line(page: &DetailPageData) -> String {
    let item = &page.item;
    if item.kind != DetailKind::Season {
        return detail_metadata_line(item);
    }
    let episodes = match page.episodes.len() {
        0 => None,
        1 => Some("1 episode".to_owned()),
        count => Some(format!("{count} episodes")),
    };
    [
        item.production_year.map(|year| year.to_string()),
        episodes,
        item.official_rating.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" • ")
}

fn series_air_span(item: &DetailItemData, start: i32) -> String {
    if item.series_continuing {
        format!("{start} – Present")
    } else {
        match item.end_year {
            Some(end) if end != start => format!("{start} – {end}"),
            _ => start.to_string(),
        }
    }
}

fn detail_metadata_line(item: &DetailItemData) -> String {
    let runtime = item.runtime_seconds.map(format_runtime);
    // Episodes read like Jellyfin's: the air date leads, and there is no
    // S01E03 label because the subheading already carries the number.
    if item.kind == DetailKind::Episode {
        let aired = item
            .premiere_date
            .map(format_air_date)
            .or_else(|| item.production_year.map(|year| year.to_string()));
        return [aired, item.official_rating.clone(), runtime]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" • ");
    }
    // A series reads its air span, "2019 – Present" or "2019 – 2022".
    if item.kind == DetailKind::Series {
        return [
            item.production_year.map(|year| series_air_span(item, year)),
            item.official_rating.clone(),
            runtime,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" • ");
    }
    let leading_label = match item.kind {
        DetailKind::Movie => None,
        _ => Some(detail_kind_name(item.kind).to_owned()),
    };
    // Community/critic ratings render as dedicated colored elements in the
    // title strip, not as part of this line.
    [
        leading_label,
        item.production_year.map(|year| year.to_string()),
        item.official_rating.clone(),
        runtime,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" • ")
}

fn detail_progress_label(item: &DetailItemData) -> String {
    if item.watched || item.resume_seconds <= 0.0 {
        String::new()
    } else {
        format!("Resume at {}", format_playback_time(item.resume_seconds))
    }
}

fn detail_series_summary(page: &DetailPageData) -> String {
    if page.item.kind != DetailKind::Series {
        return String::new();
    }
    if page.seasons.is_empty() && page.episodes.is_empty() {
        return if page.children_error.is_some() {
            "Series contents unavailable".to_owned()
        } else {
            "No seasons available".to_owned()
        };
    }
    let mut parts = Vec::new();
    if !page.seasons.is_empty() {
        parts.push(if page.seasons.len() == 1 {
            "1 season".to_owned()
        } else {
            format!("{} seasons", page.seasons.len())
        });
    }
    if !page.episodes.is_empty() {
        parts.push(if page.episodes.len() == 1 {
            "1 episode".to_owned()
        } else {
            format!("{} episodes", page.episodes.len())
        });
        let unwatched = page
            .episodes
            .iter()
            .filter(|episode| !episode.watched)
            .count();
        parts.push(if unwatched == 0 {
            "All watched".to_owned()
        } else {
            format!("{unwatched} unwatched")
        });
    }
    parts.join(" • ")
}

fn detail_series_play_context(page: &DetailPageData) -> String {
    if page.item.kind != DetailKind::Series {
        return String::new();
    }
    let Some(target) = page.play_target() else {
        return String::new();
    };
    let episode = detail_episode_number(target).unwrap_or_else(|| target.title.clone());
    if target.resume_seconds > 0.0 && !target.watched {
        format!(
            "{episode} • Resume at {}",
            format_playback_time(target.resume_seconds)
        )
    } else if !target.watched {
        format!("{episode} • Up next")
    } else {
        format!("{episode} • Start again")
    }
}

fn detail_season_summary(page: &DetailPageData) -> String {
    if page.item.kind != DetailKind::Season {
        return String::new();
    }
    let episode_count = page.episodes.len();
    if episode_count == 0 {
        return if page.children_error.is_some() {
            "Episodes unavailable".to_owned()
        } else {
            "No episodes available".to_owned()
        };
    }
    let unwatched_count = page
        .episodes
        .iter()
        .filter(|episode| !episode.watched)
        .count();
    let episode_label = if episode_count == 1 {
        "1 episode".to_owned()
    } else {
        format!("{episode_count} episodes")
    };
    if unwatched_count == 0 {
        format!("{episode_label} • All watched")
    } else {
        format!("{episode_label} • {unwatched_count} unwatched")
    }
}

#[derive(Clone)]
enum DetailArtworkTarget {
    HeroPoster(String),
    HeroBackdrop(String),
    HeroLogo(String),
    Season { index: usize, id: String },
    Episode { index: usize, id: String },
    NextUp { id: String },
    Similar { index: usize, id: String },
    /// A cast photo; `guest` picks the Guest Stars shelf over Cast & Crew.
    Person {
        index: usize,
        id: String,
        name: String,
        guest: bool,
    },
}

fn load_detail_artwork(context: &AppContext, generation: u64, page: &DetailPageData) {
    let item_id = page.item.id.clone();
    let mut requests = Vec::new();
    if let Some(image) = primary_card_image(&page.item).cloned() {
        requests.push((DetailArtworkTarget::HeroPoster(item_id.clone()), image));
    }
    if let Some(image) = page.item.backdrop.clone() {
        requests.push((DetailArtworkTarget::HeroBackdrop(item_id.clone()), image));
    }
    if let Some(image) = page.item.logo.clone() {
        requests.push((DetailArtworkTarget::HeroLogo(item_id), image));
    }
    requests.extend(page.seasons.iter().enumerate().filter_map(|(index, item)| {
        primary_card_image(item).cloned().map(|image| {
            (
                DetailArtworkTarget::Season {
                    index,
                    id: item.id.clone(),
                },
                image,
            )
        })
    }));
    if page.item.kind == DetailKind::Series {
        // `detail-screen.slint` only renders `detail-episodes` for season and
        // episode pages. A series page shows the Next Up card instead, whose
        // artwork comes from this same cache -- so fetching every episode's
        // still was up to 47 wasted round trips and decodes on a long show.
        if let Some(target) = page.play_target()
            && let Some(image) = primary_card_image(target).cloned()
        {
            requests.push((
                DetailArtworkTarget::NextUp {
                    id: target.id.clone(),
                },
                image,
            ));
        }
    } else {
        requests.extend(
            page.episodes
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    primary_card_image(item).cloned().map(|image| {
                        (
                            DetailArtworkTarget::Episode {
                                index,
                                id: item.id.clone(),
                            },
                            image,
                        )
                    })
                }),
        );
    }
    requests.extend(page.similar.iter().enumerate().filter_map(|(index, item)| {
        primary_card_image(item).cloned().map(|image| {
            (
                DetailArtworkTarget::Similar {
                    index,
                    id: item.id.clone(),
                },
                image,
            )
        })
    }));
    for (guest, people) in [(false, &page.item.people), (true, &page.item.guest_stars)] {
        requests.extend(people.iter().enumerate().filter_map(|(index, person)| {
            person.image.clone().map(|image| {
                (
                    DetailArtworkTarget::Person {
                        index,
                        id: person.id.clone().unwrap_or_default(),
                        name: person.name.clone(),
                        guest,
                    },
                    image,
                )
            })
        }));
    }
    for (target, image) in requests {
        load_detail_image(context, generation, target, image);
    }
}

fn load_detail_image(
    context: &AppContext,
    generation: u64,
    target: DetailArtworkTarget,
    image: ImageRef,
) {
    let Some(api) = context.api() else { return };
    // Already decoded and resident: the presenter reads the same key, so the
    // row on screen is already showing it.
    if cached_image(&image_key(&image)).is_some() {
        return;
    }
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let cache_id = format!(
            "{}-{}-{}",
            image.item_id,
            image.image_type.jellyfin_name(),
            image.index.unwrap_or(0)
        );
        // Above the cache path, which is keyed on it: an entry fetched at one
        // size must not be served for a request at another.
        let max_width = match &target {
            DetailArtworkTarget::HeroBackdrop(_) => 1600,
            DetailArtworkTarget::Episode { .. } | DetailArtworkTarget::HeroPoster(_)
                if image.image_type == ImageType::Thumb =>
            {
                900
            }
            DetailArtworkTarget::Person { .. } => 320,
            _ => 600,
        };
        let path = image_cache_path(
            &ctx.image_cache,
            api.session().server_url.as_str(),
            &cache_id,
            &image.tag,
            max_width,
        );
        // Same reason as the catalog loader: blocking std::fs, including an
        // fsync on the write, must not run on a runtime worker.
        let probe = path.clone();
        let cached = tokio::task::spawn_blocking(move || existing_cached_image(&probe))
            .await
            .ok()
            .flatten();
        let result = if let Some(path) = cached {
            Ok(path)
        } else {
            // Taken here, not at the top of the task: the permit bounds
            // concurrent downloads, and a hero image already on disk should not
            // wait behind six in-flight fetches for a permit it will not use.
            let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
                return;
            };
            match api.fetch_detail_image(&image, max_width).await {
                Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => {
                    tokio::task::spawn_blocking(move || write_cached_image(&path, &bytes))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()))
                }
                Ok(_) => Err("detail artwork exceeded 20 MiB".to_owned()),
                Err(error) => Err(error.to_string()),
            }
        };
        if let Ok(path) = result {
            let key = image_key(&image);
            let apply_ctx = ctx.clone();
            ctx.ui.update(move |window| {
                // On the event loop for the same measured reason as the catalog
                // loader: moving the decode off it costs more memory than the
                // UI-thread time it saves.
                let Ok(image) = Image::load_from_path(&path) else {
                    return;
                };
                // A fetched image stays valid regardless of navigation, so it
                // is cached even when this request's generation went stale.
                store_cached_image(key, image.clone());
                if !apply_ctx.is_current(generation) {
                    return;
                }
                match target {
                    DetailArtworkTarget::HeroPoster(id)
                        if window.get_detail_id().as_str() == id =>
                    {
                        window.set_detail_artwork(image);
                    }
                    DetailArtworkTarget::HeroBackdrop(id)
                        if window.get_detail_id().as_str() == id =>
                    {
                        window.set_detail_backdrop(image);
                    }
                    DetailArtworkTarget::HeroLogo(id) if window.get_detail_id().as_str() == id => {
                        window.set_detail_logo(image);
                    }
                    DetailArtworkTarget::Season { index, id } => {
                        let model = window.get_detail_seasons();
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    DetailArtworkTarget::Episode { index, id } => {
                        let model = window.get_detail_episodes();
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    DetailArtworkTarget::Similar { index, id } => {
                        let model = window.get_detail_similar();
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    DetailArtworkTarget::NextUp { id } => {
                        let model = window.get_detail_next_up();
                        if let Some(mut row) = model.row_data(0)
                            && row.id.as_str() == id
                        {
                            row.artwork = image;
                            model.set_row_data(0, row);
                        }
                    }
                    DetailArtworkTarget::Person {
                        index,
                        id,
                        name,
                        guest,
                    } => {
                        let model = if guest {
                            window.get_detail_guest_stars()
                        } else {
                            window.get_detail_people()
                        };
                        if let Some(mut row) = model.row_data(index)
                            && row.id.as_str() == id
                            && row.name.as_str() == name
                        {
                            row.artwork = image;
                            model.set_row_data(index, row);
                        }
                    }
                    _ => {}
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn season_summary_reports_episode_and_unwatched_counts() {
        let mut watched = DetailItemData::fixture("watched", DetailKind::Episode);
        watched.watched = true;
        let unwatched = DetailItemData::fixture("unwatched", DetailKind::Episode);
        let page = DetailPageData {
            item: DetailItemData::fixture("season", DetailKind::Season),
            seasons: Vec::new(),
            episodes: vec![watched, unwatched],
            similar: Vec::new(),
            children_error: None,
        };

        assert_eq!(detail_season_summary(&page), "2 episodes • 1 unwatched");
    }

    #[test]
    fn season_summary_distinguishes_empty_from_unavailable() {
        let mut page = DetailPageData {
            item: DetailItemData::fixture("season", DetailKind::Season),
            seasons: Vec::new(),
            episodes: Vec::new(),
            similar: Vec::new(),
            children_error: None,
        };
        assert_eq!(detail_season_summary(&page), "No episodes available");

        page.children_error = Some("offline".into());
        assert_eq!(detail_season_summary(&page), "Episodes unavailable");
    }

    #[test]
    fn episode_metadata_leads_with_the_air_date_like_jellyfin() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.series_name = Some("Example Show".into());
        episode.season_name = Some("Season 2".into());
        episode.parent_index_number = Some(2);
        episode.index_number = Some(3);
        episode.production_year = Some(2026);
        episode.premiere_date = Some((2026, 1, 31));
        episode.official_rating = Some("TV-MA".into());
        episode.runtime_seconds = Some(2_700.0);

        let metadata = detail_metadata_line(&episode);
        // The subheading already carries the number and the parents.
        assert_eq!(metadata, "1/31/2026 • TV-MA • 45m");

        episode.premiere_date = None;
        assert_eq!(detail_metadata_line(&episode), "2026 • TV-MA • 45m");
    }

    #[test]
    fn series_info_line_reads_the_air_span() {
        let mut series = DetailItemData::fixture("series", DetailKind::Series);
        series.production_year = Some(2019);
        series.official_rating = Some("TV-MA".into());
        series.runtime_seconds = Some(3_000.0);
        series.series_continuing = true;
        assert_eq!(detail_metadata_line(&series), "2019 – Present • TV-MA • 50m");

        series.series_continuing = false;
        series.end_year = Some(2022);
        assert_eq!(detail_metadata_line(&series), "2019 – 2022 • TV-MA • 50m");

        series.end_year = Some(2019);
        assert_eq!(detail_metadata_line(&series), "2019 • TV-MA • 50m");

        series.production_year = None;
        assert_eq!(detail_metadata_line(&series), "TV-MA • 50m");
    }

    #[test]
    fn next_up_cards_name_the_season_and_episode() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.parent_index_number = Some(2);
        episode.index_number = Some(3);
        assert_eq!(next_up_label(&episode), "S2:E3 -");
        episode.parent_index_number = None;
        assert_eq!(next_up_label(&episode), "E3 -");
    }

    #[test]
    fn season_info_line_counts_its_episodes() {
        let mut season = DetailItemData::fixture("season", DetailKind::Season);
        season.production_year = Some(2025);
        season.official_rating = Some("TV-MA".into());
        let page = |count: usize| DetailPageData {
            item: season.clone(),
            seasons: Vec::new(),
            episodes: (0..count)
                .map(|number| DetailItemData::fixture(&format!("e{number}"), DetailKind::Episode))
                .collect(),
            similar: Vec::new(),
            children_error: None,
        };
        assert_eq!(detail_info_line(&page(10)), "2025 • 10 episodes • TV-MA");
        assert_eq!(detail_info_line(&page(1)), "2025 • 1 episode • TV-MA");
        assert_eq!(detail_info_line(&page(0)), "2025 • TV-MA");
    }

    #[test]
    fn episode_cards_show_when_they_aired_and_how_long_they_run() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.runtime_seconds = Some(2_940.0);
        assert_eq!(detail_card_subtitle(&episode), "49m");
        episode.premiere_date = Some((2025, 1, 31));
        assert_eq!(detail_card_subtitle(&episode), "1/31/2025 • 49m");
        episode.runtime_seconds = Some(3_900.0);
        assert_eq!(detail_card_subtitle(&episode), "1/31/2025 • 1h 5m");
    }

    #[test]
    fn episode_pages_know_where_the_episode_sits_in_its_season() {
        let episodes = (1..=20)
            .map(|number| DetailItemData::fixture(&format!("e{number}"), DetailKind::Episode))
            .collect::<Vec<_>>();
        let page = DetailPageData {
            item: DetailItemData::fixture("e12", DetailKind::Episode),
            seasons: Vec::new(),
            episodes,
            similar: Vec::new(),
            children_error: None,
        };
        assert_eq!(current_episode_index(&page), 11);

        let season = DetailPageData {
            item: DetailItemData::fixture("e12", DetailKind::Season),
            ..page
        };
        assert_eq!(current_episode_index(&season), -1);
    }

    #[test]
    fn detail_progress_label_formats_resume_position() {
        let mut movie = DetailItemData::fixture("movie", DetailKind::Movie);
        movie.resume_seconds = 3_661.0;
        assert_eq!(detail_progress_label(&movie), "Resume at 1:01:01");

        movie.watched = true;
        assert_eq!(detail_progress_label(&movie), "");
    }

    #[test]
    fn series_summary_reports_loaded_seasons_and_episode_state() {
        let mut watched = DetailItemData::fixture("watched", DetailKind::Episode);
        watched.watched = true;
        let unwatched = DetailItemData::fixture("unwatched", DetailKind::Episode);
        let page = DetailPageData {
            item: DetailItemData::fixture("series", DetailKind::Series),
            seasons: vec![
                DetailItemData::fixture("season-1", DetailKind::Season),
                DetailItemData::fixture("season-2", DetailKind::Season),
            ],
            episodes: vec![watched, unwatched],
            similar: Vec::new(),
            children_error: None,
        };

        assert_eq!(
            detail_series_summary(&page),
            "2 seasons • 2 episodes • 1 unwatched"
        );
    }

    #[test]
    fn series_play_context_describes_the_selected_episode() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.parent_index_number = Some(1);
        episode.index_number = Some(4);
        episode.resume_seconds = 1_227.0;
        let mut page = DetailPageData {
            item: DetailItemData::fixture("series", DetailKind::Series),
            seasons: Vec::new(),
            episodes: vec![episode],
            similar: Vec::new(),
            children_error: None,
        };

        assert_eq!(
            detail_series_play_context(&page),
            "S01E04 • Resume at 20:27"
        );

        page.episodes[0].resume_seconds = 0.0;
        assert_eq!(detail_series_play_context(&page), "S01E04 • Up next");

        page.episodes[0].watched = true;
        assert_eq!(detail_series_play_context(&page), "S01E04 • Start again");
    }
}
