//! Detail screen: presentation into the view model, drawing, formatters.

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::images::{Art, ImageCache};
use super::jf::{self, CardModel, Shape};
use super::scrollbar;
use super::top_bar;
use super::widgets::{self, Shelf};
use super::{Screen, UiState, detail_art, primary_card_image, theme, web_art};
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
    pub audio_options: Vec<String>,
    pub subtitle_options: Vec<String>,
    pub audio_index: i32,
    pub subtitle_index: i32,
    /// The ribbon's info row, item by item as jellyfin-web lays it out.
    pub misc: Vec<MiscItem>,
    /// The item has a trailer (local or online): the ribbon offers it.
    pub has_trailer: bool,
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
    /// Sized for its card when drawn (the size depends on the window).
    pub image: Option<crate::features::detail::ImageRef>,
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
        // jellyfin-web's page backdrop and logo requests.
        backdrop: item
            .backdrop
            .as_ref()
            .map(|image| web_art(image, Some(3840), 80)),
        logo: item.logo.as_ref().map(|image| web_art(image, None, 90)),
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
        audio_options,
        subtitle_options,
        audio_index: tracks.audio_index,
        subtitle_index: tracks.subtitle_index,
        misc: misc_items(page),
        has_trailer: item.local_trailers > 0 || !item.trailer_urls.is_empty(),
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
    // jellyfin-web names each row in the singular for one entry.
    let row = |one: &'static str, many: &'static str, links: Vec<CreditLink>| {
        (if links.len() == 1 { one } else { many }, links)
    };
    [
        row("Genre", "Genres", links(&item.genres, &genre)),
        row("Director", "Directors", links(&item.directors, &person)),
        row("Writer", "Writers", links(&item.writers, &person)),
        row("Studio", "Studios", links(&item.studios, &studio)),
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
        image: person.image.clone(),
    }
}

/// Opens an external metadata link in the default browser, off the UI
/// thread's critical path (spawning a process can take milliseconds).
pub fn open_external_url(url: String) {
    std::thread::spawn(move || open_in_browser(&url));
}

/// The ribbon across the bottom of the backdrop band (7.2em).
const RIBBON_HEIGHT: f32 = jf::layout_unit(7.2 * jf::EM);
/// The poster's rise above the band's bottom (12.96em).
const POSTER_RISE: f32 = jf::layout_unit(12.96 * jf::EM);
/// Space above the primary and secondary content (1.25em).
const CONTENT_PADDING: f32 = jf::layout_unit(1.25 * jf::EM);
const LABEL_WIDTH: f32 = 93.0;

/// Page geometry, as jellyfin-web's detail page lays it out in viewport
/// units.
struct Layout {
    /// The page's width (the window less the page scrollbar).
    width: f32,
    /// The backdrop band: 40vh.
    band: f32,
    /// 3.3% of the page: the poster's inset and the primary column's
    /// right padding.
    padding: f32,
    poster: egui::Vec2,
    /// Where the text column starts: 32.45vw.
    content_left: f32,
    /// The logo box: 25vw by 16vh, 25vw from the right, 10vh down.
    logo: Rect,
}

impl Layout {
    fn new(width: f32, window: egui::Vec2, kind: Option<DetailKind>) -> Self {
        let poster_width = jf::layout_unit(window.x * 0.25);
        let ratio = match kind {
            Some(DetailKind::Episode | DetailKind::Video) => 9.0 / 16.0,
            Some(DetailKind::Playlist) => 1.0,
            _ => 1.5,
        };
        Self {
            width,
            band: jf::layout_unit(window.y * 0.4),
            padding: jf::page_padding(width),
            poster: vec2(poster_width, poster_width * ratio),
            content_left: jf::layout_unit(window.x * 0.3245),
            logo: Rect::from_min_size(
                pos2(width - 2.0 * window.x * 0.25, window.y * 0.1),
                vec2(window.x * 0.25, window.y * 0.16),
            ),
        }
    }

    /// The primary column's text width (3.3% padding on the right).
    fn content_width(&self) -> f32 {
        (self.width - self.content_left - self.padding).max(120.0)
    }

    fn ribbon_top(&self) -> f32 {
        self.band - RIBBON_HEIGHT
    }

    fn poster_bottom(&self) -> f32 {
        self.band - POSTER_RISE + self.poster.y
    }
}

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, 0.0, theme::JF_BG);
    let page_width = screen.width() - scrollbar::reserved(ui.ctx());
    let page = Rect::from_min_size(screen.min, vec2(page_width, screen.height()));
    let bar = Rect::from_min_size(screen.min, vec2(page_width, top_bar::HEIGHT));
    if state.busy {
        ui.put(
            Rect::from_center_size(screen.center(), vec2(56.0, 56.0)),
            egui::Spinner::new().size(56.0).color(theme::JF_ACCENT),
        );
        top_bar::draw(ui, state, bar, actions);
        return;
    }
    // jellyfin-web shows the item's backdrop behind the whole page, fixed
    // to the window, under 86% black.
    if let Some((texture, size)) = resident(&mut state.images, state.detail.backdrop.as_ref()) {
        widgets::image_cover(ui.painter(), texture, size, page, 0.0);
        ui.painter()
            .rect_filled(page, 0.0, Color32::from_black_alpha(219));
    }
    let window = vec2(jf::window_width(ui), screen.height());
    let layout = Layout::new(page_width, window, state.detail.kind);
    let mut page_ui = widgets::clipped_child(ui, page);
    let output = ScrollArea::vertical()
        .id_salt(("detail-page", state.detail.id.as_str()))
        .scroll_bar_visibility(scrollbar::HIDDEN)
        .auto_shrink(false)
        .show(&mut page_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let page_top = ui.cursor().top();
            // Below the band the page is opaque #101010
            // (`.noBackdropTransparency`); its height is known at the end.
            let ground = ui.painter().add(egui::Shape::Noop);
            draw_band(ui, state, &layout);
            draw_ribbon(ui, state, &layout, page_top, actions);
            if !state.message.is_empty() {
                draw_error_banner(ui, state, &layout, actions);
            }
            draw_info(ui, state, &layout, actions);
            draw_primary_sections(ui, state, &layout, actions);
            // The lower sections start under the poster when it reaches
            // further than the text.
            let below_poster = page_top + layout.poster_bottom() - ui.cursor().top();
            if below_poster > 0.0 {
                ui.allocate_exact_size(vec2(layout.width, below_poster), Sense::hover());
            }
            ui.allocate_exact_size(vec2(layout.width, CONTENT_PADDING), Sense::hover());
            draw_secondary_sections(ui, state, &layout, actions);
            ui.allocate_exact_size(vec2(layout.width, jf::PAGE_BOTTOM), Sense::hover());
            let below_band = Rect::from_min_max(
                pos2(ui.max_rect().left(), page_top + layout.band),
                pos2(ui.max_rect().left() + layout.width, ui.cursor().top()),
            );
            ui.painter().set(
                ground,
                egui::Shape::rect_filled(below_band, 0.0, theme::JF_BG),
            );
            draw_poster(ui, state, &layout, page_top);
        });
    scrollbar::draw(&mut page_ui, &output);
    top_bar::draw(ui, state, bar, actions);
}

/// The backdrop band (its backdrop is the page's), with the logo.
fn draw_band(ui: &mut Ui, state: &mut UiState, layout: &Layout) {
    let (band, _) = ui.allocate_exact_size(vec2(layout.width, layout.band), Sense::hover());
    if !ui.is_rect_visible(band) {
        return;
    }
    // `.detailLogo` is hidden on windows 68.75em wide or less.
    if jf::window_width(ui) <= 1100.0 {
        return;
    }
    if let Some((texture, size)) = resident(&mut state.images, state.detail.logo.as_ref()) {
        let logo = layout.logo.translate(band.min.to_vec2());
        widgets::image_contain(ui.painter(), texture, size, logo);
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
    // A playlist or collection offers Play and Shuffle even when empty.
    let container = matches!(kind, DetailKind::BoxSet | DetailKind::Playlist);
    if detail.playable || container {
        buttons.push((
            Icon::Play,
            if resumable { "Resume" } else { "Play" },
            Color32::WHITE,
            "play",
        ));
        if resumable {
            buttons.push((Icon::Replay, "Play", Color32::WHITE, "restart"));
        }
    }
    if detail.has_trailer {
        buttons.push((Icon::Theaters, "Trailer", Color32::WHITE, "trailer"));
    }
    if (detail.playable || container)
        && matches!(
            kind,
            DetailKind::Series | DetailKind::Season | DetailKind::BoxSet | DetailKind::Playlist
        )
    {
        buttons.push((Icon::Shuffle, "Shuffle", Color32::WHITE, "shuffle"));
    }
    if !matches!(kind, DetailKind::Person | DetailKind::Playlist) {
        buttons.push((
            Icon::Check,
            if detail.watched {
                "Watched"
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
    // `.mainDetailButtons`: flat buttons (1.6em icons, 0.85em by 0.7em
    // padding) at 108% on windows 100em wide, right-aligned in the ribbon.
    let scale = if jf::window_width(ui) >= 1600.0 {
        1.08
    } else {
        1.0
    };
    let font = jf::EM * scale;
    let icon_size = 1.6 * font;
    let button = vec2(
        icon_size + 1.7 * font,
        jf::layout_unit(icon_size + 1.4 * font),
    );
    let mut x = ribbon.right() - layout.padding - buttons.len() as f32 * button.x;
    let text_right = x;
    for (icon, label, color, name) in buttons {
        let rect = Rect::from_min_size(pos2(x, ribbon.center().y - button.y / 2.0), button);
        x += button.x;
        let response = ui.interact(rect, Id::new(("detail-action", name)), Sense::click());
        widgets::describe_button(&response, !busy, label);
        // `.button-flat:hover` turns the icon accent; a coloured icon
        // (played, favourite) keeps its colour.
        let tint = if color != Color32::WHITE {
            color
        } else if response.hovered() && !busy {
            theme::JF_ACCENT
        } else {
            jf::TEXT_80
        };
        icons::paint_centered(&painter, icon, rect.center(), icon_size, tint);
        widgets::pointer_on_hover(&response);
        if !response.clicked() || busy {
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
            "trailer" => UiAction::PlayTrailer(detail.id.clone()),
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

    // `.infoWrapper`, vertically centred: the name (an `h1`, and for an
    // episode or season the series as a link over an `h3`), then the info
    // row with 0.6em under it.
    let left = ribbon.left() + layout.content_left;
    let width = (text_right - left).max(80.0);
    let parented = !detail.series_id.is_empty()
        && matches!(detail.kind, Some(DetailKind::Episode | DetailKind::Season));
    let h1 = 1.8 * jf::EM;
    let h1_line = jf::layout_unit(1.35 * h1);
    let h3 = 1.17 * jf::EM;
    let h3_line = jf::layout_unit(1.35 * h3);
    let has_sub = !detail.subheading.is_empty();
    let name_height = if has_sub {
        jf::layout_unit(65.7)
    } else {
        h1_line
    };
    let misc = misc_layout(ui, &detail.misc);
    let misc_block = misc
        .as_ref()
        .map_or(0.0, |(_, height)| height + jf::layout_unit(0.6 * jf::EM));
    let block = name_height + misc_block;
    let top = ribbon.top() + (RIBBON_HEIGHT - block) / 2.0;
    let heading_text = if detail.heading.is_empty() {
        &detail.title
    } else {
        &detail.heading
    };
    let heading = widgets::elided(ui, heading_text, theme::bold(h1), jf::TEXT_80, width);
    let heading_rect = Rect::from_min_size(pos2(left, top), vec2(heading.size().x, h1_line));
    widgets::galley_on_baseline(&painter, left, jf::baseline(top, h1, h1_line), heading);
    if parented {
        let response = ui.interact(heading_rect, Id::new("detail-heading"), Sense::click());
        if response.hovered() {
            underline(ui, heading_rect, h1, h1_line);
        }
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            actions.push(UiAction::OpenDetail {
                id: detail.series_id.clone(),
                kind: "Series".to_owned(),
            });
        }
    }
    if has_sub {
        // "Season 1 - 1. Name": the season part links to the season.
        let sub_top = top + jf::layout_unit(38.75);
        let (season, rest) = match detail.subheading.split_once(" - ") {
            Some((season, rest)) if !detail.season_id.is_empty() => {
                (season.to_owned(), format!(" - {rest}"))
            }
            _ => (String::new(), detail.subheading.clone()),
        };
        let mut x = left;
        if !season.is_empty() {
            let galley = painter.layout_no_wrap(season.clone(), theme::bold(h3), jf::TEXT_80);
            let rect = Rect::from_min_size(pos2(x, sub_top), vec2(galley.size().x, h3_line));
            x = rect.right();
            widgets::galley_on_baseline(
                &painter,
                rect.left(),
                jf::baseline(sub_top, h3, h3_line),
                galley,
            );
            let response = ui.interact(rect, Id::new("detail-subheading"), Sense::click());
            if response.hovered() {
                underline(ui, rect, h3, h3_line);
            }
            widgets::pointer_on_hover(&response);
            if response.clicked() {
                actions.push(UiAction::OpenDetail {
                    id: detail.season_id.clone(),
                    kind: "Season".to_owned(),
                });
            }
        }
        let galley = widgets::elided(
            ui,
            &rest,
            theme::bold(h3),
            jf::TEXT_80,
            (left + width - x).max(0.0),
        );
        widgets::galley_on_baseline(&painter, x, jf::baseline(sub_top, h3, h3_line), galley);
    }
    if let Some((items, height)) = misc {
        draw_misc(
            &painter,
            items,
            pos2(left, top + name_height),
            height,
            left + width,
        );
    }
}

/// One laid-out item of the info row.
enum MiscPiece {
    Text(std::sync::Arc<egui::Galley>),
    Badge(std::sync::Arc<egui::Galley>),
    Stars(std::sync::Arc<egui::Galley>),
    Critic(std::sync::Arc<egui::Galley>, bool),
}

/// The info row's items and its height (the tallest item: text 1.35em,
/// the rating badge 0.92em text padded 0.22em, a 1.4em star).
fn misc_layout(ui: &Ui, misc: &[MiscItem]) -> Option<(Vec<MiscPiece>, f32)> {
    if misc.is_empty() {
        return None;
    }
    let font = theme::regular(jf::EM);
    let mut height = jf::layout_unit(1.35 * jf::EM);
    let pieces = misc
        .iter()
        .map(|item| match item {
            MiscItem::Text(text) => MiscPiece::Text(ui.painter().layout_no_wrap(
                text.clone(),
                font.clone(),
                jf::TEXT_80,
            )),
            MiscItem::Badge(text) => {
                let size = 0.92 * jf::EM;
                height = height.max(jf::layout_unit(1.35 * size + 0.44 * size));
                MiscPiece::Badge(ui.painter().layout_no_wrap(
                    text.clone(),
                    theme::regular(size),
                    theme::JF_BADGE_TEXT,
                ))
            }
            MiscItem::Stars(stars) => {
                height = height.max(jf::layout_unit(1.4 * jf::EM));
                MiscPiece::Stars(ui.painter().layout_no_wrap(
                    format!("{}", (stars * 10.0).round() / 10.0),
                    font.clone(),
                    jf::TEXT_80,
                ))
            }
            MiscItem::Critic(score) => MiscPiece::Critic(
                ui.painter()
                    .layout_no_wrap(score.to_string(), font.clone(), jf::TEXT_80),
                *score >= 60,
            ),
        })
        .collect();
    Some((pieces, height))
}

/// `.itemMiscInfo`: items 1em apart (the badge 1em of its own size),
/// vertically centred in the row.
fn draw_misc(
    painter: &egui::Painter,
    pieces: Vec<MiscPiece>,
    origin: egui::Pos2,
    height: f32,
    right: f32,
) {
    let em = jf::EM;
    let line = jf::layout_unit(1.35 * em);
    let center = origin.y + height / 2.0;
    let text_top = center - line / 2.0;
    let mut x = origin.x;
    for piece in pieces {
        if x >= right {
            break;
        }
        match piece {
            MiscPiece::Text(galley) => {
                let width = galley.size().x;
                widgets::galley_on_baseline(painter, x, jf::baseline(text_top, em, line), galley);
                x += width + em;
            }
            MiscPiece::Badge(galley) => {
                let size = 0.92 * em;
                let badge_line = jf::layout_unit(1.35 * size);
                let pad = vec2(0.5 * size, 0.22 * size);
                let rect = Rect::from_min_size(
                    pos2(x, center - (badge_line + 2.0 * pad.y) / 2.0),
                    vec2(galley.size().x + 2.0 * pad.x, badge_line + 2.0 * pad.y),
                );
                painter.rect_filled(rect, 0.25 * size, theme::JF_BADGE);
                let width = galley.size().x;
                widgets::galley_on_baseline(
                    painter,
                    rect.left() + pad.x,
                    jf::baseline(rect.top() + pad.y, size, badge_line),
                    galley,
                );
                x += width + 2.0 * pad.x + size;
            }
            MiscPiece::Stars(galley) => {
                let star = 1.4 * em;
                icons::paint_centered(
                    painter,
                    Icon::Star,
                    pos2(x + star / 2.0, center),
                    star,
                    theme::RATING_GOLD,
                );
                x += star + 0.125 * star;
                let width = galley.size().x;
                widgets::galley_on_baseline(painter, x, jf::baseline(text_top, em, line), galley);
                x += width + em;
            }
            MiscPiece::Critic(galley, fresh) => {
                // The tomato: 1.2em tall at the left of 1.5em of padding.
                tomato(painter, pos2(x + 0.6 * em, center), fresh);
                x += 1.5 * em;
                let width = galley.size().x;
                widgets::galley_on_baseline(painter, x, jf::baseline(text_top, em, line), galley);
                x += width + em;
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
/// The text column under the ribbon, as jellyfin-web stacks it: track
/// selections, tagline, overview, person facts, tags, external links, and
/// the Genres / Director / Writers / Studios group. CSS margins are kept,
/// including where they collapse.
fn draw_info(ui: &mut Ui, state: &mut UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    const EM: f32 = jf::EM;
    let small = 0.92 * EM;
    let detail = &mut state.detail;
    let origin = ui.cursor().min;
    let left = origin.x + layout.content_left;
    let width = layout.content_width();
    // `.trackSelections` and `.detailsGroupItem` stop at 44em.
    let narrow = width.min(jf::layout_unit(44.0 * EM));
    let line = jf::layout_unit(1.35 * EM);
    let mut y = origin.y + CONTENT_PADDING;

    // Track selections: rows of 1.75em, 0.3em apart.
    let row_height = jf::layout_unit(1.75 * EM);
    let value_left = left + jf::layout_unit(LABEL_WIDTH + 0.2 * EM);
    let value_width = narrow - (value_left - left);
    let mut rows = 0;
    let mut track_row = |ui: &mut Ui, y: &mut f32, title: &str| -> Rect {
        if rows > 0 {
            *y += jf::layout_unit(0.3 * EM);
        }
        rows += 1;
        let label = ui
            .painter()
            .layout_no_wrap(title.to_owned(), theme::regular(EM), jf::TEXT_80);
        widgets::galley_on_baseline(ui.painter(), left, baseline(*y, EM, row_height), label);
        let field = Rect::from_min_size(pos2(value_left, *y), vec2(value_width, row_height));
        *y += row_height;
        field
    };
    // A track with no choice is a disabled select: its text at 70%.
    let fixed = |ui: &Ui, field: Rect, text: &str| {
        let color = theme::with_alpha(jf::TEXT_80, 0.7 * 0.8);
        let galley = widgets::elided(ui, text, theme::regular(EM), color, field.width());
        widgets::galley_on_baseline(
            ui.painter(),
            field.left(),
            baseline(field.top(), EM, field.height()),
            galley,
        );
    };
    if !detail.media_info.is_empty() {
        let field = track_row(ui, &mut y, "Video");
        fixed(ui, field, &detail.media_info);
    }
    if detail.audio_options.len() == 1 {
        let field = track_row(ui, &mut y, "Audio");
        fixed(ui, field, &detail.audio_options[0]);
    } else if detail.audio_options.len() > 1 {
        let field = track_row(ui, &mut y, "Audio");
        if let Some(picked) = track_select(
            ui,
            field,
            &detail.audio_options,
            detail.audio_index,
            ("detail-track", true),
        ) {
            detail.audio_index = picked;
            actions.push(UiAction::DetailTrack {
                index: picked,
                audio: true,
            });
        }
    }
    if detail.subtitle_options.len() > 1 {
        let field = track_row(ui, &mut y, "Subtitles");
        if let Some(picked) = track_select(
            ui,
            field,
            &detail.subtitle_options,
            detail.subtitle_index,
            ("detail-track", false),
        ) {
            detail.subtitle_index = picked;
            actions.push(UiAction::DetailTrack {
                index: picked,
                audio: false,
            });
        }
    }

    // A pending bottom margin, collapsed with the next top margin. The
    // empty `p.itemGenres` always leaves 1em.
    let p_margin = jf::layout_unit(EM);
    let mut pending = p_margin;
    if !detail.tagline.is_empty() {
        // `h3.tagline`: 1.17em, 1em margins of its own size.
        let size = 1.17 * EM;
        let margin = jf::layout_unit(size);
        y += pending.max(margin);
        let galley = widgets::elided(
            ui,
            &detail.tagline,
            theme::regular(size),
            jf::TEXT_80,
            width,
        );
        let tagline_line = jf::layout_unit(1.35 * size);
        widgets::galley_on_baseline(ui.painter(), left, baseline(y, size, tagline_line), galley);
        y += tagline_line;
        pending = margin;
    }
    let mut clamped = false;
    let expanded_id = Id::new(("detail-overview-expanded", detail.id.as_str()));
    let expanded = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(expanded_id))
        .unwrap_or(false);
    if !detail.overview.is_empty() {
        // `p.overview` holds the overview's paragraphs (1em apart inside
        // the box) and clamps them to 6 lines in all (12 on narrow
        // windows); a clamp cuts the last margin.
        y += pending.max(p_margin) + p_margin;
        let clamp = if jf::window_width(ui) >= 640.0 { 6 } else { 12 };
        let mut left_lines = if expanded { usize::MAX } else { clamp };
        for (index, paragraph) in overview_paragraphs(&detail.overview).iter().enumerate() {
            if left_lines == 0 {
                clamped = true;
                break;
            }
            if index > 0 {
                y += p_margin;
            }
            let lines = widgets::wrap_lines(ui, paragraph, theme::regular(EM), width);
            let shown = lines.len().min(left_lines);
            for text in lines.iter().take(shown) {
                let galley =
                    ui.painter()
                        .layout_no_wrap(text.clone(), theme::regular(EM), jf::TEXT_80);
                widgets::galley_on_baseline(ui.painter(), left, baseline(y, EM, line), galley);
                y += line;
            }
            left_lines -= shown;
            if shown < lines.len() {
                clamped = true;
                break;
            }
        }
        if !clamped {
            y += p_margin;
        }
        pending = p_margin;
    }
    // `.overview-controls` (a flex row, so margins do not collapse across
    // it): "Show more" when the overview is clamped.
    y += pending;
    pending = 0.0;
    if clamped || expanded {
        let controls = jf::layout_unit(55.77);
        let text = if expanded { "Show less" } else { "Show more" };
        let galley = ui
            .painter()
            .layout_no_wrap(text.to_owned(), theme::bold(EM), jf::TEXT_80);
        // An `emby-button`: 1.075em either side, 0.9em above and below.
        let button = Rect::from_min_size(
            pos2(
                left + width - galley.size().x - 2.0 * 16.0,
                y + (controls - 46.86) / 2.0,
            ),
            vec2(galley.size().x + 2.0 * 16.0, 46.86),
        );
        let response = ui.interact(button, expanded_id.with("button"), Sense::click());
        widgets::describe_button(&response, true, text);
        let text_rect = Rect::from_min_size(
            pos2(button.left() + 16.0, button.center().y - 10.0),
            vec2(galley.size().x, 20.0),
        );
        widgets::galley_on_baseline(
            ui.painter(),
            text_rect.left(),
            text_rect.top() + 16.0,
            galley,
        );
        if response.hovered() {
            underline(ui, text_rect, EM, line);
        }
        widgets::pointer_on_hover(&response);
        if response.clicked() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(expanded_id, !expanded));
        }
        y += controls;
    }
    // The person facts: one paragraph each.
    for (label, value) in &detail.person_rows {
        y += pending.max(p_margin);
        let flow = vec![CreditLink {
            name: value.clone(),
            target: None,
        }];
        let bold = *label == "Birth place";
        let prefix = format!("{label}: ");
        y += link_flow(
            ui,
            pos2(left, y),
            width,
            &flow,
            EM,
            &prefix,
            bold,
            "person-row",
            actions,
        );
        pending = p_margin;
    }
    if !detail.tags.is_empty() {
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
        let margin = jf::layout_unit(0.7 * small);
        y += pending.max(margin);
        y += link_flow(
            ui,
            pos2(left, y),
            width,
            &tags,
            small,
            "Tags: ",
            true,
            "detail-tag",
            actions,
        );
        pending = margin;
    }
    if !detail.links.is_empty() {
        let links = detail
            .links
            .iter()
            .map(|(name, url)| CreditLink {
                name: name.clone(),
                target: Some(crate::features::items::LinkTarget::Url(url.clone())),
            })
            .collect::<Vec<_>>();
        let margin = jf::layout_unit(0.7 * small);
        y += pending.max(margin);
        y += link_flow(
            ui,
            pos2(left, y),
            width,
            &links,
            small,
            "",
            true,
            "detail-link",
            actions,
        );
        pending = margin;
    }
    // `.itemDetailsGroup`: 1.5em down; rows of a 6.25em label and links,
    // 0.5em apart.
    y += pending.max(jf::layout_unit(1.5 * EM));
    let content_left = left + jf::layout_unit(LABEL_WIDTH + 0.6 * EM);
    for (title, links) in &detail.credits {
        let label =
            ui.painter()
                .layout_no_wrap((*title).to_owned(), theme::regular(EM), jf::TEXT_80);
        widgets::galley_on_baseline(ui.painter(), left, baseline(y, EM, line), label);
        let height = link_flow(
            ui,
            pos2(content_left, y),
            narrow - (content_left - left),
            links,
            EM,
            "",
            true,
            title,
            actions,
        );
        y += height + jf::layout_unit(0.5 * EM);
    }
    ui.allocate_exact_size(vec2(layout.width, y - origin.y), Sense::hover());
}

fn baseline(top: f32, size: f32, line_height: f32) -> f32 {
    jf::baseline(top, size, line_height)
}

/// The overview's paragraphs, as jellyfin-web's Markdown renders them:
/// blank lines separate paragraphs, a single line break is a space.
fn overview_paragraphs(overview: &str) -> Vec<String> {
    overview
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(|paragraph| paragraph.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|paragraph| !paragraph.is_empty())
        .collect()
}

/// A link's hover underline, under the text of a line box.
fn underline(ui: &Ui, rect: Rect, size: f32, line_height: f32) {
    let y = baseline(rect.top(), size, line_height) + (0.1 * size).max(1.0);
    ui.painter().line_segment(
        [pos2(rect.left(), y), pos2(rect.right(), y)],
        egui::Stroke::new((size / 14.0).max(1.0), jf::TEXT_80),
    );
}

/// Inline text as jellyfin-web writes credits and tags: `prefix`, then the
/// names (bold when `bold`) separated by a plain ", ", wrapping between
/// names at `width`, on 1.35 line boxes. Names with a target are links
/// (underlined on hover). Returns the block's height.
#[allow(clippy::too_many_arguments)]
fn link_flow(
    ui: &mut Ui,
    origin: egui::Pos2,
    width: f32,
    links: &[CreditLink],
    size: f32,
    prefix: &str,
    bold: bool,
    id: &str,
    actions: &mut Vec<UiAction>,
) -> f32 {
    let line = jf::layout_unit(1.35 * size);
    let regular = theme::regular(size);
    let name_font = if bold {
        theme::bold(size)
    } else {
        theme::regular(size)
    };
    let separator = widgets::text_width(ui, ", ", regular.clone());
    let mut x = 0.0;
    let mut y = 0.0;
    if !prefix.is_empty() {
        let galley = ui
            .painter()
            .layout_no_wrap(prefix.to_owned(), regular.clone(), jf::TEXT_80);
        x = galley.size().x;
        widgets::galley_on_baseline(
            ui.painter(),
            origin.x,
            baseline(origin.y, size, line),
            galley,
        );
    }
    for (index, link) in links.iter().enumerate() {
        let galley = ui
            .painter()
            .layout_no_wrap(link.name.clone(), name_font.clone(), jf::TEXT_80);
        let name_width = galley.size().x.min(width);
        if x > 0.0 && x + name_width > width {
            x = 0.0;
            y += line;
        }
        let rect = Rect::from_min_size(pos2(origin.x + x, origin.y + y), vec2(name_width, line));
        widgets::galley_on_baseline(
            ui.painter(),
            rect.left(),
            baseline(rect.top(), size, line),
            galley,
        );
        if let Some(target) = &link.target {
            let response = ui.interact(rect, Id::new((id, index)), Sense::click());
            widgets::describe_button(&response, true, &link.name);
            widgets::pointer_on_hover(&response);
            if response.hovered() {
                underline(ui, rect, size, line);
            }
            if response.clicked() {
                actions.push(match target {
                    crate::features::items::LinkTarget::Url(url) => {
                        UiAction::OpenExternalUrl(url.clone())
                    }
                    target => UiAction::OpenLink(target.clone()),
                });
            }
        }
        x += name_width;
        if index + 1 < links.len() {
            let comma = ui
                .painter()
                .layout_no_wrap(",".to_owned(), regular.clone(), jf::TEXT_80);
            widgets::galley_on_baseline(
                ui.painter(),
                origin.x + x,
                baseline(rect.top(), size, line),
                comma,
            );
            x += separator;
        }
    }
    y + line
}

/// A track `<select>` as jellyfin-web draws one in the text column: the
/// choice flush left on a #292929 box with an arrow 0.3em from the right;
/// a click opens jellyfin's action sheet of the options.
fn track_select(
    ui: &mut Ui,
    field: Rect,
    options: &[String],
    index: i32,
    id: (&str, bool),
) -> Option<i32> {
    let em = jf::EM;
    let selected = usize::try_from(index).unwrap_or(0);
    let id = Id::new(id);
    let response = ui.interact(field, id, Sense::click());
    widgets::describe_button(
        &response,
        true,
        options.get(selected).map_or("", String::as_str),
    );
    let painter = ui.painter();
    painter.rect_filled(field, 0.2 * em, Color32::from_gray(41));
    let arrow = 1.4 * em;
    let text_width = field.width() - arrow - 0.3 * em;
    if let Some(text) = options.get(selected) {
        let galley = widgets::elided(ui, text, theme::regular(em), jf::TEXT_80, text_width);
        widgets::galley_on_baseline(
            painter,
            field.left(),
            baseline(field.top(), em, field.height()),
            galley,
        );
    }
    let arrow_box = Rect::from_min_size(
        pos2(field.right() - 0.3 * em - arrow, field.top() + 0.2 * em),
        vec2(arrow, arrow),
    );
    icons::paint_centered(
        painter,
        Icon::ExpandMore,
        arrow_box.center(),
        arrow,
        jf::TEXT_80,
    );
    widgets::pointer_on_hover(&response);
    let open_id = id.with("open");
    let mut open = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(open_id))
        .unwrap_or(false);
    if response.clicked() {
        open = true;
    }
    let mut picked = None;
    if open {
        let items = options
            .iter()
            .enumerate()
            .map(|(option, label)| super::dialogs::SheetItem {
                label,
                icon: None,
                checked: option == selected, aside: None,
            })
            .collect::<Vec<_>>();
        let (choice, close) =
            super::dialogs::action_sheet(ui.ctx(), id.with("sheet"), field, None, &items, Some(selected));
        if let Some(choice) = choice {
            picked = (choice != selected).then(|| i32::try_from(choice).unwrap_or(0));
            open = false;
        }
        if close {
            open = false;
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(open_id, open));
    picked
}

/// The sections in the text column, right after the text: Next Up and
/// Seasons, a season's episodes, a person's filmography, a collection's or
/// playlist's items.
fn draw_primary_sections(
    ui: &mut Ui,
    state: &mut UiState,
    layout: &Layout,
    actions: &mut Vec<UiAction>,
) {
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
}

/// The sections under the poster: cast, guest stars, more from the
/// season, more like this.
fn draw_secondary_sections(
    ui: &mut Ui,
    state: &mut UiState,
    layout: &Layout,
    actions: &mut Vec<UiAction>,
) {
    let detail = &state.detail;
    let images = &mut state.images;
    let shelf = ShelfSpec { layout };
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
    // jellyfin-web shows the rest of the season only when there is a rest.
    if detail.kind == Some(DetailKind::Episode) && detail.episodes.len() > 1 {
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
        // `.detailVerticalSection`: a 45.72px title row, then the cards;
        // sections follow one another without a gap.
        let title_height = jf::SECTION_TITLE;
        let (rect, _) = ui.allocate_exact_size(
            vec2(layout.width, title_height + card_height),
            Sense::hover(),
        );
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let viewport = Rect::from_min_max(
            pos2(rect.left() + layout.content_left, rect.top() + title_height),
            rect.max,
        );
        let content_width = count as f32 * card_width + layout.padding;
        let mut shelf = Shelf::begin(ui, id, viewport, content_width);
        let offset = shelf.scroll(ui);
        let title_row = Rect::from_min_max(
            pos2(rect.left() + layout.content_left, rect.top()),
            pos2(rect.right(), rect.top() + title_height),
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
        let width = jf::overflow_width(jf::window_width(ui), shape);
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
        // A filmography or collection: jellyfin-web's overflow-sized cards
        // wrapping in the text column, under a bare heading.
        let layout = self.layout;
        let grid = layout.content_width();
        let width = jf::overflow_width(jf::window_width(ui), shape);
        let columns = ((grid / width).floor() as usize).max(1);
        // Episodes link both lines; anything else has a plain year.
        let text = jf::CardText {
            lines: 2,
            title_link: true,
            subtitle_link: shape == Shape::Backdrop,
        };
        let height = jf::card_height_for(width, shape, text);
        let rows = cards.len().div_ceil(columns);
        let title_height = jf::SECTION_HEADING;
        let (rect, _) = ui.allocate_exact_size(
            vec2(layout.width, title_height + rows as f32 * height),
            Sense::hover(),
        );
        if !ui.is_rect_visible(rect) {
            return;
        }
        let id = Id::new(("detail-grid", title, page.to_owned()));
        let title_row = Rect::from_min_max(
            pos2(rect.left() + layout.content_left, rect.top()),
            pos2(rect.right(), rect.top() + title_height),
        );
        jf::section_title(ui, title_row, title, 0.0, None, id.with("title"));
        for (index, card) in cards.iter().enumerate() {
            let origin = pos2(
                rect.left() + layout.content_left + (index % columns) as f32 * width,
                rect.top() + title_height + (index / columns) as f32 * height,
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
        let width = jf::overflow_width(jf::window_width(ui), Shape::Portrait);
        let text = jf::CardText {
            lines: 2,
            title_link: true,
            subtitle_link: false,
        };
        let height = jf::card_height_for(width, Shape::Portrait, text);
        let sizes = super::catalog::ArtSizes::new(ui.ctx(), true);
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
            let art = person
                .image
                .clone()
                .map(|image| sizes.image_art(image, Shape::Portrait, None));
            let model = CardModel {
                placeholder_icon: Some(Icon::Person),
                title: &person.name,
                subtitle: &person.role,
                art: art.as_ref(),
                watched: false,
                unplayed: 0,
                progress: 0.0,
                favorite: false,
                playable: false,
                actions: false,
                more: false,
                title_link: true,
                // The role ("as …", "Director") is plain text.
                subtitle_link: false,
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

/// jellyfin-web's large list rows (a season's episodes, a playlist's
/// entries): `.listItem-largeImage` in the text column, whose container
/// keeps 3.3% of its width free on the right.
fn draw_rows(
    ui: &mut Ui,
    images: &mut ImageCache,
    cards: &[DetailCard],
    busy: bool,
    layout: &Layout,
    origin: &dyn Fn(&DetailCard) -> super::menu::MenuOrigin,
    actions: &mut Vec<UiAction>,
) {
    let window = jf::window_width(ui);
    let width = layout.content_width();
    let row_width = width - jf::layout_unit(width * 0.033);
    // 13vw image plus the row's 0.25em padding above and below.
    let height = jf::layout_unit(window * 0.13) + 2.0 * jf::layout_unit(0.25 * jf::EM);
    for (index, card) in cards.iter().enumerate() {
        let (row, _) = ui.allocate_exact_size(vec2(layout.width, height), Sense::hover());
        if !ui.is_rect_visible(row) {
            continue;
        }
        let row = Rect::from_min_size(
            pos2(row.left() + layout.content_left, row.top()),
            vec2(row_width, height),
        );
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
    const EM: f32 = jf::EM;
    let window = jf::window_width(ui);
    let painter = ui.painter().clone();
    // `.listItem:hover` lights the whole row.
    let hovered = ui.rect_contains_pointer(row) && !busy;
    if hovered {
        painter.rect_filled(row, 0.0, Color32::from_gray(36));
    }
    // Padding 0.25em 0.25em 0.25em 0.5em.
    let inner = Rect::from_min_max(
        row.min + vec2(jf::layout_unit(0.5 * EM), jf::layout_unit(0.25 * EM)),
        row.max - vec2(jf::layout_unit(0.25 * EM), jf::layout_unit(0.25 * EM)),
    );
    // `.listItemImage-large`: 19.5vw by 13vw.
    let image = Rect::from_min_size(
        inner.min,
        vec2(
            jf::layout_unit(window * 0.195),
            jf::layout_unit(window * 0.13),
        ),
    );
    let response = ui.interact(
        image,
        Id::new(("episode-thumb", index, episode.id.as_str())),
        if busy { Sense::hover() } else { Sense::click() },
    );
    widgets::describe_button(&response, !busy, &format!("Play {}", episode.title));
    if let Some((texture, size)) = resident(images, episode.art.as_ref()) {
        widgets::image_cover(&painter, texture, size, image, 0.0);
    }
    // `.listItemImageButton`: a faint play button, always over the image
    // (jellyfin-web's list rows show no progress bar).
    painter.circle_filled(image.center(), 82.08 / 2.0, Color32::from_black_alpha(102));
    icons::paint_centered(
        &painter,
        Icon::Play,
        image.center(),
        55.61,
        Color32::from_white_alpha(153),
    );
    if response.has_focus() {
        widgets::border(&painter, image, 0.0, 2.0, theme::JF_ACCENT);
    }
    if !busy {
        widgets::pointer_on_hover(&response);
    }
    if response.clicked() {
        actions.push(UiAction::PlayDetail(episode.id.clone()));
    }

    // `.listViewUserDataButtons`: info, played, favourite, more, at the
    // right, vertically centred.
    let button = vec2(41.36, 41.38);
    let buttons_left = inner.right() - 4.0 * button.x;
    let mut x = buttons_left;
    let mut slot = || {
        let rect = Rect::from_min_size(pos2(x, inner.center().y - button.y / 2.0), button);
        x += button.x;
        rect
    };
    let info = slot();
    if jf::round_icon_button(
        ui,
        info,
        Icon::InfoOutline,
        jf::TEXT_80,
        jf::TOOLBAR_ICON,
        "Info",
        !busy,
        Id::new(("episode-action", index, "info")),
    )
    .clicked()
    {
        actions.push(UiAction::OpenDetail {
            id: episode.id.clone(),
            kind: episode.kind.to_owned(),
        });
    }
    let played = slot();
    let played_color = if episode.watched {
        theme::JF_PLAYED_RED
    } else {
        jf::TEXT_80
    };
    let played_label = if episode.watched {
        "Watched"
    } else {
        "Mark played"
    };
    if jf::round_icon_button(
        ui,
        played,
        Icon::Check,
        played_color,
        jf::TOOLBAR_ICON,
        played_label,
        !busy,
        Id::new(("episode-action", index, "played")),
    )
    .clicked()
    {
        actions.push(UiAction::SetWatched {
            id: episode.id.clone(),
            watched: !episode.watched,
        });
    }
    let favorite = slot();
    let favorite_color = if episode.favorite {
        theme::JF_PLAYED_RED
    } else {
        jf::TEXT_80
    };
    let favorite_label = if episode.favorite {
        "Remove from favorites"
    } else {
        "Add to favorites"
    };
    if jf::round_icon_button(
        ui,
        favorite,
        Icon::Favorite,
        favorite_color,
        jf::TOOLBAR_ICON,
        favorite_label,
        !busy,
        Id::new(("episode-action", index, "favorite")),
    )
    .clicked()
    {
        actions.push(UiAction::SetFavorite {
            id: episode.id.clone(),
            favorite: !episode.favorite,
        });
    }
    let more = slot();
    if jf::round_icon_button(
        ui,
        more,
        Icon::MoreVert,
        jf::TEXT_80,
        jf::TOOLBAR_ICON,
        "More",
        !busy,
        Id::new(("episode-action", index, "more")),
    )
    .clicked()
    {
        actions.push(UiAction::OpenMenu(Box::new(super::menu::MenuRequest {
            item_id: episode.id.clone(),
            name: episode.title.clone(),
            kind: episode.kind.to_owned(),
            series_id: episode.series_id.clone(),
            season_id: episode.season_id.clone(),
            anchor: more,
            origin: origin(),
        })));
    }

    // `.listItemBody`: 0.85em by 0.75em of padding, centred; lines of
    // 1.2em with 0.1em padding; the info row and overview 50% white.
    let pad = vec2(jf::layout_unit(0.75 * EM), jf::layout_unit(0.85 * EM));
    let text_left = image.right() + jf::layout_unit(0.75 * EM) + pad.x;
    let text_width = (buttons_left - pad.x - text_left).max(60.0);
    let line = jf::layout_unit(1.2 * EM);
    let line_pad = jf::layout_unit(0.1 * EM);
    let title_height = line + 2.0 * line_pad;
    let star = 1.4 * EM;
    let info_height = line.max(star) + 2.0 * line_pad;
    let overview_lines = if episode.overview.is_empty() || window <= 800.0 {
        Vec::new()
    } else {
        overview_paragraphs(&episode.overview)
            .iter()
            .flat_map(|paragraph| {
                widgets::wrap_lines(ui, paragraph, theme::regular(EM), text_width)
            })
            .collect::<Vec<_>>()
    };
    let p_margin = jf::layout_unit(EM);
    let overview_height = if overview_lines.is_empty() {
        0.0
    } else {
        2.0 * line_pad + 2.0 * p_margin + overview_lines.len() as f32 * line
    };
    let body = title_height + info_height + overview_height;
    let mut y = inner.center().y - body / 2.0;
    let label = if episode.index_label.is_empty() {
        episode.title.clone()
    } else {
        format!("{} {}", episode.index_label, episode.title)
    };
    let title = widgets::elided(ui, &label, theme::regular(EM), jf::TEXT_80, text_width);
    let title_rect = Rect::from_min_size(pos2(text_left, y), vec2(title.size().x, title_height));
    widgets::galley_on_baseline(
        &painter,
        text_left,
        jf::baseline(y + line_pad, EM, line),
        title,
    );
    let title_response = ui.interact(
        title_rect,
        Id::new(("episode-title", index)),
        Sense::click(),
    );
    widgets::pointer_on_hover(&title_response);
    if title_response.clicked() {
        actions.push(UiAction::OpenDetail {
            id: episode.id.clone(),
            kind: episode.kind.to_owned(),
        });
    }
    y += title_height;
    let secondary = theme::JF_TEXT_SECONDARY;
    let center = y + info_height / 2.0;
    let text_top = center - line / 2.0;
    let mut meta_x = text_left;
    let runtime = episode
        .subtitle
        .rsplit(" \u{2022} ")
        .next()
        .unwrap_or_default()
        .to_owned();
    for (text, has_star) in [
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
        if has_star {
            icons::paint_centered(
                &painter,
                Icon::Star,
                pos2(meta_x + star / 2.0, center),
                star,
                theme::RATING_GOLD,
            );
            meta_x += star + 0.125 * star;
        }
        let galley = widgets::elided(
            ui,
            &text,
            theme::regular(EM),
            secondary,
            text_left + text_width - meta_x,
        );
        let width = galley.size().x;
        widgets::galley_on_baseline(&painter, meta_x, jf::baseline(text_top, EM, line), galley);
        meta_x += width + EM;
    }
    y += info_height;
    if !overview_lines.is_empty() {
        y += line_pad + p_margin;
        for text in &overview_lines {
            let galley = painter.layout_no_wrap(text.clone(), theme::regular(EM), secondary);
            widgets::galley_on_baseline(&painter, text_left, jf::baseline(y, EM, line), galley);
            y += line;
        }
    }
}

/// The poster (or episode still), overlapping the ribbon on the left.
fn draw_poster(ui: &mut Ui, state: &mut UiState, layout: &Layout, page_top: f32) {
    let detail = &state.detail;
    // `.detailImageContainer .card`: 25vw wide at 3.3%, risen 12.96em
    // above the band's bottom.
    let rect = Rect::from_min_size(
        pos2(
            ui.max_rect().left() + layout.padding,
            page_top + layout.band - POSTER_RISE,
        ),
        layout.poster,
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().clone();
    // The card's padder shows under (or instead of) the image; its shadow
    // is clipped to the card, which is just the image here.
    painter.rect_filled(rect, jf::CARD_RADIUS, Color32::from_gray(36));
    if let Some((texture, size)) = resident(&mut state.images, detail.artwork.as_ref()) {
        widgets::image_cover(&painter, texture, size, rect, jf::CARD_RADIUS);
    }
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
