use reqwest::header::{HeaderValue, InvalidHeaderValue};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::features::catalog::CatalogSort;
use crate::features::detail::{
    DetailItemData, DetailKind, DetailPageData, ImageRef, ImageType, PersonData, StreamData,
};
use crate::services::session::StoredSession;

const CLIENT_NAME: &str = "JellyBridge";
const CLIENT_VERSION: &str = "0.1.0-rust";
const DEVICE_NAME: &str = "Desktop";
const DEVICE_ID: &str = "jellybridge-rust-desktop";
const ITEM_FIELDS: &str = "ParentId,SeriesId,SeriesPrimaryImageTag,SeasonId,SeasonName,ParentPrimaryImageItemId,ImageTags,UserData,Overview,RunTimeTicks,MediaStreams,ChildCount,RecursiveItemCount,PrimaryImageAspectRatio,ProductionYear,SeriesName,IndexNumber,ParentIndexNumber,CollectionType";
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
}

#[derive(Clone, Debug)]
pub struct CatalogSectionData {
    pub id: String,
    pub title: String,
    pub items: Vec<CatalogItemData>,
}

#[derive(Clone, Debug)]
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
}

#[derive(Clone, Debug)]
pub struct CatalogPage {
    pub items: Vec<CatalogItemData>,
    pub total_record_count: usize,
}

impl JellyfinApi {
    pub fn from_session(session: StoredSession) -> Result<Self, ApiError> {
        normalize_server_url(&session.server_url)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self { http, session })
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
        Ok((Self::from_session(session)?, account_name))
    }

    pub fn session(&self) -> &StoredSession {
        &self.session
    }

    pub async fn home(&self) -> Result<Vec<CatalogSectionData>, ApiError> {
        let (libraries, resume, next_up, latest, favorites, recent) = tokio::join!(
            self.libraries(),
            self.resume_items(),
            self.next_up(),
            self.latest(),
            self.favorites(),
            self.recent(),
        );
        let mut sections = vec![CatalogSectionData {
            id: "libraries".into(),
            title: "Libraries".into(),
            items: libraries?.into_iter().map(Into::into).collect(),
        }];
        append_optional_section(&mut sections, "continue", "Continue Watching", resume);
        append_optional_section(&mut sections, "next-up", "Next Up", next_up);
        append_optional_section(&mut sections, "latest", "Latest Media", latest);
        append_optional_section(&mut sections, "favorites", "Favorites", favorites);
        append_optional_section(&mut sections, "recent", "Recently Added", recent);
        Ok(sections)
    }

    pub async fn browse(
        &self,
        item: &CatalogItemData,
        sort: CatalogSort,
    ) -> Result<CatalogPage, ApiError> {
        let (sort_by, sort_order) = sort.query();
        let response = if item.kind.eq_ignore_ascii_case("Series") {
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
            self.items_page_request(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                &[
                    ("ParentId", item.id.as_str()),
                    ("Recursive", "false"),
                    ("SortBy", sort_by),
                    ("SortOrder", sort_order),
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
            ("Limit", "120"),
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
        let (seasons, episodes, children_error) = match kind {
            DetailKind::Series => {
                let (seasons, episodes) =
                    tokio::join!(self.series_seasons(item_id), self.episodes(item_id, true),);
                combine_detail_children(seasons, episodes)
            }
            DetailKind::Season => match self.episodes(item_id, false).await {
                Ok(episodes) => (Vec::new(), episodes, None),
                Err(error) => (Vec::new(), Vec::new(), Some(error.to_string())),
            },
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
        };
        Ok(DetailPageData {
            item: item.into(),
            seasons: seasons.into_iter().map(Into::into).collect(),
            episodes: episodes.into_iter().map(Into::into).collect(),
            children_error,
        })
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

    pub async fn fetch_primary_image(&self, item_id: &str) -> Result<Vec<u8>, ApiError> {
        let response = self
            .request(
                Method::GET,
                &format!("Items/{}/Images/Primary", encode_path_segment(item_id)),
            )?
            .query(&[("maxWidth", "320"), ("quality", "85"), ("format", "Jpg")])
            .send()
            .await?;
        let response = check_status(response, "loading artwork")?;
        Ok(response.bytes().await?.to_vec())
    }

    async fn libraries(&self) -> Result<Vec<LibraryItem>, ApiError> {
        Ok(self
            .items_request(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                &[("Recursive", "false")],
                "loading libraries",
            )
            .await?
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

    async fn latest(&self) -> Result<Vec<LibraryItem>, ApiError> {
        self.send_json(
            self.request(
                Method::GET,
                &format!(
                    "Users/{}/Items/Latest",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .query(&[
                ("Fields", ITEM_FIELDS),
                ("EnableUserData", "true"),
                ("EnableImages", "true"),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary"),
                ("Limit", "20"),
                ("IncludeItemTypes", "Movie,Episode,Series"),
                ("GroupItems", "false"),
            ]),
            "loading latest media",
        )
        .await
    }

    async fn favorites(&self) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
            &[
                ("Recursive", "true"),
                ("IncludeItemTypes", "Movie,Episode,Series"),
                ("IsFavorite", "true"),
                ("SortBy", "SortName"),
                ("SortOrder", "Ascending"),
                ("Limit", "20"),
            ],
            "loading favorites",
        )
        .await
    }

    async fn recent(&self) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
            &[
                ("Recursive", "true"),
                ("IncludeItemTypes", "Movie,Episode,Series"),
                ("SortBy", "DateCreated"),
                ("SortOrder", "Descending"),
                ("Limit", "20"),
            ],
            "loading recently added",
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
        let mut query = vec![
            ("Fields", ITEM_FIELDS),
            ("EnableUserData", "true"),
            ("EnableImages", "true"),
            ("ImageTypeLimit", "1"),
            ("EnableImageTypes", "Primary"),
        ];
        query.extend_from_slice(extra_query);
        self.send_json(self.request(Method::GET, path)?.query(&query), operation)
            .await
    }

    fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder, ApiError> {
        let server = normalize_server_url(&self.session.server_url)?;
        let url = server.join(path).map_err(|_| ApiError::InvalidServerUrl)?;
        Ok(self
            .http
            .request(method, url)
            .header("x-emby-authorization", self.authorization()?))
    }

    fn authorization(&self) -> Result<HeaderValue, ApiError> {
        Ok(authorization_value(&self.session.token)?)
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
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
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
        let is_container = !playable;
        let subtitle = item_subtitle(&item);
        let (image_item_id, image_tag) = artwork_identity(&item);
        Self {
            id: item.id,
            title: item.name,
            subtitle,
            kind: item.kind,
            image_item_id,
            image_tag,
            watched: item.user_data.as_ref().is_some_and(|data| data.played),
            progress,
            is_container,
            playable,
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
        let people = item
            .people
            .into_iter()
            .filter(|person| person.kind.is_empty() || person.kind.eq_ignore_ascii_case("Actor"))
            .take(24)
            .map(|person| {
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
                PersonData {
                    id: person.id,
                    name: person.name,
                    role: person.role,
                    image,
                }
            })
            .collect();
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
    [
        stream.language.as_deref().map(str::to_uppercase),
        stream.codec.as_deref().map(str::to_uppercase),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" - ")
}

fn media_summary(streams: &[ItemMediaStream]) -> String {
    let video = streams
        .iter()
        .find(|stream| stream.kind.eq_ignore_ascii_case("video"));
    let audio = streams
        .iter()
        .find(|stream| stream.kind.eq_ignore_ascii_case("audio"));
    let resolution = video.and_then(|stream| match (stream.width, stream.height) {
        (_, Some(height)) if height > 0 => Some(format!("{height}p")),
        (Some(width), _) if width > 0 => Some(format!("{width}px")),
        _ => None,
    });
    [
        resolution,
        video.and_then(|stream| stream.codec.as_deref().map(str::to_uppercase)),
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
    match (item.production_year, item.child_count) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(detail.people[0].role, "Lead");
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
}
