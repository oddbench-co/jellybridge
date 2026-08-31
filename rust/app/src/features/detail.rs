use std::collections::HashMap;

use crate::app::{AppContext, AppState, DetailTrackSelection, RetryAction, cancel_catalog_request};
use crate::features::playback::{best_preference_match, clean_preference, playback_preference_key};
use crate::services::session::{StoredPlaybackPreference, StoredSettings};
use crate::ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailKind {
    Movie,
    Series,
    Season,
    Episode,
    Other,
}

impl DetailKind {
    pub fn from_jellyfin(value: &str) -> Self {
        match value {
            "Movie" => Self::Movie,
            "Series" => Self::Series,
            "Season" => Self::Season,
            "Episode" => Self::Episode,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailTarget {
    pub id: String,
    pub kind: DetailKind,
}

impl DetailTarget {
    pub fn new(id: impl Into<String>, kind: DetailKind) -> Self {
        Self {
            id: id.into(),
            kind,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImageRef {
    pub item_id: String,
    pub image_type: ImageType,
    pub index: Option<usize>,
    pub tag: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Primary,
    Backdrop,
    Logo,
    Thumb,
}

impl ImageType {
    pub fn jellyfin_name(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Backdrop => "Backdrop",
            Self::Logo => "Logo",
            Self::Thumb => "Thumb",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PersonData {
    pub id: Option<String>,
    pub name: String,
    pub role: String,
    pub image: Option<ImageRef>,
}

#[derive(Clone, Debug)]
pub struct StreamData {
    pub index: i32,
    pub kind: String,
    pub label: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct DetailItemData {
    pub id: String,
    pub title: String,
    pub kind: DetailKind,
    pub parent_id: Option<String>,
    pub series_id: Option<String>,
    pub series_name: Option<String>,
    pub season_id: Option<String>,
    pub season_name: Option<String>,
    pub index_number: Option<i32>,
    pub parent_index_number: Option<i32>,
    pub overview: String,
    pub tagline: String,
    pub production_year: Option<i32>,
    pub official_rating: Option<String>,
    pub community_rating: Option<f32>,
    pub critic_rating: Option<f32>,
    pub runtime_seconds: Option<f64>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub external_urls: Vec<(String, String)>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub studios: Vec<String>,
    pub people: Vec<PersonData>,
    pub media_summary: String,
    pub poster: Option<ImageRef>,
    pub backdrop: Option<ImageRef>,
    pub logo: Option<ImageRef>,
    pub thumbnail: Option<ImageRef>,
    pub watched: bool,
    pub favorite: bool,
    pub progress: f32,
    pub resume_seconds: f64,
    pub unplayed_count: Option<i32>,
    pub playable: bool,
    pub streams: Vec<StreamData>,
}

#[cfg(test)]
impl DetailItemData {
    pub(crate) fn fixture(id: &str, kind: DetailKind) -> Self {
        Self {
            id: id.into(),
            title: id.into(),
            kind,
            parent_id: None,
            series_id: None,
            series_name: None,
            season_id: None,
            season_name: None,
            index_number: None,
            parent_index_number: None,
            overview: String::new(),
            tagline: String::new(),
            production_year: None,
            official_rating: None,
            community_rating: None,
            critic_rating: None,
            runtime_seconds: None,
            genres: Vec::new(),
            tags: Vec::new(),
            external_urls: Vec::new(),
            directors: Vec::new(),
            writers: Vec::new(),
            studios: Vec::new(),
            people: Vec::new(),
            media_summary: String::new(),
            poster: None,
            backdrop: None,
            logo: None,
            thumbnail: None,
            watched: false,
            favorite: false,
            progress: 0.0,
            resume_seconds: 0.0,
            unplayed_count: None,
            playable: matches!(kind, DetailKind::Movie | DetailKind::Episode),
            streams: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DetailPageData {
    pub item: DetailItemData,
    pub seasons: Vec<DetailItemData>,
    pub episodes: Vec<DetailItemData>,
    pub children_error: Option<String>,
}

impl DetailPageData {
    pub(crate) fn play_target(&self) -> Option<&DetailItemData> {
        if matches!(self.item.kind, DetailKind::Movie | DetailKind::Episode) {
            return Some(&self.item);
        }
        self.episodes
            .iter()
            .find(|episode| episode.resume_seconds > 0.0 && !episode.watched)
            .or_else(|| self.episodes.iter().find(|episode| !episode.watched))
            .or_else(|| self.episodes.first())
    }

    pub(crate) fn primary_play_label(&self) -> Option<&'static str> {
        let target = self.play_target()?;
        if target.resume_seconds > 0.0 && !target.watched {
            Some("Resume")
        } else if matches!(self.item.kind, DetailKind::Series | DetailKind::Season)
            && !target.watched
        {
            Some("Play next")
        } else if target.watched {
            Some("Play again")
        } else {
            Some("Play")
        }
    }
}

#[derive(Clone, Debug)]
pub enum DetailMessage {
    Open(DetailTarget),
    Loaded(Box<DetailPageData>),
    LoadFailed(String),
    Play,
    SetWatched {
        id: String,
        watched: bool,
    },
    SetFavorite {
        id: String,
        favorite: bool,
    },
    MutationFinished {
        id: String,
        kind: DetailMutationKind,
        error: Option<String>,
    },
    Back,
    Reset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailEffect {
    Load(DetailTarget),
    Play(String),
    SetWatched { id: String, watched: bool },
    SetFavorite { id: String, favorite: bool },
    ShowCatalog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DetailMutationKind {
    Watched,
    Favorite,
}

#[derive(Default)]
pub struct DetailModel {
    page: Option<DetailPageData>,
    history: Vec<DetailPageData>,
    pending: Option<DetailTarget>,
    mutations: HashMap<(String, DetailMutationKind), bool>,
    error: Option<String>,
}

pub struct DetailSnapshot<'a> {
    pub page: Option<&'a DetailPageData>,
    pub busy: bool,
    pub error: Option<&'a str>,
    pub can_go_back: bool,
}

impl DetailModel {
    pub fn update(&mut self, message: DetailMessage) -> Vec<DetailEffect> {
        match message {
            DetailMessage::Open(target) => {
                if let Some(page) = self.page.take() {
                    self.history.push(page);
                }
                self.pending = Some(target.clone());
                self.error = None;
                vec![DetailEffect::Load(target)]
            }
            DetailMessage::Loaded(page) => {
                if self
                    .pending
                    .as_ref()
                    .is_none_or(|target| target.id == page.item.id)
                {
                    self.page = Some(*page);
                    self.pending = None;
                    self.error = None;
                }
                Vec::new()
            }
            DetailMessage::LoadFailed(error) => {
                self.pending = None;
                self.error = Some(error);
                Vec::new()
            }
            DetailMessage::Play => self
                .page
                .as_ref()
                .and_then(DetailPageData::play_target)
                .map(|item| vec![DetailEffect::Play(item.id.clone())])
                .unwrap_or_default(),
            DetailMessage::SetWatched { id, watched } => {
                let Some(previous) = self.item(&id).map(|item| item.watched) else {
                    return Vec::new();
                };
                self.mutations
                    .insert((id.clone(), DetailMutationKind::Watched), previous);
                self.update_item(&id, |item| item.watched = watched);
                self.error = None;
                vec![DetailEffect::SetWatched { id, watched }]
            }
            DetailMessage::SetFavorite { id, favorite } => {
                let Some(previous) = self.item(&id).map(|item| item.favorite) else {
                    return Vec::new();
                };
                self.mutations
                    .insert((id.clone(), DetailMutationKind::Favorite), previous);
                self.update_item(&id, |item| item.favorite = favorite);
                self.error = None;
                vec![DetailEffect::SetFavorite { id, favorite }]
            }
            DetailMessage::MutationFinished { id, kind, error } => {
                let previous = self.mutations.remove(&(id.clone(), kind));
                if let (Some(previous), Some(error)) = (previous, error) {
                    match kind {
                        DetailMutationKind::Watched => {
                            self.update_item(&id, |item| item.watched = previous);
                        }
                        DetailMutationKind::Favorite => {
                            self.update_item(&id, |item| item.favorite = previous);
                        }
                    }
                    self.error = Some(error);
                }
                Vec::new()
            }
            DetailMessage::Back => {
                self.pending = None;
                self.error = None;
                if let Some(previous) = self.history.pop() {
                    self.page = Some(previous);
                    Vec::new()
                } else {
                    self.page = None;
                    vec![DetailEffect::ShowCatalog]
                }
            }
            DetailMessage::Reset => {
                *self = Self::default();
                Vec::new()
            }
        }
    }

    pub fn snapshot(&self) -> DetailSnapshot<'_> {
        DetailSnapshot {
            page: self.page.as_ref(),
            busy: self.pending.is_some(),
            error: self.error.as_deref(),
            can_go_back: self.page.is_some() || !self.history.is_empty(),
        }
    }

    pub fn item_by_id(&self, id: &str) -> Option<DetailItemData> {
        self.item(id).cloned()
    }

    fn item(&self, id: &str) -> Option<&DetailItemData> {
        self.page
            .as_ref()
            .and_then(|page| find_item(page, id))
            .or_else(|| {
                self.history
                    .iter()
                    .rev()
                    .find_map(|page| find_item(page, id))
            })
    }

    fn update_item(&mut self, id: &str, mut update: impl FnMut(&mut DetailItemData)) {
        if let Some(page) = self.page.as_mut() {
            update_matching_items(page, id, &mut update);
        }
        for page in &mut self.history {
            update_matching_items(page, id, &mut update);
        }
    }
}

fn find_item<'a>(page: &'a DetailPageData, id: &str) -> Option<&'a DetailItemData> {
    std::iter::once(&page.item)
        .chain(page.seasons.iter())
        .chain(page.episodes.iter())
        .find(|item| item.id == id)
}

fn update_matching_items(
    page: &mut DetailPageData,
    id: &str,
    update: &mut impl FnMut(&mut DetailItemData),
) {
    for item in std::iter::once(&mut page.item)
        .chain(page.seasons.iter_mut())
        .chain(page.episodes.iter_mut())
    {
        if item.id == id {
            update(item);
        }
    }
}

pub fn start_detail(context: &AppContext, target: DetailTarget) {
    let Some(api) = context.api() else { return };
    let generation = context.next_generation();
    cancel_catalog_request(context);
    if let Ok(mut state) = context.state.lock() {
        state.retry = RetryAction::Detail(target.clone());
        state.detail.update(DetailMessage::Open(target.clone()));
    }
    ui::set_screen_busy(context, ui::Screen::Detail, true, "");
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = api.detail_page(&target.id).await;
        if !ctx.is_current(generation) {
            return;
        }
        match result {
            Ok(page) => {
                let settings = ctx.settings_store.load().unwrap_or_default();
                if let Ok(mut state) = ctx.state.lock() {
                    state.detail.update(DetailMessage::Loaded(Box::new(page)));
                    state.detail_tracks = detail_track_defaults(&state.detail, &settings);
                }
                ui::detail::present(&ctx, generation);
            }
            Err(error) => {
                let message = error.to_string();
                if let Ok(mut state) = ctx.state.lock() {
                    state
                        .detail
                        .update(DetailMessage::LoadFailed(message.clone()));
                }
                ui::detail::present(&ctx, generation);
            }
        }
    });
}

pub fn update_detail_mutation(context: &AppContext, message: DetailMessage) {
    let effects = context
        .state
        .lock()
        .map(|mut state| state.detail.update(message))
        .unwrap_or_default();
    // Deliberately reuses the current generation rather than bumping it, so a
    // watched/favorite refresh does not cancel in-flight artwork loads.
    let generation = context.current_generation();
    ui::detail::present(context, generation);
    for effect in effects {
        let (id, kind, future) = match effect {
            DetailEffect::SetWatched { id, watched } => {
                let Some(api) = context.api() else { continue };
                let call_id = id.clone();
                (
                    id,
                    DetailMutationKind::Watched,
                    context.runtime.spawn(async move {
                        api.set_watched(&call_id, watched)
                            .await
                            .map_err(|error| error.to_string())
                    }),
                )
            }
            DetailEffect::SetFavorite { id, favorite } => {
                let Some(api) = context.api() else { continue };
                let call_id = id.clone();
                (
                    id,
                    DetailMutationKind::Favorite,
                    context.runtime.spawn(async move {
                        api.set_favorite(&call_id, favorite)
                            .await
                            .map_err(|error| error.to_string())
                    }),
                )
            }
            _ => continue,
        };
        let ctx = context.clone();
        context.runtime.spawn(async move {
            let error = match future.await {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(error) => Some(format!("detail update task failed: {error}")),
            };
            if let Ok(mut state) = ctx.state.lock() {
                state
                    .detail
                    .update(DetailMessage::MutationFinished { id, kind, error });
            }
            if ctx.is_current(generation) {
                ui::detail::present(&ctx, generation);
            }
        });
    }
}

pub fn select_detail_track(context: &AppContext, index: i32, audio: bool) {
    let preference = context.state.lock().ok().and_then(|mut state| {
        if audio {
            if usize::try_from(index)
                .ok()
                .is_none_or(|index| index >= state.detail_tracks.audio_ids.len())
            {
                return None;
            }
            state.detail_tracks.audio_index = index;
        } else {
            if usize::try_from(index)
                .ok()
                .is_none_or(|index| index >= state.detail_tracks.subtitle_ids.len())
            {
                return None;
            }
            state.detail_tracks.subtitle_index = index;
        }
        selected_detail_preference(&state)
    });
    let Some((key, preference)) = preference else {
        return;
    };
    // The settings store does blocking file I/O, so persist off the UI thread.
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let store = ctx.settings_store.clone();
        let result =
            tokio::task::spawn_blocking(move || store.save_playback_preference(&key, preference))
                .await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!("playback preference could not be saved");
            ui::set_message(&ctx, "Playback preference could not be saved");
        }
    });
}

fn selected_detail_preference(state: &AppState) -> Option<(String, StoredPlaybackPreference)> {
    let item = &state.detail.snapshot().page?.item;
    let settings = state.detail_tracks.clone();
    let audio_stream = usize::try_from(settings.audio_index)
        .ok()
        .and_then(|index| settings.audio_ids.get(index))
        .and_then(|selected| {
            item.streams.iter().find(|stream| {
                stream.kind.eq_ignore_ascii_case("audio") && stream.index == *selected
            })
        });
    let subtitle_selection = usize::try_from(settings.subtitle_index)
        .ok()
        .and_then(|index| settings.subtitle_ids.get(index))
        .copied()
        .flatten();
    let subtitle_stream = subtitle_selection.and_then(|selected| {
        item.streams
            .iter()
            .find(|stream| stream.kind.eq_ignore_ascii_case("subtitle") && stream.index == selected)
    });
    Some((
        playback_preference_key(item),
        StoredPlaybackPreference {
            audio_language: audio_stream.and_then(|stream| clean_preference(&stream.language)),
            audio_title: audio_stream.and_then(|stream| clean_preference(&stream.title)),
            subtitle_language: subtitle_stream
                .and_then(|stream| clean_preference(&stream.language)),
            subtitle_title: subtitle_stream.and_then(|stream| clean_preference(&stream.title)),
            subtitles_disabled: subtitle_selection.is_none() && settings.subtitle_ids.len() > 1,
        },
    ))
}

pub fn detail_track_defaults(
    model: &DetailModel,
    settings: &StoredSettings,
) -> DetailTrackSelection {
    let Some(page) = model.snapshot().page else {
        return DetailTrackSelection::default();
    };
    let audio = page
        .item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .collect::<Vec<_>>();
    let subtitles = page
        .item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
        .collect::<Vec<_>>();
    let preference = settings
        .playback_preferences
        .get(&playback_preference_key(&page.item));
    let audio_index = preference
        .and_then(|preference| {
            best_preference_match(&audio, &preference.audio_language, &preference.audio_title)
        })
        .or_else(|| audio.iter().position(|stream| stream.is_default))
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(0);
    let subtitle_default = if preference.is_some_and(|preference| preference.subtitles_disabled) {
        0
    } else {
        preference
            .and_then(|preference| {
                best_preference_match(
                    &subtitles,
                    &preference.subtitle_language,
                    &preference.subtitle_title,
                )
            })
            .or_else(|| subtitles.iter().position(|stream| stream.is_default))
            .and_then(|index| i32::try_from(index + 1).ok())
            .unwrap_or(0)
    };
    DetailTrackSelection {
        audio_ids: audio.iter().map(|stream| stream.index).collect(),
        subtitle_ids: std::iter::once(None)
            .chain(subtitles.iter().map(|stream| Some(stream.index)))
            .collect(),
        audio_index,
        subtitle_index: subtitle_default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &str, kind: DetailKind) -> DetailPageData {
        DetailPageData {
            item: DetailItemData::fixture(id, kind),
            seasons: Vec::new(),
            episodes: Vec::new(),
            children_error: None,
        }
    }

    #[test]
    fn nested_details_restore_each_parent_before_returning_to_the_catalog() {
        let mut model = DetailModel::default();

        assert_eq!(
            model.update(DetailMessage::Open(DetailTarget::new(
                "series",
                DetailKind::Series
            ))),
            vec![DetailEffect::Load(DetailTarget::new(
                "series",
                DetailKind::Series
            ))]
        );
        model.update(DetailMessage::Loaded(Box::new(page(
            "series",
            DetailKind::Series,
        ))));
        model.update(DetailMessage::Open(DetailTarget::new(
            "season",
            DetailKind::Season,
        )));
        model.update(DetailMessage::Loaded(Box::new(page(
            "season",
            DetailKind::Season,
        ))));
        model.update(DetailMessage::Open(DetailTarget::new(
            "episode",
            DetailKind::Episode,
        )));
        model.update(DetailMessage::Loaded(Box::new(page(
            "episode",
            DetailKind::Episode,
        ))));

        assert!(model.update(DetailMessage::Back).is_empty());
        assert_eq!(model.snapshot().page.unwrap().item.id, "season");
        assert!(model.update(DetailMessage::Back).is_empty());
        assert_eq!(model.snapshot().page.unwrap().item.id, "series");
        assert_eq!(
            model.update(DetailMessage::Back),
            vec![DetailEffect::ShowCatalog]
        );
        assert!(model.snapshot().page.is_none());
    }

    #[test]
    fn container_play_prefers_progress_then_unwatched_then_first_episode() {
        let mut first = DetailItemData::fixture("first", DetailKind::Episode);
        first.watched = true;
        let mut unwatched = DetailItemData::fixture("unwatched", DetailKind::Episode);
        unwatched.watched = false;
        let mut progress = DetailItemData::fixture("progress", DetailKind::Episode);
        progress.resume_seconds = 312.0;
        let mut series = page("series", DetailKind::Series);
        series.episodes = vec![first.clone(), unwatched.clone(), progress.clone()];
        let mut model = DetailModel::default();
        model.update(DetailMessage::Loaded(Box::new(series)));

        assert_eq!(
            model.update(DetailMessage::Play),
            vec![DetailEffect::Play("progress".into())]
        );

        progress.resume_seconds = 0.0;
        let mut season = page("season", DetailKind::Season);
        season.episodes = vec![first.clone(), unwatched, progress];
        model.update(DetailMessage::Loaded(Box::new(season)));
        assert_eq!(
            model.update(DetailMessage::Play),
            vec![DetailEffect::Play("unwatched".into())]
        );

        let mut watched_season = page("season", DetailKind::Season);
        watched_season.episodes = vec![first];
        model.update(DetailMessage::Loaded(Box::new(watched_season)));
        assert_eq!(
            model.update(DetailMessage::Play),
            vec![DetailEffect::Play("first".into())]
        );
    }

    #[test]
    fn primary_play_labels_follow_the_selected_media() {
        let mut season = page("season", DetailKind::Season);
        assert_eq!(season.primary_play_label(), None);

        let mut watched = DetailItemData::fixture("watched", DetailKind::Episode);
        watched.watched = true;
        let mut next = DetailItemData::fixture("next", DetailKind::Episode);
        next.watched = false;
        let mut resumable = DetailItemData::fixture("resumable", DetailKind::Episode);
        resumable.resume_seconds = 312.0;
        season.episodes = vec![watched.clone(), next.clone(), resumable];
        assert_eq!(season.primary_play_label(), Some("Resume"));

        season.episodes = vec![watched.clone(), next];
        assert_eq!(season.primary_play_label(), Some("Play next"));

        season.episodes = vec![watched];
        assert_eq!(season.primary_play_label(), Some("Play again"));

        let mut movie = page("movie", DetailKind::Movie);
        assert_eq!(movie.primary_play_label(), Some("Play"));
        movie.item.resume_seconds = 91.0;
        assert_eq!(movie.primary_play_label(), Some("Resume"));
        movie.item.watched = true;
        assert_eq!(movie.primary_play_label(), Some("Play again"));
    }

    #[test]
    fn failed_optimistic_actions_restore_the_previous_state() {
        let mut detail = page("episode", DetailKind::Episode);
        detail.item.watched = false;
        detail.item.favorite = false;
        let mut model = DetailModel::default();
        model.update(DetailMessage::Loaded(Box::new(detail)));

        assert_eq!(
            model.update(DetailMessage::SetWatched {
                id: "episode".into(),
                watched: true,
            }),
            vec![DetailEffect::SetWatched {
                id: "episode".into(),
                watched: true,
            }]
        );
        assert!(model.snapshot().page.unwrap().item.watched);
        model.update(DetailMessage::MutationFinished {
            id: "episode".into(),
            kind: DetailMutationKind::Watched,
            error: Some("server rejected watched state".into()),
        });
        let snapshot = model.snapshot();
        assert!(!snapshot.page.unwrap().item.watched);
        assert_eq!(snapshot.error, Some("server rejected watched state"));

        model.update(DetailMessage::SetFavorite {
            id: "episode".into(),
            favorite: true,
        });
        assert!(model.snapshot().page.unwrap().item.favorite);
        model.update(DetailMessage::MutationFinished {
            id: "episode".into(),
            kind: DetailMutationKind::Favorite,
            error: Some("server rejected favorite state".into()),
        });
        assert!(!model.snapshot().page.unwrap().item.favorite);
    }

    #[test]
    fn successful_optimistic_actions_keep_the_requested_state() {
        let mut model = DetailModel::default();
        model.update(DetailMessage::Loaded(Box::new(page(
            "episode",
            DetailKind::Episode,
        ))));
        model.update(DetailMessage::SetWatched {
            id: "episode".into(),
            watched: true,
        });

        model.update(DetailMessage::MutationFinished {
            id: "episode".into(),
            kind: DetailMutationKind::Watched,
            error: None,
        });

        assert!(model.snapshot().page.unwrap().item.watched);
        assert!(model.snapshot().error.is_none());
    }

    #[test]
    fn stale_detail_responses_do_not_replace_the_newer_page() {
        let mut model = DetailModel::default();
        model.update(DetailMessage::Open(DetailTarget::new(
            "series",
            DetailKind::Series,
        )));
        model.update(DetailMessage::Open(DetailTarget::new(
            "season",
            DetailKind::Season,
        )));

        model.update(DetailMessage::Loaded(Box::new(page(
            "series",
            DetailKind::Series,
        ))));

        assert!(model.snapshot().page.is_none());
        assert!(model.snapshot().busy);
    }
}
