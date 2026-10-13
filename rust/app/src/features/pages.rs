//! jellyfin-web's pages as data: where the user is ([`PageRoute`]), which
//! library tab is open, how a grid is sorted, filtered and drawn
//! ([`GridSettings`]), and what a loaded page holds ([`PageContent`]).
//!
//! Nothing here touches the network or the window. The API turns a route
//! into content (`api::pages`), the catalog feature navigates between routes,
//! and the UI draws whatever content it is handed.

use crate::api::CatalogItemData;
use crate::services::session::StoredGridSettings;

/// Rows per grid page: jellyfin-web's default "Library page size".
pub const PAGE_SIZE: usize = 100;

/// A top-level library ("user view") as the drawer and My Media list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRef {
    pub id: String,
    pub name: String,
    /// "movies", "tvshows", "music", "playlists", "books", … or empty for a
    /// mixed folder.
    pub collection_type: String,
}

impl LibraryRef {
    pub fn tabs(&self) -> &'static [LibraryTab] {
        LibraryTab::tabs_for(&self.collection_type)
    }
}

/// The tabs under a library's header, as jellyfin-web names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryTab {
    Movies,
    Suggestions,
    Favorites,
    Collections,
    Genres,
    Shows,
    Upcoming,
    Networks,
    Episodes,
    /// Any other library: its folder contents.
    Items,
}

impl LibraryTab {
    const MOVIES: [Self; 5] = [
        Self::Movies,
        Self::Suggestions,
        Self::Favorites,
        Self::Collections,
        Self::Genres,
    ];
    const SHOWS: [Self; 6] = [
        Self::Shows,
        Self::Suggestions,
        Self::Upcoming,
        Self::Genres,
        Self::Networks,
        Self::Episodes,
    ];
    const ITEMS: [Self; 1] = [Self::Items];

    pub fn tabs_for(collection_type: &str) -> &'static [Self] {
        match collection_type {
            "movies" => &Self::MOVIES,
            "tvshows" => &Self::SHOWS,
            _ => &Self::ITEMS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Movies => "Movies",
            Self::Suggestions => "Suggestions",
            Self::Favorites => "Favorites",
            Self::Collections => "Collections",
            Self::Genres => "Genres",
            Self::Shows => "Shows",
            Self::Upcoming => "Upcoming",
            Self::Networks => "TV Networks",
            Self::Episodes => "Episodes",
            Self::Items => "Items",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Suggestions => "suggestions",
            Self::Favorites => "favorites",
            Self::Collections => "collections",
            Self::Genres => "genres",
            Self::Shows => "shows",
            Self::Upcoming => "upcoming",
            Self::Networks => "networks",
            Self::Episodes => "episodes",
            Self::Items => "items",
        }
    }
}

/// What a generic list page (jellyfin-web's `#/list`) narrows its items to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ListFilter {
    /// The direct children of `ListQuery::parent_id`.
    Children,
    Genre(String),
    Studio(String),
    Tag(String),
    /// The user's Next Up episodes (the Home shelf's "›").
    NextUp,
    /// Favorites of `ListQuery::item_types`.
    Favorites,
}

/// A generic list page: a grid of items under a folder, or of every item with
/// a genre, studio, person or tag.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ListQuery {
    pub title: String,
    pub parent_id: Option<String>,
    pub filter: ListFilter,
    /// `IncludeItemTypes`, e.g. "Movie" or "Series"; empty for any.
    pub item_types: String,
}

impl ListQuery {
    pub fn children(title: impl Into<String>, parent_id: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            parent_id: Some(parent_id.into()),
            filter: ListFilter::Children,
            item_types: String::new(),
        }
    }
}

/// Where the user is, for every page that is not an item's detail page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageRoute {
    Home {
        favorites: bool,
    },
    Library {
        library: LibraryRef,
        tab: LibraryTab,
    },
    List(ListQuery),
    Search,
}

impl Default for PageRoute {
    fn default() -> Self {
        Self::home()
    }
}

impl PageRoute {
    pub fn home() -> Self {
        Self::Home { favorites: false }
    }

    pub fn is_home(&self) -> bool {
        matches!(self, Self::Home { .. })
    }

    /// The header title: jellyfin-web names the library, not the tab.
    pub fn title(&self) -> String {
        match self {
            Self::Home { .. } => "Home".to_owned(),
            Self::Library { library, .. } => library.name.clone(),
            Self::List(query) => query.title.clone(),
            Self::Search => "Search".to_owned(),
        }
    }

    /// The header tabs and which one is active.
    pub fn tabs(&self) -> (Vec<&'static str>, usize) {
        match self {
            Self::Home { favorites } => (vec!["Home", "Favorites"], usize::from(*favorites)),
            Self::Library { library, tab } => {
                let tabs = library.tabs();
                if tabs.len() < 2 {
                    return (Vec::new(), 0);
                }
                let active = tabs
                    .iter()
                    .position(|candidate| candidate == tab)
                    .unwrap_or(0);
                (tabs.iter().map(|tab| tab.label()).collect(), active)
            }
            Self::List(_) | Self::Search => (Vec::new(), 0),
        }
    }

    /// The same page on another header tab.
    pub fn with_tab(&self, index: usize) -> Option<Self> {
        match self {
            Self::Home { .. } => (index < 2).then_some(Self::Home {
                favorites: index == 1,
            }),
            Self::Library { library, .. } => library.tabs().get(index).map(|tab| Self::Library {
                library: library.clone(),
                tab: *tab,
            }),
            Self::List(_) | Self::Search => None,
        }
    }

    /// What kind of grid this page shows, or `None` for a page of shelves.
    pub fn grid(&self) -> Option<GridKind> {
        match self {
            Self::Library { tab, library } => match tab {
                LibraryTab::Movies => Some(GridKind::Movies),
                LibraryTab::Favorites => Some(GridKind::Favorites),
                LibraryTab::Collections => Some(GridKind::Collections),
                LibraryTab::Shows => Some(GridKind::Shows),
                LibraryTab::Episodes => Some(GridKind::Episodes),
                LibraryTab::Networks => Some(GridKind::Studios),
                LibraryTab::Items => Some(if library.collection_type == "playlists" {
                    GridKind::Playlists
                } else {
                    GridKind::Folder
                }),
                LibraryTab::Suggestions | LibraryTab::Genres | LibraryTab::Upcoming => None,
            },
            Self::List(query) => Some(match query.item_types.as_str() {
                "Movie" => GridKind::Movies,
                "Series" => GridKind::Shows,
                "Episode" => GridKind::Episodes,
                _ => GridKind::Folder,
            }),
            Self::Home { .. } | Self::Search => None,
        }
    }

    /// The display-preferences key jellyfin-web saves this grid's sort
    /// under, so both clients share it.
    pub fn server_sort_key(&self) -> Option<String> {
        let Self::Library { library, tab } = self else {
            return None;
        };
        let key = match tab {
            LibraryTab::Movies => "movies",
            LibraryTab::Collections => "moviecollections",
            LibraryTab::Shows => "series",
            LibraryTab::Episodes => "episodes",
            _ => return None,
        };
        Some(format!("{}-{key}", library.id))
    }

    /// The key a grid's settings are remembered under: one per library tab,
    /// as jellyfin-web keeps them, and one per kind of list page.
    pub fn settings_key(&self) -> String {
        match self {
            Self::Library { library, tab } => format!("{}:{}", library.id, tab.key()),
            Self::List(query) => format!(
                "list:{}:{}",
                query.item_types,
                query.parent_id.as_deref().unwrap_or("")
            ),
            Self::Home { .. } => "home".to_owned(),
            Self::Search => "search".to_owned(),
        }
    }
}

/// The kinds of grid, which decide the toolbar, the sort orders offered and
/// the default card shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKind {
    Movies,
    /// A library's Favorites tab: just the view switch, sorted by name.
    Favorites,
    Shows,
    Episodes,
    Collections,
    Studios,
    Playlists,
    Folder,
}

impl GridKind {
    /// jellyfin-web's sort menu for this grid: (label, `SortBy` value).
    pub fn sorts(self) -> &'static [SortOption] {
        const fn o(label: &'static str, value: &'static str) -> SortOption {
            SortOption { label, value }
        }
        const MOVIES: [SortOption; 10] = [
            o("Name", "SortName,ProductionYear"),
            o("Random", "Random"),
            o(
                "Community Rating",
                "CommunityRating,SortName,ProductionYear",
            ),
            o("Critics Rating", "CriticRating,SortName,ProductionYear"),
            o("Date Added", "DateCreated,SortName,ProductionYear"),
            o("Date Played", "DatePlayed,SortName,ProductionYear"),
            o("Parental Rating", "OfficialRating,SortName,ProductionYear"),
            o("Play Count", "PlayCount,SortName,ProductionYear"),
            o("Release Date", "PremiereDate,SortName,ProductionYear"),
            o("Runtime", "Runtime,SortName,ProductionYear"),
        ];
        const SHOWS: [SortOption; 8] = [
            o("Name", "SortName"),
            o("Random", "Random"),
            o("Community Rating", "CommunityRating,SortName"),
            o("Date Show Added", "DateCreated,SortName"),
            o("Date Episode Added", "DateLastContentAdded,SortName"),
            o("Date Played", "SeriesDatePlayed,SortName"),
            o("Parental Rating", "OfficialRating,SortName"),
            o("Release Date", "PremiereDate,SortName"),
        ];
        const EPISODES: [SortOption; 8] = [
            o("Name", "SeriesSortName,SortName"),
            o("TheTVDB Rating", "CommunityRating,SeriesSortName,SortName"),
            o("Date Added", "DateCreated,SeriesSortName,SortName"),
            o("Premiere Date", "PremiereDate,SeriesSortName,SortName"),
            o("Date Played", "DatePlayed,SeriesSortName,SortName"),
            o("Parental Rating", "OfficialRating,SeriesSortName,SortName"),
            o("Play Count", "PlayCount,SeriesSortName,SortName"),
            o("Runtime", "Runtime,SeriesSortName,SortName"),
        ];
        const COLLECTIONS: [SortOption; 5] = [
            o("Name", "SortName"),
            o("Community Rating", "CommunityRating,SortName"),
            o("Date Added", "DateCreated,SortName"),
            o("Parental Rating", "OfficialRating,SortName"),
            o("Release Date", "PremiereDate,SortName"),
        ];
        match self {
            Self::Movies | Self::Folder => &MOVIES,
            Self::Shows => &SHOWS,
            Self::Episodes => &EPISODES,
            Self::Collections | Self::Playlists => &COLLECTIONS,
            Self::Studios | Self::Favorites => &[],
        }
    }

    pub fn default_view(self) -> ViewMode {
        match self {
            Self::Episodes | Self::Studios | Self::Playlists => ViewMode::Thumb,
            _ => ViewMode::Poster,
        }
    }

    /// Toolbar buttons: Play All where the grid holds videos, Shuffle where
    /// it holds something to play.
    pub fn plays(self) -> bool {
        matches!(self, Self::Movies | Self::Folder)
    }

    pub fn shuffles(self) -> bool {
        matches!(self, Self::Movies | Self::Folder)
    }

    pub fn filters(self) -> bool {
        matches!(
            self,
            Self::Movies | Self::Shows | Self::Episodes | Self::Folder
        )
    }

    pub fn alphabet(self) -> bool {
        matches!(
            self,
            Self::Movies | Self::Shows | Self::Collections | Self::Folder
        )
    }
}

/// One entry of a sort menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortOption {
    pub label: &'static str,
    /// The `SortBy` query value, also what jellyfin-web stores.
    pub value: &'static str,
}

/// jellyfin-web's "Select view" choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Banner,
    List,
    Poster,
    PosterCard,
    Thumb,
    ThumbCard,
}

impl ViewMode {
    pub const ALL: [Self; 6] = [
        Self::Banner,
        Self::List,
        Self::Poster,
        Self::PosterCard,
        Self::Thumb,
        Self::ThumbCard,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Banner => "Banner",
            Self::List => "List",
            Self::Poster => "Poster",
            Self::PosterCard => "Poster Card",
            Self::Thumb => "Thumb",
            Self::ThumbCard => "Thumb Card",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Banner => "banner",
            Self::List => "list",
            Self::Poster => "poster",
            Self::PosterCard => "posterCard",
            Self::Thumb => "thumb",
            Self::ThumbCard => "thumbCard",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.key() == key)
    }
}

/// The filter dialog's checkboxes, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusFilter {
    Played,
    Unplayed,
    Resumable,
    Favorites,
}

impl StatusFilter {
    pub const ALL: [Self; 4] = [
        Self::Played,
        Self::Unplayed,
        Self::Resumable,
        Self::Favorites,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Played => "Played",
            Self::Unplayed => "Unplayed",
            Self::Resumable => "Resumable",
            Self::Favorites => "Favorites",
        }
    }

    fn query(self) -> &'static str {
        match self {
            Self::Played => "IsPlayed",
            Self::Unplayed => "IsUnplayed",
            Self::Resumable => "IsResumable",
            Self::Favorites => "IsFavorite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeriesStatus {
    Continuing,
    Ended,
    Unreleased,
}

impl SeriesStatus {
    pub const ALL: [Self; 3] = [Self::Continuing, Self::Ended, Self::Unreleased];

    pub fn label(self) -> &'static str {
        match self {
            Self::Continuing => "Continuing",
            Self::Ended => "Ended",
            Self::Unreleased => "Not yet released",
        }
    }

    fn query(self) -> &'static str {
        match self {
            Self::Continuing => "Continuing",
            Self::Ended => "Ended",
            Self::Unreleased => "Unreleased",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    Subtitles,
    Trailer,
    SpecialFeatures,
    ThemeSong,
    ThemeVideo,
}

impl Feature {
    pub const ALL: [Self; 5] = [
        Self::Subtitles,
        Self::Trailer,
        Self::SpecialFeatures,
        Self::ThemeSong,
        Self::ThemeVideo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Subtitles => "Subtitles",
            Self::Trailer => "Trailer",
            Self::SpecialFeatures => "Special Features",
            Self::ThemeSong => "Theme Song",
            Self::ThemeVideo => "Theme Video",
        }
    }

    fn query(self) -> &'static str {
        match self {
            Self::Subtitles => "HasSubtitles",
            Self::Trailer => "HasTrailer",
            Self::SpecialFeatures => "HasSpecialFeature",
            Self::ThemeSong => "HasThemeSong",
            Self::ThemeVideo => "HasThemeVideo",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoType {
    BluRay,
    Dvd,
    Hd,
    FourK,
    Sd,
    ThreeD,
}

impl VideoType {
    pub const ALL: [Self; 6] = [
        Self::BluRay,
        Self::Dvd,
        Self::Hd,
        Self::FourK,
        Self::Sd,
        Self::ThreeD,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::BluRay => "BD",
            Self::Dvd => "DVD",
            Self::Hd => "HD",
            Self::FourK => "4K",
            Self::Sd => "SD",
            Self::ThreeD => "3D",
        }
    }
}

/// What the filter dialog has ticked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridFilters {
    pub status: Vec<StatusFilter>,
    pub series_status: Vec<SeriesStatus>,
    pub features: Vec<Feature>,
    pub video_types: Vec<VideoType>,
    pub genres: Vec<String>,
    pub ratings: Vec<String>,
    pub tags: Vec<String>,
    pub years: Vec<i32>,
    /// The A–Z picker's letter; "#" is everything before "A".
    pub starts_with: Option<String>,
}

impl GridFilters {
    /// How many boxes are ticked, for the badge on the filter button (the
    /// A–Z letter is shown on the picker instead).
    pub fn count(&self) -> usize {
        self.status.len()
            + self.series_status.len()
            + self.features.len()
            + self.video_types.len()
            + self.genres.len()
            + self.ratings.len()
            + self.tags.len()
            + self.years.len()
    }

    /// The query parameters these filters add, as jellyfin-web sends them.
    pub fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        let join = |values: &[String]| values.join("|");
        if !self.status.is_empty() {
            let filters = self
                .status
                .iter()
                .map(|filter| filter.query())
                .collect::<Vec<_>>()
                .join(",");
            query.push(("Filters", filters));
        }
        if !self.series_status.is_empty() {
            let status = self
                .series_status
                .iter()
                .map(|status| status.query())
                .collect::<Vec<_>>()
                .join(",");
            query.push(("SeriesStatus", status));
        }
        for feature in &self.features {
            query.push((feature.query(), "true".to_owned()));
        }
        let mut video_types = Vec::new();
        for video_type in &self.video_types {
            match video_type {
                VideoType::BluRay => video_types.push("BluRay"),
                VideoType::Dvd => video_types.push("Dvd"),
                VideoType::Hd => query.push(("IsHD", "true".to_owned())),
                VideoType::Sd => query.push(("IsHD", "false".to_owned())),
                VideoType::FourK => query.push(("Is4K", "true".to_owned())),
                VideoType::ThreeD => query.push(("Is3D", "true".to_owned())),
            }
        }
        if !video_types.is_empty() {
            query.push(("VideoTypes", video_types.join(",")));
        }
        if !self.genres.is_empty() {
            query.push(("Genres", join(&self.genres)));
        }
        if !self.ratings.is_empty() {
            query.push(("OfficialRatings", join(&self.ratings)));
        }
        if !self.tags.is_empty() {
            query.push(("Tags", join(&self.tags)));
        }
        match self.starts_with.as_deref() {
            Some("#") => query.push(("NameLessThan", "A".to_owned())),
            Some(letter) => query.push(("NameStartsWith", letter.to_owned())),
            None => {}
        }
        if !self.years.is_empty() {
            let years = self
                .years
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            query.push(("Years", years));
        }
        query
    }
}

/// How one grid is sorted, filtered and drawn. Remembered per grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSettings {
    /// The `SortBy` value, in jellyfin-web's vocabulary.
    pub sort: String,
    pub descending: bool,
    pub view: ViewMode,
    pub filters: GridFilters,
}

impl GridSettings {
    pub fn default_for(kind: GridKind) -> Self {
        Self {
            sort: match kind {
                GridKind::Favorites => "SortName,ProductionYear",
                _ => kind
                    .sorts()
                    .first()
                    .map_or("SortName", |option| option.value),
            }
            .to_owned(),
            descending: false,
            view: kind.default_view(),
            filters: GridFilters::default(),
        }
    }

    /// The part that is remembered between runs. Filters last for the
    /// session only.
    pub fn to_stored(&self) -> StoredGridSettings {
        StoredGridSettings {
            sort_by: self.sort.clone(),
            descending: self.descending,
            view: self.view.key().to_owned(),
        }
    }

    pub fn from_stored(stored: &StoredGridSettings) -> Option<Self> {
        Some(Self {
            sort: Some(stored.sort_by.clone()).filter(|sort| !sort.is_empty())?,
            descending: stored.descending,
            view: ViewMode::from_key(&stored.view)?,
            filters: GridFilters::default(),
        })
    }
}

/// What the filter dialog can offer for one grid: the server's genres,
/// ratings, tags and years for those items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterOptions {
    pub genres: Vec<String>,
    pub ratings: Vec<String>,
    pub tags: Vec<String>,
    pub years: Vec<i32>,
}

/// A card shape a shelf asks for; `Auto` lets the items decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardShape {
    Auto,
    Portrait,
    Backdrop,
    Square,
}

/// One titled row of cards.
#[derive(Clone, Debug)]
pub struct Shelf {
    pub id: String,
    pub title: String,
    /// The page the title's "›" opens.
    pub link: Option<PageRoute>,
    pub shape: CardShape,
    /// Text rows under each card.
    pub lines: usize,
    /// Wrap into rows (Genres, Upcoming) instead of scrolling sideways.
    pub wrap: bool,
    pub title_style: TitleStyle,
    pub items: Vec<CatalogItemData>,
}

/// How a section is titled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TitleStyle {
    /// A title row, taller when the title links somewhere.
    #[default]
    Auto,
    /// The taller row even without a link (the Genres tabs).
    Tall,
    /// A bare `h2` heading (the library Suggestions tabs).
    Heading,
}

/// One page of a grid.
#[derive(Clone, Debug, Default)]
pub struct GridPage {
    pub items: Vec<CatalogItemData>,
    pub total: usize,
    pub start: usize,
}

/// What a loaded page shows.
#[derive(Clone, Debug)]
pub enum PageContent {
    Loading,
    Failed(String),
    Shelves(Vec<Shelf>),
    Grid(GridPage),
    Search {
        /// The suggestions shown while the box is empty.
        suggestions: Vec<CatalogItemData>,
        term: String,
        results: Vec<Shelf>,
    },
}

/// A page and what it holds.
#[derive(Clone, Debug)]
pub struct Page {
    pub route: PageRoute,
    pub content: PageContent,
}

impl Page {
    pub fn loading(route: PageRoute) -> Self {
        Self {
            route,
            content: PageContent::Loading,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.content, PageContent::Loading)
    }

    /// Every card on the page, for toggles and lookups by id.
    pub fn items_mut(&mut self) -> Box<dyn Iterator<Item = &mut CatalogItemData> + '_> {
        match &mut self.content {
            PageContent::Shelves(shelves) => {
                Box::new(shelves.iter_mut().flat_map(|shelf| shelf.items.iter_mut()))
            }
            PageContent::Grid(grid) => Box::new(grid.items.iter_mut()),
            PageContent::Search {
                suggestions,
                results,
                ..
            } => Box::new(
                suggestions
                    .iter_mut()
                    .chain(results.iter_mut().flat_map(|shelf| shelf.items.iter_mut())),
            ),
            PageContent::Loading | PageContent::Failed(_) => Box::new(std::iter::empty()),
        }
    }

    pub fn find(&self, id: &str) -> Option<&CatalogItemData> {
        match &self.content {
            PageContent::Shelves(shelves) => shelves
                .iter()
                .flat_map(|shelf| shelf.items.iter())
                .find(|item| item.id == id),
            PageContent::Grid(grid) => grid.items.iter().find(|item| item.id == id),
            PageContent::Search {
                suggestions,
                results,
                ..
            } => suggestions
                .iter()
                .chain(results.iter().flat_map(|shelf| shelf.items.iter()))
                .find(|item| item.id == id),
            PageContent::Loading | PageContent::Failed(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movies() -> LibraryRef {
        LibraryRef {
            id: "lib".into(),
            name: "Movies".into(),
            collection_type: "movies".into(),
        }
    }

    #[test]
    fn libraries_get_jellyfin_tabs() {
        let route = PageRoute::Library {
            library: movies(),
            tab: LibraryTab::Collections,
        };
        let (tabs, active) = route.tabs();
        assert_eq!(
            tabs,
            [
                "Movies",
                "Suggestions",
                "Favorites",
                "Collections",
                "Genres"
            ]
        );
        assert_eq!(active, 3);
        assert_eq!(
            route.with_tab(4),
            Some(PageRoute::Library {
                library: movies(),
                tab: LibraryTab::Genres
            })
        );
        let tv = LibraryTab::tabs_for("tvshows")
            .iter()
            .map(|tab| tab.label())
            .collect::<Vec<_>>();
        assert_eq!(
            tv,
            [
                "Shows",
                "Suggestions",
                "Upcoming",
                "Genres",
                "TV Networks",
                "Episodes"
            ]
        );
        // A plain folder library has no tab strip.
        let music = PageRoute::Library {
            library: LibraryRef {
                collection_type: "music".into(),
                ..movies()
            },
            tab: LibraryTab::Items,
        };
        assert!(music.tabs().0.is_empty());
    }

    #[test]
    fn filters_become_jellyfin_query_parameters() {
        let filters = GridFilters {
            status: vec![StatusFilter::Unplayed, StatusFilter::Favorites],
            features: vec![Feature::Subtitles],
            video_types: vec![VideoType::BluRay, VideoType::FourK],
            genres: vec!["Action".into(), "Drama".into()],
            years: vec![1999, 2004],
            ..GridFilters::default()
        };
        assert_eq!(filters.count(), 9);
        assert_eq!(
            filters.query(),
            vec![
                ("Filters", "IsUnplayed,IsFavorite".to_owned()),
                ("HasSubtitles", "true".to_owned()),
                ("Is4K", "true".to_owned()),
                ("VideoTypes", "BluRay".to_owned()),
                ("Genres", "Action|Drama".to_owned()),
                ("Years", "1999,2004".to_owned()),
            ]
        );
    }

    #[test]
    fn sort_menus_use_jellyfin_values() {
        assert_eq!(
            GridKind::Movies.sorts()[4].value,
            "DateCreated,SortName,ProductionYear"
        );
        assert_eq!(GridKind::Shows.sorts()[3].label, "Date Show Added");
        assert_eq!(
            GridKind::Episodes.sorts()[0].value,
            "SeriesSortName,SortName"
        );
        assert_eq!(
            GridSettings::default_for(GridKind::Movies).sort,
            "SortName,ProductionYear"
        );
    }
}
