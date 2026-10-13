//! Detail screen: presentation into the view model, drawing, formatters.

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::images::{Art, ImageCache};
use super::jf::{self, CardModel, Shape};
use super::scrollbar;
use super::top_bar;
use super::widgets::{self, Shelf};
use super::{Screen, UiState, detail_art, primary_card_image, theme};
use crate::app::AppContext;
use crate::features::detail::{
    DetailItemData, DetailKind, DetailPageData, ImageType, PersonData, StreamData,
};
use crate::features::playback::clean_preference;

/// What the detail page shows. `Default` is the blank page: resetting is
/// replacing the whole value, so nothing can be left behind from the previous
/// item -- the class of bug that made opening a second movie flash the first
/// one's poster, tagline and cast.
#[derive(Default)]
pub struct DetailVm {
    pub id: String,
    pub title: String,
    pub heading: String,
    pub subheading: String,
    pub tags: Vec<String>,
    pub links: Vec<(String, String)>,
    /// Genres, Director, Writers, Studios: each name links to its page.
    pub credits: Vec<(&'static str, Vec<CreditLink>)>,
    /// Which item types a genre or studio link lists ("Movie", "Series").
    pub link_types: String,
    /// A person's Born / Died / Birth place rows.
    pub person_rows: Vec<(&'static str, String)>,
    pub overview: String,
    pub artwork: Option<Art>,
    pub backdrop: Option<Art>,
    pub logo: Option<Art>,
    pub kind: Option<DetailKind>,
    pub tagline: String,
    pub media_info: String,
    /// Jellyfin's type name, for the item menu.
    pub kind_name: &'static str,
    /// Filmography, collection contents, or a playlist's entries.
    pub sections: Vec<SectionVm>,
    pub series_id: String,
    pub season_id: String,
    pub season_name: String,
    pub progress: f32,
    pub playable: bool,
    pub watched: bool,
    pub favorite: bool,
    pub seasons: Vec<DetailCard>,
    pub next_up: Vec<DetailCard>,
    pub similar: Vec<DetailCard>,
    pub episodes: Vec<DetailCard>,
    pub people: Vec<DetailPerson>,
    pub guest_stars: Vec<DetailPerson>,
    /// Unwatched episodes left in a series or season, for the poster badge.
    pub unplayed_count: i32,
    pub audio_options: Vec<String>,
    pub subtitle_options: Vec<String>,
    pub audio_index: i32,
    pub subtitle_index: i32,
    /// The ribbon's info row, item by item as jellyfin-web lays it out.
    pub misc: Vec<MiscItem>,
}

/// One entry of the info row under a detail page's title.
#[derive(Clone, Debug, PartialEq)]
pub enum MiscItem {
    Text(String),
    /// The official rating, drawn in a tinted box ("FSK-12").
    Badge(String),
    /// Community rating, after a gold star.
    Stars(f32),
    /// Critic rating in percent, after a tomato.
    Critic(i32),
}

/// A name in a credit row and where it leads.
#[derive(Clone, Debug, PartialEq)]
pub struct CreditLink {
    pub name: String,
    pub target: Option<crate::features::items::LinkTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectionVm {
    pub title: String,
    pub wide: bool,
    pub list: bool,
    pub cards: Vec<DetailCard>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailCard {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub overview: String,
    pub kind: &'static str,
    pub art: Option<Art>,
    pub watched: bool,
    pub favorite: bool,
    pub progress: f32,
    pub unplayed_count: i32,
    pub rating: f32,
    pub ends_at: String,
    pub index_label: String,
    pub series_id: Option<String>,
    pub season_id: Option<String>,
    /// The entry's id within a playlist.
    pub playlist_entry: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailPerson {
    pub id: Option<String>,
    pub name: String,
    pub role: String,
    pub art: Option<Art>,
}

/// Switches to an empty detail page while the real one loads.
pub fn present_loading(context: &AppContext) {
    let can_go_back = context
        .state
        .lock()
        .map(|state| !state.history.is_empty())
        .unwrap_or(false);
    context.ui.update(move |state| {
        state.screen = Screen::Detail;
        state.busy = true;
        state.message.clear();
        state.can_go_back = can_go_back;
        state.detail = DetailVm::default();
    });
}

/// Presents the current detail page.
pub fn present(context: &AppContext, _generation: u64) {
    present_page(context, false);
}

/// Re-presents the page after a background refresh. Leaves the screen, busy
/// state, and message alone, so a refresh never pulls the user off the
/// player or clears a status line.
pub fn present_refresh(context: &AppContext, _generation: u64) {
    present_page(context, true);
}

fn present_page(context: &AppContext, background: bool) {
    let (page, error, can_go_back, tracks) = match context.state.lock() {
        Ok(state) => {
            let snapshot = state.detail.snapshot();
            (
                snapshot.page.cloned(),
                snapshot.error.unwrap_or_default().to_owned(),
                !state.history.is_empty(),
                state.detail_tracks.clone(),
            )
        }
        Err(_) => return,
    };
    let Some(page) = page else {
        if background {
            return;
        }
        context.ui.update(move |state| {
            state.screen = Screen::Detail;
            state.busy = false;
            state.message = error;
            state.can_go_back = can_go_back;
            state.detail = DetailVm::default();
        });
        return;
    };
    let message = page.children_error.clone().unwrap_or(error);
    let vm = detail_vm(&page, &tracks);
    context.ui.update(move |state| {
        if !background {
            state.screen = Screen::Detail;
            state.busy = false;
            state.message = message;
        }
        state.can_go_back = can_go_back;
        state.detail = vm;
    });
}

/// Everything the page draws, derived from the loaded page in one place.
fn detail_vm(page: &DetailPageData, tracks: &crate::app::DetailTrackSelection) -> DetailVm {
    let item = &page.item;
    let episode_page = item.kind == DetailKind::Episode;
    let progress_item = if item.kind == DetailKind::Series {
        page.play_target().unwrap_or(item)
    } else {
        item
    };
    let (heading, subheading) = detail_headings(item);
    // `next_up` is only shown on series pages, so only they carry it.
    let next_up = (item.kind == DetailKind::Series)
        .then(|| page.play_target())
        .flatten()
        .map(to_next_up_card)
        .into_iter()
        .collect();
    let audio_options = item
        .streams
        .iter()
        .filter(|stream| stream.kind.eq_ignore_ascii_case("audio"))
        .enumerate()
        .map(|(index, stream)| detail_stream_label(stream, "Audio", index))
        .collect();
    let subtitle_options = std::iter::once("Off".to_owned())
        .chain(
            item.streams
                .iter()
                .filter(|stream| stream.kind.eq_ignore_ascii_case("subtitle"))
                .enumerate()
                .map(|(index, stream)| detail_stream_label(stream, "Subtitle", index)),
        )
        .collect();
    DetailVm {
        id: item.id.clone(),
        title: item.title.clone(),
        heading,
        subheading,
        tags: item.tags.clone(),
        links: item.external_urls.clone(),
        credits: credit_rows(item),
        link_types: link_types(item.kind).to_owned(),
        person_rows: person_rows(item),
        overview: item.overview.clone(),
        // The hero poster asks for a still-sized thumbnail on episode pages.
        artwork: primary_card_image(item).map(|image| {
            detail_art(
                image,
                if image.image_type == ImageType::Thumb {
                    900
                } else {
                    600
                },
            )
        }),
        backdrop: item.backdrop.as_ref().map(|image| detail_art(image, 1600)),
        logo: item.logo.as_ref().map(|image| detail_art(image, 600)),
        kind: Some(item.kind),
        tagline: item.tagline.clone(),
        media_info: item.media_summary.clone(),
        kind_name: detail_kind_name(item.kind),
        sections: page
            .sections
            .iter()
            .map(|section| SectionVm {
                title: section.title.clone(),
                wide: section.wide,
                list: section.list,
                cards: section
                    .items
                    .iter()
                    .map(|item| {
                        let mut card = to_detail_card(item, 900);
                        if !section.list && item.kind == DetailKind::Episode {
                            // jellyfin-web's episode card away from its
                            // season: the show, then "S1:E10 - Title".
                            card.title = item.series_name.clone().unwrap_or_default();
                            card.index_label.clear();
                            card.subtitle = format!("{} {}", next_up_label(item), item.title)
                                .trim()
                                .to_owned();
                        } else if !section.list {
                            card.subtitle = item
                                .production_year
                                .map(|year| year.to_string())
                                .unwrap_or_default();
                        }
                        card
                    })
                    .collect(),
            })
            .collect(),
        series_id: item.series_id.clone().unwrap_or_default(),
        season_id: episode_page
            .then(|| item.season_id.clone())
            .flatten()
            .unwrap_or_default(),
        season_name: episode_page
            .then(|| item.season_name.clone())
            .flatten()
            .unwrap_or_default(),
        progress: progress_item.progress,
        playable: item.kind != DetailKind::Person
            && (item.playable
                || !page.episodes.is_empty()
                || page
                    .sections
                    .iter()
                    .any(|section| !section.items.is_empty())),
        watched: item.watched,
        favorite: item.favorite,
        seasons: page
            .seasons
            .iter()
            .map(|season| to_detail_card(season, 600))
            .collect(),
        next_up,
        // jellyfin-web's poster cards read title and year.
        similar: page
            .similar
            .iter()
            .map(|similar| DetailCard {
                subtitle: similar
                    .production_year
                    .map(|year| year.to_string())
                    .unwrap_or_default(),
                ..to_detail_card(similar, 600)
            })
            .collect(),
        // Episode stills are wide thumbnails and ask for more pixels.
        episodes: page
            .episodes
            .iter()
            .map(|episode| to_detail_card(episode, 900))
            .collect(),
        people: item.people.iter().map(to_detail_person).collect(),
        guest_stars: item.guest_stars.iter().map(to_detail_person).collect(),
        unplayed_count: matches!(item.kind, DetailKind::Series | DetailKind::Season)
            .then_some(item.unplayed_count)
            .flatten()
            .unwrap_or(0),
        audio_options,
        subtitle_options,
        audio_index: tracks.audio_index,
        subtitle_index: tracks.subtitle_index,
        misc: misc_items(page),
    }
}

/// The info row jellyfin-web shows under a detail page's title.
fn misc_items(page: &DetailPageData) -> Vec<MiscItem> {
    let item = &page.item;
    let mut misc = Vec::new();
    match item.kind {
        DetailKind::Season => return misc,
        DetailKind::Episode => {
            if let Some(date) = item.premiere_date.map(format_air_date) {
                misc.push(MiscItem::Text(date));
            } else if let Some(year) = item.production_year {
                misc.push(MiscItem::Text(year.to_string()));
            }
        }
        DetailKind::Series => {
            if let Some(start) = item.production_year {
                let span = if item.series_continuing {
                    format!("{start} - Present")
                } else {
                    match item.end_year {
                        Some(end) if end != start => format!("{start} - {end}"),
                        _ => start.to_string(),
                    }
                };
                misc.push(MiscItem::Text(span));
            }
        }
        _ => {
            if let Some(year) = item.production_year {
                misc.push(MiscItem::Text(year.to_string()));
            }
        }
    }
    if item.kind != DetailKind::Series
        && let Some(runtime) = item.runtime_seconds.filter(|seconds| *seconds > 0.0)
    {
        misc.push(MiscItem::Text(format_runtime(runtime)));
    }
    if let Some(rating) = item
        .official_rating
        .clone()
        .filter(|rating| !rating.is_empty())
    {
        misc.push(MiscItem::Badge(rating));
    }
    if let Some(stars) = item.community_rating.filter(|stars| *stars > 0.0) {
        misc.push(MiscItem::Stars(stars));
    }
    if let Some(critic) = item.critic_rating {
        misc.push(MiscItem::Critic(critic.round() as i32));
    }
    let ends_at = ends_at_label(item);
    if !ends_at.is_empty() {
        misc.push(MiscItem::Text(ends_at));
    }
    misc
}

/// The item types a genre or studio link on this page lists.
fn link_types(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Series | DetailKind::Season | DetailKind::Episode => "Series",
        DetailKind::Movie => "Movie",
        _ => "Movie,Series",
    }
}

/// jellyfin-web's label/value credit rows, each name a link.
fn credit_rows(item: &DetailItemData) -> Vec<(&'static str, Vec<CreditLink>)> {
    use crate::features::items::LinkTarget;
    let types = link_types(item.kind);
    let links = |names: &[crate::features::detail::NamedId],
                 make: &dyn Fn(String, String) -> LinkTarget| {
        names
            .iter()
            .map(|named| CreditLink {
                name: named.name.clone(),
                target: named.id.clone().map(|id| make(id, named.name.clone())),
            })
            .collect::<Vec<_>>()
    };
    let genre = |id, name| LinkTarget::Genre {
        id,
        name,
        item_types: types.to_owned(),
    };
    let studio = |id, name| LinkTarget::Studio {
        id,
        name,
        item_types: types.to_owned(),
    };
    let person = |id, _name| LinkTarget::Person(id);
    [
        ("Genres", links(&item.genres, &genre)),
        ("Director", links(&item.directors, &person)),
        ("Writers", links(&item.writers, &person)),
        ("Studios", links(&item.studios, &studio)),
    ]
    .into_iter()
    .filter(|(_, links)| !links.is_empty())
    .collect()
}

/// A person's "Born", "Died" and "Birth place" rows.
fn person_rows(item: &DetailItemData) -> Vec<(&'static str, String)> {
    if item.kind != DetailKind::Person {
        return Vec::new();
    }
    let mut rows = Vec::new();
    if let Some(born) = item.premiere_date {
        let age = age_in_years(born, item.end_date.unwrap_or_else(today));
        let text = if item.end_date.is_some() {
            format_short_date(born)
        } else {
            format!("{} ({age} years old)", format_short_date(born))
        };
        rows.push(("Born", text));
    }
    if let Some(died) = item.end_date {
        let age = item.premiere_date.map(|born| age_in_years(born, died));
        rows.push((
            "Died",
            match age {
                Some(age) => format!("{} ({age} years old)", format_short_date(died)),
                None => format_short_date(died),
            },
        ));
    }
    if let Some(place) = item.birth_place.clone().filter(|place| !place.is_empty()) {
        rows.push(("Birth place", place));
    }
    rows
}

/// "11/30/1965", as jellyfin-web writes dates in the en-US locale.
fn format_short_date((year, month, day): (i32, u32, u32)) -> String {
    format!("{month}/{day}/{year}")
}

fn age_in_years(born: (i32, u32, u32), at: (i32, u32, u32)) -> i32 {
    let birthday_passed = (at.1, at.2) >= (born.1, born.2);
    at.0 - born.0 - i32::from(!birthday_passed)
}

/// Today's date in local time.
fn today() -> (i32, u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    // libc::localtime_r fills the struct from the epoch in local time.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time: libc::time_t = now;
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return (1970, 1, 1);
    }
    (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
}

/// `thumb_width` is the size asked for when the card art is a 16:9 still.
fn to_detail_card(item: &DetailItemData, thumb_width: u32) -> DetailCard {
    let index_label = (item.kind == DetailKind::Episode)
        .then_some(item.index_number)
        .flatten()
        .map(|number| format!("{number}."))
        .unwrap_or_default();
    DetailCard {
        id: item.id.clone(),
        title: item.title.clone(),
        subtitle: detail_card_subtitle(item),
        overview: item.overview.clone(),
        kind: detail_kind_name(item.kind),
        art: primary_card_image(item).map(|image| {
            detail_art(
                image,
                if image.image_type == ImageType::Thumb {
                    thumb_width
                } else {
                    600
                },
            )
        }),
        watched: item.watched,
        favorite: item.favorite,
        progress: item.progress,
        unplayed_count: item.unplayed_count.unwrap_or(0),
        rating: item.community_rating.unwrap_or(0.0),
        ends_at: ends_at_label(item),
        index_label,
        series_id: item.series_id.clone(),
        season_id: item.season_id.clone(),
        playlist_entry: item.playlist_item_id.clone(),
    }
}

/// The Next Up card names its season too, as Jellyfin's does:
/// "S2:E3 - Title".
fn to_next_up_card(item: &DetailItemData) -> DetailCard {
    let mut card = to_detail_card(item, 600);
    card.index_label = next_up_label(item);
    card
}

fn to_detail_person(person: &PersonData) -> DetailPerson {
    DetailPerson {
        id: person.id.clone(),
        name: person.name.clone(),
        role: person.role.clone(),
        art: person.image.as_ref().map(|image| detail_art(image, 320)),
    }
}

/// Opens an external metadata link in the default browser, off the UI
/// thread's critical path (spawning a process can take milliseconds).
pub fn open_external_url(url: String) {
    std::thread::spawn(move || open_in_browser(&url));
}

/// The ribbon under the backdrop that carries the title and the actions.
const RIBBON_HEIGHT: f32 = 107.0;
const LABEL_WIDTH: f32 = 93.0;
const MEDIA_ROW: f32 = 30.8;

/// Page geometry, in jellyfin-web's viewport units.
struct Layout {
    width: f32,
    /// The backdrop: 40vh.
    backdrop_height: f32,
    padding: f32,
    poster: egui::Vec2,
    /// Where the text column starts: 32.45vw.
    content_left: f32,
}

impl Layout {
    fn new(width: f32, height: f32, kind: Option<DetailKind>, backdrop: bool) -> Self {
        let poster_width = width * 0.25;
        let ratio = match kind {
            Some(DetailKind::Episode | DetailKind::Video) => 9.0 / 16.0,
            Some(DetailKind::Playlist) => 1.0,
            _ => 1.5,
        };
        Self {
            width,
            // Without a backdrop jellyfin-web puts the ribbon right under
            // the header.
            backdrop_height: if backdrop {
                (height * 0.4).max(RIBBON_HEIGHT + 120.0)
            } else {
                top_bar::HEIGHT + 30.0 + RIBBON_HEIGHT
            },
            padding: jf::page_padding(width),
            poster: vec2(poster_width, poster_width * ratio),
            content_left: width * 0.3245,
        }
    }

    fn content_width(&self) -> f32 {
        (self.width - self.content_left - self.padding).max(120.0)
    }

    fn ribbon_top(&self) -> f32 {
        self.backdrop_height - RIBBON_HEIGHT
    }
}

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);
    let page_width = screen.width() - scrollbar::reserved(ui);
    let bar = Rect::from_min_size(screen.min, vec2(page_width, top_bar::HEIGHT));
    if state.busy {
        ui.put(
            Rect::from_center_size(screen.center(), vec2(56.0, 56.0)),
            egui::Spinner::new().size(56.0).color(theme::JF_ACCENT),
        );
        top_bar::draw(ui, state, bar, actions);
        return;
    }
    let layout = Layout::new(
        page_width,
        screen.height(),
        state.detail.kind,
        state.detail.backdrop.is_some(),
    );
    let page = Rect::from_min_size(screen.min, vec2(page_width, screen.height()));
    let mut page_ui = widgets::clipped_child(ui, page);
    let output = ScrollArea::vertical()
        .id_salt(("detail-page", state.detail.id.as_str()))
        .scroll_bar_visibility(scrollbar::HIDDEN)
        .auto_shrink(false)
        .show(&mut page_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let page_top = ui.cursor().top();
            draw_backdrop(ui, state, &layout);
            draw_ribbon(ui, state, &layout, page_top, actions);
            if !state.message.is_empty() {
                draw_error_banner(ui, state, &layout, actions);
            }
            draw_info(ui, state, &layout, actions);
            draw_shelves(ui, state, &layout, actions);
            ui.allocate_exact_size(vec2(layout.width, 40.0), Sense::hover());
            draw_poster(ui, state, &layout, page_top);
        });
    scrollbar::draw(&mut page_ui, &output);
    // The header floats over the backdrop, translucent.
    top_bar::draw(ui, state, bar, actions);
}

fn draw_backdrop(ui: &mut Ui, state: &mut UiState, layout: &Layout) {
    let (hero, _) =
        ui.allocate_exact_size(vec2(layout.width, layout.backdrop_height), Sense::hover());
    if !ui.is_rect_visible(hero) {
        return;
    }
    let painter = ui.painter().with_clip_rect(hero.intersect(ui.clip_rect()));
    if let Some((texture, size)) = resident(&mut state.images, state.detail.backdrop.as_ref()) {
        widgets::image_cover(&painter, texture, size, hero, 0.0);
        // jellyfin-web dims the backdrop under its translucent header and
        // ribbon; the page ground shows through at 50%.
        painter.rect_filled(hero, 0.0, theme::with_alpha(theme::JF_BG, 0.5));
    }
    if let Some((texture, size)) = resident(&mut state.images, state.detail.logo.as_ref()) {
        // Centred in the space above the ribbon, right of the poster column.
        let area = Rect::from_min_max(
            pos2(
                hero.left() + layout.content_left,
                hero.top() + top_bar::HEIGHT,
            ),
            pos2(
                hero.right() - layout.padding,
                hero.top() + layout.ribbon_top(),
            ),
        );
        let logo = Rect::from_center_size(
            area.center(),
            vec2(
                (area.width() * 0.4).min(500.0),
                (area.height() * 0.75).min(200.0),
            ),
        );
        widgets::image_contain(&painter, texture, size, logo);
    }
}

/// The resident texture for `art`, queueing its fetch when missing.
fn resident(images: &mut ImageCache, art: Option<&Art>) -> Option<(egui::TextureId, [usize; 2])> {
    let art = art?;
    images.texture(art).zip(images.size(&art.key))
}

/// The translucent band across the bottom of the backdrop: title lines and
/// the info row on the left, round action buttons on the right.
fn draw_ribbon(
    ui: &mut Ui,
    state: &mut UiState,
    layout: &Layout,
    page_top: f32,
    actions: &mut Vec<UiAction>,
) {
    let detail = &state.detail;
    let busy = state.busy;
    let ribbon = Rect::from_min_size(
        pos2(ui.max_rect().left(), page_top + layout.ribbon_top()),
        vec2(layout.width, RIBBON_HEIGHT),
    );
    let painter = ui.painter().clone();
    painter.rect_filled(ribbon, 0.0, theme::JF_RIBBON);

    // Buttons, right-aligned and vertically centred.
    // Resume/restart are for a single video; a series or season plays its
    // next episode instead.
    let resumable = detail.playable
        && detail.progress > 0.0
        && detail.progress < 1.0
        && detail.kind.is_some_and(DetailKind::is_video);
    let kind = detail.kind.unwrap_or(DetailKind::Other);
    let mut buttons: Vec<(Icon, &str, Color32, &str)> = Vec::new();
    if detail.playable {
        buttons.push((
            Icon::Play,
            if resumable { "Resume" } else { "Play" },
            Color32::WHITE,
            "play",
        ));
        if resumable {
            buttons.push((
                Icon::Replay,
                "Play from beginning",
                Color32::WHITE,
                "restart",
            ));
        }
        if matches!(
            kind,
            DetailKind::Series | DetailKind::Season | DetailKind::BoxSet | DetailKind::Playlist
        ) {
            buttons.push((Icon::Shuffle, "Shuffle", Color32::WHITE, "shuffle"));
        }
    }
    if !matches!(kind, DetailKind::Person | DetailKind::Playlist) {
        buttons.push((
            Icon::Check,
            if detail.watched {
                "Mark unplayed"
            } else {
                "Mark played"
            },
            if detail.watched {
                theme::JF_PLAYED_RED
            } else {
                Color32::WHITE
            },
            "played",
        ));
    }
    buttons.push((
        Icon::Favorite,
        if detail.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        },
        if detail.favorite {
            theme::JF_PLAYED_RED
        } else {
            Color32::WHITE
        },
        "favorite",
    ));
    let menu = super::menu::MenuRequest {
        item_id: detail.id.clone(),
        name: detail.title.clone(),
        kind: detail.kind_name.to_owned(),
        series_id: None,
        season_id: None,
        anchor: Rect::NOTHING,
        origin: super::menu::MenuOrigin::Detail,
    };
    if !super::menu::menu_commands(&menu).is_empty() {
        buttons.push((Icon::MoreVert, "More", Color32::WHITE, "more"));
    }
    let buttons_width = buttons.len() as f32 * 49.0;
    let mut x = ribbon.right() - layout.padding - buttons_width;
    let text_right = x - 16.0;
    for (icon, label, color, name) in buttons {
        let rect = Rect::from_min_size(pos2(x, ribbon.center().y - 22.5), vec2(49.0, 45.0));
        x += 49.0;
        let clicked = jf::round_icon_button(
            ui,
            rect,
            icon,
            if color == Color32::WHITE {
                theme::JF_TEXT
            } else {
                color
            },
            24.8,
            label,
            !busy,
            Id::new(("detail-action", name)),
        )
        .clicked();
        if !clicked {
            continue;
        }
        let queue = matches!(kind, DetailKind::BoxSet | DetailKind::Playlist);
        actions.push(match name {
            "play" if queue => UiAction::PlayQueue {
                source: crate::features::items::queue_source(detail.kind_name, &detail.id),
                shuffle: false,
            },
            "play" => UiAction::PlayCurrent { from_start: false },
            "restart" => UiAction::PlayCurrent { from_start: true },
            "shuffle" => UiAction::PlayQueue {
                source: crate::features::items::queue_source(detail.kind_name, &detail.id),
                shuffle: true,
            },
            "played" => UiAction::SetWatched {
                id: detail.id.clone(),
                watched: !detail.watched,
            },
            "more" => UiAction::OpenMenu(Box::new(super::menu::MenuRequest {
                anchor: rect,
                ..menu.clone()
            })),
            _ => UiAction::SetFavorite {
                id: detail.id.clone(),
                favorite: !detail.favorite,
            },
        });
    }

    // Title block, vertically centred in the ribbon.
    let left = ribbon.left() + layout.content_left;
    let width = (text_right - left).max(80.0);
    let heading_links = !detail.series_id.is_empty()
        && matches!(detail.kind, Some(DetailKind::Episode | DetailKind::Season));
    let heading_text = if detail.heading.is_empty() {
        &detail.title
    } else {
        &detail.heading
    };
    let heading = widgets::elided(
        ui,
        heading_text,
        theme::bold(theme::JF_FONT_TITLE),
        theme::JF_TEXT,
        width,
    );
    let subheading = (!detail.subheading.is_empty()).then(|| {
        widgets::elided(
            ui,
            &detail.subheading,
            theme::bold(theme::JF_FONT_SUBTITLE),
            theme::JF_TEXT,
            width,
        )
    });
    let misc_height = if detail.misc.is_empty() { 0.0 } else { 24.0 };
    let block = heading.size().y
        + subheading.as_ref().map_or(0.0, |line| line.size().y + 2.0)
        + misc_height;
    let mut y = ribbon.top() + (RIBBON_HEIGHT - block) / 2.0;
    let heading_rect = Rect::from_min_size(pos2(left, y), heading.size());
    let sense = if heading_links {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(heading_rect, Id::new("detail-heading"), sense);
    painter.galley(heading_rect.min, heading, Color32::WHITE);
    if heading_links {
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            actions.push(UiAction::OpenDetail {
                id: detail.series_id.clone(),
                kind: "Series".to_owned(),
            });
        }
    }
    y = heading_rect.bottom();
    if let Some(subheading) = subheading {
        let rect = Rect::from_min_size(pos2(left, y + 2.0), subheading.size());
        let links = !detail.season_id.is_empty();
        let sense = if links {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, Id::new("detail-subheading"), sense);
        painter.galley(rect.min, subheading, Color32::WHITE);
        if links {
            widgets::pointer_on_hover(&response);
            if response.clicked() {
                actions.push(UiAction::OpenDetail {
                    id: detail.season_id.clone(),
                    kind: "Season".to_owned(),
                });
            }
        }
        y = rect.bottom();
    }
    if !detail.misc.is_empty() {
        draw_misc(
            ui,
            &painter,
            &detail.misc,
            pos2(left, y + 12.0),
            left + width,
        );
    }
}

/// The info row: plain items 15 px apart, the rating in a box, stars and
/// the critic score with their icons.
fn draw_misc(ui: &Ui, painter: &egui::Painter, misc: &[MiscItem], start: egui::Pos2, right: f32) {
    let center_y = start.y;
    let mut x = start.x;
    let font = theme::regular(theme::JF_FONT);
    for item in misc {
        if x >= right {
            break;
        }
        match item {
            MiscItem::Text(text) => {
                let galley = widgets::elided(ui, text, font.clone(), theme::JF_TEXT, right - x);
                x = widgets::galley_v_centered(painter, x, center_y, galley).right() + 15.0;
            }
            MiscItem::Badge(text) => {
                let galley = widgets::elided(
                    ui,
                    text,
                    theme::regular(theme::JF_FONT_SMALL),
                    theme::JF_BADGE_TEXT,
                    right - x,
                );
                let rect = Rect::from_min_size(
                    pos2(x, center_y - 12.0),
                    vec2(galley.size().x + 13.7, 24.0),
                );
                painter.rect_filled(rect, theme::JF_RADIUS, theme::JF_BADGE);
                widgets::galley_centered(painter, rect, galley);
                x = rect.right() + 15.0;
            }
            MiscItem::Stars(stars) => {
                icons::paint_centered(
                    painter,
                    Icon::Star,
                    pos2(x + 9.0, center_y),
                    18.0,
                    theme::RATING_GOLD,
                );
                let galley = painter.layout_no_wrap(
                    format!("{}", (stars * 10.0).round() / 10.0),
                    font.clone(),
                    theme::JF_TEXT,
                );
                x = widgets::galley_v_centered(painter, x + 21.0, center_y, galley).right() + 15.0;
            }
            MiscItem::Critic(score) => {
                tomato(painter, pos2(x + 9.0, center_y), *score >= 60);
                let galley =
                    painter.layout_no_wrap(score.to_string(), font.clone(), theme::JF_TEXT);
                x = widgets::galley_v_centered(painter, x + 22.0, center_y, galley).right() + 15.0;
            }
        }
    }
}

/// Rotten Tomatoes' mark: a red tomato when fresh, a green splat when not.
fn tomato(painter: &egui::Painter, center: egui::Pos2, fresh: bool) {
    if fresh {
        painter.circle_filled(
            center + vec2(0.0, 1.0),
            7.5,
            Color32::from_rgb(0xfa, 0x32, 0x0a),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + vec2(-4.0, -6.5),
                center + vec2(0.0, -4.0),
                center + vec2(4.0, -6.5),
                center + vec2(1.5, -3.0),
                center + vec2(-1.5, -3.0),
            ],
            Color32::from_rgb(0x00, 0x91, 0x2d),
            egui::Stroke::NONE,
        ));
    } else {
        painter.circle_filled(center, 7.5, Color32::from_rgb(0x0a, 0xc8, 0x55));
        for angle in [0.0_f32, 1.3, 2.6, 3.9, 5.2] {
            painter.circle_filled(
                center + egui::Vec2::angled(angle) * 7.0,
                2.5,
                Color32::from_rgb(0x0a, 0xc8, 0x55),
            );
        }
    }
}

fn draw_error_banner(ui: &mut Ui, state: &UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    let (rect, _) = ui.allocate_exact_size(vec2(layout.width, 44.0), Sense::hover());
    let row = Rect::from_min_max(
        pos2(rect.left() + layout.content_left, rect.top() + 6.0),
        pos2(rect.right() - layout.padding, rect.bottom()),
    );
    let painter = ui.painter();
    let galley = widgets::elided(
        ui,
        &state.message,
        theme::regular(theme::JF_FONT),
        theme::DANGER_TEXT,
        row.width() - 140.0,
    );
    widgets::galley_v_centered(painter, row.left(), row.center().y, galley);
    let button = Rect::from_min_size(
        pos2(row.right() - 120.0, row.top()),
        vec2(120.0, row.height()),
    );
    if super::catalog::raised_button(ui, button, "Retry", Id::new("detail-retry")) {
        actions.push(UiAction::Retry);
    }
}

/// Media rows, tagline, overview, tags, links and credits in the text
/// column under the ribbon.
fn draw_info(ui: &mut Ui, state: &mut UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    let detail = &mut state.detail;
    let left = layout.content_left;
    let width = layout.content_width();
    ui.allocate_exact_size(vec2(layout.width, 22.0), Sense::hover());
    let row = |ui: &mut Ui, height: f32, gap_before: f32| -> Rect {
        if gap_before > 0.0 {
            ui.allocate_exact_size(vec2(layout.width, gap_before), Sense::hover());
        }
        let (rect, _) = ui.allocate_exact_size(vec2(layout.width, height), Sense::hover());
        Rect::from_min_size(pos2(rect.left() + left, rect.top()), vec2(width, height))
    };
    let label = |ui: &Ui, rect: Rect, text: &str| {
        ui.painter().text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            text,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT,
        );
    };
    let value_width = (width - LABEL_WIDTH).min(559.0);
    let mut media_rows = 0;
    if !detail.media_info.is_empty() {
        let rect = row(ui, MEDIA_ROW, 0.0);
        label(ui, rect, "Video");
        let galley = widgets::elided(
            ui,
            &detail.media_info,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT_SECONDARY,
            value_width,
        );
        widgets::galley_v_centered(
            ui.painter(),
            rect.left() + LABEL_WIDTH,
            rect.center().y,
            galley,
        );
        media_rows += 1;
    }
    if detail.audio_options.len() == 1 {
        let rect = row(ui, MEDIA_ROW, 0.0);
        label(ui, rect, "Audio");
        let galley = widgets::elided(
            ui,
            &detail.audio_options[0],
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT_SECONDARY,
            value_width,
        );
        widgets::galley_v_centered(
            ui.painter(),
            rect.left() + LABEL_WIDTH,
            rect.center().y,
            galley,
        );
        media_rows += 1;
    }
    let mut select =
        |ui: &mut Ui, title: &str, options: &[String], index: &mut i32, audio: bool| {
            let rect = row(ui, MEDIA_ROW, 0.0);
            label(ui, rect, title);
            let field = Rect::from_min_size(
                pos2(rect.left() + LABEL_WIDTH, rect.center().y - 13.0),
                vec2(value_width, 26.0),
            );
            if let Some(picked) = track_select(ui, field, options, *index, ("detail-track", audio))
                && picked != *index
            {
                *index = picked;
                actions.push(UiAction::DetailTrack {
                    index: picked,
                    audio,
                });
            }
        };
    if detail.audio_options.len() > 1 {
        let options = std::mem::take(&mut detail.audio_options);
        select(ui, "Audio", &options, &mut detail.audio_index, true);
        detail.audio_options = options;
        media_rows += 1;
    }
    if detail.subtitle_options.len() > 1 {
        let options = std::mem::take(&mut detail.subtitle_options);
        select(ui, "Subtitles", &options, &mut detail.subtitle_index, false);
        detail.subtitle_options = options;
        media_rows += 1;
    }
    let mut gap = if media_rows > 0 { 14.0 } else { 0.0 };
    let paragraph = |ui: &mut Ui, text: &str, font: egui::FontId, color: Color32, gap: &mut f32| {
        let galley = widgets::wrapped(ui, text, font, color, width, None);
        let rect = row(ui, galley.size().y, *gap);
        ui.painter().galley(rect.min, galley, Color32::WHITE);
        *gap = 18.0;
    };
    if !detail.tagline.is_empty() {
        paragraph(
            ui,
            &detail.tagline,
            theme::regular(theme::JF_FONT_SUBTITLE),
            theme::JF_TEXT,
            &mut gap,
        );
    }
    if !detail.overview.is_empty() {
        // jellyfin-web shows five lines and a "Show more" under long text.
        let expanded_id = Id::new(("detail-overview-expanded", detail.id.as_str()));
        let expanded = ui
            .ctx()
            .data(|data| data.get_temp::<bool>(expanded_id))
            .unwrap_or(false);
        let full = widgets::wrapped(
            ui,
            &detail.overview,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT,
            width,
            None,
        );
        let long = full.rows.len() > 5;
        let galley = if long && !expanded {
            widgets::wrapped(
                ui,
                &detail.overview,
                theme::regular(theme::JF_FONT),
                theme::JF_TEXT,
                width,
                Some(5),
            )
        } else {
            full
        };
        let rect = row(ui, galley.size().y, gap);
        ui.painter().galley(rect.min, galley, Color32::WHITE);
        gap = 18.0;
        if long {
            let text = if expanded { "Show less" } else { "Show more" };
            let galley =
                widgets::elided(ui, text, theme::bold(theme::JF_FONT), theme::JF_TEXT, width);
            let rect = row(ui, galley.size().y + 12.0, 6.0);
            let button = Rect::from_min_size(
                pos2(rect.right() - galley.size().x - 8.0, rect.top()),
                galley.size() + vec2(8.0, 12.0),
            );
            let response = ui.interact(button, expanded_id.with("button"), Sense::click());
            widgets::describe_button(&response, true, text);
            ui.painter()
                .galley(button.min + vec2(4.0, 6.0), galley, Color32::WHITE);
            widgets::pointer_on_hover(&response);
            if response.clicked() {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(expanded_id, !expanded));
            }
        }
    }
    for (label, value) in &detail.person_rows {
        let rect = row(ui, 24.0, gap.max(8.0));
        gap = 0.0;
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &format!("{label}: "),
            0.0,
            egui::TextFormat::simple(theme::regular(theme::JF_FONT), theme::JF_TEXT_SECONDARY),
        );
        let bold = *label == "Birth place";
        job.append(
            value,
            0.0,
            egui::TextFormat::simple(
                if bold {
                    theme::bold(theme::JF_FONT)
                } else {
                    theme::regular(theme::JF_FONT)
                },
                if bold {
                    Color32::WHITE
                } else {
                    theme::JF_TEXT_SECONDARY
                },
            ),
        );
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        widgets::galley_v_centered(ui.painter(), rect.left(), rect.center().y, galley);
        if !detail.person_rows.is_empty() {
            gap = 0.0;
        }
    }
    if !detail.person_rows.is_empty() {
        gap = 12.0;
    }
    if !detail.tags.is_empty() {
        // "Tags:" then each tag in bold, linking to everything with it.
        let tags = detail
            .tags
            .iter()
            .map(|tag| CreditLink {
                name: tag.clone(),
                target: Some(crate::features::items::LinkTarget::Tag {
                    name: tag.clone(),
                    item_types: detail.link_types.clone(),
                }),
            })
            .collect::<Vec<_>>();
        let height = link_flow_height(ui, &tags, theme::JF_FONT_SMALL, width, "Tags: ");
        let rect = row(ui, height, gap.max(12.0));
        draw_link_flow(
            ui,
            rect,
            &tags,
            theme::JF_FONT_SMALL,
            "Tags: ",
            "detail-tag",
            actions,
        );
        gap = 8.0;
    }
    if !detail.links.is_empty() {
        let rect = row(ui, 20.0, gap);
        let mut x = rect.left();
        let count = detail.links.len();
        for (index, (name, url)) in detail.links.iter().enumerate() {
            let text = if index + 1 < count {
                format!("{name},")
            } else {
                name.clone()
            };
            let galley = widgets::elided(
                ui,
                &text,
                theme::bold(theme::JF_FONT_SMALL),
                theme::JF_TEXT,
                rect.right() - x,
            );
            let link = Rect::from_min_size(
                pos2(x, rect.center().y - galley.size().y / 2.0),
                galley.size(),
            );
            let response = ui.interact(link, Id::new(("detail-link", index)), Sense::click());
            widgets::describe_button(&response, true, name);
            let color = if response.hovered() {
                theme::JF_ACCENT
            } else {
                Color32::WHITE
            };
            ui.painter().galley(link.min, galley, color);
            widgets::pointer_on_hover(&response);
            if response.on_hover_text(url).clicked() {
                actions.push(UiAction::OpenExternalUrl(url.clone()));
            }
            x = link.right() + 5.0;
            if x >= rect.right() {
                break;
            }
        }
        gap = 18.0;
    }
    let value_width = (width - LABEL_WIDTH).min(450.0);
    for (title, links) in &detail.credits {
        let height = link_flow_height(ui, links, theme::JF_FONT, value_width, "");
        let rect = row(ui, height + 7.3, gap);
        gap = 0.0;
        ui.painter().text(
            rect.min,
            Align2::LEFT_TOP,
            *title,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT,
        );
        let values =
            Rect::from_min_size(rect.min + vec2(LABEL_WIDTH, 0.0), vec2(value_width, height));
        draw_link_flow(ui, values, links, theme::JF_FONT, "", title, actions);
    }
    ui.allocate_exact_size(vec2(layout.width, 20.0), Sense::hover());
}

/// Lays out `prefix` then the names, comma separated, bold, wrapping at
/// `width`: (position, size, index) for each name, and the total height.
fn link_flow_layout(
    ui: &Ui,
    links: &[CreditLink],
    size: f32,
    width: f32,
    prefix: &str,
) -> (Vec<(egui::Pos2, egui::Vec2)>, f32) {
    let line = size * 1.45;
    let prefix_width = if prefix.is_empty() {
        0.0
    } else {
        widgets::text_width(ui, prefix, theme::regular(size))
    };
    let mut x = prefix_width;
    let mut y = 0.0;
    let mut placed = Vec::with_capacity(links.len());
    for (index, link) in links.iter().enumerate() {
        let text = if index + 1 < links.len() {
            format!("{},", link.name)
        } else {
            link.name.clone()
        };
        let text_width = widgets::text_width(ui, &text, theme::bold(size)).min(width);
        if x > 0.0 && x + text_width > width {
            x = 0.0;
            y += line;
        }
        placed.push((pos2(x, y), vec2(text_width, line)));
        x += text_width + size * 0.3;
    }
    (placed, y + line)
}

fn link_flow_height(ui: &Ui, links: &[CreditLink], size: f32, width: f32, prefix: &str) -> f32 {
    link_flow_layout(ui, links, size, width, prefix).1
}

/// Draws a link flow in `rect`; a clicked name becomes an `OpenLink`.
fn draw_link_flow(
    ui: &mut Ui,
    rect: Rect,
    links: &[CreditLink],
    size: f32,
    prefix: &str,
    id: &str,
    actions: &mut Vec<UiAction>,
) {
    let (placed, _) = link_flow_layout(ui, links, size, rect.width(), prefix);
    if !prefix.is_empty() {
        ui.painter().text(
            rect.min,
            Align2::LEFT_TOP,
            prefix,
            theme::regular(size),
            theme::JF_TEXT,
        );
    }
    for (index, (link, (offset, extent))) in links.iter().zip(placed).enumerate() {
        let area = Rect::from_min_size(rect.min + offset.to_vec2(), extent);
        let text = if index + 1 < links.len() {
            format!("{},", link.name)
        } else {
            link.name.clone()
        };
        let color = match &link.target {
            Some(target) => {
                let response = ui.interact(area, Id::new((id, index)), Sense::click());
                widgets::describe_button(&response, true, &link.name);
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    actions.push(UiAction::OpenLink(target.clone()));
                }
                if response.hovered() {
                    theme::JF_ACCENT
                } else {
                    Color32::WHITE
                }
            }
            None => Color32::WHITE,
        };
        let galley = widgets::elided(ui, &text, theme::bold(size), color, area.width() + 1.0);
        ui.painter().galley(area.min, galley, color);
    }
}

/// jellyfin-web's select: a `#292929` field with a chevron, opening a list.
fn track_select(
    ui: &mut Ui,
    field: Rect,
    options: &[String],
    index: i32,
    id: (&str, bool),
) -> Option<i32> {
    let selected = usize::try_from(index).unwrap_or(0);
    let mut picked = selected;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
    child.spacing_mut().interact_size.y = field.height();
    child.visuals_mut().widgets.inactive.weak_bg_fill = theme::JF_INPUT;
    child.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
    child.visuals_mut().widgets.hovered.weak_bg_fill = theme::JF_RAISED;
    egui::ComboBox::from_id_salt(id)
        .width(field.width())
        .selected_text(
            egui::RichText::new(options.get(selected).map_or("", String::as_str))
                .font(theme::regular(theme::JF_FONT))
                .color(theme::JF_TEXT),
        )
        .show_ui(&mut child, |ui| {
            for (option_index, option) in options.iter().enumerate() {
                ui.selectable_value(&mut picked, option_index, option);
            }
        });
    (picked != selected).then(|| i32::try_from(picked).unwrap_or(0))
}

fn draw_shelves(ui: &mut Ui, state: &mut UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    let detail = &state.detail;
    let images = &mut state.images;
    let shelf = ShelfSpec { layout };
    match detail.kind {
        Some(DetailKind::Series) => {
            shelf.cards(
                ui,
                images,
                "next-up",
                "Next Up",
                &detail.next_up,
                Shape::Backdrop,
                1,
                &detail.id,
                actions,
            );
            if detail.seasons.is_empty() {
                let (rect, _) = ui.allocate_exact_size(vec2(layout.width, 46.0), Sense::hover());
                let text = if state.message.is_empty() {
                    "No seasons are available for this series."
                } else {
                    "Seasons could not be loaded."
                };
                ui.painter().text(
                    pos2(rect.left() + layout.content_left, rect.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    theme::regular(theme::JF_FONT),
                    theme::JF_TEXT_SECONDARY,
                );
            } else {
                shelf.cards(
                    ui,
                    images,
                    "seasons",
                    "Seasons",
                    &detail.seasons,
                    Shape::Portrait,
                    1,
                    &detail.id,
                    actions,
                );
            }
        }
        Some(DetailKind::Season) => {
            draw_episode_list(
                ui,
                images,
                detail,
                !state.message.is_empty(),
                state.busy,
                layout,
                actions,
            );
        }
        Some(DetailKind::Episode) => {
            let heading = if detail.season_name.is_empty() {
                "More from this season".to_owned()
            } else {
                format!("More from {}", detail.season_name)
            };
            shelf.cards(
                ui,
                images,
                "season-episodes",
                &heading,
                &detail.episodes,
                Shape::Backdrop,
                1,
                &detail.id,
                actions,
            );
        }
        _ => {}
    }
    for section in &detail.sections {
        if section.list {
            let origin = |card: &DetailCard| super::menu::MenuOrigin::PlaylistEntry {
                playlist_id: detail.id.clone(),
                entry_id: card.playlist_entry.clone().unwrap_or_default(),
            };
            draw_rows(
                ui,
                images,
                &section.cards,
                state.busy,
                layout,
                &origin,
                actions,
            );
        } else {
            shelf.wrapped(
                ui,
                images,
                &section.title,
                &section.cards,
                if section.wide {
                    Shape::Backdrop
                } else {
                    Shape::Portrait
                },
                &detail.id,
                actions,
            );
        }
    }
    shelf.people(
        ui,
        images,
        "Cast & Crew",
        &detail.people,
        &detail.id,
        actions,
    );
    shelf.people(
        ui,
        images,
        "Guest Stars",
        &detail.guest_stars,
        &detail.id,
        actions,
    );
    shelf.cards(
        ui,
        images,
        "similar",
        "More Like This",
        &detail.similar,
        Shape::Portrait,
        2,
        &detail.id,
        actions,
    );
}

/// Shelves in the text column: the title row, then a row of cards that
/// scrolls from the column's left edge to the window's right edge.
struct ShelfSpec<'a> {
    layout: &'a Layout,
}

impl ShelfSpec<'_> {
    /// The shelf's frame: title row and card viewport, plus its scroller.
    fn frame(
        &self,
        ui: &mut Ui,
        title: &str,
        count: usize,
        card_width: f32,
        card_height: f32,
        id: Id,
    ) -> Option<(Shelf, Rect, f32)> {
        let layout = self.layout;
        ui.allocate_exact_size(vec2(layout.width, 10.0), Sense::hover());
        let (rect, _) =
            ui.allocate_exact_size(vec2(layout.width, 46.0 + card_height), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let viewport = Rect::from_min_max(
            pos2(rect.left() + layout.content_left, rect.top() + 46.0),
            rect.max,
        );
        let content_width = count as f32 * card_width + layout.padding;
        let mut shelf = Shelf::begin(ui, id, viewport, content_width);
        let offset = shelf.scroll(ui);
        let title_row = Rect::from_min_max(
            pos2(rect.left() + layout.content_left, rect.top()),
            pos2(rect.right(), rect.top() + 46.0),
        );
        let paging = (content_width > viewport.width())
            .then(|| (shelf.can_page_back(offset), shelf.can_page_forward(offset)));
        match jf::section_title(ui, title_row, title, 0.0, paging, id.with("paging")) {
            -1 => shelf.page(false),
            1 => shelf.page(true),
            _ => {}
        }
        Some((shelf, viewport, offset))
    }

    #[allow(clippy::too_many_arguments)]
    fn cards(
        &self,
        ui: &mut Ui,
        images: &mut ImageCache,
        name: &str,
        title: &str,
        cards: &[DetailCard],
        shape: Shape,
        lines: usize,
        page: &str,
        actions: &mut Vec<UiAction>,
    ) {
        if cards.is_empty() {
            return;
        }
        let width = jf::overflow_width(self.layout.width, shape);
        let height = jf::card_height(width, shape, lines);
        let id = Id::new(("detail-shelf", name, page.to_owned()));
        let Some((mut shelf, viewport, offset)) =
            self.frame(ui, title, cards.len(), width, height, id)
        else {
            return;
        };
        let mut strip = widgets::clipped_child(ui, viewport);
        for (index, card) in cards.iter().enumerate() {
            let left = index as f32 * width;
            let x = viewport.left() + offset + left;
            if x + width < viewport.left() - viewport.width()
                || x > viewport.right() + viewport.width()
            {
                continue;
            }
            let title = card_title(card);
            let model = CardModel {
                placeholder_icon: None,
                title: &title,
                subtitle: &card.subtitle,
                art: card.art.as_ref(),
                watched: card.watched,
                unplayed: u32::try_from(card.unplayed_count).unwrap_or(0),
                progress: if matches!(card.kind, "Movie" | "Episode") {
                    card.progress
                } else {
                    0.0
                },
                favorite: card.favorite,
                playable: true,
                actions: true,
                more: true,
                title_link: true,
                subtitle_link: true,
            };
            let response = jf::card(
                &mut strip,
                images,
                &model,
                pos2(x, viewport.top()),
                width,
                shape,
                lines,
                id.with(index),
            );
            if response.gained_focus {
                shelf.reveal(left, width);
            }
            if let Some(anchor) = response.more {
                actions.push(card_menu(&cards[index..], anchor));
            }
            if response.open {
                actions.push(UiAction::OpenDetail {
                    id: card.id.clone(),
                    kind: card.kind.to_owned(),
                });
            }
            if response.play {
                actions.push(UiAction::PlayItem(card.id.clone()));
            }
            if response.toggle_played {
                actions.push(UiAction::SetWatched {
                    id: card.id.clone(),
                    watched: !card.watched,
                });
            }
            if response.toggle_favorite {
                actions.push(UiAction::SetFavorite {
                    id: card.id.clone(),
                    favorite: !card.favorite,
                });
            }
        }
        shelf.end(ui);
    }

    /// A titled grid of cards wrapping within the text column, as person and
    /// collection pages list their items.
    #[allow(clippy::too_many_arguments)]
    fn wrapped(
        &self,
        ui: &mut Ui,
        images: &mut ImageCache,
        title: &str,
        cards: &[DetailCard],
        shape: Shape,
        page: &str,
        actions: &mut Vec<UiAction>,
    ) {
        if cards.is_empty() {
            return;
        }
        let layout = self.layout;
        let grid = layout.content_width();
        let width = jf::grid_card_width(layout.width, grid, shape);
        let columns = ((grid / width).floor() as usize).max(1);
        let height = jf::card_height(width, shape, 2);
        let rows = cards.len().div_ceil(columns);
        ui.allocate_exact_size(vec2(layout.width, 10.0), Sense::hover());
        let (rect, _) = ui.allocate_exact_size(
            vec2(layout.width, 46.0 + rows as f32 * height),
            Sense::hover(),
        );
        if !ui.is_rect_visible(rect) {
            return;
        }
        let id = Id::new(("detail-grid", title, page.to_owned()));
        let title_row = Rect::from_min_max(
            pos2(rect.left() + layout.content_left, rect.top()),
            pos2(rect.right(), rect.top() + 46.0),
        );
        jf::section_title(ui, title_row, title, 0.0, None, id.with("title"));
        for (index, card) in cards.iter().enumerate() {
            let origin = pos2(
                rect.left() + layout.content_left + (index % columns) as f32 * width,
                rect.top() + 46.0 + (index / columns) as f32 * height,
            );
            if !ui.is_rect_visible(Rect::from_min_size(origin, vec2(width, height))) {
                continue;
            }
            let label = card_title(card);
            let model = CardModel {
                placeholder_icon: None,
                title: &label,
                subtitle: &card.subtitle,
                art: card.art.as_ref(),
                watched: card.watched,
                unplayed: u32::try_from(card.unplayed_count).unwrap_or(0),
                progress: if matches!(card.kind, "Movie" | "Episode") {
                    card.progress
                } else {
                    0.0
                },
                favorite: card.favorite,
                playable: true,
                actions: true,
                more: true,
                title_link: true,
                subtitle_link: true,
            };
            let response = jf::card(ui, images, &model, origin, width, shape, 2, id.with(index));
            if let Some(anchor) = response.more {
                actions.push(card_menu(&cards[index..], anchor));
            }
            if response.open {
                actions.push(UiAction::OpenDetail {
                    id: card.id.clone(),
                    kind: card.kind.to_owned(),
                });
            }
            if response.play {
                actions.push(UiAction::PlayItem(card.id.clone()));
            }
            if response.toggle_played {
                actions.push(UiAction::SetWatched {
                    id: card.id.clone(),
                    watched: !card.watched,
                });
            }
            if response.toggle_favorite {
                actions.push(UiAction::SetFavorite {
                    id: card.id.clone(),
                    favorite: !card.favorite,
                });
            }
        }
    }

    /// Cast and crew: portrait photos with the name and role.
    fn people(
        &self,
        ui: &mut Ui,
        images: &mut ImageCache,
        title: &str,
        people: &[DetailPerson],
        page: &str,
        actions: &mut Vec<UiAction>,
    ) {
        if people.is_empty() {
            return;
        }
        let width = jf::overflow_width(self.layout.width, Shape::Portrait);
        let height = jf::card_height(width, Shape::Portrait, 2);
        let id = Id::new(("detail-people", title, page.to_owned()));
        let Some((shelf, viewport, offset)) =
            self.frame(ui, title, people.len(), width, height, id)
        else {
            return;
        };
        let mut strip = widgets::clipped_child(ui, viewport);
        for (index, person) in people.iter().enumerate() {
            let x = viewport.left() + offset + index as f32 * width;
            if x + width < viewport.left() - viewport.width()
                || x > viewport.right() + viewport.width()
            {
                continue;
            }
            let model = CardModel {
                placeholder_icon: None,
                title: &person.name,
                subtitle: &person.role,
                art: person.art.as_ref(),
                watched: false,
                unplayed: 0,
                progress: 0.0,
                favorite: false,
                playable: false,
                actions: false,
                more: false,
                title_link: true,
                subtitle_link: true,
            };
            let response = jf::card(
                &mut strip,
                images,
                &model,
                pos2(x, viewport.top()),
                width,
                Shape::Portrait,
                2,
                id.with(index),
            );
            if response.open
                && let Some(person) = &person.id
            {
                actions.push(UiAction::OpenLink(
                    crate::features::items::LinkTarget::Person(person.clone()),
                ));
            }
        }
        shelf.end(ui);
    }
}

/// The item menu for the first of `rest`, with "Play all from here" over
/// the videos from it on.
fn card_menu(rest: &[DetailCard], anchor: Rect) -> UiAction {
    let card = &rest[0];
    let queue = rest
        .iter()
        .filter(|card| matches!(card.kind, "Movie" | "Episode" | "Video"))
        .map(|card| card.id.clone())
        .collect();
    UiAction::OpenMenu(Box::new(super::menu::MenuRequest {
        item_id: card.id.clone(),
        name: card.title.clone(),
        kind: card.kind.to_owned(),
        series_id: card.series_id.clone(),
        season_id: card.season_id.clone(),
        anchor,
        origin: super::menu::MenuOrigin::Card { queue },
    }))
}

/// A shelf card's text: "1. Title" for an episode, "S1:E16 - Title" on
/// Next Up (the presenter puts the number in the label), else the title.
fn card_title(card: &DetailCard) -> String {
    if card.index_label.is_empty() {
        card.title.clone()
    } else {
        format!("{} {}", card.index_label, card.title)
    }
}

/// A season page's episode list, as jellyfin-web's: thumbnail with a play
/// button, number and title, runtime/rating/ends-at, overview, and the
/// info / played / favorite buttons.
fn draw_episode_list(
    ui: &mut Ui,
    images: &mut ImageCache,
    detail: &DetailVm,
    failed: bool,
    busy: bool,
    layout: &Layout,
    actions: &mut Vec<UiAction>,
) {
    let left = layout.content_left;
    let width = layout.content_width();
    if detail.episodes.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(layout.width, 60.0), Sense::hover());
        let text = if failed {
            "Episodes could not be loaded."
        } else {
            "No episodes are available for this season."
        };
        ui.painter().text(
            pos2(rect.left() + left, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT_SECONDARY,
        );
        return;
    }
    let origin = |card: &DetailCard| {
        let from = detail
            .episodes
            .iter()
            .position(|episode| episode.id == card.id)
            .unwrap_or(0);
        super::menu::MenuOrigin::Card {
            queue: detail.episodes[from..]
                .iter()
                .map(|episode| episode.id.clone())
                .collect(),
        }
    };
    draw_rows(ui, images, &detail.episodes, busy, layout, &origin, actions);
    let _ = (left, width);
}

/// jellyfin-web's list rows (a season's episodes, a playlist's entries).
fn draw_rows(
    ui: &mut Ui,
    images: &mut ImageCache,
    cards: &[DetailCard],
    busy: bool,
    layout: &Layout,
    origin: &dyn Fn(&DetailCard) -> super::menu::MenuOrigin,
    actions: &mut Vec<UiAction>,
) {
    let left = layout.content_left;
    let width = layout.content_width();
    for (index, card) in cards.iter().enumerate() {
        let (row, _) = ui.allocate_exact_size(vec2(layout.width, 181.0), Sense::hover());
        if !ui.is_rect_visible(row) {
            continue;
        }
        let row = Rect::from_min_size(pos2(row.left() + left, row.top()), vec2(width, 181.0));
        episode_row(
            ui,
            images,
            card,
            row,
            busy,
            index,
            &|| origin(card),
            actions,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn episode_row(
    ui: &mut Ui,
    images: &mut ImageCache,
    episode: &DetailCard,
    row: Rect,
    busy: bool,
    index: usize,
    origin: &dyn Fn() -> super::menu::MenuOrigin,
    actions: &mut Vec<UiAction>,
) {
    let painter = ui.painter().clone();
    let thumb = Rect::from_min_size(row.min + vec2(8.0, 9.0), vec2(246.0, 164.0));
    let response = ui.interact(
        thumb,
        Id::new(("episode-thumb", index, episode.id.as_str())),
        if busy { Sense::hover() } else { Sense::click() },
    );
    widgets::describe_button(&response, !busy, &format!("Play {}", episode.title));
    match resident(images, episode.art.as_ref()) {
        Some((texture, size)) => widgets::image_cover(&painter, texture, size, thumb, 0.0),
        None => {
            painter.rect_filled(thumb, 0.0, theme::JF_CARD_EMPTY);
        }
    }
    if episode.progress > 0.0 && episode.progress < 1.0 {
        let track = Rect::from_min_max(
            pos2(thumb.left(), thumb.bottom() - 4.0),
            thumb.right_bottom(),
        );
        painter.rect_filled(track, 0.0, theme::JF_PROGRESS_TRACK);
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(track.width() * episode.progress, 4.0)),
            0.0,
            theme::JF_ACCENT,
        );
    }
    // jellyfin-web's list play button: a large, faint play icon centred on
    // the thumbnail, brighter under the pointer.
    let play_color = if response.hovered() {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(153)
    };
    icons::paint_centered(&painter, Icon::Play, thumb.center(), 40.0, play_color);
    if response.has_focus() {
        widgets::border(&painter, thumb, 0.0, 2.0, theme::JF_ACCENT);
    }
    if !busy {
        widgets::pointer_on_hover(&response);
    }
    if response.clicked() {
        actions.push(UiAction::PlayDetail(episode.id.clone()));
    }

    // Buttons on the right: info, played, favorite, more.
    let buttons_left = row.right() - 4.0 * 41.5;
    let center_y = row.top() + 90.0;
    let mut x = buttons_left;
    let mut button = |icon: Icon, color: Color32, label: &str, name: &str| {
        let rect = Rect::from_min_size(pos2(x, center_y - 20.5), vec2(41.0, 41.0));
        x += 41.5;
        jf::round_icon_button(
            ui,
            rect,
            icon,
            color,
            24.8,
            label,
            !busy,
            Id::new(("episode-action", index, name)),
        )
        .clicked()
    };
    if button(Icon::InfoOutline, theme::JF_TEXT, "Info", "info") {
        actions.push(UiAction::OpenDetail {
            id: episode.id.clone(),
            kind: episode.kind.to_owned(),
        });
    }
    let played_color = if episode.watched {
        theme::JF_PLAYED_RED
    } else {
        theme::JF_TEXT
    };
    if button(
        Icon::Check,
        played_color,
        if episode.watched {
            "Mark unplayed"
        } else {
            "Mark played"
        },
        "played",
    ) {
        actions.push(UiAction::SetWatched {
            id: episode.id.clone(),
            watched: !episode.watched,
        });
    }
    let favorite_color = if episode.favorite {
        theme::JF_PLAYED_RED
    } else {
        theme::JF_TEXT
    };
    if button(
        Icon::Favorite,
        favorite_color,
        if episode.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        },
        "favorite",
    ) {
        actions.push(UiAction::SetFavorite {
            id: episode.id.clone(),
            favorite: !episode.favorite,
        });
    }
    let more_rect = Rect::from_min_size(
        pos2(buttons_left + 3.0 * 41.5, center_y - 20.5),
        vec2(41.0, 41.0),
    );
    if button(Icon::MoreVert, theme::JF_TEXT, "More", "more") {
        actions.push(UiAction::OpenMenu(Box::new(super::menu::MenuRequest {
            item_id: episode.id.clone(),
            name: episode.title.clone(),
            kind: episode.kind.to_owned(),
            series_id: episode.series_id.clone(),
            season_id: episode.season_id.clone(),
            anchor: more_rect,
            origin: origin(),
        })));
    }

    // Text column between the thumbnail and the buttons.
    let text_left = thumb.right() + 22.0;
    let text_width = (buttons_left - 12.0 - text_left).max(60.0);
    let label = if episode.index_label.is_empty() {
        episode.title.clone()
    } else {
        format!("{} {}", episode.index_label, episode.title)
    };
    let title = widgets::elided(
        ui,
        &label,
        theme::regular(theme::JF_FONT),
        theme::JF_TEXT,
        text_width,
    );
    let title_rect = Rect::from_min_size(pos2(text_left, row.top() + 17.0), title.size());
    let title_response = ui.interact(
        title_rect,
        Id::new(("episode-title", index)),
        Sense::click(),
    );
    painter.galley(title_rect.min, title, Color32::WHITE);
    widgets::pointer_on_hover(&title_response);
    if title_response.clicked() {
        actions.push(UiAction::OpenDetail {
            id: episode.id.clone(),
            kind: episode.kind.to_owned(),
        });
    }
    let meta_y = title_rect.bottom() + 12.0;
    let mut meta_x = text_left;
    let runtime = episode
        .subtitle
        .rsplit(" • ")
        .next()
        .unwrap_or_default()
        .to_owned();
    for (text, star) in [
        (runtime, false),
        (
            if episode.rating > 0.0 {
                format!("{}", (episode.rating * 10.0).round() / 10.0)
            } else {
                String::new()
            },
            true,
        ),
        (episode.ends_at.clone(), false),
    ] {
        if text.is_empty() {
            continue;
        }
        if star {
            icons::paint_centered(
                &painter,
                Icon::Star,
                pos2(meta_x + 9.0, meta_y),
                18.0,
                theme::RATING_GOLD,
            );
            meta_x += 21.0;
        }
        let galley = widgets::elided(
            ui,
            &text,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT_SECONDARY,
            text_left + text_width - meta_x,
        );
        meta_x = widgets::galley_v_centered(&painter, meta_x, meta_y, galley).right() + 15.0;
    }
    if !episode.overview.is_empty() {
        let overview = widgets::wrapped(
            ui,
            &episode.overview,
            theme::regular(theme::JF_FONT),
            theme::JF_TEXT_SECONDARY,
            text_width,
            Some(5),
        );
        painter.galley(pos2(text_left, meta_y + 22.0), overview, Color32::WHITE);
    }
}

/// The poster (or episode still), overlapping the ribbon on the left.
fn draw_poster(ui: &mut Ui, state: &mut UiState, layout: &Layout, page_top: f32) {
    let detail = &state.detail;
    let rect = Rect::from_min_size(
        pos2(
            ui.max_rect().left() + layout.padding,
            page_top + layout.ribbon_top() - 86.0,
        ),
        layout.poster,
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().clone();
    match resident(&mut state.images, detail.artwork.as_ref()) {
        Some((texture, size)) => {
            widgets::image_cover(&painter, texture, size, rect, theme::JF_RADIUS)
        }
        None => {
            painter.rect_filled(rect, theme::JF_RADIUS, theme::JF_CARD_EMPTY);
        }
    }
    if detail.progress > 0.0
        && detail.progress < 1.0
        && !matches!(detail.kind, Some(DetailKind::Series | DetailKind::Season))
    {
        let track = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 4.0), rect.right_bottom());
        painter.rect_filled(track, 0.0, theme::JF_PROGRESS_TRACK);
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(track.width() * detail.progress, 4.0)),
            0.0,
            theme::JF_ACCENT,
        );
    }
    // The same corner indicator as on cards.
    let model = CardModel {
        placeholder_icon: None,
        title: "",
        subtitle: "",
        art: None,
        watched: detail.watched
            && matches!(detail.kind, Some(DetailKind::Season | DetailKind::Series)),
        unplayed: u32::try_from(detail.unplayed_count).unwrap_or(0),
        progress: 0.0,
        favorite: false,
        playable: false,
        actions: false,
        more: false,
        title_link: true,
        subtitle_link: true,
    };
    jf::corner_indicator(&painter, rect, &model);
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

fn detail_kind_name(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Movie => "Movie",
        DetailKind::Series => "Series",
        DetailKind::Season => "Season",
        DetailKind::Episode => "Episode",
        DetailKind::Person => "Person",
        DetailKind::BoxSet => "BoxSet",
        DetailKind::Playlist => "Playlist",
        DetailKind::Video => "Video",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn page(item: DetailItemData) -> DetailPageData {
        DetailPageData {
            item,
            seasons: Vec::new(),
            episodes: Vec::new(),
            similar: Vec::new(),
            sections: Vec::new(),
            children_error: None,
        }
    }

    #[test]
    fn an_episode_info_row_leads_with_the_air_date_like_jellyfin() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.production_year = Some(2026);
        episode.premiere_date = Some((2026, 1, 31));
        episode.official_rating = Some("TV-MA".into());
        episode.runtime_seconds = Some(2_700.0);
        episode.community_rating = Some(8.24);
        episode.playable = false;
        assert_eq!(
            misc_items(&page(episode.clone())),
            [
                MiscItem::Text("1/31/2026".into()),
                MiscItem::Text("45m".into()),
                MiscItem::Badge("TV-MA".into()),
                MiscItem::Stars(8.24),
            ]
        );
        episode.premiere_date = None;
        assert_eq!(misc_items(&page(episode))[0], MiscItem::Text("2026".into()));
    }

    #[test]
    fn a_series_info_row_reads_the_air_span() {
        let mut series = DetailItemData::fixture("series", DetailKind::Series);
        series.production_year = Some(2019);
        series.runtime_seconds = Some(3_000.0);
        series.series_continuing = true;
        series.critic_rating = Some(91.4);
        assert_eq!(
            misc_items(&page(series.clone())),
            [
                MiscItem::Text("2019 - Present".into()),
                MiscItem::Critic(91)
            ]
        );
        series.series_continuing = false;
        series.end_year = Some(2022);
        assert_eq!(
            misc_items(&page(series.clone()))[0],
            MiscItem::Text("2019 - 2022".into())
        );
        series.end_year = Some(2019);
        assert_eq!(misc_items(&page(series))[0], MiscItem::Text("2019".into()));
    }

    #[test]
    fn a_season_has_no_info_row() {
        let mut season = DetailItemData::fixture("season", DetailKind::Season);
        season.production_year = Some(2025);
        assert!(misc_items(&page(season)).is_empty());
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
    fn episode_cards_show_when_they_aired_and_how_long_they_run() {
        let mut episode = DetailItemData::fixture("episode", DetailKind::Episode);
        episode.runtime_seconds = Some(2_940.0);
        assert_eq!(detail_card_subtitle(&episode), "49m");
        episode.premiere_date = Some((2025, 1, 31));
        assert_eq!(detail_card_subtitle(&episode), "1/31/2025 • 49m");
        episode.runtime_seconds = Some(3_900.0);
        assert_eq!(detail_card_subtitle(&episode), "1/31/2025 • 1h 5m");
    }
}
