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
    window.on_play_current(move || features::playback::start_playback(&ctx));

    let ctx = context.clone();
    window.on_play_detail(move |id| {
        let item = ctx
            .state
            .lock()
            .ok()
            .and_then(|state| state.detail.item_by_id(id.as_str()));
        if let Some(item) = item {
            features::playback::start_playback_item(&ctx, item);
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

/// Presents the current detail page and kicks off its artwork loading.
pub fn present(context: &AppContext, generation: u64) {
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
        context.ui.update(move |window| {
            window.set_screen(super::Screen::Detail);
            window.set_busy(false);
            window.set_message(error.into());
            window.set_can_go_back(can_go_back);
            window.set_detail_id("".into());
            window.set_detail_title("".into());
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
            window.set_detail_people(ModelRc::new(VecModel::from(Vec::new())));
        });
        return;
    };
    let detail_id = page.item.id.clone();
    let title = page.item.title.clone();
    let subtitle = detail_metadata_line(&page.item);
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
    let wide_art = page.item.kind == DetailKind::Episode;
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
    let people = page.item.people.clone();
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
        let people = people
            .iter()
            .map(|person| DetailPerson {
                id: person.id.as_deref().unwrap_or_default().into(),
                name: person.name.as_str().into(),
                role: person.role.as_str().into(),
                artwork: person
                    .image
                    .as_ref()
                    .and_then(|image| cached_image(&image_key(image)))
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        window.set_screen(super::Screen::Detail);
        window.set_busy(false);
        window.set_message(message.into());
        window.set_detail_id(detail_id.into());
        window.set_detail_title(title.into());
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
        window.set_detail_wide_art(wide_art);
        window.set_detail_artwork(hero_image(hero_poster_key));
        window.set_detail_backdrop(hero_image(hero_backdrop_key));
        window.set_detail_logo(hero_image(hero_logo_key));
        if !patch_rows(&window.get_detail_seasons(), &seasons, |a, b| a.id == b.id) {
            window.set_detail_seasons(ModelRc::new(VecModel::from(seasons)));
        }
        if !patch_rows(&window.get_detail_episodes(), &episodes, |a, b| a.id == b.id) {
            window.set_detail_episodes(ModelRc::new(VecModel::from(episodes)));
        }
        if !patch_rows(&window.get_detail_people(), &people, |a, b| {
            a.id == b.id && a.name == b.name
        }) {
            window.set_detail_people(ModelRc::new(VecModel::from(people)));
        }
        window.set_detail_audio_options(ModelRc::new(VecModel::from(
            audio_options
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )));
        window.set_detail_subtitle_options(ModelRc::new(VecModel::from(
            subtitle_options
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )));
        window.set_detail_audio_index(audio_index);
        window.set_detail_subtitle_index(subtitle_index);
        window.set_can_go_back(can_go_back);
    });
    load_detail_artwork(context, generation, &page);
}

fn to_detail_card(item: &DetailItemData) -> DetailCard {
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
    }
}

fn detail_card_subtitle(item: &DetailItemData) -> String {
    let episode_number = detail_episode_number(item);
    let runtime = item.runtime_seconds.map(|seconds| {
        let minutes = (seconds / 60.0).round() as u64;
        format!("{minutes} min")
    });
    let unplayed = item
        .unplayed_count
        .filter(|count| *count > 0)
        .map(|count| format!("{count} unwatched"));
    [
        episode_number,
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

fn detail_stream_label(stream: &StreamData, fallback: &str, index: usize) -> String {
    if !stream.label.is_empty() {
        stream.label.clone()
    } else if let Some(codec) = clean_preference(&stream.codec) {
        codec.to_uppercase()
    } else {
        format!("{fallback} {}", index + 1)
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

fn detail_metadata_line(item: &DetailItemData) -> String {
    let runtime = item.runtime_seconds.map(|seconds| {
        let minutes = (seconds / 60.0).round() as u64;
        if minutes >= 60 {
            format!("{}h {}m", minutes / 60, minutes % 60)
        } else {
            format!("{minutes}m")
        }
    });
    let leading_label = match item.kind {
        DetailKind::Movie => None,
        DetailKind::Episode => detail_episode_number(item),
        _ => Some(detail_kind_name(item.kind).to_owned()),
    };
    [
        leading_label,
        item.production_year.map(|year| year.to_string()),
        item.official_rating.clone(),
        runtime,
        item.community_rating.map(|rating| format!("★ {rating:.1}")),
        item.critic_rating
            .map(|rating| format!("Critics {rating:.0}%")),
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
    Person { index: usize, id: String, name: String },
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
    requests.extend(
        page.item
            .people
            .iter()
            .enumerate()
            .filter_map(|(index, person)| {
                person.image.clone().map(|image| {
                    (
                        DetailArtworkTarget::Person {
                            index,
                            id: person.id.clone().unwrap_or_default(),
                            name: person.name.clone(),
                        },
                        image,
                    )
                })
            }),
    );
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
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let Ok(_permit) = ctx.image_limit.clone().acquire_owned().await else {
            return;
        };
        let cache_id = format!(
            "{}-{}-{}",
            image.item_id,
            image.image_type.jellyfin_name(),
            image.index.unwrap_or(0)
        );
        let path = image_cache_path(
            &ctx.image_cache,
            api.session().server_url.as_str(),
            &cache_id,
            &image.tag,
        );
        let result = if let Some(path) = existing_cached_image(&path) {
            Ok(path)
        } else {
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
            match api.fetch_detail_image(&image, max_width).await {
                Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => write_cached_image(&path, &bytes),
                Ok(_) => Err("detail artwork exceeded 20 MiB".to_owned()),
                Err(error) => Err(error.to_string()),
            }
        };
        if let Ok(path) = result {
            let key = image_key(&image);
            let apply_ctx = ctx.clone();
            ctx.ui.update(move |window| {
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
                    DetailArtworkTarget::Person { index, id, name } => {
                        let model = window.get_detail_people();
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
            children_error: None,
        };
        assert_eq!(detail_season_summary(&page), "No episodes available");

        page.children_error = Some("offline".into());
        assert_eq!(detail_season_summary(&page), "Episodes unavailable");
    }

    #[test]
    fn episode_metadata_uses_number_without_repeating_its_parents() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.series_name = Some("Example Show".into());
        episode.season_name = Some("Season 2".into());
        episode.parent_index_number = Some(2);
        episode.index_number = Some(3);
        episode.production_year = Some(2026);
        episode.runtime_seconds = Some(2_700.0);

        let metadata = detail_metadata_line(&episode);
        assert_eq!(metadata, "S02E03 • 2026 • 45m");
        assert!(!metadata.contains("Example Show"));
        assert!(!metadata.contains("Season 2"));
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
