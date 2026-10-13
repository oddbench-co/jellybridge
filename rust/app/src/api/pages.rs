//! Loads jellyfin-web's pages: turns a [`PageRoute`] into the requests the
//! web client makes for it and the shelves or grid it shows.
//!
//! Requests for one page run concurrently, and shelves that fail are dropped
//! with a warning rather than failing the page, as on jellyfin-web.

use futures_util::{StreamExt, stream};
use reqwest::Method;
use serde::Deserialize;

use super::{
    ApiError, CatalogItemData, CatalogSectionData, ItemsResponse, JellyfinApi, LIST_FIELDS,
    LibraryItem, encode_path_segment, parse_iso_date,
};
use crate::features::pages::{
    CardShape, FilterOptions, GridKind, GridPage, GridSettings, LibraryRef, LibraryTab, ListFilter,
    ListQuery, PAGE_SIZE, PageContent, PageRoute, Shelf,
};

/// Cards per Genres-tab section, as jellyfin-web shows them.
const GENRE_SECTION_LIMIT: &str = "9";
/// Concurrent requests for one page's sections, so a library with thirty
/// genres does not open thirty connections at once.
const SECTION_CONCURRENCY: usize = 6;
/// Images a grid card can draw in any of the view modes.
const GRID_IMAGE_TYPES: &str = "Primary,Backdrop,Thumb,Banner";

impl JellyfinApi {
    /// Loads a whole page. `settings` applies to grid pages, `start` is the
    /// grid's first row.
    pub async fn load_page(
        &self,
        route: &PageRoute,
        settings: &GridSettings,
        start: usize,
    ) -> Result<PageContent, ApiError> {
        if let Some(kind) = route.grid() {
            return self
                .grid_page(route, kind, settings, start)
                .await
                .map(PageContent::Grid);
        }
        match route {
            PageRoute::Home { favorites: false } => {
                let mut shelves = home_shelves(self.home().await?.sections);
                let libraries = home_libraries(&shelves);
                let deferred = self.deferred_home_sections(&libraries).await;
                merge_home_shelves(&mut shelves, deferred);
                Ok(PageContent::Shelves(shelves))
            }
            PageRoute::Home { favorites: true } => self.favorites_page().await,
            PageRoute::Library { library, tab } => match tab {
                LibraryTab::Suggestions if library.collection_type == "tvshows" => {
                    self.tv_suggestions(library).await
                }
                LibraryTab::Suggestions => self.movie_suggestions(library).await,
                LibraryTab::Genres => self.genres_page(library).await,
                LibraryTab::Upcoming => self.upcoming_page(library).await,
                _ => Ok(PageContent::Shelves(Vec::new())),
            },
            PageRoute::Search => Ok(PageContent::Search {
                suggestions: self.search_suggestions().await.unwrap_or_default(),
                term: String::new(),
                results: Vec::new(),
            }),
            PageRoute::List(_) => Ok(PageContent::Shelves(Vec::new())),
        }
    }

    async fn grid_page(
        &self,
        route: &PageRoute,
        _kind: GridKind,
        settings: &GridSettings,
        start: usize,
    ) -> Result<GridPage, ApiError> {
        let start_text = start.to_string();
        let limit = PAGE_SIZE.to_string();
        let order = if settings.descending {
            "Descending"
        } else {
            "Ascending"
        };
        let mut query: Vec<(&str, String)> = vec![
            ("SortBy", settings.sort.clone()),
            ("SortOrder", order.to_owned()),
            ("StartIndex", start_text),
            ("Limit", limit),
            ("EnableTotalRecordCount", "true".to_owned()),
        ];
        let path = match route {
            PageRoute::Library { library, tab } => {
                match tab {
                    LibraryTab::Networks => {
                        return self.studios_page(library, &query).await;
                    }
                    LibraryTab::Movies | LibraryTab::Favorites => {
                        query.push(("IncludeItemTypes", "Movie".to_owned()));
                        query.push(("Recursive", "true".to_owned()));
                        query.push(("ParentId", library.id.clone()));
                    }
                    LibraryTab::Shows => {
                        query.push(("IncludeItemTypes", "Series".to_owned()));
                        query.push(("Recursive", "true".to_owned()));
                        query.push(("ParentId", library.id.clone()));
                    }
                    LibraryTab::Episodes => {
                        query.push(("IncludeItemTypes", "Episode".to_owned()));
                        query.push(("Recursive", "true".to_owned()));
                        query.push(("ParentId", library.id.clone()));
                    }
                    // Collections live outside the movie library, in the
                    // server's own collections folder.
                    LibraryTab::Collections => {
                        query.push(("IncludeItemTypes", "BoxSet".to_owned()));
                        query.push(("Recursive", "true".to_owned()));
                    }
                    _ => {
                        query.push(("ParentId", library.id.clone()));
                    }
                }
                if *tab == LibraryTab::Favorites {
                    push_status_filter(&mut query, "IsFavorite");
                }
                format!("Users/{}/Items", encode_path_segment(&self.session.user_id))
            }
            PageRoute::List(list) if list.filter == ListFilter::NextUp => {
                let start_text = start.to_string();
                let limit = PAGE_SIZE.to_string();
                let cutoff = super::iso_days_ago(365);
                let response = self
                    .grid_request(
                        "Shows/NextUp",
                        &[
                            ("UserId", self.session.user_id.as_str()),
                            ("StartIndex", start_text.as_str()),
                            ("Limit", limit.as_str()),
                            ("EnableTotalRecordCount", "true"),
                            ("DisableFirstEpisode", "false"),
                            ("NextUpDateCutoff", cutoff.as_str()),
                            ("EnableResumable", "false"),
                        ],
                        "loading next up",
                    )
                    .await?;
                return Ok(GridPage {
                    total: response
                        .total_record_count
                        .unwrap_or(start + response.items.len()),
                    items: response.items.into_iter().map(Into::into).collect(),
                    start,
                });
            }
            PageRoute::List(list) => {
                list_query(list, &mut query);
                format!("Users/{}/Items", encode_path_segment(&self.session.user_id))
            }
            PageRoute::Home { .. } | PageRoute::Search => return Ok(GridPage::default()),
        };
        for (key, value) in settings.filters.query() {
            if key == "Filters" {
                push_status_filter(&mut query, &value);
            } else {
                query.push((key, value));
            }
        }
        let query = query
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect::<Vec<_>>();
        let response = self
            .grid_request(&path, &query, "loading a library page")
            .await?;
        Ok(GridPage {
            total: response
                .total_record_count
                .unwrap_or(start + response.items.len()),
            items: response.items.into_iter().map(Into::into).collect(),
            start,
        })
    }

    async fn grid_request(
        &self,
        path: &str,
        query: &[(&str, &str)],
        operation: &'static str,
    ) -> Result<ItemsResponse, ApiError> {
        let mut full = vec![
            ("Fields", LIST_FIELDS),
            ("EnableUserData", "true"),
            ("EnableImages", "true"),
            ("ImageTypeLimit", "1"),
            ("EnableImageTypes", GRID_IMAGE_TYPES),
        ];
        full.extend_from_slice(query);
        self.send_json(self.request(Method::GET, path)?.query(&full), operation)
            .await
    }

    async fn studios_page(
        &self,
        library: &LibraryRef,
        paging: &[(&str, String)],
    ) -> Result<GridPage, ApiError> {
        let start = paging
            .iter()
            .find(|(key, _)| *key == "StartIndex")
            .and_then(|(_, value)| value.parse().ok())
            .unwrap_or(0);
        let start_text = start.to_string();
        let limit = PAGE_SIZE.to_string();
        let response = self
            .grid_request(
                "Studios",
                &[
                    ("UserId", self.session.user_id.as_str()),
                    ("ParentId", library.id.as_str()),
                    ("IncludeItemTypes", "Series"),
                    ("SortBy", "SortName"),
                    ("SortOrder", "Ascending"),
                    ("StartIndex", start_text.as_str()),
                    ("Limit", limit.as_str()),
                    ("EnableTotalRecordCount", "true"),
                ],
                "loading TV networks",
            )
            .await?;
        Ok(GridPage {
            total: response
                .total_record_count
                .unwrap_or(start + response.items.len()),
            items: response.items.into_iter().map(Into::into).collect(),
            start,
        })
    }

    /// The genres, ratings, tags and years the filter dialog offers.
    pub async fn filter_options(&self, route: &PageRoute) -> Result<FilterOptions, ApiError> {
        let (parent_id, item_types) = match route {
            PageRoute::Library { library, tab } => (
                (*tab != LibraryTab::Collections).then(|| library.id.clone()),
                match tab {
                    LibraryTab::Shows => "Series",
                    LibraryTab::Episodes => "Episode",
                    LibraryTab::Collections => "BoxSet",
                    LibraryTab::Movies | LibraryTab::Favorites => "Movie",
                    _ => "",
                },
            ),
            PageRoute::List(list) => (list.parent_id.clone(), list.item_types.as_str()),
            _ => (None, ""),
        };
        let mut query = vec![("UserId", self.session.user_id.as_str())];
        if let Some(parent_id) = parent_id.as_deref() {
            query.push(("ParentId", parent_id));
        }
        if !item_types.is_empty() {
            query.push(("IncludeItemTypes", item_types));
        }
        let response: FiltersResponse = self
            .send_json(
                self.request(Method::GET, "Items/Filters")?.query(&query),
                "loading filters",
            )
            .await?;
        let mut years = response.years;
        years.sort_unstable();
        Ok(FilterOptions {
            genres: response.genres,
            ratings: response.official_ratings,
            tags: response.tags,
            years,
        })
    }

    /// The Home page's Favorites tab: one shelf per kind of favorite.
    async fn favorites_page(&self) -> Result<PageContent, ApiError> {
        const KINDS: [(&str, &str, CardShape); 5] = [
            ("Movie", "Movies", CardShape::Portrait),
            ("Series", "Shows", CardShape::Portrait),
            ("Episode", "Episodes", CardShape::Backdrop),
            ("BoxSet", "Collections", CardShape::Portrait),
            ("Playlist", "Playlists", CardShape::Square),
        ];
        let user_items = format!("Users/{}/Items", encode_path_segment(&self.session.user_id));
        let shelves = futures_util::future::join_all(KINDS.iter().map(|(kind, title, shape)| {
            let path = user_items.clone();
            async move {
                let response = self
                    .grid_request(
                        &path,
                        &[
                            ("IncludeItemTypes", kind),
                            ("Filters", "IsFavorite"),
                            ("Recursive", "true"),
                            ("SortBy", "SortName"),
                            ("SortOrder", "Ascending"),
                            ("Limit", "20"),
                        ],
                        "loading favorites",
                    )
                    .await;
                shelf_from(
                    response,
                    &format!("favorites-{kind}"),
                    title,
                    *shape,
                    Some(PageRoute::List(ListQuery {
                        title: (*title).to_owned(),
                        parent_id: None,
                        filter: ListFilter::Favorites,
                        item_types: (*kind).to_owned(),
                    })),
                )
            }
        }));
        let people = async {
            let response = self
                .grid_request(
                    "Persons",
                    &[
                        ("UserId", self.session.user_id.as_str()),
                        ("IsFavorite", "true"),
                        ("SortBy", "SortName"),
                        ("Limit", "20"),
                    ],
                    "loading favorite people",
                )
                .await;
            shelf_from(
                response,
                "favorites-people",
                "People",
                CardShape::Portrait,
                None,
            )
        };
        let (mut shelves, people) = tokio::join!(shelves, people);
        shelves.push(people);
        Ok(PageContent::Shelves(
            shelves.into_iter().flatten().collect(),
        ))
    }

    async fn movie_suggestions(&self, library: &LibraryRef) -> Result<PageContent, ApiError> {
        let resume = self.resume_in(&library.id, "Movie");
        let latest = self.latest_of(&library.id, "Movie");
        let recommendations = self.send_json::<Vec<Recommendation>>(
            self.request(Method::GET, "Movies/Recommendations")?
                .query(&[
                    ("UserId", self.session.user_id.as_str()),
                    ("ParentId", library.id.as_str()),
                    ("categoryLimit", "6"),
                    ("ItemLimit", "8"),
                    ("Fields", LIST_FIELDS),
                    ("EnableImageTypes", "Primary,Backdrop,Thumb"),
                    ("ImageTypeLimit", "1"),
                ]),
            "loading recommendations",
        );
        let (resume, latest, recommendations) = tokio::join!(resume, latest, recommendations);
        let mut shelves = Vec::new();
        shelves.extend(shelf_from_items(
            resume,
            "continue",
            "Continue Watching",
            CardShape::Backdrop,
        ));
        shelves.extend(shelf_from_items(
            latest,
            "latest",
            "Recently Added Movies",
            CardShape::Portrait,
        ));
        match recommendations {
            Ok(categories) => {
                for (index, category) in categories.into_iter().enumerate() {
                    if category.items.is_empty() {
                        continue;
                    }
                    shelves.push(Shelf {
                        id: format!("recommendation-{index}"),
                        title: category.title(),
                        link: None,
                        shape: CardShape::Portrait,
                        lines: 2,
                        wrap: false,
                        items: category.items.into_iter().map(Into::into).collect(),
                    });
                }
            }
            Err(error) => tracing::warn!(%error, "recommendations failed"),
        }
        Ok(PageContent::Shelves(shelves))
    }

    async fn tv_suggestions(&self, library: &LibraryRef) -> Result<PageContent, ApiError> {
        let resume = self.resume_in(&library.id, "Episode");
        let latest = self.latest_of(&library.id, "Episode");
        let next_up_query = [
            ("UserId", self.session.user_id.as_str()),
            ("ParentId", library.id.as_str()),
            ("Limit", "24"),
            ("EnableTotalRecordCount", "false"),
        ];
        let next_up = self.items_request("Shows/NextUp", &next_up_query, "loading next up");
        let (resume, latest, next_up) = tokio::join!(resume, latest, next_up);
        let mut shelves = Vec::new();
        shelves.extend(shelf_from_items(
            resume,
            "continue",
            "Continue Watching",
            CardShape::Backdrop,
        ));
        shelves.extend(shelf_from_items(
            latest,
            "latest",
            "Recently Added Episodes",
            CardShape::Portrait,
        ));
        shelves.extend(shelf_from_items(
            next_up,
            "next-up",
            "Next Up",
            CardShape::Backdrop,
        ));
        Ok(PageContent::Shelves(shelves))
    }

    async fn resume_in(&self, parent_id: &str, kind: &str) -> Result<Vec<LibraryItem>, ApiError> {
        self.items_request(
            &format!(
                "Users/{}/Items/Resume",
                encode_path_segment(&self.session.user_id)
            ),
            &[
                ("ParentId", parent_id),
                ("IncludeItemTypes", kind),
                ("Limit", "12"),
                ("MediaTypes", "Video"),
            ],
            "loading continue watching",
        )
        .await
    }

    async fn latest_of(&self, parent_id: &str, kind: &str) -> Result<Vec<LibraryItem>, ApiError> {
        self.send_json(
            self.request(
                Method::GET,
                &format!(
                    "Users/{}/Items/Latest",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .query(&[
                ("ParentId", parent_id),
                ("IncludeItemTypes", kind),
                ("Fields", LIST_FIELDS),
                ("EnableUserData", "true"),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary,Backdrop,Thumb"),
                ("Limit", "18"),
            ]),
            "loading latest media",
        )
        .await
    }

    /// The Genres tab: each genre with a few of its items, and a link to all
    /// of them.
    async fn genres_page(&self, library: &LibraryRef) -> Result<PageContent, ApiError> {
        let item_type = if library.collection_type == "tvshows" {
            "Series"
        } else {
            "Movie"
        };
        let genres = self
            .items_request(
                "Genres",
                &[
                    ("UserId", self.session.user_id.as_str()),
                    ("ParentId", library.id.as_str()),
                    ("IncludeItemTypes", item_type),
                    ("SortBy", "SortName"),
                    ("SortOrder", "Ascending"),
                ],
                "loading genres",
            )
            .await?;
        let user_items = format!("Users/{}/Items", encode_path_segment(&self.session.user_id));
        let shelves = stream::iter(genres.into_iter().map(|genre| {
            let path = user_items.clone();
            async move {
                let items = self
                    .items_request(
                        &path,
                        &[
                            ("GenreIds", genre.id.as_str()),
                            ("ParentId", library.id.as_str()),
                            ("IncludeItemTypes", item_type),
                            ("Recursive", "true"),
                            ("SortBy", "Random"),
                            ("Limit", GENRE_SECTION_LIMIT),
                            ("EnableTotalRecordCount", "false"),
                        ],
                        "loading a genre",
                    )
                    .await;
                let link = PageRoute::List(ListQuery {
                    title: genre.name.clone(),
                    parent_id: Some(library.id.clone()),
                    filter: ListFilter::Genre(genre.id.clone()),
                    item_types: item_type.to_owned(),
                });
                shelf_from_items(
                    items,
                    &format!("genre-{}", genre.id),
                    &genre.name,
                    CardShape::Portrait,
                )
                .map(|shelf| Shelf {
                    link: Some(link),
                    wrap: true,
                    ..shelf
                })
            }
        }))
        .buffered(SECTION_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        Ok(PageContent::Shelves(
            shelves.into_iter().flatten().collect(),
        ))
    }

    /// The Upcoming tab: episodes not yet aired, one section per day.
    async fn upcoming_page(&self, library: &LibraryRef) -> Result<PageContent, ApiError> {
        let items = self
            .items_request(
                "Shows/Upcoming",
                &[
                    ("UserId", self.session.user_id.as_str()),
                    ("ParentId", library.id.as_str()),
                    ("Limit", "40"),
                    ("EnableTotalRecordCount", "false"),
                ],
                "loading upcoming episodes",
            )
            .await?;
        let mut shelves: Vec<Shelf> = Vec::new();
        for item in items {
            let date = item.premiere_date.as_deref().and_then(parse_iso_date);
            let title = date.map_or_else(|| "Unknown".to_owned(), day_heading);
            match shelves.last_mut() {
                Some(shelf) if shelf.title == title => shelf.items.push(item.into()),
                _ => shelves.push(Shelf {
                    id: format!("upcoming-{title}"),
                    title,
                    link: None,
                    shape: CardShape::Backdrop,
                    lines: 2,
                    wrap: true,
                    items: vec![item.into()],
                }),
            }
        }
        Ok(PageContent::Shelves(shelves))
    }

    /// Names suggested on the empty search page.
    async fn search_suggestions(&self) -> Result<Vec<CatalogItemData>, ApiError> {
        let items = self
            .items_request(
                &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                &[
                    ("SortBy", "IsFavoriteOrLiked,Random"),
                    ("IncludeItemTypes", "Movie,Series"),
                    ("Recursive", "true"),
                    ("Limit", "20"),
                    ("EnableTotalRecordCount", "false"),
                ],
                "loading search suggestions",
            )
            .await?;
        Ok(items.into_iter().map(Into::into).collect())
    }

    /// Search results, one shelf per kind, in jellyfin-web's order.
    pub async fn search_results(&self, term: &str) -> Result<Vec<Shelf>, ApiError> {
        const KINDS: [(&str, &str, CardShape); 6] = [
            ("Movie", "Movies", CardShape::Portrait),
            ("Series", "Shows", CardShape::Portrait),
            ("Episode", "Episodes", CardShape::Backdrop),
            ("Video", "Videos", CardShape::Backdrop),
            ("BoxSet", "Collections", CardShape::Portrait),
            ("Playlist", "Playlists", CardShape::Square),
        ];
        let term = term.trim();
        if term.is_empty() {
            return Ok(Vec::new());
        }
        let user_items = format!("Users/{}/Items", encode_path_segment(&self.session.user_id));
        let kinds = futures_util::future::join_all(KINDS.iter().map(|(kind, title, shape)| {
            let path = user_items.clone();
            async move {
                let items = self
                    .items_request(
                        &path,
                        &[
                            ("SearchTerm", term),
                            ("IncludeItemTypes", kind),
                            ("Recursive", "true"),
                            ("Limit", "24"),
                            ("EnableTotalRecordCount", "false"),
                        ],
                        "searching",
                    )
                    .await;
                shelf_from_items(items, &format!("search-{kind}"), title, *shape)
            }
        }));
        let people = async {
            let items = self
                .items_request(
                    "Persons",
                    &[
                        ("UserId", self.session.user_id.as_str()),
                        ("SearchTerm", term),
                        ("Limit", "24"),
                    ],
                    "searching people",
                )
                .await;
            shelf_from_items(items, "search-people", "People", CardShape::Portrait)
        };
        let (shelves, people) = tokio::join!(kinds, people);
        let mut shelves = shelves.into_iter().flatten().collect::<Vec<_>>();
        // People come after the media kinds and before collections.
        if let Some(people) = people {
            let at = shelves
                .iter()
                .position(|shelf| shelf.id == "search-BoxSet" || shelf.id == "search-Playlist")
                .unwrap_or(shelves.len());
            shelves.insert(at, people);
        }
        Ok(shelves)
    }
}

/// jellyfin-web's home sections as shelves, with the title links it gives
/// Next Up and each "Recently Added in …".
pub fn home_shelves(sections: Vec<CatalogSectionData>) -> Vec<Shelf> {
    let libraries = sections
        .iter()
        .find(|section| section.id == "libraries")
        .map(|section| section.items.clone())
        .unwrap_or_default();
    sections
        .into_iter()
        .map(|section| {
            let (shape, lines) = match section.id.as_str() {
                "libraries" => (CardShape::Backdrop, 1),
                id if id.starts_with("continue") || id == "next-up" => (CardShape::Backdrop, 2),
                _ => (CardShape::Auto, 2),
            };
            let link = if section.id == "next-up" {
                Some(PageRoute::List(ListQuery {
                    title: "Next Up".to_owned(),
                    parent_id: None,
                    filter: ListFilter::NextUp,
                    item_types: "Episode".to_owned(),
                }))
            } else {
                section.id.strip_prefix("latest-").and_then(|library_id| {
                    libraries
                        .iter()
                        .find(|library| library.id == library_id)
                        .map(library_route)
                })
            };
            let mut items = section.items;
            if section.id.starts_with("latest-") {
                // jellyfin-web's Recently Added: a show's badge counts the
                // new episodes grouped into it, not its unplayed ones.
                for item in &mut items {
                    if item.kind == "Series" {
                        item.unplayed_count = item.child_count;
                    }
                }
            }
            Shelf {
                id: section.id,
                title: section.title,
                link,
                shape,
                lines,
                wrap: false,
                items,
            }
        })
        .collect()
}

fn home_libraries(shelves: &[Shelf]) -> Vec<CatalogItemData> {
    shelves
        .iter()
        .find(|shelf| shelf.id == "libraries")
        .map(|shelf| shelf.items.clone())
        .unwrap_or_default()
}

/// Puts late-arriving "Recently Added in …" shelves into `home`, in My Media
/// order, replacing any older copy.
pub fn merge_home_shelves(home: &mut Vec<Shelf>, sections: Vec<CatalogSectionData>) {
    if sections.is_empty() {
        return;
    }
    let libraries = home_libraries(home);
    let mut all = std::mem::take(home)
        .into_iter()
        .filter(|shelf| !sections.iter().any(|section| section.id == shelf.id))
        .collect::<Vec<_>>();
    let mut with_libraries = vec![CatalogSectionData {
        id: "libraries".into(),
        title: String::new(),
        items: libraries.clone(),
    }];
    with_libraries.extend(sections);
    all.extend(home_shelves(with_libraries).into_iter().skip(1));
    // Latest shelves follow the libraries' order; everything before them
    // keeps its place (the sort is stable).
    let rank = |shelf: &Shelf| {
        shelf.id.strip_prefix("latest-").map_or(0, |id| {
            1 + libraries
                .iter()
                .position(|library| library.id == id)
                .unwrap_or(libraries.len())
        })
    };
    all.sort_by_key(rank);
    *home = all;
}

/// The page a library card opens.
pub fn library_route(library: &CatalogItemData) -> PageRoute {
    let library = LibraryRef {
        id: library.id.clone(),
        name: library.title.clone(),
        collection_type: library.collection_type.clone(),
    };
    let tab = library.tabs()[0];
    PageRoute::Library { library, tab }
}

fn list_query(list: &ListQuery, query: &mut Vec<(&'static str, String)>) {
    if let Some(parent_id) = &list.parent_id {
        query.push(("ParentId", parent_id.clone()));
    }
    if !list.item_types.is_empty() {
        query.push(("IncludeItemTypes", list.item_types.clone()));
    }
    let recursive = !matches!(list.filter, ListFilter::Children);
    query.push(("Recursive", recursive.to_string()));
    match &list.filter {
        ListFilter::Children => {}
        ListFilter::Genre(id) => query.push(("GenreIds", id.clone())),
        ListFilter::Studio(id) => query.push(("StudioIds", id.clone())),
        ListFilter::Tag(tag) => query.push(("Tags", tag.clone())),
        ListFilter::NextUp => {}
        ListFilter::Favorites => push_status_filter(query, "IsFavorite"),
    }
}

/// Adds to the `Filters` parameter, which several sources share.
fn push_status_filter(query: &mut Vec<(&'static str, String)>, value: &str) {
    if let Some((_, existing)) = query.iter_mut().find(|(key, _)| *key == "Filters") {
        if !existing.split(',').any(|filter| filter == value) {
            existing.push(',');
            existing.push_str(value);
        }
    } else {
        query.push(("Filters", value.to_owned()));
    }
}

fn shelf_from(
    response: Result<ItemsResponse, ApiError>,
    id: &str,
    title: &str,
    shape: CardShape,
    link: Option<PageRoute>,
) -> Option<Shelf> {
    shelf_from_items(response.map(|response| response.items), id, title, shape)
        .map(|shelf| Shelf { link, ..shelf })
}

fn shelf_from_items(
    items: Result<Vec<LibraryItem>, ApiError>,
    id: &str,
    title: &str,
    shape: CardShape,
) -> Option<Shelf> {
    match items {
        Ok(items) if !items.is_empty() => Some(Shelf {
            id: id.to_owned(),
            title: title.to_owned(),
            link: None,
            shape,
            lines: 2,
            wrap: false,
            items: items.into_iter().map(Into::into).collect(),
        }),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, section = title, "page section failed");
            None
        }
    }
}

/// "Tuesday, Oct 13", as jellyfin-web heads an Upcoming day.
fn day_heading((year, month, day): (i32, u32, u32)) -> String {
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Sakamoto's day-of-week.
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let weekday =
        (y + y / 4 - y / 100 + y / 400 + OFFSETS[month as usize - 1] + day as i32).rem_euclid(7);
    format!(
        "{}, {} {}",
        WEEKDAYS[weekday as usize],
        MONTHS[month as usize - 1],
        day
    )
}

#[derive(Deserialize)]
struct FiltersResponse {
    #[serde(rename = "Genres", default)]
    genres: Vec<String>,
    #[serde(rename = "OfficialRatings", default)]
    official_ratings: Vec<String>,
    #[serde(rename = "Tags", default)]
    tags: Vec<String>,
    #[serde(rename = "Years", default)]
    years: Vec<i32>,
}

#[derive(Deserialize)]
struct Recommendation {
    #[serde(rename = "BaselineItemName", default)]
    baseline: String,
    #[serde(rename = "RecommendationType", default)]
    kind: String,
    #[serde(rename = "Items", default)]
    items: Vec<LibraryItem>,
}

impl Recommendation {
    /// jellyfin-web's heading for each kind of recommendation.
    fn title(&self) -> String {
        let baseline = &self.baseline;
        match self.kind.as_str() {
            "SimilarToRecentlyPlayed" => format!("Because you watched {baseline}"),
            "SimilarToLikedItem" => format!("Because you like {baseline}"),
            "HasDirectorFromRecentlyPlayed" | "HasLikedDirector" => {
                format!("Directed by {baseline}")
            }
            "HasActorFromRecentlyPlayed" | "HasLikedActor" => format!("Starring {baseline}"),
            _ => baseline.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upcoming_days_read_like_jellyfin() {
        assert_eq!(day_heading((2026, 10, 13)), "Tuesday, Oct 13");
        assert_eq!(day_heading((2026, 11, 3)), "Tuesday, Nov 3");
        assert_eq!(day_heading((2024, 2, 29)), "Thursday, Feb 29");
    }

    fn section(id: &str) -> CatalogSectionData {
        CatalogSectionData {
            id: id.into(),
            title: id.into(),
            items: vec![CatalogItemData {
                id: format!("{id}-item"),
                ..CatalogItemData::default()
            }],
        }
    }

    #[test]
    fn late_shelves_take_their_library_place() {
        let library = |id: &str| CatalogItemData {
            id: id.into(),
            kind: "CollectionFolder".into(),
            ..CatalogItemData::default()
        };
        let mut home = home_shelves(vec![
            CatalogSectionData {
                id: "libraries".into(),
                title: "My Media".into(),
                items: vec![library("books"), library("music"), library("shows")],
            },
            section("next-up"),
            section("latest-books"),
            section("latest-shows"),
        ]);
        merge_home_shelves(&mut home, vec![section("latest-music")]);
        let ids = home
            .iter()
            .map(|shelf| shelf.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "libraries",
                "next-up",
                "latest-books",
                "latest-music",
                "latest-shows"
            ]
        );
        assert!(home[3].link.is_some());
    }

    #[test]
    fn status_filters_merge_into_one_parameter() {
        let mut query = vec![("Filters", "IsFavorite".to_owned())];
        push_status_filter(&mut query, "IsUnplayed");
        push_status_filter(&mut query, "IsFavorite");
        assert_eq!(query, vec![("Filters", "IsFavorite,IsUnplayed".to_owned())]);
    }
}
