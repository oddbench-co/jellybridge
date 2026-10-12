use reqwest::header::{HeaderValue, InvalidHeaderValue};
use reqwest::{Method, StatusCode};
use std::collections::HashMap;
use std::future::Future;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::features::catalog::CatalogSort;
use crate::features::detail::{
    DetailItemData, DetailKind, DetailPageData, ImageRef, ImageType, PersonData, StreamData,
};
use crate::features::now_playing::{MediaSegment, SegmentKind, split_adjacent};
use crate::features::progress::{ReportTarget, position_ticks};
use crate::services::session::StoredSession;

const CLIENT_NAME: &str = "JellyBridge";
const CLIENT_VERSION: &str = "0.1.0-rust";
const DEVICE_NAME: &str = "Desktop";
const DEVICE_ID: &str = "jellybridge-rust-desktop";
// Fields for list queries. `RecursiveItemCount` is deliberately absent: it
// makes the server recursively count the contents of every folder-typed row,
// which measured 17.8-18.1 s against a 323-item music library versus 0.37 s
// without it. The only thing that reads it is the `CollectionFolder` subtitle
// (see `item_subtitle`), and the only query that returns those is
// `libraries()`, which uses LIBRARY_FIELDS below and returns a handful of rows.
// `Overview` and `MediaStreams` are absent for the same reason: nothing on a
// catalog row reads them. `CatalogItemData` has no overview, and the media
// summary is built only for detail pages, which fetch DETAIL_FIELDS
// separately. Carrying them made the six home requests ~900 KiB instead of
// ~75 KiB and roughly doubled the slowest one.
const LIST_FIELDS: &str = "ParentId,SeriesId,SeriesPrimaryImageTag,SeasonId,SeasonName,ParentPrimaryImageItemId,ImageTags,UserData,RunTimeTicks,ChildCount,PrimaryImageAspectRatio,ProductionYear,SeriesName,IndexNumber,ParentIndexNumber,CollectionType";
// The top-level library rows, which need the recursive count for their
// "Library - N items" subtitle. Cheap here because there are only a few.
// How many rows one browse or search page returns.
const PAGE_LIMIT: &str = "120";
/// Rows fetched for "Recently Added" before episodes are collapsed into their
/// show, and rows kept afterwards.
const HOME_SHELF_LIMIT: usize = 16;
const LIBRARY_FIELDS: &str = "ParentId,ImageTags,UserData,Overview,ChildCount,RecursiveItemCount,PrimaryImageAspectRatio,ProductionYear,CollectionType";
const DETAIL_FIELDS: &str = "ParentId,SeriesId,SeriesPrimaryImageTag,SeasonId,SeasonName,ParentPrimaryImageItemId,ParentBackdropItemId,ParentBackdropImageTags,ImageTags,BackdropImageTags,UserData,Overview,RunTimeTicks,MediaStreams,Genres,People,Studios,OfficialRating,CommunityRating,CriticRating,Taglines,PremiereDate,DateCreated,ChildCount,RecursiveItemCount,PrimaryImageAspectRatio,ProductionYear,SeriesName,IndexNumber,ParentIndexNumber,CollectionType,Tags,ExternalUrls,ParentLogoItemId,ParentLogoImageTag";

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
    pub subtitle: String,
    pub kind: String,
    pub image_item_id: String,
    pub image_tag: Option<String>,
    pub watched: bool,
    pub progress: f32,
    pub is_container: bool,
    pub playable: bool,
    /// The card's first text line, as Jellyfin writes it: an episode is
    /// named by its series, everything else by itself.
    pub card_title: String,
    /// The card's second line: "S1:E16 - Name" for an episode, the year
    /// for a movie or series, nothing for folders.
    pub card_subtitle: String,
    pub favorite: bool,
    /// Unwatched episodes in a series or season, for the corner badge.
    pub unplayed_count: u32,
    /// Artwork for a 16:9 card, chosen like Jellyfin's `preferThumb`: the
    /// item's Thumb, the series' Thumb, a backdrop, then an episode still.
    pub thumb: Option<ImageRef>,
}

#[derive(Clone, Debug)]
pub struct CatalogPage {
    pub items: Vec<CatalogItemData>,
    pub total_record_count: usize,
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
    fn from_session_with(
        session: StoredSession,
        http: reqwest::Client,
    ) -> Result<Self, ApiError> {
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

    pub fn session(&self) -> &StoredSession {
        &self.session
    }

    /// The home screen, laid out as jellyfin-web's default home: My Media,
    /// Continue Watching, Next Up, then "Recently Added in <library>" for
    /// each library.
    pub async fn home(&self) -> Result<Vec<CatalogSectionData>, ApiError> {
        // The per-library shelves need the library list, so they start the
        // moment it lands, while Continue Watching and Next Up are still in
        // flight, rather than after everything else.
        let libraries_and_latest = async {
            let libraries = timed("libraries", self.libraries()).await?;
            let latest = futures_util::future::join_all(
                libraries
                    .iter()
                    .map(|library| timed("latest", self.latest_in(&library.id))),
            )
            .await;
            Ok::<_, ApiError>((libraries, latest))
        };
        let (libraries_and_latest, resume, next_up) = tokio::join!(
            libraries_and_latest,
            timed("resume", self.resume_items()),
            timed("next_up", self.next_up()),
        );
        let (libraries, latest) = libraries_and_latest?;
        let latest_titles = libraries
            .iter()
            .map(|library| (library.id.clone(), format!("Recently Added in {}", library.name)))
            .collect::<Vec<_>>();
        let mut sections = vec![CatalogSectionData {
            id: "libraries".into(),
            title: "My Media".into(),
            items: libraries.into_iter().map(Into::into).collect(),
        }];
        append_optional_section(&mut sections, "continue", "Continue Watching", resume);
        append_optional_section(&mut sections, "next-up", "Next Up", next_up);
        for ((library_id, title), items) in latest_titles.into_iter().zip(latest) {
            append_series_grouped_section(
                &mut sections,
                &format!("latest-{library_id}"),
                &title,
                HOME_SHELF_LIMIT,
                items,
            );
        }
        Ok(sections)
    }

    pub async fn browse(
        &self,
        item: &CatalogItemData,
        sort: CatalogSort,
    ) -> Result<CatalogPage, ApiError> {
        let (sort_by, sort_order) = sort.query();
        let response = if item.kind.eq_ignore_ascii_case("Series") {
            // A series has a handful of seasons; no page limit is needed.
            self.items_page_request(
                &format!("Shows/{}/Seasons", encode_path_segment(&item.id)),
                &[
                    ("UserId", self.session.user_id.as_str()),
                    ("SortBy", sort_by),
                    ("SortOrder", sort_order),
                ],
                "loading series seasons",
            )
            .await?
        } else {
            // Bounded like `search`: an unbounded folder returns every row in
            // the library, and each row costs a spawned artwork task and a
            // decode. The UI already reports "Showing first N of M".
            self.items_page_request(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                &[
                    ("ParentId", item.id.as_str()),
                    ("Recursive", "false"),
                    ("SortBy", sort_by),
                    ("SortOrder", sort_order),
                    ("StartIndex", "0"),
                    ("Limit", PAGE_LIMIT),
                    ("EnableTotalRecordCount", "true"),
                ],
                "loading a library folder",
            )
            .await?
        };
        Ok(response.into())
    }

    pub async fn search(
        &self,
        term: &str,
        parent_id: Option<&str>,
        sort: CatalogSort,
    ) -> Result<CatalogPage, ApiError> {
        let term = term.trim();
        if term.is_empty() {
            return Ok(CatalogPage {
                items: Vec::new(),
                total_record_count: 0,
            });
        }
        let (sort_by, sort_order) = sort.query();
        let mut query = vec![
            ("Recursive", "true"),
            ("IncludeItemTypes", "Movie,Series,Episode"),
            ("SearchTerm", term),
            ("SortBy", sort_by),
            ("SortOrder", sort_order),
            ("StartIndex", "0"),
            ("Limit", PAGE_LIMIT),
            ("EnableTotalRecordCount", "true"),
        ];
        if let Some(parent_id) = parent_id {
            query.push(("ParentId", parent_id));
        }
        let response = self
            .items_page_request(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                &query,
                "searching the library",
            )
            .await?;
        Ok(response.into())
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
        let ((seasons, episodes, children_error), similar) = tokio::join!(children, similar);
        Ok(DetailPageData {
            item: item.into(),
            seasons: seasons.into_iter().map(Into::into).collect(),
            episodes: episodes.into_iter().map(Into::into).collect(),
            similar: similar.into_iter().map(Into::into).collect(),
            children_error,
        })
    }

    /// The server's "More Like This" for an item.
    async fn similar_items(&self, item_id: &str) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!("Items/{}/Similar", encode_path_segment(item_id)),
            &[
                ("UserId", self.session.user_id.as_str()),
                ("Limit", "12"),
            ],
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

    pub async fn fetch_primary_image(
        &self,
        item_id: &str,
        max_width: u32,
    ) -> Result<Vec<u8>, ApiError> {
        let response = self
            .request(
                Method::GET,
                &format!("Items/{}/Images/Primary", encode_path_segment(item_id)),
            )?
            .query(&[
                ("maxWidth", max_width.to_string().as_str()),
                ("quality", "90"),
                ("format", "Jpg"),
            ])
            .send()
            .await?;
        let response = check_status(response, "loading artwork")?;
        Ok(response.bytes().await?.to_vec())
    }

    async fn libraries(&self) -> Result<Vec<LibraryItem>, ApiError> {
        Ok(self
            .items_page_request_with_fields(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                LIBRARY_FIELDS,
                &[("Recursive", "false")],
                "loading libraries",
            )
            .await?
            .items
            .into_iter()
            .filter(is_visible_library)
            .collect())
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

    async fn resume_items(&self) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!(
                "Users/{}/Items/Resume",
                encode_path_segment(&self.session.user_id)
            ),
            &[
                ("StartIndex", "0"),
                ("Limit", "20"),
                ("MediaTypes", "Video"),
                ("IncludeItemTypes", "Movie,Episode"),
            ],
            "loading continue watching",
        )
        .await
    }

    async fn next_up(&self) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            "Shows/NextUp",
            &[
                ("UserId", self.session.user_id.as_str()),
                ("StartIndex", "0"),
                ("Limit", "20"),
                ("EnableTotalRecordCount", "false"),
            ],
            "loading next up",
        )
        .await
    }

    /// The newest items in one library, as jellyfin-web's home asks for
    /// them: grouped, so a batch of new episodes arrives as its show.
    async fn latest_in(&self, library_id: &str) -> Result<Vec<LibraryItem>, ApiError> {
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
                ("Limit", "16"),
                // Collapses a batch of new episodes into their series and puts
                // the number of them in `ChildCount`.
                ("GroupItems", "true"),
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

/// Appends a shelf, collapsing the episodes of one show into a single row.
///
/// Downloading a season otherwise fills the shelf with one card per episode:
/// nine of the first fourteen "Recently Added" rows were episodes of the same
/// show, plus a tenth for the show itself, which arrived at the same time.
///
/// Used for the two "what is new" shelves only. Continue Watching and Next Up
/// are meant to name a specific episode, and favourites are chosen one at a
/// time.
fn append_series_grouped_section(
    sections: &mut Vec<CatalogSectionData>,
    id: &str,
    title: &str,
    limit: usize,
    result: Result<Vec<LibraryItem>, ApiError>,
) {
    match result {
        Ok(items) if !items.is_empty() => {
            let mut items = group_episodes_by_series(items);
            // Grouping shrinks the shelf, so the queries behind it fetch more
            // than they show; trim back to the shelf's real size.
            items.truncate(limit);
            sections.push(CatalogSectionData {
                id: id.to_owned(),
                title: title.to_owned(),
                items,
            });
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, section = title, "dashboard section failed"),
    }
}

/// One row per show, counting how many of its episodes are in `items`.
///
/// Keyed on the series: an episode by its `SeriesId`, a series by its own id.
/// That single key is what merges the two ways a show turns up twice -- the
/// Latest endpoint returns a series both bare and grouped when the show and
/// its episodes are new together, and the Recently Added query returns the
/// series alongside every one of its episodes. Both share an id, so the larger
/// count wins and one row survives.
///
/// Movies and folders pass through untouched, and first-appearance order is
/// preserved so the server's newest-first ordering survives.
///
/// The count only sees the rows it was given: a show with thirty new episodes
/// reports twenty when twenty were fetched. The server's own grouped
/// `ChildCount` truncates the same way, so the shelves stay consistent with
/// each other and with Jellyfin.
fn group_episodes_by_series(items: Vec<LibraryItem>) -> Vec<CatalogItemData> {
    let mut rows: Vec<CatalogItemData> = Vec::with_capacity(items.len());
    // Series id -> (index into `rows`, episodes counted so far).
    let mut groups: HashMap<String, (usize, u32)> = HashMap::new();

    for item in items {
        let Some((key, contribution)) = group_key(&item) else {
            rows.push(item.into());
            continue;
        };
        if let Some((index, count)) = groups.get_mut(&key) {
            *count = contribution.merged_into(*count);
            rows[*index].subtitle = new_episode_subtitle(*count);
            continue;
        }
        let count = contribution.merged_into(0);
        groups.insert(key.clone(), (rows.len(), count));
        rows.push(series_row(item, key, count));
    }
    rows
}

/// What one row adds to its group's episode count.
#[derive(Clone, Copy)]
enum GroupContribution {
    /// A single episode. Occurrences add up.
    Episode,
    /// A series row that already carries the whole count, so occurrences do
    /// not add: the larger figure is the right one.
    Total(u32),
}

impl GroupContribution {
    fn merged_into(self, count: u32) -> u32 {
        match self {
            Self::Episode => count.saturating_add(1),
            Self::Total(total) => count.max(total),
        }
    }
}

/// The series a row belongs to and what it contributes to the count. `None` for
/// rows that are not part of a show.
fn group_key(item: &LibraryItem) -> Option<(String, GroupContribution)> {
    match item.kind.as_str() {
        // Without a series id there is nothing to group an episode under, so
        // it stays a row of its own rather than disappearing.
        "Episode" => item
            .series_id
            .clone()
            .map(|id| (id, GroupContribution::Episode)),
        // `ChildCount` on a row from the Latest endpoint with `GroupItems=true`
        // is the number of newly added episodes -- not its usual meaning, which
        // for the same series on an ordinary query is the season count.
        "Series" => Some((
            item.id.clone(),
            GroupContribution::Total(item.child_count.unwrap_or(0).max(0) as u32),
        )),
        _ => None,
    }
}

/// Rewrites one row of a group as the show it belongs to.
///
/// The artwork needs no special handling: `artwork_identity` already makes an
/// episode borrow its series poster, so this resolves to the same image, and
/// the same cache key, that was going to be drawn anyway.
fn series_row(item: LibraryItem, series_id: String, count: u32) -> CatalogItemData {
    let series_name = item.series_name.clone();
    let mut row = CatalogItemData::from(item);
    if count == 0 {
        // A show that is itself new, with no episodes grouped under it. Leave
        // it exactly as the server described it.
        return row;
    }
    row.id = series_id;
    row.title = series_name.unwrap_or(row.title);
    row.kind = "Series".to_owned();
    row.subtitle = new_episode_subtitle(count);
    // Jellyfin shows a grouped show as the show, with the number of new
    // episodes in the corner badge.
    row.card_title = row.title.clone();
    row.card_subtitle = String::new();
    row.unplayed_count = count;
    row.favorite = false;
    // `playable` and `is_container` follow the kind, and a Series opens its
    // detail page, where `play_target` picks the right episode.
    row.playable = false;
    row.is_container = true;
    // Progress taken from whichever episode happened to come first would be
    // meaningless for a group.
    row.watched = false;
    row.progress = 0.0;
    row
}

fn new_episode_subtitle(count: u32) -> String {
    if count == 1 {
        "1 new episode".to_owned()
    } else {
        format!("{count} new episodes")
    }
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
        let is_container = !playable;
        let subtitle = item_subtitle(&item);
        let (image_item_id, image_tag) = artwork_identity(&item);
        let (card_title, card_subtitle) = card_lines(&item);
        let thumb = thumb_image(&item);
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
            subtitle,
            kind: item.kind,
            image_item_id,
            image_tag,
            progress,
            is_container,
            playable,
            card_title,
            card_subtitle,
            thumb,
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
        let (guest_stars, people): (Vec<_>, Vec<_>) = item
            .people
            .into_iter()
            .filter(|person| {
                person.kind.is_empty()
                    || person.kind.eq_ignore_ascii_case("Actor")
                    || person.kind.eq_ignore_ascii_case("GuestStar")
            })
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
            genres: item.genres,
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
                .map(|studio| studio.name)
                .filter(|name| !name.is_empty())
                .collect(),
            people,
            guest_stars,
            premiere_date: item.premiere_date.as_deref().and_then(parse_iso_date),
            end_year: item
                .end_date
                .as_deref()
                .and_then(parse_iso_date)
                .map(|(year, _, _)| year),
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
    let month = parts.next()?.parse().ok().filter(|month| (1..=12).contains(month))?;
    let day = parts.next()?.parse().ok().filter(|day| (1..=31).contains(day))?;
    Some((year, month, day))
}

fn person_names(people: &[BaseItemPerson], kind: &str) -> Vec<String> {
    people
        .iter()
        .filter(|person| person.kind.eq_ignore_ascii_case(kind))
        .map(|person| person.name.clone())
        .filter(|name| !name.is_empty())
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

fn artwork_identity(item: &LibraryItem) -> (String, Option<String>) {
    if item.kind == "Episode"
        && let (Some(series_id), Some(series_tag)) =
            (&item.series_id, &item.series_primary_image_tag)
    {
        return (series_id.clone(), Some(series_tag.clone()));
    }

    let image_item_id = if item.image_tags.contains_key("Primary") {
        item.id.clone()
    } else {
        item.parent_primary_image_item_id
            .clone()
            .unwrap_or_else(|| item.id.clone())
    };
    (image_item_id, item.image_tags.get("Primary").cloned())
}

/// A card's two text lines, as jellyfin-web's card builder writes them.
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
        "Movie" | "Series" | "Video" | "MusicVideo" => (
            item.name.clone(),
            item.production_year
                .map(|year| year.to_string())
                .unwrap_or_default(),
        ),
        _ => (item.name.clone(), String::new()),
    }
}

/// Artwork for a 16:9 card, in jellyfin-web's `preferThumb` order.
fn thumb_image(item: &LibraryItem) -> Option<ImageRef> {
    let image = |item_id: &str, image_type: ImageType, tag: &str| ImageRef {
        item_id: item_id.to_owned(),
        image_type,
        index: (image_type == ImageType::Backdrop).then_some(0),
        tag: tag.to_owned(),
    };
    if let Some(tag) = item.image_tags.get("Thumb") {
        return Some(image(&item.id, ImageType::Thumb, tag));
    }
    if let (Some(id), Some(tag)) = (&item.parent_thumb_item_id, &item.parent_thumb_image_tag) {
        return Some(image(id, ImageType::Thumb, tag));
    }
    if let Some(tag) = item.backdrop_image_tags.first() {
        return Some(image(&item.id, ImageType::Backdrop, tag));
    }
    if let (Some(id), Some(tag)) = (&item.parent_backdrop_item_id, item.parent_backdrop_image_tags.first()) {
        return Some(image(id, ImageType::Backdrop, tag));
    }
    // Libraries and episodes have 16:9 primary images of their own.
    item.image_tags
        .get("Primary")
        .filter(|_| matches!(item.kind.as_str(), "Episode" | "CollectionFolder" | "UserView"))
        .map(|tag| image(&item.id, ImageType::Primary, tag))
}

fn item_subtitle(item: &LibraryItem) -> String {
    if item.kind == "Episode" {
        let index = match (item.parent_index_number, item.index_number) {
            (Some(season), Some(episode)) => format!("S{season:02}E{episode:02}"),
            (_, Some(episode)) => format!("Episode {episode}"),
            _ => String::new(),
        };
        return [item.series_name.as_deref().unwrap_or_default(), &index]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" • ");
    }
    if item.kind == "CollectionFolder" {
        return item
            .recursive_item_count
            .map(|count| format!("Library • {count} items"))
            .unwrap_or_else(|| "Library".to_owned());
    }
    // A zero count is not information: the Latest endpoint reports
    // `ChildCount = 0` on every row it did not group, which read as
    // "Spider-Man - 2003 • 0 items".
    match (item.production_year, item.child_count.filter(|count| *count > 0)) {
        (Some(year), Some(count)) => format!("{year} • {count} items"),
        (Some(year), None) => year.to_string(),
        (None, Some(count)) => format!("{count} items"),
        (None, None) => String::new(),
    }
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

impl From<ItemsResponse> for CatalogPage {
    fn from(response: ItemsResponse) -> Self {
        let total_record_count = response
            .total_record_count
            .unwrap_or(response.items.len())
            .max(response.items.len());
        Self {
            items: response.items.into_iter().map(Into::into).collect(),
            total_record_count,
        }
    }
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
    #[serde(rename = "Tags", default)]
    tags: Vec<String>,
    #[serde(rename = "ExternalUrls", default)]
    external_urls: Vec<ExternalUrl>,
    #[serde(rename = "Studios", default)]
    studios: Vec<NameGuidPair>,
    #[serde(rename = "People", default)]
    people: Vec<BaseItemPerson>,
    #[serde(rename = "SeriesName")]
    series_name: Option<String>,
    #[serde(rename = "IndexNumber")]
    index_number: Option<i32>,
    #[serde(rename = "ParentIndexNumber")]
    parent_index_number: Option<i32>,
    #[serde(rename = "ChildCount")]
    child_count: Option<i32>,
    #[serde(rename = "RecursiveItemCount")]
    recursive_item_count: Option<i32>,
    #[serde(rename = "Overview")]
    overview: Option<String>,
    #[serde(rename = "RunTimeTicks")]
    run_time_ticks: Option<i64>,
    #[serde(rename = "MediaStreams", default)]
    media_streams: Vec<ItemMediaStream>,
}

fn is_visible_library(item: &LibraryItem) -> bool {
    !matches!(
        item.kind.as_str(),
        "ManualPlaylistsFolder" | "PlaylistsFolder" | "Playlist"
    ) && !matches!(
        item.collection_type.as_deref(),
        Some(collection_type)
            if collection_type.eq_ignore_ascii_case("music")
                || collection_type.eq_ignore_ascii_case("playlists")
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
    fn row(json: serde_json::Value) -> LibraryItem {
        serde_json::from_value(json).expect("library item")
    }

    fn episode(series_id: &str, series_name: &str, number: i64) -> LibraryItem {
        row(serde_json::json!({
            "Id": format!("ep-{series_id}-{number}"),
            "Name": format!("Episode {number}"),
            "Type": "Episode",
            "SeriesId": series_id,
            "SeriesName": series_name,
            "SeriesPrimaryImageTag": "poster-tag",
            "ParentIndexNumber": 1,
            "IndexNumber": number,
        }))
    }

    #[test]
    fn a_zero_child_count_is_not_shown_as_zero_items() {
        // The Latest endpoint sets ChildCount = 0 on every row it did not
        // group, which turned a movie's subtitle into "2003 • 0 items".
        let rows = group_episodes_by_series(vec![row(serde_json::json!({
            "Id": "m1", "Name": "Spider-Man", "Type": "Movie",
            "ProductionYear": 2003, "ChildCount": 0
        }))]);
        assert_eq!(rows[0].subtitle, "2003");
    }

    #[test]
    fn a_season_of_new_episodes_becomes_one_row_for_the_show() {
        let rows = group_episodes_by_series(vec![
            episode("s1670", "1670", 8),
            episode("s1670", "1670", 7),
            episode("s1670", "1670", 6),
        ]);

        assert_eq!(rows.len(), 1, "the shelf still shows one card per episode");
        let show = &rows[0];
        assert_eq!(show.id, "s1670", "the row must open the series, not an episode");
        assert_eq!(show.kind, "Series");
        assert_eq!(show.title, "1670");
        assert_eq!(show.subtitle, "3 new episodes");
        assert!(show.is_container && !show.playable);
    }

    #[test]
    fn a_show_added_with_its_episodes_appears_once() {
        // What "Recently Added" returns when a whole season lands: the series
        // itself, then every episode of it.
        let rows = group_episodes_by_series(vec![
            row(serde_json::json!({"Id": "s1670", "Name": "1670", "Type": "Series"})),
            episode("s1670", "1670", 2),
            episode("s1670", "1670", 1),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "s1670");
        assert_eq!(rows[0].subtitle, "2 new episodes");
    }

    #[test]
    fn the_latest_endpoints_bare_and_grouped_rows_collapse_to_the_larger_count() {
        // `Users/{id}/Items/Latest` with GroupItems=true returns a series twice
        // when the show and its episodes are both new: once bare, once grouped.
        let rows = group_episodes_by_series(vec![
            row(serde_json::json!({"Id": "s1670", "Name": "1670", "Type": "Series", "ChildCount": 0})),
            row(serde_json::json!({"Id": "s1670", "Name": "1670", "Type": "Series", "ChildCount": 8})),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subtitle, "8 new episodes");
    }

    #[test]
    fn one_new_episode_reads_as_one() {
        let rows = group_episodes_by_series(vec![episode("s9", "Say Nothing", 5)]);
        assert_eq!(rows[0].subtitle, "1 new episode");
    }

    #[test]
    fn movies_are_left_alone_and_order_is_preserved() {
        let rows = group_episodes_by_series(vec![
            episode("s1670", "1670", 8),
            row(serde_json::json!({
                "Id": "m1", "Name": "Spider-Man", "Type": "Movie", "ProductionYear": 2024
            })),
            episode("s1670", "1670", 7),
            row(serde_json::json!({"Id": "m2", "Name": "Smoke", "Type": "Movie"})),
        ]);

        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(
            ids,
            ["s1670", "m1", "m2"],
            "grouping must not reorder the server's newest-first list"
        );
        let movie = &rows[1];
        assert_eq!(movie.title, "Spider-Man");
        assert_eq!(movie.subtitle, "2024", "a movie's own subtitle was rewritten");
        assert!(movie.playable);
    }

    #[test]
    fn an_episode_with_no_series_stays_a_row_of_its_own() {
        // Nothing to group it under; dropping it would lose the row entirely.
        let orphan = row(serde_json::json!({
            "Id": "ep-orphan", "Name": "Loose Episode", "Type": "Episode"
        }));
        let rows = group_episodes_by_series(vec![orphan]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "ep-orphan");
        assert_eq!(rows[0].kind, "Episode");
    }

    #[test]
    fn a_newly_added_show_with_no_new_episodes_keeps_its_own_description() {
        let rows = group_episodes_by_series(vec![row(serde_json::json!({
            "Id": "s5", "Name": "Smoke", "Type": "Series", "ProductionYear": 2025
        }))]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Smoke");
        assert_eq!(rows[0].subtitle, "2025");
    }

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

    /// The library rows are the one list that does need the recursive count,
    /// for their "Library - N items" subtitle. It is cheap there because the
    /// query only ever returns the handful of top-level libraries.
    #[test]
    fn library_rows_keep_the_recursive_count_their_subtitle_needs() {
        assert!(fields(LIBRARY_FIELDS).contains(&"RecursiveItemCount"));
        let item: LibraryItem = serde_json::from_str(
            r#"{"Id":"l1","Name":"Movies","Type":"CollectionFolder","RecursiveItemCount":33}"#,
        )
        .unwrap();
        assert_eq!(item_subtitle(&item), "Library • 33 items");
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

    /// An unbounded folder query returns every row in the library, and each row
    /// costs a spawned artwork task and a decode.
    #[test]
    fn browsing_a_folder_is_a_bounded_page() {
        assert!(PAGE_LIMIT.parse::<u32>().is_ok_and(|limit| limit > 0));
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
        assert!(!item.is_container);
    }

    #[test]
    fn episode_tiles_use_the_series_primary_image() {
        let response: ItemsResponse = serde_json::from_str(
            r#"{
                "Items": [{
                    "Id": "episode-1",
                    "Name": "Pilot",
                    "Type": "Episode",
                    "SeriesId": "series-1",
                    "SeriesPrimaryImageTag": "series-poster-tag",
                    "ImageTags": {"Primary": "episode-image-tag"}
                }]
            }"#,
        )
        .unwrap();
        let item: CatalogItemData = response.items.into_iter().next().unwrap().into();

        assert_eq!(item.image_item_id, "series-1");
        assert_eq!(item.image_tag.as_deref(), Some("series-poster-tag"));
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
            people.iter().map(|person| person.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names(&detail.people), ["Regular"]);
        assert_eq!(names(&detail.guest_stars), ["Visitor"]);
        assert_eq!(detail.directors, ["Helmer"]);

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
        assert_eq!(detail.genres, ["Drama", "Mystery"]);
        assert_eq!(detail.studios, ["Example Studio"]);
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
    fn hides_music_and_playlists_from_top_level_libraries() {
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
            .filter(|item| is_visible_library(item))
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(visible_ids, ["movies", "shows"]);
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
        let stopped =
            serde_json::to_value(PlaybackReportBody::stopped(&target, 61.5)).expect("serialize stop");
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
