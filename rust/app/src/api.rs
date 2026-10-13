use reqwest::header::{HeaderValue, InvalidHeaderValue};
use reqwest::{Method, StatusCode};
use std::collections::HashMap;
use std::future::Future;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

mod items;
mod pages;
mod settings;
use crate::features::detail::{
    DetailItemData, DetailKind, DetailPageData, DetailSection, ImageRef, ImageType, NamedId,
    PersonData, StreamData,
};
use crate::features::now_playing::{MediaSegment, SegmentKind, split_adjacent};
use crate::features::prefs::UserPrefs;
use crate::features::progress::{ReportTarget, position_ticks};
use crate::services::session::StoredSession;
pub use items::{Container, InfoBlock, QueueSource};
pub use pages::{home_shelves, library_route, merge_home_shelves};

const CLIENT_NAME: &str = "JellyBridge";
const CLIENT_VERSION: &str = "0.1.0-rust";
const DEVICE_NAME: &str = "Desktop";
const DEVICE_ID: &str = "jellybridge-rust-desktop";
// Fields for list queries. `RecursiveItemCount` is deliberately absent: it
// makes the server recursively count the contents of every folder-typed row,
// which measured 17.8-18.1 s against a 323-item music library versus 0.37 s
// without it. No card shows a recursive count: jellyfin-web's library cards
// carry their name only.
// `Overview` and `MediaStreams` are absent for the same reason: nothing on a
// catalog row reads them. `CatalogItemData` has no overview, and the media
// summary is built only for detail pages, which fetch DETAIL_FIELDS
// separately. Carrying them made the six home requests ~900 KiB instead of
// ~75 KiB and roughly doubled the slowest one.
const LIST_FIELDS: &str = "ParentId,SeriesId,SeriesPrimaryImageTag,SeasonId,SeasonName,ParentPrimaryImageItemId,ImageTags,UserData,RunTimeTicks,ChildCount,PrimaryImageAspectRatio,ProductionYear,SeriesName,IndexNumber,ParentIndexNumber,CollectionType";
const DETAIL_FIELDS: &str = "GenreItems,ProductionLocations,ParentId,SeriesId,SeriesPrimaryImageTag,SeasonId,SeasonName,ParentPrimaryImageItemId,ParentBackdropItemId,ParentBackdropImageTags,ImageTags,BackdropImageTags,UserData,Overview,RunTimeTicks,MediaStreams,Genres,People,Studios,OfficialRating,CommunityRating,CriticRating,Taglines,PremiereDate,DateCreated,ChildCount,RecursiveItemCount,PrimaryImageAspectRatio,ProductionYear,SeriesName,IndexNumber,ParentIndexNumber,CollectionType,Tags,ExternalUrls,ParentLogoItemId,ParentLogoImageTag";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Server URL must be an http:// or https:// address without credentials")]
    InvalidServerUrl,
    #[error("Username is required")]
    MissingUsername,
    #[error("The saved Jellyfin token is invalid")]
    InvalidToken(#[from] InvalidHeaderValue),
    #[error("Jellyfin rejected this session; please sign in again")]
    Unauthorized,
    #[error("Jellyfin returned {status} while {operation}")]
    Status {
        status: StatusCode,
        operation: &'static str,
    },
    #[error("Could not communicate with Jellyfin: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct JellyfinApi {
    http: reqwest::Client,
    session: StoredSession,
    // Both are derived from `session` and never change, but `request` runs for
    // every call including the artwork fetches, where re-parsing the URL and
    // rebuilding the header string per request is pure overhead.
    server: Url,
    authorization: HeaderValue,
}

/// Home's sections and the user's settings they were built from.
#[derive(Clone, Debug)]
pub struct HomeData {
    pub sections: Vec<CatalogSectionData>,
    pub prefs: UserPrefs,
}

/// `days` ago as an ISO 8601 UTC timestamp, for `NextUpDateCutoff`.
fn iso_days_ago(days: u32) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    let then = now - i64::from(days) * 86_400;
    let (day_count, seconds) = (then.div_euclid(86_400), then.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = day_count + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[derive(Clone, Debug)]
pub struct CatalogSectionData {
    pub id: String,
    pub title: String,
    pub items: Vec<CatalogItemData>,
}

#[derive(Clone, Debug, Default)]
pub struct CatalogItemData {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub watched: bool,
    pub progress: f32,
    pub playable: bool,
    /// The card's first text line, as Jellyfin writes it: an episode is
    /// named by its series, everything else by itself.
    pub card_title: String,
    /// The card's second line: "S1:E16 - Name" for an episode, the year
    /// for a movie or series, nothing for folders.
    pub card_subtitle: String,
    /// The year line (a series' run), for sections that show title and year.
    pub year: String,
    /// Whether each line is a link in jellyfin-web, which sets its height.
    pub title_link: bool,
    pub subtitle_link: bool,
    pub favorite: bool,
    /// Unwatched episodes in a series or season, for the corner badge.
    pub unplayed_count: u32,
    /// Every image a card could show, for jellyfin-web's choice.
    pub images: CardImages,
    pub series_id: Option<String>,
    pub season_id: Option<String>,
    /// Children, or for a show on a Recently Added shelf the new episodes
    /// grouped into it.
    pub child_count: u32,
    /// A library's kind ("movies", "tvshows", …); empty for anything else.
    pub collection_type: String,
}

impl CatalogItemData {
    /// The card's lines as jellyfin-web's library Continue Watching
    /// writes them (`showTitle`, `showYear`): the item's own name (an
    /// episode's "S1:E1 - Name") over its year.
    pub fn title_over_year(mut self) -> Self {
        if self.kind == "Episode" && !self.card_subtitle.is_empty() {
            self.card_title = std::mem::take(&mut self.card_subtitle);
        }
        self.card_subtitle = std::mem::take(&mut self.year);
        self.title_link = true;
        self.subtitle_link = false;
        self
    }
}

/// Times one dashboard request.
///
/// The dashboard is server-bound and `home` installs nothing until all six
/// requests finish, so the slowest one sets the first frame. Which one that is
/// varies with the library, and it is not guessable from the client -- run with
/// `RUST_LOG=jellybridge=debug` to see it.
async fn timed<T>(request: &'static str, work: impl Future<Output = T>) -> T {
    let started = std::time::Instant::now();
    let value = work.await;
    tracing::debug!(
        ms = started.elapsed().as_secs_f64() * 1000.0,
        request,
        "dashboard_request"
    );
    value
}

/// The one place an HTTP client is built.
///
/// The caching resolver is the reason this is shared: hyper resolves once per
/// connection with no in-flight deduplication, so the dashboard's six parallel
/// requests each paid the same lookup. See `services::dns`.
fn build_http_client() -> Result<reqwest::Client, ApiError> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(30))
        .dns_resolver(crate::services::dns::shared())
        .build()?)
}

impl JellyfinApi {
    pub fn from_session(session: StoredSession) -> Result<Self, ApiError> {
        Self::from_session_with(session, build_http_client()?)
    }

    /// Builds an API for a session over an existing client, so sign-in does not
    /// throw away the connection it just opened (and warmed the DNS cache with)
    /// before the dashboard fetch starts on an empty pool.
    fn from_session_with(session: StoredSession, http: reqwest::Client) -> Result<Self, ApiError> {
        let server = normalize_server_url(&session.server_url)?;
        let authorization = authorization_value(&session.token)?;
        Ok(Self {
            http,
            session,
            server,
            authorization,
        })
    }

    pub async fn authenticate(
        server: &str,
        username: &str,
        password: &str,
    ) -> Result<(Self, String), ApiError> {
        let server = normalize_server_url(server)?;
        let username = username.trim();
        if username.is_empty() {
            return Err(ApiError::MissingUsername);
        }
        let provisional = StoredSession {
            server_url: server.as_str().trim_end_matches('/').to_owned(),
            token: String::new(),
            user_id: String::new(),
        };
        let api = Self::from_session(provisional)?;
        let response: AuthResponse = api
            .send_json(
                api.request(Method::POST, "Users/AuthenticateByName")?
                    .json(&AuthRequest { username, password }),
                "authenticating",
            )
            .await?;
        let session = StoredSession {
            server_url: api.session.server_url,
            token: response.access_token,
            user_id: response.user.id,
        };
        let account_name = response.user.name;
        Ok((Self::from_session_with(session, api.http)?, account_name))
    }

    /// An item's local trailers, as item ids to play.
    pub async fn local_trailers(&self, item_id: &str) -> Result<Vec<String>, ApiError> {
        let items: Vec<LibraryItem> = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!(
                        "Users/{}/Items/{}/LocalTrailers",
                        encode_path_segment(&self.session.user_id),
                        encode_path_segment(item_id)
                    ),
                )?,
                "loading trailers",
            )
            .await?;
        Ok(items.into_iter().map(|item| item.id).collect())
    }

    /// Whether the server still accepts this session's token: `Ok(false)`
    /// only on a definite 401 for the signed-in user.
    pub async fn session_is_valid(&self) -> Result<bool, ApiError> {
        let response = self.request(Method::GET, "Users/Me")?.send().await?;
        match check_status(response, "checking the sign-in") {
            Ok(_) => Ok(true),
            Err(ApiError::Unauthorized) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn session(&self) -> &StoredSession {
        &self.session
    }

    /// The home screen as jellyfin-web builds it: the user's settings, then
    /// the sections in their Home order with the requests jellyfin-web
    /// makes.
    ///
    /// Everything starts at once. Next Up assumes jellyfin-web's default
    /// cutoff and is only asked again if the user changed it, so launch does
    /// not wait for the settings. Music libraries' shelves are left out:
    /// they sit below the first screen, and their query competes on the
    /// server with Next Up, the slowest request, which held launch back by
    /// ~50 ms. They follow through [`Self::deferred_home_sections`].
    pub async fn home(&self) -> Result<HomeData, ApiError> {
        use crate::features::prefs::HomeSection;
        let default_prefs = UserPrefs::default();
        let libraries_and_latest = async {
            let libraries = timed("libraries", self.libraries()).await?;
            let latest = futures_util::future::join_all(
                libraries
                    .iter()
                    .filter(|library| has_latest_shelf(library) && !defers_latest_shelf(library))
                    .map(|library| async move {
                        (
                            library.id.clone(),
                            timed("latest", self.latest_in(&library.id, 16)).await,
                        )
                    }),
            )
            .await;
            Ok::<_, ApiError>((libraries, latest))
        };
        let (libraries_and_latest, prefs, resume, resume_audio, resume_book, next_up) = tokio::join!(
            libraries_and_latest,
            timed("prefs", self.user_prefs()),
            timed("resume", self.resume_items("Video")),
            timed("resume_audio", self.resume_items("Audio")),
            timed("resume_book", self.resume_items("Book")),
            timed("next_up", self.next_up(&default_prefs)),
        );
        let (libraries, latest) = libraries_and_latest?;
        let prefs = prefs.unwrap_or_else(|error| {
            tracing::warn!(%error, "user settings could not be loaded");
            default_prefs.clone()
        });
        let next_up = if prefs.next_up_days() != default_prefs.next_up_days()
            || prefs.rewatching_in_next_up() != default_prefs.rewatching_in_next_up()
        {
            self.next_up(&prefs).await
        } else {
            next_up
        };
        let mut resume = Some(resume);
        let mut resume_audio = Some(resume_audio);
        let mut resume_book = Some(resume_book);
        let mut next_up = Some(next_up);
        let mut latest = Some(latest);
        let names: HashMap<String, String> = libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect();
        let mut library_rows = Some(
            libraries
                .into_iter()
                .filter(|library| !prefs.my_media_excludes.contains(&library.id))
                .map(CatalogItemData::from)
                .collect::<Vec<_>>(),
        );
        let mut sections = Vec::new();
        for section in prefs.home_sections() {
            match section {
                HomeSection::LibraryTiles | HomeSection::LibraryButtons => {
                    if let Some(items) = library_rows.take() {
                        sections.push(CatalogSectionData {
                            id: "libraries".into(),
                            title: "My Media".into(),
                            items,
                        });
                    }
                }
                HomeSection::Resume => {
                    if let Some(items) = resume.take() {
                        append_optional_section(
                            &mut sections,
                            "continue",
                            "Continue Watching",
                            items,
                        );
                    }
                }
                HomeSection::ResumeAudio => {
                    if let Some(items) = resume_audio.take() {
                        append_optional_section(
                            &mut sections,
                            "continue-audio",
                            "Continue Listening",
                            items,
                        );
                    }
                }
                HomeSection::ResumeBook => {
                    if let Some(items) = resume_book.take() {
                        append_optional_section(
                            &mut sections,
                            "continue-book",
                            "Continue Reading",
                            items,
                        );
                    }
                }
                HomeSection::NextUp => {
                    if let Some(items) = next_up.take() {
                        append_optional_section(&mut sections, "next-up", "Next Up", items);
                    }
                }
                HomeSection::LatestMedia => {
                    for (library_id, items) in latest.take().into_iter().flatten() {
                        if prefs.latest_excludes.contains(&library_id) {
                            continue;
                        }
                        let name = names.get(&library_id).cloned().unwrap_or_default();
                        append_optional_section(
                            &mut sections,
                            &format!("latest-{library_id}"),
                            &format!("Recently Added in {name}"),
                            items,
                        );
                    }
                }
                HomeSection::ActiveRecordings | HomeSection::LiveTv | HomeSection::None => {}
            }
        }
        // "Use episode images in 'Next Up' and 'Continue Watching'": the
        // episode's own image instead of the show's (no `inheritThumb`).
        if prefs.use_episode_images() {
            for section in sections
                .iter_mut()
                .filter(|section| section.id == "continue" || section.id == "next-up")
            {
                for item in &mut section.items {
                    item.images.series_thumb = None;
                    item.images.parent_thumb = None;
                }
            }
        }
        Ok(HomeData { sections, prefs })
    }

    /// The user's server-side settings (display preferences and user
    /// configuration), read together.
    pub async fn user_prefs(&self) -> Result<UserPrefs, ApiError> {
        let display = self.send_json::<serde_json::Value>(
            self.request(Method::GET, "DisplayPreferences/usersettings")?
                .query(&[
                    ("userId", self.session.user_id.as_str()),
                    ("client", "emby"),
                ]),
            "loading display preferences",
        );
        let user = self.send_json::<serde_json::Value>(
            self.request(
                Method::GET,
                &format!("Users/{}", encode_path_segment(&self.session.user_id)),
            )?,
            "loading user settings",
        );
        let (display, user) = tokio::join!(display, user);
        let display = display?;
        let user = user?;
        let strings = |value: &serde_json::Value| -> Vec<String> {
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        };
        let config = &user["Configuration"];
        Ok(UserPrefs {
            custom: display["CustomPrefs"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                .collect(),
            hide_played_in_latest: config["HidePlayedInLatest"].as_bool().unwrap_or(true),
            my_media_excludes: strings(&config["MyMediaExcludes"]),
            latest_excludes: strings(&config["LatestItemsExcludes"]),
            next_episode_autoplay: config["EnableNextEpisodeAutoPlay"]
                .as_bool()
                .unwrap_or(true),
        })
    }

    /// Saves one of jellyfin-web's display preferences (a library sort, a
    /// player option), keeping all the others.
    pub async fn save_custom_pref(&self, key: &str, value: &str) -> Result<(), ApiError> {
        let query = [
            ("userId", self.session.user_id.as_str()),
            ("client", "emby"),
        ];
        let mut display: serde_json::Value = self
            .send_json(
                self.request(Method::GET, "DisplayPreferences/usersettings")?
                    .query(&query),
                "loading display preferences",
            )
            .await?;
        display["CustomPrefs"][key] = serde_json::Value::String(value.to_owned());
        let response = self
            .request(Method::POST, "DisplayPreferences/usersettings")?
            .query(&query)
            .json(&display)
            .send()
            .await?;
        check_status(response, "saving display preferences")?;
        Ok(())
    }

    /// The home shelves [`Self::home`] leaves for later: "Recently Added in"
    /// each music library among `libraries` (the My Media rows).
    pub async fn deferred_home_sections(
        &self,
        libraries: &[CatalogItemData],
    ) -> Vec<CatalogSectionData> {
        let deferred = libraries
            .iter()
            .filter(|library| library.collection_type == "music")
            .collect::<Vec<_>>();
        let latest = futures_util::future::join_all(
            deferred
                .iter()
                .map(|library| self.latest_in(&library.id, 30)),
        )
        .await;
        let mut sections = Vec::new();
        for (library, items) in deferred.into_iter().zip(latest) {
            append_optional_section(
                &mut sections,
                &format!("latest-{}", library.id),
                &format!("Recently Added in {}", library.title),
                items,
            );
        }
        sections
    }

    pub async fn detail_page(&self, item_id: &str) -> Result<DetailPageData, ApiError> {
        let item = self.detail_item(item_id).await?;
        let kind = DetailKind::from_jellyfin(&item.kind);
        // "More Like This" on series and movie pages, fetched alongside the
        // children. Like the season extras it degrades silently.
        let similar = async {
            if !matches!(kind, DetailKind::Series | DetailKind::Movie) {
                return Vec::new();
            }
            self.similar_items(item_id).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "could not load similar items");
                Vec::new()
            })
        };
        let children = async {
            match kind {
                DetailKind::Series => {
                    let (seasons, episodes) =
                        tokio::join!(self.series_seasons(item_id), self.episodes(item_id, true),);
                    combine_detail_children(seasons, episodes)
                }
                // A season page also lists its series' other seasons ("More
                // Seasons"). That shelf is an extra: failing it must not raise
                // the page's error banner the way failed episodes do.
                DetailKind::Season => {
                    let other_seasons = async {
                        match item.series_id.as_deref() {
                            Some(series_id) => self.series_seasons(series_id).await,
                            None => Ok(Vec::new()),
                        }
                    };
                    let (episodes, seasons) =
                        tokio::join!(self.episodes(item_id, false), other_seasons);
                    let seasons = seasons.unwrap_or_else(|error| {
                        tracing::warn!(%error, "could not load the series' other seasons");
                        Vec::new()
                    });
                    match episodes {
                        Ok(episodes) => (seasons, episodes, None),
                        Err(error) => (seasons, Vec::new(), Some(error.to_string())),
                    }
                }
                // An episode page shows its season's episodes ("More from
                // Season N"); failure only degrades the shelf.
                DetailKind::Episode => match item.season_id.as_deref() {
                    Some(season_id) => match self.episodes(season_id, false).await {
                        Ok(episodes) => (Vec::new(), episodes, None),
                        Err(error) => (Vec::new(), Vec::new(), Some(error.to_string())),
                    },
                    None => (Vec::new(), Vec::new(), None),
                },
                _ => (Vec::new(), Vec::new(), None),
            }
        };
        let sections = async {
            match kind {
                DetailKind::Person => self.person_sections(item_id).await,
                DetailKind::BoxSet => self.collection_sections(item_id).await,
                DetailKind::Playlist => self.playlist_sections(item_id).await,
                _ => Ok(Vec::new()),
            }
        };
        let (((seasons, episodes, children_error), similar), sections) =
            tokio::join!(async { tokio::join!(children, similar) }, sections);
        let (sections, sections_error) = match sections {
            Ok(sections) => (sections, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        Ok(DetailPageData {
            item: item.into(),
            seasons: seasons.into_iter().map(Into::into).collect(),
            episodes: episodes.into_iter().map(Into::into).collect(),
            similar: similar.into_iter().map(Into::into).collect(),
            sections,
            children_error: children_error.or(sections_error),
        })
    }

    /// A person's filmography: movies, shows and episodes, newest first.
    async fn person_sections(&self, person_id: &str) -> Result<Vec<DetailSection>, ApiError> {
        const KINDS: [(&str, &str, bool); 3] = [
            ("Movie", "Movies", false),
            ("Series", "Shows", false),
            ("Episode", "Episodes", true),
        ];
        let path = format!("Users/{}/Items", encode_path_segment(&self.session.user_id));
        let path = &path;
        let results = futures_util::future::join_all(KINDS.iter().map(|(kind, _, _)| async move {
            let query = [
                ("PersonIds", person_id),
                ("IncludeItemTypes", kind),
                ("Recursive", "true"),
                ("SortBy", "PremiereDate,ProductionYear,SortName"),
                ("SortOrder", "Descending"),
                ("Limit", "100"),
                ("EnableTotalRecordCount", "false"),
            ];
            self.items_request(path, &query, "loading a filmography")
                .await
        }))
        .await;
        let mut sections = Vec::new();
        for ((_, title, wide), items) in KINDS.iter().zip(results) {
            let items = items?;
            if !items.is_empty() {
                sections.push(DetailSection {
                    title: (*title).to_owned(),
                    wide: *wide,
                    list: false,
                    items: items.into_iter().map(Into::into).collect(),
                });
            }
        }
        Ok(sections)
    }

    /// A collection's items, one section per kind as jellyfin-web groups
    /// them.
    async fn collection_sections(&self, id: &str) -> Result<Vec<DetailSection>, ApiError> {
        let items = self
            .items_request(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                &[
                    ("ParentId", id),
                    ("SortBy", "PremiereDate,ProductionYear,SortName"),
                    ("SortOrder", "Ascending"),
                ],
                "loading a collection",
            )
            .await?;
        let mut sections: Vec<DetailSection> = Vec::new();
        for item in items {
            let title = match item.kind.as_str() {
                "Movie" => "Movies",
                "Series" => "Shows",
                "Episode" => "Episodes",
                "Season" => "Seasons",
                _ => "Items",
            };
            let wide = item.kind == "Episode";
            match sections.iter_mut().find(|section| section.title == title) {
                Some(section) => section.items.push(item.into()),
                None => sections.push(DetailSection {
                    title: title.to_owned(),
                    wide,
                    list: false,
                    items: vec![item.into()],
                }),
            }
        }
        Ok(sections)
    }

    /// A playlist's entries, in playlist order, as one list.
    async fn playlist_sections(&self, id: &str) -> Result<Vec<DetailSection>, ApiError> {
        let items = self
            .items_request(
                &format!("Playlists/{}/Items", encode_path_segment(id)),
                &[("UserId", self.session.user_id.as_str())],
                "loading a playlist",
            )
            .await?;
        Ok(vec![DetailSection {
            title: String::new(),
            wide: true,
            list: true,
            items: items.into_iter().map(Into::into).collect(),
        }])
    }

    /// The server's "More Like This" for an item.
    async fn similar_items(&self, item_id: &str) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!("Items/{}/Similar", encode_path_segment(item_id)),
            &[("UserId", self.session.user_id.as_str()), ("Limit", "12")],
            "loading similar items",
        )
        .await
    }

    /// Tells the server playback began, which makes this client the item's
    /// active session so other clients see what is playing.
    pub async fn report_playback_start(
        &self,
        target: &ReportTarget,
        position_seconds: f64,
    ) -> Result<(), ApiError> {
        self.report(
            "Sessions/Playing",
            PlaybackReportBody::playing(target, position_seconds, false, None),
            "reporting playback start",
        )
        .await
    }

    /// Heartbeats the current position so the server's resume point tracks the
    /// player rather than only updating when playback stops.
    pub async fn report_playback_progress(
        &self,
        target: &ReportTarget,
        position_seconds: f64,
        paused: bool,
    ) -> Result<(), ApiError> {
        let event = if paused { "pause" } else { "timeupdate" };
        self.report(
            "Sessions/Playing/Progress",
            PlaybackReportBody::playing(target, position_seconds, paused, Some(event)),
            "reporting playback progress",
        )
        .await
    }

    /// Ends the session and commits the final position. Jellyfin decides from
    /// this whether the item counts as watched.
    pub async fn report_playback_stopped(
        &self,
        target: &ReportTarget,
        position_seconds: f64,
    ) -> Result<(), ApiError> {
        self.report(
            "Sessions/Playing/Stopped",
            PlaybackReportBody::stopped(target, position_seconds),
            "reporting playback stop",
        )
        .await
    }

    async fn report(
        &self,
        path: &str,
        body: PlaybackReportBody<'_>,
        operation: &'static str,
    ) -> Result<(), ApiError> {
        // `JELLYBRIDGE_NO_PLAYBACK_REPORTS`: automated test and measurement
        // runs play real media from a real account, and must not move its
        // resume points or mark anything watched.
        static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DISABLED.get_or_init(|| std::env::var_os("JELLYBRIDGE_NO_PLAYBACK_REPORTS").is_some()) {
            tracing::debug!(operation, "playback reporting disabled");
            return Ok(());
        }
        let response = self.request(Method::POST, path)?.json(&body).send().await?;
        check_status(response, operation)?;
        Ok(())
    }

    pub async fn set_watched(&self, item_id: &str, watched: bool) -> Result<(), ApiError> {
        let method = if watched {
            Method::POST
        } else {
            Method::DELETE
        };
        let response = self
            .request(
                method,
                &format!("UserPlayedItems/{}", encode_path_segment(item_id)),
            )?
            .send()
            .await?;
        check_status(response, "updating watched state")?;
        Ok(())
    }

    pub async fn set_favorite(&self, item_id: &str, favorite: bool) -> Result<(), ApiError> {
        let method = if favorite {
            Method::POST
        } else {
            Method::DELETE
        };
        let response = self
            .request(
                method,
                &format!("UserFavoriteItems/{}", encode_path_segment(item_id)),
            )?
            .send()
            .await?;
        check_status(response, "updating favorite state")?;
        Ok(())
    }

    /// The episodes either side of `item_id` in its series, across season
    /// boundaries, for the player's previous/next and Up Next.
    pub async fn adjacent_episodes(
        &self,
        series_id: &str,
        item_id: &str,
    ) -> Result<(Option<DetailItemData>, Option<DetailItemData>), ApiError> {
        let response: ItemsResponse = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!("Shows/{}/Episodes", encode_path_segment(series_id)),
                )?
                .query(&[
                    ("UserId", self.session.user_id.as_str()),
                    ("AdjacentTo", item_id),
                    ("Fields", DETAIL_FIELDS),
                    ("EnableUserData", "true"),
                    ("EnableImages", "true"),
                    ("EnableImageTypes", "Primary,Thumb,Backdrop"),
                ]),
                "loading adjacent episodes",
            )
            .await?;
        let episodes = response
            .items
            .into_iter()
            .map(DetailItemData::from)
            .collect();
        Ok(split_adjacent(episodes, item_id))
    }

    /// The item's intro/credits/recap segments. Servers before Jellyfin 10.10
    /// have no such endpoint, which reads as "no segments".
    pub async fn media_segments(&self, item_id: &str) -> Result<Vec<MediaSegment>, ApiError> {
        let response = self
            .request(
                Method::GET,
                &format!("MediaSegments/{}", encode_path_segment(item_id)),
            )?
            .send()
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let response = check_status(response, "loading media segments")?;
        let segments: SegmentsResponse = response.json().await?;
        Ok(segments.into_segments())
    }

    pub async fn fetch_detail_image(
        &self,
        image: &ImageRef,
        max_width: u32,
    ) -> Result<Vec<u8>, ApiError> {
        let suffix = image
            .index
            .map(|index| format!("/{index}"))
            .unwrap_or_default();
        let response = self
            .request(
                Method::GET,
                &format!(
                    "Items/{}/Images/{}{}",
                    encode_path_segment(&image.item_id),
                    image.image_type.jellyfin_name(),
                    suffix
                ),
            )?
            .query(&[
                ("maxWidth", max_width.to_string()),
                ("quality", "90".into()),
                (
                    "format",
                    if image.image_type == ImageType::Logo {
                        "Png".into()
                    } else {
                        "Jpg".into()
                    },
                ),
            ])
            .send()
            .await?;
        let response = check_status(response, "loading detail artwork")?;
        Ok(response.bytes().await?.to_vec())
    }

    /// A card image as jellyfin-web asks for it: filled to an exact size at
    /// quality 96, in whatever format the server prefers for a browser
    /// (WebP), so the pixels are the ones jellyfin-web shows.
    /// An image with jellyfin-web's detail-page parameters: an optional
    /// width cap and a quality; the format follows the Accept header.
    pub async fn fetch_web_image(
        &self,
        image: &ImageRef,
        max_width: Option<u32>,
        quality: u8,
    ) -> Result<Vec<u8>, ApiError> {
        let suffix = image
            .index
            .map(|index| format!("/{index}"))
            .unwrap_or_default();
        let mut query = vec![("tag", image.tag.clone())];
        if let Some(width) = max_width {
            query.push(("maxWidth", width.to_string()));
        }
        query.push(("quality", quality.to_string()));
        let response = self
            .request(
                Method::GET,
                &format!(
                    "Items/{}/Images/{}{}",
                    encode_path_segment(&image.item_id),
                    image.image_type.jellyfin_name(),
                    suffix
                ),
            )?
            .header(
                reqwest::header::ACCEPT,
                "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            )
            .query(&query)
            .send()
            .await?;
        let response = check_status(response, "loading artwork")?;
        Ok(response.bytes().await?.to_vec())
    }

    pub async fn fetch_card_image(
        &self,
        image: &ImageRef,
        fill_width: u32,
        fill_height: u32,
    ) -> Result<Vec<u8>, ApiError> {
        let suffix = image
            .index
            .filter(|index| *index > 0)
            .map(|index| format!("/{index}"))
            .unwrap_or_default();
        let response = self
            .request(
                Method::GET,
                &format!(
                    "Items/{}/Images/{}{}",
                    encode_path_segment(&image.item_id),
                    image.image_type.jellyfin_name(),
                    suffix
                ),
            )?
            .header(
                reqwest::header::ACCEPT,
                "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            )
            .query(&[
                ("fillHeight", fill_height.to_string()),
                ("fillWidth", fill_width.to_string()),
                ("quality", "96".to_owned()),
                ("tag", image.tag.clone()),
            ])
            .send()
            .await?;
        let response = check_status(response, "loading artwork")?;
        Ok(response.bytes().await?.to_vec())
    }

    async fn detail_item(&self, item_id: &str) -> Result<LibraryItem, ApiError> {
        self.send_json(
            self.request(
                Method::GET,
                &format!(
                    "Users/{}/Items/{}",
                    encode_path_segment(&self.session.user_id),
                    encode_path_segment(item_id)
                ),
            )?
            .query(&[
                ("Fields", DETAIL_FIELDS),
                ("EnableUserData", "true"),
                ("EnableImages", "true"),
                ("EnableImageTypes", "Primary,Backdrop,Logo,Thumb"),
            ]),
            "loading item details",
        )
        .await
    }

    /// The user's libraries ("views") in their configured order, as My Media
    /// lists them.
    async fn libraries(&self) -> Result<Vec<LibraryItem>, ApiError> {
        let response: ItemsResponse = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!("Users/{}/Views", encode_path_segment(&self.session.user_id)),
                )?,
                "loading libraries",
            )
            .await?;
        Ok(response.items)
    }

    async fn series_seasons(&self, series_id: &str) -> Result<Vec<LibraryItem>, ApiError> {
        let mut response: ItemsResponse = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!("Shows/{}/Seasons", encode_path_segment(series_id)),
                )?
                .query(&[
                    ("UserId", self.session.user_id.as_str()),
                    ("Fields", DETAIL_FIELDS),
                    ("EnableUserData", "true"),
                    ("EnableImages", "true"),
                    ("EnableImageTypes", "Primary,Backdrop,Logo,Thumb"),
                ]),
                "loading series seasons",
            )
            .await?;
        sort_detail_items(&mut response.items);
        Ok(response.items)
    }

    async fn episodes(
        &self,
        parent_id: &str,
        recursive: bool,
    ) -> Result<Vec<LibraryItem>, ApiError> {
        let mut response: ItemsResponse = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                )?
                .query(&[
                    ("ParentId", parent_id),
                    ("Recursive", if recursive { "true" } else { "false" }),
                    ("IncludeItemTypes", "Episode"),
                    ("SortBy", "ParentIndexNumber,IndexNumber,SortName"),
                    ("SortOrder", "Ascending"),
                    ("Fields", DETAIL_FIELDS),
                    ("EnableUserData", "true"),
                    ("EnableImages", "true"),
                    ("EnableImageTypes", "Primary,Thumb,Backdrop"),
                ]),
                "loading season episodes",
            )
            .await?;
        sort_detail_items(&mut response.items);
        Ok(response.items)
    }

    /// Continue Watching / Listening / Reading, as jellyfin-web asks.
    async fn resume_items(&self, media_type: &str) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!(
                "Users/{}/Items/Resume",
                encode_path_segment(&self.session.user_id)
            ),
            &[
                ("Limit", "12"),
                ("Recursive", "true"),
                ("EnableTotalRecordCount", "false"),
                ("MediaTypes", media_type),
            ],
            "loading continue watching",
        )
        .await
    }

    /// Next Up with jellyfin-web's parameters: shows watched in the last
    /// `maxDaysForNextUp` days, no resumable episodes, rewatching as set.
    async fn next_up(&self, prefs: &UserPrefs) -> Result<Vec<LibraryItem>, ApiError> {
        let cutoff = iso_days_ago(prefs.next_up_days());
        let rewatching = if prefs.rewatching_in_next_up() {
            "true"
        } else {
            "false"
        };
        self.items_request(
            "Shows/NextUp",
            &[
                ("UserId", self.session.user_id.as_str()),
                ("Limit", "24"),
                ("EnableTotalRecordCount", "false"),
                ("DisableFirstEpisode", "false"),
                ("NextUpDateCutoff", cutoff.as_str()),
                ("EnableResumable", "false"),
                ("EnableRewatching", rewatching),
            ],
            "loading next up",
        )
        .await
    }

    /// The newest items in one library, as jellyfin-web's home asks for
    /// them (the server groups a batch of new episodes into their show).
    async fn latest_in(&self, library_id: &str, limit: u32) -> Result<Vec<LibraryItem>, ApiError> {
        let limit = limit.to_string();
        self.send_json(
            self.request(
                Method::GET,
                &format!(
                    "Users/{}/Items/Latest",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .query(&[
                ("ParentId", library_id),
                ("Fields", LIST_FIELDS),
                ("EnableUserData", "true"),
                ("EnableImages", "true"),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary,Backdrop,Thumb"),
                ("Limit", limit.as_str()),
            ]),
            "loading latest media",
        )
        .await
    }

    async fn items_request(
        &self,
        path: &str,
        extra_query: &[(&str, &str)],
        operation: &'static str,
    ) -> Result<Vec<LibraryItem>, ApiError> {
        Ok(self
            .items_page_request(path, extra_query, operation)
            .await?
            .items)
    }

    async fn items_page_request(
        &self,
        path: &str,
        extra_query: &[(&str, &str)],
        operation: &'static str,
    ) -> Result<ItemsResponse, ApiError> {
        self.items_page_request_with_fields(path, LIST_FIELDS, extra_query, operation)
            .await
    }

    async fn items_page_request_with_fields(
        &self,
        path: &str,
        fields: &'static str,
        extra_query: &[(&str, &str)],
        operation: &'static str,
    ) -> Result<ItemsResponse, ApiError> {
        let mut query = vec![
            ("Fields", fields),
            ("EnableUserData", "true"),
            ("EnableImages", "true"),
            ("ImageTypeLimit", "1"),
            ("EnableImageTypes", "Primary,Backdrop,Thumb"),
        ];
        query.extend_from_slice(extra_query);
        self.send_json(self.request(Method::GET, path)?.query(&query), operation)
            .await
    }

    fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder, ApiError> {
        let url = self
            .server
            .join(path)
            .map_err(|_| ApiError::InvalidServerUrl)?;
        Ok(self
            .http
            .request(method, url)
            .header("x-emby-authorization", self.authorization.clone()))
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        operation: &'static str,
    ) -> Result<T, ApiError> {
        let response = check_status(request.send().await?, operation)?;
        Ok(response.json().await?)
    }
}

/// Body shared by the three `Sessions/Playing` endpoints. Fields that do not
/// apply to a given endpoint are omitted rather than sent as defaults.
#[derive(Serialize)]
struct PlaybackReportBody<'a> {
    #[serde(rename = "ItemId")]
    item_id: &'a str,
    #[serde(rename = "MediaSourceId")]
    media_source_id: &'a str,
    #[serde(rename = "PlaySessionId", skip_serializing_if = "Option::is_none")]
    play_session_id: Option<&'a str>,
    #[serde(rename = "PositionTicks")]
    position_ticks: i64,
    #[serde(rename = "IsPaused", skip_serializing_if = "Option::is_none")]
    is_paused: Option<bool>,
    #[serde(rename = "CanSeek", skip_serializing_if = "Option::is_none")]
    can_seek: Option<bool>,
    #[serde(rename = "PlayMethod", skip_serializing_if = "Option::is_none")]
    play_method: Option<&'static str>,
    #[serde(rename = "EventName", skip_serializing_if = "Option::is_none")]
    event_name: Option<&'static str>,
}

impl<'a> PlaybackReportBody<'a> {
    fn playing(
        target: &'a ReportTarget,
        position_seconds: f64,
        paused: bool,
        event_name: Option<&'static str>,
    ) -> Self {
        Self {
            item_id: &target.item_id,
            media_source_id: &target.media_source_id,
            play_session_id: target.play_session_id.as_deref(),
            position_ticks: position_ticks(position_seconds),
            is_paused: Some(paused),
            can_seek: Some(true),
            // The proxy hands mpv the untranscoded source, so every session is
            // a direct play as far as the server is concerned.
            play_method: Some("DirectPlay"),
            event_name,
        }
    }

    fn stopped(target: &'a ReportTarget, position_seconds: f64) -> Self {
        Self {
            item_id: &target.item_id,
            media_source_id: &target.media_source_id,
            play_session_id: target.play_session_id.as_deref(),
            position_ticks: position_ticks(position_seconds),
            is_paused: None,
            can_seek: None,
            play_method: None,
            event_name: None,
        }
    }
}

fn check_status(
    response: reqwest::Response,
    operation: &'static str,
) -> Result<reqwest::Response, ApiError> {
    if !response.status().is_success() {
        // The path only: queries can carry ids, never the token (a header).
        tracing::warn!(
            status = response.status().as_u16(),
            path = response.url().path(),
            operation,
            "request failed"
        );
    }
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if !response.status().is_success() {
        return Err(ApiError::Status {
            status: response.status(),
            operation,
        });
    }
    Ok(response)
}

fn combine_detail_children(
    seasons: Result<Vec<LibraryItem>, ApiError>,
    episodes: Result<Vec<LibraryItem>, ApiError>,
) -> (Vec<LibraryItem>, Vec<LibraryItem>, Option<String>) {
    let mut errors = Vec::new();
    let seasons = seasons.unwrap_or_else(|error| {
        errors.push(error.to_string());
        Vec::new()
    });
    let episodes = episodes.unwrap_or_else(|error| {
        errors.push(error.to_string());
        Vec::new()
    });
    let error = (!errors.is_empty()).then(|| errors.join("; "));
    (seasons, episodes, error)
}

fn sort_detail_items(items: &mut [LibraryItem]) {
    items.sort_by(|left, right| {
        left.parent_index_number
            .unwrap_or(i32::MAX)
            .cmp(&right.parent_index_number.unwrap_or(i32::MAX))
            .then_with(|| {
                left.index_number
                    .unwrap_or(i32::MAX)
                    .cmp(&right.index_number.unwrap_or(i32::MAX))
            })
            // Same ordering as comparing two lowercased Strings, without
            // allocating one per comparison.
            .then_with(|| {
                left.name
                    .chars()
                    .flat_map(char::to_lowercase)
                    .cmp(right.name.chars().flat_map(char::to_lowercase))
            })
    });
}

fn append_optional_section(
    sections: &mut Vec<CatalogSectionData>,
    id: &str,
    title: &str,
    result: Result<Vec<LibraryItem>, ApiError>,
) {
    match result {
        Ok(items) if !items.is_empty() => sections.push(CatalogSectionData {
            id: id.to_owned(),
            title: title.to_owned(),
            items: items.into_iter().map(Into::into).collect(),
        }),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, section = title, "dashboard section failed"),
    }
}

/// The one Jellyfin client identity this application reports, shared by the
/// browsing client and the playback pipeline.
pub fn authorization_value(token: &str) -> Result<HeaderValue, InvalidHeaderValue> {
    let token = token.replace(['\\', '"'], "");
    let value = if token.is_empty() {
        format!(
            "MediaBrowser Client=\"{CLIENT_NAME}\", Device=\"{DEVICE_NAME}\", DeviceId=\"{DEVICE_ID}\", Version=\"{CLIENT_VERSION}\""
        )
    } else {
        format!(
            "MediaBrowser Client=\"{CLIENT_NAME}\", Device=\"{DEVICE_NAME}\", DeviceId=\"{DEVICE_ID}\", Version=\"{CLIENT_VERSION}\", Token=\"{token}\""
        )
    };
    HeaderValue::from_str(&value)
}

pub fn normalize_server_url(value: &str) -> Result<Url, ApiError> {
    let mut url = Url::parse(value.trim()).map_err(|_| ApiError::InvalidServerUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::InvalidServerUrl);
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

impl From<LibraryItem> for CatalogItemData {
    fn from(item: LibraryItem) -> Self {
        let progress = item
            .user_data
            .as_ref()
            .and_then(|user| user.played_percentage)
            .map(|value| (value as f32 / 100.0).clamp(0.0, 1.0))
            .or_else(|| {
                let position = item.user_data.as_ref()?.playback_position_ticks as f64;
                let runtime = item.run_time_ticks? as f64;
                (runtime > 0.0).then(|| (position / runtime) as f32)
            })
            .unwrap_or(0.0);
        let playable = matches!(item.kind.as_str(), "Movie" | "Episode");
        // Match the Kotlin browser's behavior: every non-playable Jellyfin item is
        // navigable. This also covers server-provided container types such as
        // ManualPlaylistsFolder and future container variants.
        let (card_title, card_subtitle) = card_lines(&item);
        let year = year_line(&item);
        let (title_link, subtitle_link) = card_links(&item);
        let images = card_images(&item);
        let user = item.user_data.as_ref();
        Self {
            watched: user.is_some_and(|data| data.played),
            favorite: user.is_some_and(|data| data.is_favorite),
            unplayed_count: user
                .and_then(|data| data.unplayed_item_count)
                .filter(|count| *count > 0)
                .map_or(0, |count| count as u32),
            id: item.id,
            title: item.name,
            kind: item.kind,
            progress,
            playable,
            card_title,
            card_subtitle,
            year,
            title_link,
            subtitle_link,
            images,
            series_id: item.series_id.clone(),
            season_id: item.season_id.clone(),
            child_count: item
                .child_count
                .filter(|count| *count > 0)
                .map_or(0, |count| count as u32),
            collection_type: item.collection_type.unwrap_or_default(),
        }
    }
}

impl From<LibraryItem> for DetailItemData {
    fn from(item: LibraryItem) -> Self {
        let kind = DetailKind::from_jellyfin(&item.kind);
        let progress = item
            .user_data
            .as_ref()
            .and_then(|user| user.played_percentage)
            .map(|value| (value as f32 / 100.0).clamp(0.0, 1.0))
            .or_else(|| {
                let position = item.user_data.as_ref()?.playback_position_ticks as f64;
                let runtime = item.run_time_ticks? as f64;
                (runtime > 0.0).then(|| (position / runtime) as f32)
            })
            .unwrap_or(0.0);
        let poster = detail_poster(&item);
        let backdrop = detail_backdrop(&item);
        // Episodes and seasons rarely have their own logo; fall back to the
        // series logo like Jellyfin web does.
        let logo = own_image(&item, "Logo", ImageType::Logo).or_else(|| {
            item.parent_logo_item_id
                .as_ref()
                .zip(item.parent_logo_image_tag.as_ref())
                .map(|(id, tag)| ImageRef {
                    item_id: id.clone(),
                    image_type: ImageType::Logo,
                    index: None,
                    tag: tag.clone(),
                })
        });
        let thumbnail = own_image(&item, "Thumb", ImageType::Thumb)
            .or_else(|| own_image(&item, "Primary", ImageType::Primary));
        let media_summary = media_summary(&item.media_streams);
        let streams = item
            .media_streams
            .iter()
            .filter(|stream| {
                stream.kind.eq_ignore_ascii_case("audio")
                    || stream.kind.eq_ignore_ascii_case("subtitle")
            })
            .map(|stream| StreamData {
                index: stream.index,
                kind: stream.kind.clone(),
                label: stream_label(stream),
                language: stream.language.clone(),
                title: stream.display_title.clone(),
                codec: stream.codec.clone(),
                is_default: stream.is_default,
            })
            .collect();
        let directors = person_names(&item.people, "Director");
        let writers = person_names(&item.people, "Writer");
        // Cast & Crew lists everyone in the server's order (actors, then
        // the crew); guest stars have their own row.
        let (guest_stars, people): (Vec<_>, Vec<_>) = item
            .people
            .into_iter()
            .partition(|person| person.kind.eq_ignore_ascii_case("GuestStar"));
        let to_person = |person: BaseItemPerson| {
            let image = person
                .id
                .as_ref()
                .zip(person.primary_image_tag.as_ref())
                .map(|(id, tag)| ImageRef {
                    item_id: id.clone(),
                    image_type: ImageType::Primary,
                    index: None,
                    tag: tag.clone(),
                });
            // jellyfin-web's card line: "as <role>" for a credited role,
            // otherwise what the person did ("Director", "Writer").
            let role = if person.role.trim().is_empty() {
                person.kind
            } else {
                format!("as {}", person.role.trim())
            };
            PersonData {
                id: person.id,
                name: person.name,
                role,
                image,
            }
        };
        let people = people.into_iter().take(24).map(to_person).collect();
        let guest_stars = guest_stars.into_iter().take(24).map(to_person).collect();
        Self {
            id: item.id,
            title: item.name,
            kind,
            parent_id: item.parent_id,
            series_id: item.series_id,
            series_name: item.series_name,
            season_id: item.season_id,
            season_name: item.season_name,
            index_number: item.index_number,
            parent_index_number: item.parent_index_number,
            overview: item.overview.unwrap_or_default(),
            tagline: item
                .taglines
                .into_iter()
                .find(|tagline| !tagline.is_empty())
                .unwrap_or_default(),
            production_year: item.production_year,
            official_rating: item.official_rating,
            community_rating: item.community_rating.map(|rating| rating as f32),
            critic_rating: item.critic_rating.map(|rating| rating as f32),
            runtime_seconds: item
                .run_time_ticks
                .map(|ticks| ticks.max(0) as f64 / 10_000_000.0),
            // `GenreItems` carries the ids the links need; older servers
            // only send the names.
            genres: if item.genre_items.is_empty() {
                item.genres
                    .into_iter()
                    .map(|name| NamedId { name, id: None })
                    .collect()
            } else {
                item.genre_items.into_iter().map(Into::into).collect()
            },
            birth_place: item.production_locations.into_iter().next(),
            end_date: item.end_date.as_deref().and_then(parse_iso_date),
            playlist_item_id: item.playlist_item_id,
            tags: item.tags,
            external_urls: item
                .external_urls
                .into_iter()
                .filter(|link| !link.name.is_empty() && !link.url.is_empty())
                .map(|link| (link.name, link.url))
                .collect(),
            directors,
            writers,
            studios: item
                .studios
                .into_iter()
                .filter(|studio| !studio.name.is_empty())
                .map(Into::into)
                .collect(),
            people,
            guest_stars,
            premiere_date: item.premiere_date.as_deref().and_then(parse_iso_date),
            end_year: item
                .end_date
                .as_deref()
                .and_then(parse_iso_date)
                .map(|(year, _, _)| year),
            local_trailers: item.local_trailer_count,
            trailer_urls: item
                .remote_trailers
                .iter()
                .filter_map(|trailer| trailer.url.clone())
                .filter(|url| url.starts_with("http"))
                .collect(),
            series_continuing: item
                .status
                .as_deref()
                .is_some_and(|status| status.eq_ignore_ascii_case("Continuing")),
            media_summary,
            poster,
            backdrop,
            logo,
            thumbnail,
            watched: item.user_data.as_ref().is_some_and(|data| data.played),
            favorite: item.user_data.as_ref().is_some_and(|data| data.is_favorite),
            progress,
            resume_seconds: item
                .user_data
                .as_ref()
                .map(|data| data.playback_position_ticks.max(0) as f64 / 10_000_000.0)
                .unwrap_or(0.0),
            unplayed_count: item
                .user_data
                .as_ref()
                .and_then(|data| data.unplayed_item_count),
            playable: matches!(kind, DetailKind::Movie | DetailKind::Episode),
            streams,
        }
    }
}

/// The calendar date of an ISO 8601 timestamp such as
/// "2025-01-31T00:00:00.0000000Z", as (year, month, day).
fn parse_iso_date(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.get(..10)?.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts
        .next()?
        .parse()
        .ok()
        .filter(|month| (1..=12).contains(month))?;
    let day = parts
        .next()?
        .parse()
        .ok()
        .filter(|day| (1..=31).contains(day))?;
    Some((year, month, day))
}

fn person_names(people: &[BaseItemPerson], kind: &str) -> Vec<NamedId> {
    people
        .iter()
        .filter(|person| person.kind.eq_ignore_ascii_case(kind))
        .filter(|person| !person.name.is_empty())
        .map(|person| NamedId {
            name: person.name.clone(),
            id: person.id.clone(),
        })
        .collect()
}

fn own_image(item: &LibraryItem, key: &str, image_type: ImageType) -> Option<ImageRef> {
    item.image_tags.get(key).map(|tag| ImageRef {
        item_id: item.id.clone(),
        image_type,
        index: None,
        tag: tag.clone(),
    })
}

fn detail_poster(item: &LibraryItem) -> Option<ImageRef> {
    if item.kind == "Episode"
        && let (Some(series_id), Some(tag)) = (&item.series_id, &item.series_primary_image_tag)
    {
        return Some(ImageRef {
            item_id: series_id.clone(),
            image_type: ImageType::Primary,
            index: None,
            tag: tag.clone(),
        });
    }
    own_image(item, "Primary", ImageType::Primary).or_else(|| {
        item.parent_primary_image_item_id
            .as_ref()
            .map(|parent_id| ImageRef {
                item_id: parent_id.clone(),
                image_type: ImageType::Primary,
                index: None,
                tag: format!("parent-{parent_id}"),
            })
    })
}

fn detail_backdrop(item: &LibraryItem) -> Option<ImageRef> {
    if let Some(tag) = item.backdrop_image_tags.first() {
        return Some(ImageRef {
            item_id: item.id.clone(),
            image_type: ImageType::Backdrop,
            index: Some(0),
            tag: tag.clone(),
        });
    }
    item.parent_backdrop_item_id
        .as_ref()
        .zip(item.parent_backdrop_image_tags.first())
        .map(|(parent_id, tag)| ImageRef {
            item_id: parent_id.clone(),
            image_type: ImageType::Backdrop,
            index: Some(0),
            tag: tag.clone(),
        })
}

fn stream_label(stream: &ItemMediaStream) -> String {
    // Jellyfin's DisplayTitle is already a complete human label
    // ("Surround 5.1 - English - DTS-HD MA - Default"); decorating it with
    // language and codec again just duplicates words.
    if let Some(display_title) = stream
        .display_title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        return display_title.to_owned();
    }
    let codec = stream
        .codec
        .as_deref()
        .map(str::to_uppercase)
        .filter(|codec| !codec.is_empty())
        .map(|codec| match stream.channels.and_then(channel_layout) {
            Some(layout) => format!("{codec} {layout}"),
            None => codec,
        });
    [stream.language.as_deref().map(str::to_uppercase), codec]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" - ")
}

/// "5.1" for six channels, as Jellyfin names audio layouts.
fn channel_layout(channels: i32) -> Option<&'static str> {
    Some(match channels {
        1 => "Mono",
        2 => "Stereo",
        6 => "5.1",
        7 => "6.1",
        8 => "7.1",
        _ => return None,
    })
}

/// The Video line: the server's own title for the video stream ("4K HEVC
/// HDR10", "1080p H264 SDR"), which is what Jellyfin shows, or one built
/// from its parts.
fn media_summary(streams: &[ItemMediaStream]) -> String {
    let video = streams
        .iter()
        .find(|stream| stream.kind.eq_ignore_ascii_case("video"));
    if let Some(title) = video
        .and_then(|stream| stream.display_title.as_deref())
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        return title.to_owned();
    }
    let audio = streams
        .iter()
        .find(|stream| stream.kind.eq_ignore_ascii_case("audio"));
    let resolution = video.and_then(|stream| match (stream.width, stream.height) {
        (_, Some(height)) if height > 0 => Some(format!("{height}p")),
        (Some(width), _) if width > 0 => Some(format!("{width}px")),
        _ => None,
    });
    // SDR is the unremarkable default; Jellyfin only calls out HDR kinds.
    let range = video
        .and_then(|stream| stream.video_range_type.clone())
        .filter(|range| !range.is_empty() && !range.eq_ignore_ascii_case("SDR"));
    [
        resolution,
        video.and_then(|stream| stream.codec.as_deref().map(str::to_uppercase)),
        range,
        audio.and_then(|stream| stream.codec.as_deref().map(str::to_uppercase)),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" • ")
}

/// The images a card can pick from, as jellyfin-web's card builder sees an
/// item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardImages {
    pub primary: Option<ImageRef>,
    /// `PrimaryImageAspectRatio`: width over height.
    pub primary_aspect: Option<f32>,
    pub series_primary: Option<ImageRef>,
    pub album_primary: Option<ImageRef>,
    pub thumb: Option<ImageRef>,
    pub series_thumb: Option<ImageRef>,
    pub parent_thumb: Option<ImageRef>,
    pub backdrop: Option<ImageRef>,
    /// An episode's show backdrop.
    pub parent_backdrop: Option<ImageRef>,
    pub banner: Option<ImageRef>,
}

/// The image a card shows, and whether the card writes the item's name
/// over it (jellyfin-web does when a wide card falls back to a plain
/// backdrop, which has no logo).
#[derive(Clone, Debug, PartialEq)]
pub struct CardImage {
    pub image: ImageRef,
    pub force_name: bool,
    /// Width over height of a primary image, when the server knows it.
    pub aspect: Option<f32>,
}

impl CardImages {
    /// jellyfin-web's `getCardImageUrl` choice: wide cards prefer a thumb,
    /// banners a banner, everything else the primary image.
    pub fn pick(&self, prefer_thumb: bool, prefer_banner: bool) -> Option<CardImage> {
        let plain = |image: &Option<ImageRef>| {
            image.clone().map(|image| CardImage {
                image,
                force_name: false,
                aspect: None,
            })
        };
        if prefer_thumb && let Some(thumb) = plain(&self.thumb) {
            return Some(thumb);
        }
        if prefer_banner && let Some(banner) = plain(&self.banner) {
            return Some(banner);
        }
        if prefer_thumb {
            if let Some(thumb) = plain(&self.series_thumb).or_else(|| plain(&self.parent_thumb)) {
                return Some(thumb);
            }
            if let Some(backdrop) = self.backdrop.clone() {
                return Some(CardImage {
                    image: backdrop,
                    force_name: true,
                    aspect: None,
                });
            }
            if let Some(backdrop) = plain(&self.parent_backdrop) {
                return Some(backdrop);
            }
        }
        if let Some(primary) = self.primary.clone() {
            return Some(CardImage {
                image: primary,
                force_name: prefer_thumb,
                aspect: self.primary_aspect,
            });
        }
        plain(&self.series_primary).or_else(|| plain(&self.album_primary))
    }
}

fn card_images(item: &LibraryItem) -> CardImages {
    let image = |item_id: &str, image_type: ImageType, tag: &str| ImageRef {
        item_id: item_id.to_owned(),
        image_type,
        index: (image_type == ImageType::Backdrop).then_some(0),
        tag: tag.to_owned(),
    };
    CardImages {
        primary: item
            .image_tags
            .get("Primary")
            .map(|tag| image(&item.id, ImageType::Primary, tag)),
        primary_aspect: item
            .primary_image_aspect_ratio
            .filter(|ratio| *ratio > 0.0)
            .map(|ratio| ratio as f32),
        series_primary: item
            .series_id
            .as_deref()
            .zip(item.series_primary_image_tag.as_deref())
            .map(|(id, tag)| image(id, ImageType::Primary, tag)),
        album_primary: item
            .album_id
            .as_deref()
            .zip(item.album_primary_image_tag.as_deref())
            .map(|(id, tag)| image(id, ImageType::Primary, tag)),
        thumb: item
            .image_tags
            .get("Thumb")
            .map(|tag| image(&item.id, ImageType::Thumb, tag)),
        series_thumb: item
            .series_id
            .as_deref()
            .zip(item.series_thumb_image_tag.as_deref())
            .map(|(id, tag)| image(id, ImageType::Thumb, tag)),
        parent_thumb: item
            .parent_thumb_item_id
            .as_deref()
            .zip(item.parent_thumb_image_tag.as_deref())
            .map(|(id, tag)| image(id, ImageType::Thumb, tag)),
        backdrop: item
            .backdrop_image_tags
            .first()
            .map(|tag| image(&item.id, ImageType::Backdrop, tag)),
        parent_backdrop: (item.kind == "Episode")
            .then(|| {
                item.parent_backdrop_item_id
                    .as_deref()
                    .zip(item.parent_backdrop_image_tags.first().map(String::as_str))
                    .map(|(id, tag)| image(id, ImageType::Backdrop, tag))
            })
            .flatten(),
        banner: item
            .image_tags
            .get("Banner")
            .map(|tag| image(&item.id, ImageType::Banner, tag)),
    }
}

/// A card's two text lines, as jellyfin-web's card builder writes them.
/// Whether each card line links somewhere, which changes its height in
/// jellyfin-web (a link's line box is taller than plain text's).
fn card_links(item: &LibraryItem) -> (bool, bool) {
    match item.kind.as_str() {
        // The show's name links to the show, the episode title to itself.
        "Episode" => (true, true),
        // The album is plain text; the track links to itself.
        "Audio" | "AudioBook" => (item.album.as_deref().is_none_or(str::is_empty), true),
        // The title links to the item; a year is plain.
        _ => (true, false),
    }
}

fn card_lines(item: &LibraryItem) -> (String, String) {
    match item.kind.as_str() {
        "Episode" => {
            let number = match (item.parent_index_number, item.index_number) {
                (Some(season), Some(episode)) => format!("S{season}:E{episode}"),
                (None, Some(episode)) => format!("E{episode}"),
                _ => String::new(),
            };
            let episode = if number.is_empty() {
                item.name.clone()
            } else {
                format!("{number} - {}", item.name)
            };
            match item.series_name.clone().filter(|name| !name.is_empty()) {
                Some(series) => (series, episode),
                None => (item.name.clone(), number),
            }
        }
        // Audio cards name the album, then the track, as jellyfin-web's
        // Continue Listening does.
        "Audio" | "AudioBook" => match item.album.clone().filter(|album| !album.is_empty()) {
            Some(album) => (album, item.name.clone()),
            None => (item.name.clone(), String::new()),
        },
        "Movie" | "Series" | "Video" | "MusicVideo" => (item.name.clone(), year_line(item)),
        _ => (item.name.clone(), String::new()),
    }
}

/// A card's year as jellyfin-web's card builder writes it: a series spans
/// its run ("2019 - Present", "2019 - 2022"), anything else its year.
fn year_line(item: &LibraryItem) -> String {
    let start = item
        .production_year
        .map(|year| year.to_string())
        .unwrap_or_default();
    if item.kind != "Series" {
        return start;
    }
    if item
        .status
        .as_deref()
        .is_some_and(|status| status.eq_ignore_ascii_case("Continuing"))
    {
        return format!("{start} - Present");
    }
    let end = item
        .end_date
        .as_deref()
        .and_then(parse_iso_date)
        .map(|(year, _, _)| year.to_string());
    match end {
        Some(end) if !start.is_empty() && end != start => format!("{start} - {end}"),
        _ => start,
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RemoteTrailer {
    #[serde(rename = "Url")]
    url: Option<String>,
}

#[derive(Serialize)]
struct AuthRequest<'a> {
    #[serde(rename = "Username")]
    username: &'a str,
    #[serde(rename = "Pw")]
    password: &'a str,
}

#[derive(Deserialize)]
struct AuthResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "User")]
    user: JellyfinUser,
}

#[derive(Deserialize)]
struct JellyfinUser {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
}

#[derive(Deserialize)]
struct SegmentsResponse {
    #[serde(rename = "Items", default)]
    items: Vec<SegmentItem>,
}

#[derive(Deserialize)]
struct SegmentItem {
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "StartTicks", default)]
    start_ticks: i64,
    #[serde(rename = "EndTicks", default)]
    end_ticks: i64,
}

impl SegmentsResponse {
    fn into_segments(self) -> Vec<MediaSegment> {
        self.items
            .into_iter()
            .filter(|item| item.end_ticks > item.start_ticks)
            .map(|item| MediaSegment {
                kind: SegmentKind::from_jellyfin(&item.kind),
                start: item.start_ticks as f64 / 10_000_000.0,
                end: item.end_ticks as f64 / 10_000_000.0,
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct ItemsResponse {
    #[serde(rename = "Items", default)]
    items: Vec<LibraryItem>,
    #[serde(rename = "TotalRecordCount")]
    total_record_count: Option<usize>,
}

#[derive(Deserialize)]
struct LibraryItem {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "PremiereDate")]
    premiere_date: Option<String>,
    #[serde(rename = "EndDate")]
    end_date: Option<String>,
    /// A series' airing state: "Continuing" or "Ended".
    #[serde(rename = "Status")]
    status: Option<String>,
    #[serde(rename = "LocalTrailerCount", default)]
    local_trailer_count: u32,
    #[serde(rename = "RemoteTrailers", default)]
    remote_trailers: Vec<RemoteTrailer>,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "CollectionType")]
    collection_type: Option<String>,
    #[serde(rename = "SeriesId")]
    series_id: Option<String>,
    #[serde(rename = "SeriesPrimaryImageTag")]
    series_primary_image_tag: Option<String>,
    #[serde(rename = "ParentId")]
    parent_id: Option<String>,
    #[serde(rename = "SeasonId")]
    season_id: Option<String>,
    #[serde(rename = "SeasonName")]
    season_name: Option<String>,
    #[serde(rename = "ParentPrimaryImageItemId")]
    parent_primary_image_item_id: Option<String>,
    #[serde(rename = "ParentBackdropItemId")]
    parent_backdrop_item_id: Option<String>,
    #[serde(rename = "ParentThumbItemId")]
    parent_thumb_item_id: Option<String>,
    #[serde(rename = "ParentThumbImageTag")]
    parent_thumb_image_tag: Option<String>,
    #[serde(rename = "SeriesThumbImageTag")]
    series_thumb_image_tag: Option<String>,
    #[serde(rename = "PrimaryImageAspectRatio")]
    primary_image_aspect_ratio: Option<f64>,
    #[serde(rename = "AlbumId")]
    album_id: Option<String>,
    #[serde(rename = "AlbumPrimaryImageTag")]
    album_primary_image_tag: Option<String>,
    #[serde(rename = "ParentLogoItemId")]
    parent_logo_item_id: Option<String>,
    #[serde(rename = "ParentLogoImageTag")]
    parent_logo_image_tag: Option<String>,
    #[serde(rename = "ParentBackdropImageTags", default)]
    parent_backdrop_image_tags: Vec<String>,
    #[serde(rename = "ImageTags", default)]
    image_tags: std::collections::HashMap<String, String>,
    #[serde(rename = "BackdropImageTags", default)]
    backdrop_image_tags: Vec<String>,
    #[serde(rename = "UserData")]
    user_data: Option<UserData>,
    #[serde(rename = "ProductionYear")]
    production_year: Option<i32>,
    #[serde(rename = "OfficialRating")]
    official_rating: Option<String>,
    #[serde(rename = "CommunityRating")]
    community_rating: Option<f64>,
    #[serde(rename = "CriticRating")]
    critic_rating: Option<f64>,
    #[serde(rename = "Taglines", default)]
    taglines: Vec<String>,
    #[serde(rename = "Genres", default)]
    genres: Vec<String>,
    #[serde(rename = "GenreItems", default)]
    genre_items: Vec<NameGuidPair>,
    #[serde(rename = "ProductionLocations", default)]
    production_locations: Vec<String>,
    #[serde(rename = "PlaylistItemId")]
    playlist_item_id: Option<String>,
    #[serde(rename = "Tags", default)]
    tags: Vec<String>,
    #[serde(rename = "ExternalUrls", default)]
    external_urls: Vec<ExternalUrl>,
    #[serde(rename = "Studios", default)]
    studios: Vec<NameGuidPair>,
    #[serde(rename = "People", default)]
    people: Vec<BaseItemPerson>,
    #[serde(rename = "Album")]
    album: Option<String>,
    #[serde(rename = "SeriesName")]
    series_name: Option<String>,
    #[serde(rename = "IndexNumber")]
    index_number: Option<i32>,
    #[serde(rename = "ParentIndexNumber")]
    parent_index_number: Option<i32>,
    #[serde(rename = "ChildCount")]
    child_count: Option<i32>,
    #[serde(rename = "Overview")]
    overview: Option<String>,
    #[serde(rename = "RunTimeTicks")]
    run_time_ticks: Option<i64>,
    #[serde(rename = "MediaStreams", default)]
    media_streams: Vec<ItemMediaStream>,
}

/// Libraries whose "Recently Added" shelf loads after the first screen.
fn defers_latest_shelf(item: &LibraryItem) -> bool {
    item.collection_type.as_deref() == Some("music")
}

/// Whether jellyfin-web's home gives a library a "Recently Added in …"
/// shelf: every library except playlists, collections and live TV.
fn has_latest_shelf(item: &LibraryItem) -> bool {
    !matches!(
        item.collection_type.as_deref(),
        Some("playlists" | "boxsets" | "livetv")
    ) && !matches!(
        item.kind.as_str(),
        "ManualPlaylistsFolder" | "PlaylistsFolder"
    )
}

#[derive(Deserialize)]
struct UserData {
    #[serde(rename = "PlaybackPositionTicks", default)]
    playback_position_ticks: i64,
    #[serde(rename = "PlayedPercentage")]
    played_percentage: Option<f64>,
    #[serde(rename = "Played", default)]
    played: bool,
    #[serde(rename = "IsFavorite", default)]
    is_favorite: bool,
    #[serde(rename = "UnplayedItemCount")]
    unplayed_item_count: Option<i32>,
}

#[derive(Deserialize)]
struct ExternalUrl {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Url", default)]
    url: String,
}

#[derive(Deserialize)]
struct NameGuidPair {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Id")]
    id: Option<String>,
}

impl From<NameGuidPair> for NamedId {
    fn from(pair: NameGuidPair) -> Self {
        Self {
            name: pair.name,
            id: pair.id,
        }
    }
}

#[derive(Deserialize)]
struct BaseItemPerson {
    #[serde(rename = "Id")]
    id: Option<String>,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Role", default)]
    role: String,
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "PrimaryImageTag")]
    primary_image_tag: Option<String>,
}

#[derive(Deserialize)]
struct ItemMediaStream {
    #[serde(rename = "Index")]
    index: i32,
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "Language")]
    language: Option<String>,
    #[serde(rename = "DisplayTitle")]
    display_title: Option<String>,
    #[serde(rename = "Codec")]
    codec: Option<String>,
    #[serde(rename = "IsDefault", default)]
    is_default: bool,
    #[serde(rename = "Width")]
    width: Option<i32>,
    #[serde(rename = "Height")]
    height: Option<i32>,
    #[serde(rename = "VideoRangeType")]
    video_range_type: Option<String>,
    #[serde(rename = "Channels")]
    channels: Option<i32>,
}

#[cfg(test)]
mod tests {
    /// Builds a row the way the server sends one, so the grouping tests
    /// exercise the real deserialization as well.
    use super::*;

    #[test]
    fn media_segments_parse_into_seconds_and_drop_empty_spans() {
        let response: SegmentsResponse = serde_json::from_str(
            r#"{"Items":[
                {"Id":"a","ItemId":"e","Type":"Intro","StartTicks":300000000,"EndTicks":900000000},
                {"Id":"b","ItemId":"e","Type":"Outro","StartTicks":14000000000,"EndTicks":15000000000},
                {"Id":"c","ItemId":"e","Type":"Recap","StartTicks":0,"EndTicks":0},
                {"Id":"d","ItemId":"e","Type":"Something","StartTicks":1,"EndTicks":2}
            ],"TotalRecordCount":4}"#,
        )
        .unwrap();
        let segments = response.into_segments();
        assert_eq!(
            segments,
            vec![
                MediaSegment {
                    kind: SegmentKind::Intro,
                    start: 30.0,
                    end: 90.0,
                },
                MediaSegment {
                    kind: SegmentKind::Outro,
                    start: 1400.0,
                    end: 1500.0,
                },
                MediaSegment {
                    kind: SegmentKind::Unknown,
                    start: 0.0000001,
                    end: 0.0000002,
                },
            ]
        );
    }

    fn fields(set: &str) -> Vec<&str> {
        set.split(',').collect()
    }

    /// Each of these was measured against a real server, and each is easy to
    /// reintroduce by copying `DETAIL_FIELDS` when adding a list query.
    /// `RecursiveItemCount` makes the server recursively count every
    /// folder-typed row: 17.8 s versus 0.37 s on a 323-item music library.
    /// `Overview` and `MediaStreams` are dead weight no catalog row reads, and
    /// carrying them made the six Home requests ~900 KiB instead of ~75 KiB,
    /// which was most of a 528 ms time-to-interactive.
    #[test]
    fn list_queries_ask_for_nothing_a_catalog_row_cannot_use() {
        for unwanted in ["RecursiveItemCount", "Overview", "MediaStreams"] {
            assert!(
                !fields(LIST_FIELDS).contains(&unwanted),
                "{unwanted} is expensive and no catalog row reads it"
            );
        }
    }

    /// The detail page renders both of the fields trimmed from LIST_FIELDS, so
    /// the trim must not be copied across to DETAIL_FIELDS.
    #[test]
    fn detail_queries_still_ask_for_what_the_detail_page_renders() {
        for wanted in ["Overview", "MediaStreams", "People", "Genres"] {
            assert!(
                fields(DETAIL_FIELDS).contains(&wanted),
                "the detail page renders {wanted}"
            );
        }
    }

    #[test]
    fn normalizes_supported_server_urls_without_accepting_credentials() {
        assert_eq!(
            normalize_server_url("https://jellyfin.example/base")
                .unwrap()
                .as_str(),
            "https://jellyfin.example/base/"
        );
        assert!(normalize_server_url("jellyfin.example").is_err());
        assert!(normalize_server_url("https://user:pass@jellyfin.example").is_err());
        assert!(normalize_server_url("file:///tmp/jellyfin").is_err());
    }

    #[test]
    fn tolerates_unknown_fields_and_missing_optional_item_values() {
        let response: ItemsResponse = serde_json::from_str(
            r#"{"Items":[{"Id":"m1","Name":"Movie","Type":"Movie","Future":true}]}"#,
        )
        .unwrap();
        let item: CatalogItemData = response.items.into_iter().next().unwrap().into();
        assert_eq!(item.title, "Movie");
        assert!(item.playable);
    }

    #[test]
    fn episode_posters_use_their_own_primary_then_the_series_one() {
        // jellyfin-web's cardBuilder: the item's own Primary first, the
        // series poster only when the episode has none.
        let response: ItemsResponse = serde_json::from_str(
            r#"{
                "Items": [{
                    "Id": "episode-1",
                    "Name": "Pilot",
                    "Type": "Episode",
                    "SeriesId": "series-1",
                    "SeriesPrimaryImageTag": "series-poster-tag",
                    "ImageTags": {"Primary": "episode-image-tag"}
                }, {
                    "Id": "episode-2",
                    "Name": "Second",
                    "Type": "Episode",
                    "SeriesId": "series-1",
                    "SeriesPrimaryImageTag": "series-poster-tag",
                    "ImageTags": {}
                }]
            }"#,
        )
        .unwrap();
        let mut items = response.items.into_iter().map(CatalogItemData::from);
        let own = items
            .next()
            .unwrap()
            .images
            .pick(false, false)
            .unwrap()
            .image;
        assert_eq!(
            (own.item_id.as_str(), own.tag.as_str()),
            ("episode-1", "episode-image-tag")
        );
        let series = items
            .next()
            .unwrap()
            .images
            .pick(false, false)
            .unwrap()
            .image;
        assert_eq!(
            (series.item_id.as_str(), series.tag.as_str()),
            ("series-1", "series-poster-tag")
        );
    }

    #[test]
    fn series_details_read_whether_the_show_still_airs() {
        let continuing: LibraryItem = serde_json::from_str(
            r#"{"Id":"s","Name":"Show","Type":"Series","ProductionYear":2019,"Status":"Continuing"}"#,
        )
        .unwrap();
        let continuing: DetailItemData = continuing.into();
        assert!(continuing.series_continuing);
        assert_eq!(continuing.end_year, None);

        let ended: LibraryItem = serde_json::from_str(
            r#"{"Id":"s","Name":"Show","Type":"Series","Status":"Ended","EndDate":"2022-05-20T00:00:00.0000000Z"}"#,
        )
        .unwrap();
        let ended: DetailItemData = ended.into();
        assert!(!ended.series_continuing);
        assert_eq!(ended.end_year, Some(2022));
    }

    #[test]
    fn episode_details_read_the_air_date_and_list_guest_stars_apart() {
        let item: LibraryItem = serde_json::from_str(
            r#"{
                "Id":"episode-1",
                "Name":"Pilot",
                "Type":"Episode",
                "PremiereDate":"2025-01-31T00:00:00.0000000Z",
                "People":[
                    {"Id":"a","Name":"Regular","Role":"Lead","Type":"Actor"},
                    {"Id":"g","Name":"Visitor","Role":"Themselves","Type":"GuestStar"},
                    {"Id":"d","Name":"Helmer","Type":"Director"}
                ]
            }"#,
        )
        .unwrap();
        let detail: DetailItemData = item.into();

        assert_eq!(detail.premiere_date, Some((2025, 1, 31)));
        let names = |people: &[PersonData]| {
            people
                .iter()
                .map(|person| person.name.clone())
                .collect::<Vec<_>>()
        };
        // Cast & Crew keeps the crew, as jellyfin-web's does.
        assert_eq!(names(&detail.people), ["Regular", "Helmer"]);
        assert_eq!(detail.people[1].role, "Director");
        assert_eq!(names(&detail.guest_stars), ["Visitor"]);
        assert_eq!(
            detail
                .directors
                .iter()
                .map(|named| named.name.as_str())
                .collect::<Vec<_>>(),
            ["Helmer"]
        );

        assert_eq!(parse_iso_date("not a date"), None);
        assert_eq!(parse_iso_date("2025-13-01T00:00:00Z"), None);
    }

    #[test]
    fn media_info_prefers_the_servers_video_title() {
        let streams: Vec<ItemMediaStream> = serde_json::from_str(
            r#"[
                {"Index":0,"Type":"Video","Codec":"hevc","Height":2160,"DisplayTitle":"4K HEVC HDR10","VideoRangeType":"HDR10"},
                {"Index":1,"Type":"Audio","Codec":"eac3","Channels":6,"Language":"eng"}
            ]"#,
        )
        .unwrap();
        assert_eq!(media_summary(&streams), "4K HEVC HDR10");
        // Without a DisplayTitle, the audio label names the channel layout.
        assert_eq!(stream_label(&streams[1]), "ENG - EAC3 5.1");

        let untitled: Vec<ItemMediaStream> = serde_json::from_str(
            r#"[
                {"Index":0,"Type":"Video","Codec":"h264","Height":1080,"VideoRangeType":"SDR"},
                {"Index":1,"Type":"Audio","Codec":"aac"}
            ]"#,
        )
        .unwrap();
        assert_eq!(media_summary(&untitled), "1080p • H264 • AAC");
        let hdr: Vec<ItemMediaStream> = serde_json::from_str(
            r#"[{"Index":0,"Type":"Video","Codec":"hevc","Height":2160,"VideoRangeType":"DOVI"}]"#,
        )
        .unwrap();
        assert_eq!(media_summary(&hdr), "2160p • HEVC • DOVI");
    }

    #[test]
    fn episode_details_keep_the_still_separate_from_the_series_poster() {
        let item: LibraryItem = serde_json::from_str(
            r#"{
                "Id":"episode-1",
                "Name":"Pilot",
                "Type":"Episode",
                "SeriesId":"series-1",
                "SeriesPrimaryImageTag":"series-poster-tag",
                "ImageTags":{"Primary":"episode-still-tag"}
            }"#,
        )
        .unwrap();
        let detail: DetailItemData = item.into();

        assert_eq!(detail.poster.as_ref().unwrap().item_id, "series-1");
        assert_eq!(detail.thumbnail.as_ref().unwrap().tag, "episode-still-tag");
    }

    #[test]
    fn parses_jellyfin_detail_metadata_into_the_detail_model() {
        let item: LibraryItem = serde_json::from_str(
            r#"{
                "Id":"series-1",
                "Name":"Example Show",
                "Type":"Series",
                "Overview":"A detailed overview.",
                "ProductionYear":2024,
                "OfficialRating":"TV-14",
                "CommunityRating":8.4,
                "RunTimeTicks":27000000000,
                "Taglines":["The example begins"],
                "Genres":["Drama","Mystery"],
                "Studios":[{"Name":"Example Studio"}],
                "People":[{"Id":"person-1","Name":"Actor Name","Role":"Lead","Type":"Actor","PrimaryImageTag":"person-tag"}],
                "ImageTags":{"Primary":"poster-tag","Logo":"logo-tag"},
                "BackdropImageTags":["backdrop-tag"],
                "UserData":{"Played":false,"IsFavorite":true,"UnplayedItemCount":3},
                "MediaStreams":[{"Index":1,"Type":"Audio","Language":"jpn","DisplayTitle":"Japanese","Codec":"aac","IsDefault":true}]
            }"#,
        )
        .unwrap();
        let detail: DetailItemData = item.into();

        assert_eq!(detail.kind, DetailKind::Series);
        assert_eq!(detail.tagline, "The example begins");
        let names = |list: &[NamedId]| {
            list.iter()
                .map(|named| named.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&detail.genres), ["Drama", "Mystery"]);
        assert_eq!(names(&detail.studios), ["Example Studio"]);
        assert_eq!(detail.people.len(), 1);
        // jellyfin-web credits a role as "as <role>".
        assert_eq!(detail.people[0].role, "as Lead");
        assert_eq!(detail.poster.as_ref().unwrap().tag, "poster-tag");
        assert_eq!(detail.backdrop.as_ref().unwrap().tag, "backdrop-tag");
        assert_eq!(detail.logo.as_ref().unwrap().tag, "logo-tag");
        assert!(detail.favorite);
        assert_eq!(detail.unplayed_count, Some(3));
        assert_eq!(detail.streams[0].label, "Japanese");
    }

    #[test]
    fn every_library_but_playlists_gets_a_recently_added_shelf() {
        let response: ItemsResponse = serde_json::from_str(
            r#"{
                "Items": [
                    {"Id":"movies","Name":"Movies","Type":"CollectionFolder","CollectionType":"movies"},
                    {"Id":"shows","Name":"Shows","Type":"CollectionFolder","CollectionType":"tvshows"},
                    {"Id":"music","Name":"Music","Type":"CollectionFolder","CollectionType":"music"},
                    {"Id":"playlists","Name":"Playlists","Type":"ManualPlaylistsFolder"},
                    {"Id":"playlist-library","Name":"Playlists","Type":"CollectionFolder","CollectionType":"playlists"}
                ]
            }"#,
        )
        .unwrap();
        let visible_ids = response
            .items
            .iter()
            .filter(|item| has_latest_shelf(item))
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(visible_ids, ["movies", "shows", "music"]);
    }
    /// Jellyfin ignores fields it does not recognise and still answers 204, so
    /// a misspelled key would leave reporting silently doing nothing. Pin the
    /// exact wire shape of all three endpoints' bodies.
    #[test]
    fn playback_reports_serialize_to_the_field_names_jellyfin_expects() {
        let target = ReportTarget {
            item_id: "item-1".into(),
            media_source_id: "source-1".into(),
            play_session_id: Some("session-1".into()),
        };

        let start = serde_json::to_value(PlaybackReportBody::playing(&target, 0.0, false, None))
            .expect("serialize start");
        assert_eq!(start["ItemId"], "item-1");
        assert_eq!(start["MediaSourceId"], "source-1");
        assert_eq!(start["PlaySessionId"], "session-1");
        assert_eq!(start["PositionTicks"], 0);
        assert_eq!(start["IsPaused"], false);
        assert_eq!(start["CanSeek"], true);
        assert_eq!(start["PlayMethod"], "DirectPlay");
        assert!(start.get("EventName").is_none());

        let progress = serde_json::to_value(PlaybackReportBody::playing(
            &target,
            61.5,
            true,
            Some("pause"),
        ))
        .expect("serialize progress");
        assert_eq!(progress["PositionTicks"], 615_000_000i64);
        assert_eq!(progress["IsPaused"], true);
        assert_eq!(progress["EventName"], "pause");

        // The stop body carries only what the endpoint uses; the playing-only
        // fields are omitted rather than sent as misleading defaults.
        let stopped = serde_json::to_value(PlaybackReportBody::stopped(&target, 61.5))
            .expect("serialize stop");
        assert_eq!(stopped["PositionTicks"], 615_000_000i64);
        for absent in ["IsPaused", "CanSeek", "PlayMethod", "EventName"] {
            assert!(
                stopped.get(absent).is_none(),
                "{absent} must not be sent on a stop report"
            );
        }
    }

    #[test]
    fn a_missing_play_session_id_is_omitted_rather_than_sent_as_null() {
        let target = ReportTarget {
            item_id: "item-1".into(),
            media_source_id: "source-1".into(),
            play_session_id: None,
        };
        let body = serde_json::to_value(PlaybackReportBody::playing(&target, 1.0, false, None))
            .expect("serialize");
        assert!(body.get("PlaySessionId").is_none());
    }
}
