//! Detail screen: presentation into the view model, drawing, formatters.

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::images::{Art, ImageCache};
use super::top_bar;
use super::widgets::{self, RoundIconButton, Shelf};
use super::{Screen, UiState, detail_art, format_playback_time, primary_card_image, theme};
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
    pub ends_at: String,
    pub tags: String,
    pub links: Vec<(String, String)>,
    pub director: String,
    pub writers: String,
    pub star_rating: f32,
    pub critic_rating: Option<i32>,
    pub subtitle: String,
    pub overview: String,
    pub artwork: Option<Art>,
    pub backdrop: Option<Art>,
    pub logo: Option<Art>,
    pub kind: Option<DetailKind>,
    pub tagline: String,
    pub genres: String,
    pub studios: String,
    pub media_info: String,
    pub series_id: String,
    pub series_summary: String,
    pub play_context: String,
    pub season_id: String,
    pub season_name: String,
    pub season_summary: String,
    pub play_label: String,
    pub progress: f32,
    pub progress_label: String,
    pub playable: bool,
    pub watched: bool,
    pub favorite: bool,
    pub seasons: Vec<DetailCard>,
    pub next_up: Vec<DetailCard>,
    pub similar: Vec<DetailCard>,
    pub episodes: Vec<DetailCard>,
    pub people: Vec<DetailPerson>,
    pub guest_stars: Vec<DetailPerson>,
    /// Position of this episode among `episodes`; its shelf opens there.
    pub current_episode_index: Option<usize>,
    /// Unwatched episodes left in a series or season, for the poster badge.
    pub unplayed_count: i32,
    pub audio_options: Vec<String>,
    pub subtitle_options: Vec<String>,
    pub audio_index: i32,
    pub subtitle_index: i32,
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailPerson {
    pub name: String,
    pub role: String,
    pub art: Option<Art>,
}

/// Switches to an empty detail page while the real one loads.
pub fn present_loading(context: &AppContext) {
    let can_go_back = context
        .state
        .lock()
        .map(|state| state.detail.snapshot().can_go_back)
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
        ends_at: ends_at_label(item),
        tags: item.tags.join(", "),
        links: item.external_urls.clone(),
        director: item.directors.join(", "),
        writers: item.writers.join(", "),
        star_rating: item.community_rating.unwrap_or(0.0),
        critic_rating: item.critic_rating.map(|value| value.round() as i32),
        subtitle: detail_info_line(page),
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
        genres: item.genres.join(" • "),
        studios: item.studios.join(" • "),
        media_info: item.media_summary.clone(),
        series_id: item.series_id.clone().unwrap_or_default(),
        series_summary: detail_series_summary(page),
        play_context: detail_series_play_context(page),
        season_id: episode_page
            .then(|| item.season_id.clone())
            .flatten()
            .unwrap_or_default(),
        season_name: episode_page
            .then(|| item.season_name.clone())
            .flatten()
            .unwrap_or_default(),
        season_summary: detail_season_summary(page),
        play_label: page.primary_play_label().unwrap_or_default().to_owned(),
        progress: progress_item.progress,
        progress_label: detail_progress_label(progress_item),
        playable: item.playable || !page.episodes.is_empty(),
        watched: item.watched,
        favorite: item.favorite,
        seasons: page
            .seasons
            .iter()
            .map(|season| to_detail_card(season, 600))
            .collect(),
        next_up,
        similar: page
            .similar
            .iter()
            .map(|similar| to_detail_card(similar, 600))
            .collect(),
        // Episode stills are wide thumbnails and ask for more pixels.
        episodes: page
            .episodes
            .iter()
            .map(|episode| to_detail_card(episode, 900))
            .collect(),
        people: item.people.iter().map(to_detail_person).collect(),
        guest_stars: item.guest_stars.iter().map(to_detail_person).collect(),
        current_episode_index: usize::try_from(current_episode_index(page)).ok(),
        unplayed_count: matches!(item.kind, DetailKind::Series | DetailKind::Season)
            .then_some(item.unplayed_count)
            .flatten()
            .unwrap_or(0),
        audio_options,
        subtitle_options,
        audio_index: tracks.audio_index,
        subtitle_index: tracks.subtitle_index,
    }
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

const HERO_HEIGHT: f32 = 320.0;
const CONTENT_MARGIN: f32 = 48.0;
const POSTER_OVERLAP: f32 = 96.0;
const LABEL_WIDTH: f32 = 86.0;

/// Page geometry shared by the sections.
struct Layout {
    width: f32,
    poster: egui::Vec2,
    content_left: f32,
}

impl Layout {
    fn new(width: f32, kind: Option<DetailKind>) -> Self {
        let poster = if kind == Some(DetailKind::Episode) {
            let width = (width * 0.30).clamp(260.0, 380.0);
            vec2(width, width * 9.0 / 16.0)
        } else {
            let width = (width * 0.22).clamp(220.0, 300.0);
            vec2(width, width * 1.5)
        };
        Self {
            width,
            poster,
            content_left: CONTENT_MARGIN + poster.x + 40.0,
        }
    }

    fn content_width(&self) -> f32 {
        (self.width - self.content_left - CONTENT_MARGIN).max(80.0)
    }
}

pub fn draw(ui: &mut Ui, state: &mut UiState, actions: &mut Vec<UiAction>) {
    let screen = ui.max_rect();
    let bar = Rect::from_min_size(screen.min, vec2(screen.width(), top_bar::HEIGHT));
    top_bar::draw(ui, state, bar, actions);
    let body = Rect::from_min_max(pos2(screen.left(), bar.bottom()), screen.max);
    ui.painter().rect_filled(body, 0.0, theme::BG);
    let mut body_ui = widgets::clipped_child(ui, body);
    if state.busy {
        body_ui.painter().text(
            body.center(),
            Align2::CENTER_CENTER,
            "Loading…",
            theme::regular(16.0),
            theme::TEXT_MUTED,
        );
        return;
    }
    let layout = Layout::new(body.width(), state.detail.kind);
    ScrollArea::vertical()
        .id_salt(("detail-page", state.detail.id.as_str()))
        .auto_shrink(false)
        .show(&mut body_ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let page_top = ui.cursor().top();
            draw_hero(ui, state, &layout);
            let block_top = ui.cursor().top();
            draw_title_strip(ui, state, &layout, actions);
            if !state.message.is_empty() {
                draw_error_banner(ui, state, &layout, actions);
            }
            draw_info(ui, state, &layout, actions);
            // Keep the block at least as tall as the poster's overhang, so the
            // shelves below never slide underneath the poster card.
            let minimum = layout.poster.y - POSTER_OVERLAP + 24.0;
            let used = ui.cursor().top() - block_top;
            if used < minimum {
                ui.allocate_exact_size(vec2(layout.width, minimum - used), Sense::hover());
            }
            draw_shelves(ui, state, &layout, actions);
            draw_poster_card(ui, state, &layout, page_top);
        });
}

fn draw_hero(ui: &mut Ui, state: &mut UiState, layout: &Layout) {
    let (hero, _) = ui.allocate_exact_size(vec2(layout.width, HERO_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(hero) {
        return;
    }
    let painter = ui.painter().with_clip_rect(hero.intersect(ui.clip_rect()));
    painter.rect_filled(hero, 0.0, theme::BG);
    if let Some((texture, size)) = resident(&mut state.images, state.detail.backdrop.as_ref()) {
        widgets::image_cover(&painter, texture, size, hero, 0.0);
    }
    painter.rect_filled(hero, 0.0, theme::with_alpha(theme::SCRIM, 0.44));
    widgets::vertical_gradient(
        &painter,
        hero,
        theme::with_alpha(theme::BG, 0.13),
        theme::with_alpha(theme::BG, 0.86),
    );
    if let Some((texture, size)) = resident(&mut state.images, state.detail.logo.as_ref()) {
        let logo = Rect::from_center_size(
            hero.center() - vec2(0.0, 10.0),
            vec2((hero.width() * 0.45).min(460.0), 130.0),
        );
        widgets::image_contain(&painter, texture, size, logo);
    }
}

/// The resident texture for `art`, queueing its fetch when missing.
fn resident(images: &mut ImageCache, art: Option<&Art>) -> Option<(egui::TextureId, [usize; 2])> {
    let art = art?;
    images.texture(art).zip(images.size(&art.key))
}

fn draw_title_strip(
    ui: &mut Ui,
    state: &mut UiState,
    layout: &Layout,
    actions: &mut Vec<UiAction>,
) {
    let detail = &state.detail;
    let busy = state.busy;
    let kind = detail.kind;
    let heading_links = !detail.series_id.is_empty()
        && matches!(kind, Some(DetailKind::Episode | DetailKind::Season));

    // Actions on the right: play pill, play-from-start, played, favorite.
    let play_label = if detail.play_label.is_empty() {
        "Play"
    } else {
        detail.play_label.as_str()
    };
    let play_width = 16.0
        + 24.0
        + 6.0
        + widgets::text_width(ui, play_label, theme::bold(theme::FONT_SUBTITLE))
        + 22.0;
    let restart = detail.playable && detail.progress > 0.0 && detail.progress < 1.0;
    let buttons: f32 = (if detail.playable {
        play_width + 9.0
    } else {
        0.0
    }) + (if restart { 44.0 + 9.0 } else { 0.0 })
        + 44.0 * 2.0
        + 9.0;
    let text_width = (layout.content_width() - buttons - 12.0).max(80.0);

    let heading = if detail.heading.is_empty() {
        &detail.title
    } else {
        &detail.heading
    };
    let heading = widgets::elided(
        ui,
        heading,
        theme::bold(27.0),
        theme::TEXT_BRIGHT,
        text_width,
    );
    let subheading = (!detail.subheading.is_empty()).then(|| {
        widgets::elided(
            ui,
            &detail.subheading,
            theme::bold(16.0),
            theme::TEXT_PRIMARY,
            text_width,
        )
    });
    let info_height = 16.0;
    let column_height = heading.size().y
        + subheading
            .as_ref()
            .map_or(0.0, |galley| 5.0 + galley.size().y)
        + 5.0
        + info_height;
    let height = column_height.max(44.0) + 32.0;
    let (strip, _) = ui.allocate_exact_size(vec2(layout.width, height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(strip, 0.0, theme::with_alpha(theme::SURFACE_DIM, 0.85));

    let left = strip.left() + layout.content_left;
    let mut y = strip.top() + 16.0;
    let heading_rect = Rect::from_min_size(pos2(left, y), heading.size());
    let heading_sense = if heading_links {
        Sense::click()
    } else {
        Sense::hover()
    };
    let heading_response = ui.interact(heading_rect, Id::new("detail-heading"), heading_sense);
    let heading_color = if heading_links && heading_response.hovered() {
        theme::ACCENT_BRIGHT
    } else {
        theme::TEXT_BRIGHT
    };
    painter.galley(heading_rect.min, heading, heading_color);
    if heading_links {
        widgets::pointer_on_hover(&heading_response);
        if heading_response.clicked() {
            actions.push(UiAction::OpenDetail {
                id: detail.series_id.clone(),
                kind: "Series".to_owned(),
            });
        }
    }
    y = heading_rect.bottom() + 5.0;
    if let Some(subheading) = subheading {
        let rect = Rect::from_min_size(pos2(left, y), subheading.size());
        let links = !detail.season_id.is_empty();
        let sense = if links {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, Id::new("detail-subheading"), sense);
        let color = if links && response.hovered() {
            theme::ACCENT_BRIGHT
        } else {
            theme::TEXT_PRIMARY
        };
        painter.galley(rect.min, subheading, color);
        if links {
            widgets::pointer_on_hover(&response);
            if response.clicked() {
                actions.push(UiAction::OpenDetail {
                    id: detail.season_id.clone(),
                    kind: "Season".to_owned(),
                });
            }
        }
        y = rect.bottom() + 5.0;
    }

    // Info row: metadata, community rating, critic rating, ends-at.
    let center_y = y + info_height / 2.0;
    let mut x = left;
    let mut info = |text: &str, font: egui::FontId, color: Color32| {
        if text.is_empty() || x > left + text_width {
            return;
        }
        let galley = widgets::elided(ui, text, font, color, left + text_width - x);
        let rect = widgets::galley_v_centered(&painter, x, center_y, galley);
        x = rect.right() + 14.0;
    };
    info(
        &detail.subtitle,
        theme::regular(13.0),
        theme::TEXT_SECONDARY,
    );
    if detail.star_rating > 0.0 {
        info(
            &format!("★ {}", (detail.star_rating * 10.0).round() / 10.0),
            theme::bold(13.0),
            theme::RATING_GOLD,
        );
    }
    if let Some(critic) = detail.critic_rating {
        let color = if critic >= 60 {
            theme::CRITIC_FRESH
        } else {
            theme::DANGER_TEXT
        };
        info(&format!("Critics {critic}%"), theme::bold(13.0), color);
    }
    info(&detail.ends_at, theme::regular(13.0), theme::TEXT_MUTED);

    // Buttons, right-aligned and vertically centered on the text column.
    let button_y = strip.top() + 16.0 + (column_height.max(44.0) - 44.0) / 2.0;
    let mut x = strip.right() - CONTENT_MARGIN - buttons + 9.0;
    if detail.playable {
        let rect = Rect::from_min_size(pos2(x, button_y), vec2(play_width, 44.0));
        if play_pill(ui, rect, play_label, !busy) {
            actions.push(UiAction::PlayCurrent { from_start: false });
        }
        x = rect.right() + 9.0;
    }
    let mut round = |icon: Icon, label: &str, active: bool, active_color: Color32, id: &str| {
        let rect = Rect::from_min_size(pos2(x, button_y), vec2(44.0, 44.0));
        x = rect.right() + 9.0;
        RoundIconButton {
            icon,
            label,
            size: 44.0,
            active,
            active_color,
            enabled: !busy,
        }
        .show(ui, rect, Id::new(("detail-action", id)))
        .clicked()
    };
    if restart
        && round(
            Icon::Replay,
            "Play from beginning",
            false,
            theme::TEXT_BRIGHT,
            "restart",
        )
    {
        actions.push(UiAction::PlayCurrent { from_start: true });
    }
    let watched_label = if detail.watched {
        "Mark unplayed"
    } else {
        "Mark played"
    };
    if round(
        Icon::Check,
        watched_label,
        detail.watched,
        theme::TEXT_BRIGHT,
        "watched",
    ) {
        actions.push(UiAction::SetWatched {
            id: detail.id.clone(),
            watched: !detail.watched,
        });
    }
    let favorite_label = if detail.favorite {
        "Remove from favorites"
    } else {
        "Add to favorites"
    };
    if round(
        Icon::Favorite,
        favorite_label,
        detail.favorite,
        theme::DANGER_TEXT,
        "favorite",
    ) {
        actions.push(UiAction::SetFavorite {
            id: detail.id.clone(),
            favorite: !detail.favorite,
        });
    }
}

/// Jellyfin's primary detail action: a pill with the play icon and what it
/// will do ("Resume", "Play", "Play again").
fn play_pill(ui: &mut Ui, rect: Rect, label: &str, enabled: bool) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, Id::new("detail-play"), sense);
    widgets::describe_button(&response, enabled, label);
    let opacity = if enabled { 1.0 } else { 0.45 };
    let painter = ui.painter();
    let fill = if response.hovered() && enabled {
        theme::ACCENT_BRIGHT
    } else {
        theme::ACCENT
    };
    painter.rect_filled(rect, rect.height() / 2.0, fill.gamma_multiply(opacity));
    if response.has_focus() {
        widgets::border(
            painter,
            rect.expand(2.0),
            rect.height() / 2.0 + 2.0,
            2.0,
            theme::TEXT_BRIGHT,
        );
    }
    icons::paint(
        painter,
        Icon::Play,
        Rect::from_min_size(
            pos2(rect.left() + 16.0, rect.center().y - 12.0),
            vec2(24.0, 24.0),
        ),
        theme::BG.gamma_multiply(opacity),
    );
    painter.text(
        pos2(rect.left() + 46.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::bold(theme::FONT_SUBTITLE),
        theme::BG.gamma_multiply(opacity),
    );
    if enabled {
        widgets::pointer_on_hover(&response);
    }
    response.clicked()
}

fn draw_error_banner(ui: &mut Ui, state: &UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    let (rect, _) = ui.allocate_exact_size(vec2(layout.width, 38.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::DANGER_SURFACE);
    let galley = widgets::elided(
        ui,
        &state.message,
        theme::regular(12.0),
        theme::DANGER_TEXT,
        rect.width() - 220.0,
    );
    widgets::galley_centered(painter, rect, galley);
    let button = Rect::from_min_size(
        pos2(rect.right() - 92.0, rect.top() + 4.0),
        vec2(80.0, 30.0),
    );
    let response = ui.put(button, egui::Button::new("Retry"));
    widgets::pointer_on_hover(&response);
    if response.clicked() {
        actions.push(UiAction::Retry);
    }
}

fn draw_info(ui: &mut Ui, state: &mut UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    let detail = &mut state.detail;
    let left = layout.content_left;
    let width = layout.content_width();
    ui.allocate_exact_size(vec2(layout.width, 18.0), Sense::hover());
    let mut first = true;
    let mut row = |ui: &mut Ui, height: f32| -> Rect {
        if !first {
            ui.allocate_exact_size(vec2(layout.width, 12.0), Sense::hover());
        }
        first = false;
        let (rect, _) = ui.allocate_exact_size(vec2(layout.width, height), Sense::hover());
        Rect::from_min_size(pos2(rect.left() + left, rect.top()), vec2(width, height))
    };

    if detail.progress > 0.0 && detail.progress < 1.0 {
        let rect = row(ui, 18.0);
        let label = if detail.kind == Some(DetailKind::Series) {
            &detail.play_context
        } else {
            &detail.progress_label
        };
        let painter = ui.painter();
        let mut x = rect.left();
        if !label.is_empty() {
            let galley = widgets::elided(
                ui,
                label,
                theme::regular(11.0),
                theme::TEXT_MUTED,
                width * 0.55,
            );
            x = widgets::galley_v_centered(painter, x, rect.center().y, galley).right() + 10.0;
        }
        let bar = Rect::from_min_size(
            pos2(x, rect.center().y - 2.5),
            vec2((width * 0.4).min(300.0), 5.0),
        );
        painter.rect_filled(bar, 2.0, theme::SURFACE_HOVER);
        painter.rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * detail.progress, bar.height())),
            2.0,
            theme::ACCENT,
        );
    }

    let labeled_line = |ui: &mut Ui, rect: Rect, label: &str, value: &str| {
        let painter = ui.painter();
        painter.text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::regular(13.0),
            theme::TEXT_MUTED,
        );
        let galley = widgets::elided(
            ui,
            value,
            theme::regular(13.0),
            theme::TEXT_PRIMARY,
            rect.width() - LABEL_WIDTH - 12.0,
        );
        widgets::galley_v_centered(
            painter,
            rect.left() + LABEL_WIDTH + 12.0,
            rect.center().y,
            galley,
        );
    };
    if !detail.media_info.is_empty() {
        let rect = row(ui, 16.0);
        labeled_line(ui, rect, "Video", &detail.media_info);
    }
    if detail.audio_options.len() == 1 {
        let rect = row(ui, 16.0);
        labeled_line(ui, rect, "Audio", &detail.audio_options[0]);
    }
    let combo_width = (width - LABEL_WIDTH - 12.0).min(520.0);
    let mut track_combo =
        |ui: &mut Ui, label: &str, options: &[String], index: &mut i32, audio: bool| {
            let rect = row(ui, 34.0);
            ui.painter().text(
                pos2(rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                label,
                theme::regular(13.0),
                theme::TEXT_MUTED,
            );
            let combo = Rect::from_min_size(
                pos2(rect.left() + LABEL_WIDTH + 12.0, rect.top()),
                vec2(combo_width, 34.0),
            );
            let mut selected = usize::try_from(*index).unwrap_or(0);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(combo));
            child.spacing_mut().interact_size.y = 34.0;
            egui::ComboBox::from_id_salt(("detail-track", audio))
                .width(combo_width)
                .selected_text(options.get(selected).map_or("", String::as_str))
                .show_ui(&mut child, |ui| {
                    for (option_index, option) in options.iter().enumerate() {
                        ui.selectable_value(&mut selected, option_index, option);
                    }
                });
            let selected = i32::try_from(selected).unwrap_or(0);
            if selected != *index {
                *index = selected;
                actions.push(UiAction::DetailTrack {
                    index: selected,
                    audio,
                });
            }
        };
    if detail.audio_options.len() > 1 {
        let options = std::mem::take(&mut detail.audio_options);
        track_combo(ui, "Audio", &options, &mut detail.audio_index, true);
        detail.audio_options = options;
    }
    if detail.subtitle_options.len() > 1 {
        let options = std::mem::take(&mut detail.subtitle_options);
        track_combo(ui, "Subtitles", &options, &mut detail.subtitle_index, false);
        detail.subtitle_options = options;
    }

    let mut paragraph = |ui: &mut Ui, text: &str, font: egui::FontId, color: Color32| {
        let galley = widgets::wrapped(ui, text, font, color, width, None);
        let rect = row(ui, galley.size().y);
        ui.painter().galley(rect.min, galley, Color32::WHITE);
    };
    if !detail.tagline.is_empty() {
        paragraph(
            ui,
            &detail.tagline,
            theme::regular(16.0),
            theme::TEXT_PRIMARY,
        );
    }
    if !detail.overview.is_empty() {
        paragraph(
            ui,
            &detail.overview,
            theme::regular(14.0),
            theme::TEXT_SECONDARY,
        );
    }
    if !detail.tags.is_empty() {
        paragraph(
            ui,
            &format!("Tags: {}", detail.tags),
            theme::regular(12.0),
            theme::TEXT_MUTED,
        );
    }
    if !detail.links.is_empty() {
        let rect = row(ui, 18.0);
        let mut x = rect.left();
        for (index, (name, url)) in detail.links.iter().enumerate() {
            let galley = widgets::elided(
                ui,
                name,
                theme::bold(13.0),
                theme::TEXT_SECONDARY,
                rect.right() - x,
            );
            let link = Rect::from_min_size(
                pos2(x, rect.center().y - galley.size().y / 2.0),
                galley.size(),
            );
            let response = ui.interact(link, Id::new(("detail-link", index)), Sense::click());
            widgets::describe_button(&response, true, name);
            let color = if response.hovered() {
                theme::ACCENT_BRIGHT
            } else {
                theme::TEXT_SECONDARY
            };
            ui.painter().galley(link.min, galley, color);
            widgets::pointer_on_hover(&response);
            if response.on_hover_text(url).clicked() {
                actions.push(UiAction::OpenExternalUrl(url.clone()));
            }
            x = link.right() + 14.0;
            if x >= rect.right() {
                break;
            }
        }
    }
    for (label, value) in [
        ("Genres", &detail.genres),
        ("Director", &detail.director),
        ("Writers", &detail.writers),
        ("Studios", &detail.studios),
    ] {
        if value.is_empty() {
            continue;
        }
        let galley = widgets::wrapped(
            ui,
            value,
            theme::bold(13.0),
            theme::TEXT_PRIMARY,
            width - LABEL_WIDTH - 12.0,
            None,
        );
        let rect = row(ui, galley.size().y);
        ui.painter().text(
            rect.min,
            Align2::LEFT_TOP,
            label,
            theme::regular(13.0),
            theme::TEXT_MUTED,
        );
        ui.painter().galley(
            rect.min + vec2(LABEL_WIDTH + 12.0, 0.0),
            galley,
            Color32::WHITE,
        );
    }
    ui.allocate_exact_size(vec2(layout.width, 24.0), Sense::hover());
}

fn draw_shelves(ui: &mut Ui, state: &mut UiState, layout: &Layout, actions: &mut Vec<UiAction>) {
    let detail = &state.detail;
    let images = &mut state.images;
    let width = layout.width;
    match detail.kind {
        Some(DetailKind::Series) => {
            if !detail.next_up.is_empty() {
                episode_shelf(
                    ui,
                    images,
                    EpisodeShelfSpec {
                        id: "next-up",
                        heading: "Next Up",
                        episodes: &detail.next_up,
                        current_id: "",
                        current_index: None,
                        plays: true,
                    },
                    width,
                    actions,
                );
            }
            season_shelf(
                ui,
                images,
                SeasonShelfSpec {
                    id: "seasons",
                    heading: "Seasons",
                    seasons: &detail.seasons,
                    current_id: "",
                    summary: &detail.series_summary,
                    failed: !state.message.is_empty(),
                },
                width,
                actions,
            );
        }
        Some(DetailKind::Season) => {
            draw_episode_list(
                ui,
                images,
                detail,
                !state.message.is_empty(),
                state.busy,
                width,
                actions,
            );
            if detail.seasons.len() > 1 {
                season_shelf(
                    ui,
                    images,
                    SeasonShelfSpec {
                        id: "more-seasons",
                        heading: "More Seasons",
                        seasons: &detail.seasons,
                        current_id: &detail.id,
                        summary: "",
                        failed: false,
                    },
                    width,
                    actions,
                );
            }
        }
        Some(DetailKind::Episode) if !detail.episodes.is_empty() => {
            let heading = if detail.season_name.is_empty() {
                "More from this season".to_owned()
            } else {
                format!("More from {}", detail.season_name)
            };
            episode_shelf(
                ui,
                images,
                EpisodeShelfSpec {
                    id: "season-episodes",
                    heading: &heading,
                    episodes: &detail.episodes,
                    current_id: &detail.id,
                    current_index: detail.current_episode_index,
                    plays: false,
                },
                width,
                actions,
            );
        }
        _ => {}
    }
    if !detail.people.is_empty() {
        people_shelf(ui, images, "Cast & Crew", &detail.people, width);
    }
    if !detail.guest_stars.is_empty() {
        people_shelf(ui, images, "Guest Stars", &detail.guest_stars, width);
    }
    if !detail.similar.is_empty() {
        season_shelf(
            ui,
            images,
            SeasonShelfSpec {
                id: "similar",
                heading: "More Like This",
                seasons: &detail.similar,
                current_id: "",
                summary: "",
                failed: false,
            },
            width,
            actions,
        );
    }
}

/// A shelf's own scroll identity: per page, so another item's shelf opens
/// at its start.
fn shelf_id(name: &str, page: &str) -> Id {
    Id::new(("detail-shelf", name, page.to_owned()))
}

struct EpisodeShelfSpec<'a> {
    id: &'a str,
    heading: &'a str,
    episodes: &'a [DetailCard],
    current_id: &'a str,
    current_index: Option<usize>,
    /// Next Up plays the episode on click, as Jellyfin's does; other uses
    /// open the episode's page.
    plays: bool,
}

/// Wide thumbnail cards: Next Up, and "More from Season N" with the open
/// episode highlighted and centered.
fn episode_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    spec: EpisodeShelfSpec<'_>,
    width: f32,
    actions: &mut Vec<UiAction>,
) {
    const CARD: f32 = 264.0;
    const GAP: f32 = 16.0;
    let card_height = CARD * 9.0 / 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, card_height + 116.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, 0.0, theme::BG);
    let viewport_width = (width - 96.0).min(1400.0);
    let viewport_x = rect.left() + (width - viewport_width) / 2.0;
    ui.painter().text(
        pos2(viewport_x, rect.top() + 14.0),
        Align2::LEFT_TOP,
        spec.heading,
        theme::bold(21.0),
        theme::TEXT_PRIMARY,
    );
    let viewport = Rect::from_min_size(
        pos2(viewport_x, rect.top() + 52.0),
        vec2(viewport_width, rect.height() - 60.0),
    );
    let content = (spec.episodes.len() as f32 * (CARD + GAP) - GAP).max(0.0);
    let id = shelf_id(spec.id, spec.current_id);
    let mut shelf = Shelf::begin(ui, id, viewport, content);
    if let Some(index) = spec.current_index {
        shelf.center_on(index, CARD, GAP);
    }
    let offset = shelf.scroll(ui);
    let strip = widgets::clipped_child(ui, viewport);
    for (index, episode) in spec.episodes.iter().enumerate() {
        let left = index as f32 * (CARD + GAP);
        let card = Rect::from_min_size(
            pos2(viewport.left() + offset + left, viewport.top()),
            vec2(CARD, card_height),
        );
        if card.right() < viewport.left() - viewport.width()
            || card.left() > viewport.right() + viewport.width()
        {
            continue;
        }
        let current = episode.id == spec.current_id;
        let response = strip.interact(card, id.with(index), Sense::click());
        widgets::describe_button(&response, true, &episode.title);
        let hovered = response.hovered();
        let painter = strip.painter().clone();
        widgets::drop_shadow(
            &painter,
            card,
            theme::RADIUS_MEDIUM,
            8.0,
            3.0,
            theme::with_alpha(theme::SCRIM, 0.44),
        );
        painter.rect_filled(card, theme::RADIUS_MEDIUM, theme::SURFACE);
        match resident(images, episode.art.as_ref()) {
            Some((texture, size)) => {
                widgets::image_cover(&painter, texture, size, card, theme::RADIUS_MEDIUM)
            }
            None => {
                let galley = widgets::wrapped(
                    &strip,
                    &episode.title,
                    theme::regular(13.0),
                    theme::BORDER_STRONG,
                    CARD - 20.0,
                    Some(3),
                );
                widgets::galley_centered(&painter, card, galley);
            }
        }
        if episode.watched {
            widgets::watched_badge(
                &painter,
                Rect::from_min_size(
                    pos2(card.right() - 33.0, card.top() + 8.0),
                    vec2(25.0, 25.0),
                ),
                16.0,
            );
        }
        widgets::card_progress(&painter, card.shrink(1.0), 5.0, episode.progress);
        if spec.plays && hovered {
            painter.circle_filled(card.center(), 26.0, theme::with_alpha(theme::SCRIM, 0.63));
            icons::paint_centered(
                &painter,
                Icon::Play,
                card.center(),
                28.0,
                theme::TEXT_BRIGHT,
            );
        }
        let (border_width, border_color) = if response.has_focus() {
            (3.0, theme::ACCENT_BRIGHT)
        } else if current {
            (3.0, theme::ACCENT)
        } else if hovered {
            (2.0, theme::BORDER_STRONG)
        } else {
            (1.0, theme::SURFACE_HOVER)
        };
        widgets::border(
            &painter,
            card,
            theme::RADIUS_MEDIUM,
            border_width,
            border_color,
        );
        let label = if episode.index_label.is_empty() {
            episode.title.clone()
        } else {
            format!("{} {}", episode.index_label, episode.title)
        };
        let (font, color) = if current {
            (theme::bold(13.0), theme::TEXT_PRIMARY)
        } else {
            (theme::regular(13.0), theme::TEXT_SECONDARY)
        };
        let title = widgets::elided(&strip, &label, font, color, CARD);
        let title_rect =
            widgets::galley_at(&painter, pos2(card.left(), card.bottom() + 4.0), title);
        if !episode.subtitle.is_empty() {
            let subtitle = widgets::elided(
                &strip,
                &episode.subtitle,
                theme::regular(12.0),
                theme::TEXT_MUTED,
                CARD,
            );
            widgets::galley_at(
                &painter,
                pos2(card.left(), title_rect.bottom() + 4.0),
                subtitle,
            );
        }
        widgets::pointer_on_hover(&response);
        if response.gained_focus() {
            shelf.reveal(left, CARD);
        }
        if response.clicked() {
            actions.push(if spec.plays {
                UiAction::PlayDetail(episode.id.clone())
            } else {
                UiAction::OpenDetail {
                    id: episode.id.clone(),
                    kind: episode.kind.to_owned(),
                }
            });
        }
    }
    shelf_arrows(ui, &mut shelf, offset, viewport, rect.top() + 80.0, id);
    shelf.end(ui);
}

/// The outlined ‹ › buttons the detail shelves use, straddling the viewport
/// edges.
fn shelf_arrows(ui: &mut Ui, shelf: &mut Shelf, offset: f32, viewport: Rect, y: f32, id: Id) {
    if shelf.can_page_back(offset) {
        let rect = Rect::from_min_size(pos2(viewport.left() - 18.0, y), vec2(38.0, 56.0));
        if widgets::shelf_arrow(
            ui,
            rect,
            id.with("left"),
            true,
            theme::OVERLAY,
            theme::OVERLAY_RAISED,
            true,
        ) {
            shelf.page(false);
        }
    }
    if shelf.can_page_forward(offset) {
        let rect = Rect::from_min_size(pos2(viewport.right() - 20.0, y), vec2(38.0, 56.0));
        if widgets::shelf_arrow(
            ui,
            rect,
            id.with("right"),
            false,
            theme::OVERLAY,
            theme::OVERLAY_RAISED,
            true,
        ) {
            shelf.page(true);
        }
    }
}

struct SeasonShelfSpec<'a> {
    id: &'a str,
    heading: &'a str,
    seasons: &'a [DetailCard],
    /// The season whose page this is, highlighted like the current episode.
    current_id: &'a str,
    summary: &'a str,
    failed: bool,
}

/// Portrait cards: Seasons, More Seasons, More Like This.
fn season_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    spec: SeasonShelfSpec<'_>,
    width: f32,
    actions: &mut Vec<UiAction>,
) {
    const CARD: f32 = 158.0;
    const GAP: f32 = 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 360.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, theme::BG);
    let viewport_width = (width - 56.0).min(1180.0);
    let viewport_x = rect.left() + (width - viewport_width) / 2.0;
    let heading = painter.text(
        pos2(viewport_x, rect.top() + 16.0),
        Align2::LEFT_TOP,
        spec.heading,
        theme::bold(22.0),
        theme::TEXT_PRIMARY,
    );
    if !spec.seasons.is_empty() && !spec.summary.is_empty() {
        let summary = widgets::elided(
            ui,
            spec.summary,
            theme::regular(12.0),
            theme::TEXT_FAINT,
            viewport_width - heading.width() - 24.0,
        );
        let x = viewport_x + viewport_width - summary.size().x;
        widgets::galley_at(&painter, pos2(x, rect.top() + 20.0), summary);
    }
    if spec.seasons.is_empty() {
        let empty = Rect::from_min_size(
            pos2(viewport_x, rect.top() + 55.0),
            vec2(viewport_width, 116.0),
        );
        painter.rect_filled(empty, theme::RADIUS_MEDIUM, theme::SURFACE);
        widgets::border(
            &painter,
            empty,
            theme::RADIUS_MEDIUM,
            1.0,
            theme::SURFACE_HOVER,
        );
        let text = if spec.failed {
            "Seasons could not be loaded."
        } else {
            "No seasons are available for this series."
        };
        painter.text(
            empty.center(),
            Align2::CENTER_CENTER,
            text,
            theme::regular(13.0),
            theme::TEXT_MUTED,
        );
        return;
    }
    let viewport = Rect::from_min_size(
        pos2(viewport_x, rect.top() + 55.0),
        vec2(viewport_width, rect.height() - 64.0),
    );
    let content = spec.seasons.len() as f32 * (CARD + GAP) - GAP;
    let id = shelf_id(spec.id, spec.current_id);
    let mut shelf = Shelf::begin(ui, id, viewport, content);
    let offset = shelf.scroll(ui);
    let strip = widgets::clipped_child(ui, viewport);
    for (index, season) in spec.seasons.iter().enumerate() {
        let left = index as f32 * (CARD + GAP);
        let poster = Rect::from_min_size(
            pos2(viewport.left() + offset + left, viewport.top()),
            vec2(CARD, CARD * 1.5),
        );
        if poster.right() < viewport.left() - viewport.width()
            || poster.left() > viewport.right() + viewport.width()
        {
            continue;
        }
        let current = !spec.current_id.is_empty() && season.id == spec.current_id;
        let response = strip.interact(poster, id.with(index), Sense::click());
        widgets::describe_button(&response, true, &season.title);
        let hovered = response.hovered();
        let painter = strip.painter().clone();
        if hovered {
            widgets::drop_shadow(
                &painter,
                poster,
                theme::RADIUS_MEDIUM,
                14.0,
                5.0,
                theme::with_alpha(theme::SCRIM, 0.63),
            );
        } else {
            widgets::drop_shadow(
                &painter,
                poster,
                theme::RADIUS_MEDIUM,
                7.0,
                3.0,
                theme::with_alpha(theme::SCRIM, 0.4),
            );
        }
        match resident(images, season.art.as_ref()) {
            Some((texture, size)) => {
                widgets::image_cover(&painter, texture, size, poster, theme::RADIUS_MEDIUM)
            }
            None => {
                widgets::diagonal_gradient(
                    &painter,
                    poster,
                    theme::RADIUS_MEDIUM,
                    theme::SURFACE_HOVER,
                    theme::BG,
                );
                let galley = widgets::wrapped(
                    &strip,
                    &season.title,
                    theme::regular(14.0),
                    theme::BORDER_STRONG,
                    CARD - 16.0,
                    Some(4),
                );
                widgets::galley_centered(&painter, poster, galley);
            }
        }
        if season.unplayed_count > 0 {
            let badge = Rect::from_min_size(
                pos2(poster.right() - 42.0, poster.top() + 9.0),
                vec2(32.0, 25.0),
            );
            painter.rect_filled(badge, theme::RADIUS_LARGE, theme::ACCENT);
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                season.unplayed_count.to_string(),
                theme::bold(11.0),
                Color32::WHITE,
            );
        } else if season.watched {
            widgets::watched_badge(
                &painter,
                Rect::from_min_size(
                    pos2(poster.right() - 35.0, poster.top() + 9.0),
                    vec2(25.0, 25.0),
                ),
                16.0,
            );
        }
        let (border_width, border_color) = if response.has_focus() {
            (3.0, theme::ACCENT_BRIGHT)
        } else if current {
            (3.0, theme::ACCENT)
        } else if hovered {
            (2.0, theme::BORDER_STRONG)
        } else {
            (1.0, theme::SURFACE_HOVER)
        };
        widgets::border(
            &painter,
            poster,
            theme::RADIUS_MEDIUM,
            border_width,
            border_color,
        );
        let title_color = if current {
            theme::ACCENT_BRIGHT
        } else {
            theme::TEXT_PRIMARY
        };
        let title = widgets::elided(&strip, &season.title, theme::bold(14.0), title_color, CARD);
        let title_rect =
            widgets::galley_at(&painter, pos2(poster.left(), poster.bottom() + 7.0), title);
        let subtitle = widgets::elided(
            &strip,
            &season.subtitle,
            theme::regular(11.0),
            theme::TEXT_FAINT,
            CARD,
        );
        widgets::galley_at(
            &painter,
            pos2(poster.left(), title_rect.bottom() + 7.0),
            subtitle,
        );
        widgets::pointer_on_hover(&response);
        if response.gained_focus() {
            shelf.reveal(left, CARD);
        }
        if response.clicked() {
            actions.push(UiAction::OpenDetail {
                id: season.id.clone(),
                kind: season.kind.to_owned(),
            });
        }
    }
    shelf_arrows(ui, &mut shelf, offset, viewport, rect.top() + 142.0, id);
    shelf.end(ui);
}

/// Round cast photos with name and role.
fn people_shelf(
    ui: &mut Ui,
    images: &mut ImageCache,
    heading: &str,
    people: &[DetailPerson],
    width: f32,
) {
    const CARD: f32 = 112.0;
    const GAP: f32 = 14.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 220.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, 0.0, theme::BG);
    let viewport_width = (width - 56.0).min(1180.0);
    let viewport_x = rect.left() + (width - viewport_width) / 2.0;
    ui.painter().text(
        pos2(viewport_x, rect.top() + 12.0),
        Align2::LEFT_TOP,
        heading,
        theme::bold(21.0),
        theme::TEXT_PRIMARY,
    );
    let viewport = Rect::from_min_size(
        pos2(viewport_x, rect.top() + 49.0),
        vec2(viewport_width, rect.height() - 57.0),
    );
    let content = people.len() as f32 * (CARD + GAP) - GAP;
    let id = Id::new((
        "detail-people",
        heading,
        people.first().map(|person| person.name.clone()),
    ));
    let mut shelf = Shelf::begin(ui, id, viewport, content);
    let offset = shelf.scroll(ui);
    let strip = widgets::clipped_child(ui, viewport);
    let painter = strip.painter().clone();
    for (index, person) in people.iter().enumerate() {
        let left = index as f32 * (CARD + GAP);
        let photo = Rect::from_min_size(
            pos2(viewport.left() + offset + left, viewport.top()),
            vec2(CARD, CARD),
        );
        if photo.right() < viewport.left() - viewport.width()
            || photo.left() > viewport.right() + viewport.width()
        {
            continue;
        }
        let radius = CARD / 2.0;
        match resident(images, person.art.as_ref()) {
            Some((texture, size)) => widgets::image_cover(&painter, texture, size, photo, radius),
            None => {
                widgets::diagonal_gradient(
                    &painter,
                    photo,
                    radius,
                    theme::SURFACE_HOVER,
                    theme::SURFACE,
                );
                painter.text(
                    photo.center(),
                    Align2::CENTER_CENTER,
                    "?",
                    theme::bold(28.0),
                    theme::BORDER_STRONG,
                );
            }
        }
        widgets::border(&painter, photo, radius, 1.0, theme::BORDER);
        let name = widgets::elided(
            &strip,
            &person.name,
            theme::bold(12.0),
            theme::TEXT_PRIMARY,
            CARD,
        );
        let name_rect = Rect::from_min_size(
            pos2(photo.center().x - name.size().x / 2.0, photo.bottom() + 6.0),
            name.size(),
        );
        painter.galley(name_rect.min, name, Color32::WHITE);
        let role = widgets::elided(
            &strip,
            &person.role,
            theme::regular(10.0),
            theme::TEXT_FAINT,
            CARD,
        );
        painter.galley(
            pos2(
                photo.center().x - role.size().x / 2.0,
                name_rect.bottom() + 6.0,
            ),
            role,
            Color32::WHITE,
        );
    }
    shelf_arrows(ui, &mut shelf, offset, viewport, rect.top() + 80.0, id);
    shelf.end(ui);
}

/// A season page's episode list: Jellyfin-style rows with a wide thumbnail
/// that plays, a title that opens, metadata, overview, and toggles.
fn draw_episode_list(
    ui: &mut Ui,
    images: &mut ImageCache,
    detail: &DetailVm,
    failed: bool,
    busy: bool,
    width: f32,
    actions: &mut Vec<UiAction>,
) {
    let side = ((width - 1180.0) / 2.0).max(28.0);
    let inner = width - side * 2.0;
    ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    let (header, _) = ui.allocate_exact_size(vec2(width, 38.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.text(
        pos2(header.left() + side, header.center().y),
        Align2::LEFT_CENTER,
        "Episodes",
        theme::bold(22.0),
        theme::TEXT_PRIMARY,
    );
    if !detail.episodes.is_empty() {
        painter.text(
            pos2(header.right() - side, header.center().y),
            Align2::RIGHT_CENTER,
            &detail.season_summary,
            theme::regular(12.0),
            theme::TEXT_FAINT,
        );
    }
    if detail.episodes.is_empty() {
        ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
        let (rect, _) = ui.allocate_exact_size(vec2(width, 116.0), Sense::hover());
        let empty = Rect::from_min_size(pos2(rect.left() + side, rect.top()), vec2(inner, 116.0));
        painter.rect_filled(empty, theme::RADIUS_MEDIUM, theme::SURFACE);
        widgets::border(
            &painter,
            empty,
            theme::RADIUS_MEDIUM,
            1.0,
            theme::SURFACE_HOVER,
        );
        let text = if failed {
            "Episodes could not be loaded."
        } else {
            "No episodes are available for this season."
        };
        painter.text(
            empty.center(),
            Align2::CENTER_CENTER,
            text,
            theme::regular(13.0),
            theme::TEXT_MUTED,
        );
    }
    for (index, episode) in detail.episodes.iter().enumerate() {
        ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
        let (row, _) = ui.allocate_exact_size(vec2(width, 158.0), Sense::hover());
        if !ui.is_rect_visible(row) {
            continue;
        }
        episode_row(
            ui,
            images,
            episode,
            Rect::from_min_size(pos2(row.left() + side, row.top()), vec2(inner, 158.0)),
            busy,
            index,
            actions,
        );
    }
    ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
}

fn episode_row(
    ui: &mut Ui,
    images: &mut ImageCache,
    episode: &DetailCard,
    row: Rect,
    busy: bool,
    index: usize,
    actions: &mut Vec<UiAction>,
) {
    let painter = ui.painter().clone();
    let thumb = Rect::from_min_size(row.min + vec2(0.0, 4.0), vec2(240.0, 135.0));
    let sense = if busy { Sense::hover() } else { Sense::click() };
    let response = ui.interact(
        thumb,
        Id::new(("episode-thumb", index, episode.id.as_str())),
        sense,
    );
    widgets::describe_button(&response, !busy, &format!("Play {}", episode.title));
    let hovered = response.hovered();
    widgets::drop_shadow(
        &painter,
        thumb,
        theme::RADIUS_MEDIUM,
        8.0,
        3.0,
        theme::with_alpha(theme::SCRIM, 0.44),
    );
    painter.rect_filled(thumb, theme::RADIUS_MEDIUM, theme::SURFACE);
    match resident(images, episode.art.as_ref()) {
        Some((texture, size)) => {
            widgets::image_cover(&painter, texture, size, thumb, theme::RADIUS_MEDIUM)
        }
        None => {
            let galley = widgets::wrapped(
                ui,
                &episode.title,
                theme::regular(13.0),
                theme::BORDER_STRONG,
                220.0,
                Some(3),
            );
            widgets::galley_centered(&painter, thumb, galley);
        }
    }
    if episode.watched {
        widgets::watched_badge(
            &painter,
            Rect::from_min_size(
                pos2(thumb.right() - 33.0, thumb.top() + 8.0),
                vec2(25.0, 25.0),
            ),
            16.0,
        );
    }
    widgets::card_progress(&painter, thumb.shrink(1.0), 5.0, episode.progress);
    if hovered {
        painter.circle_filled(thumb.center(), 23.0, theme::with_alpha(theme::SCRIM, 0.63));
        icons::paint_centered(
            &painter,
            Icon::Play,
            thumb.center(),
            26.0,
            theme::TEXT_BRIGHT,
        );
    }
    let border_color = if response.has_focus() {
        theme::ACCENT_BRIGHT
    } else if hovered {
        theme::BORDER_STRONG
    } else {
        theme::SURFACE_HOVER
    };
    widgets::border(
        &painter,
        thumb,
        theme::RADIUS_MEDIUM,
        if response.has_focus() { 3.0 } else { 1.0 },
        border_color,
    );
    if !busy {
        widgets::pointer_on_hover(&response);
    }
    if response.clicked() {
        actions.push(UiAction::PlayDetail(episode.id.clone()));
    }

    // Toggles on the right.
    let toggles_left = row.right() - 34.0 * 2.0 - 8.0;
    let watched = RoundIconButton {
        icon: Icon::Check,
        label: "Played",
        size: 34.0,
        active: episode.watched,
        active_color: theme::TEXT_BRIGHT,
        enabled: !busy,
    }
    .show(
        ui,
        Rect::from_min_size(pos2(toggles_left, row.top() + 4.0), vec2(34.0, 34.0)),
        Id::new(("episode-played", index)),
    );
    if watched.clicked() {
        actions.push(UiAction::SetWatched {
            id: episode.id.clone(),
            watched: !episode.watched,
        });
    }
    let favorite = RoundIconButton {
        icon: Icon::Favorite,
        label: "Favorite",
        size: 34.0,
        active: episode.favorite,
        active_color: theme::DANGER_TEXT,
        enabled: !busy,
    }
    .show(
        ui,
        Rect::from_min_size(pos2(toggles_left + 42.0, row.top() + 4.0), vec2(34.0, 34.0)),
        Id::new(("episode-favorite", index)),
    );
    if favorite.clicked() {
        actions.push(UiAction::SetFavorite {
            id: episode.id.clone(),
            favorite: !episode.favorite,
        });
    }

    // Text column between the thumbnail and the toggles.
    let left = thumb.right() + 18.0;
    let text_width = (toggles_left - 18.0 - left).max(40.0);
    let label = if episode.index_label.is_empty() {
        episode.title.clone()
    } else {
        format!("{} {}", episode.index_label, episode.title)
    };
    let title = widgets::elided(
        ui,
        &label,
        theme::bold(16.0),
        theme::TEXT_PRIMARY,
        text_width,
    );
    let title_rect = Rect::from_min_size(pos2(left, row.top()), title.size());
    let title_response = ui.interact(
        title_rect,
        Id::new(("episode-title", index)),
        Sense::click(),
    );
    widgets::describe_button(&title_response, true, &episode.title);
    let color = if title_response.hovered() {
        theme::ACCENT_BRIGHT
    } else {
        theme::TEXT_PRIMARY
    };
    painter.galley(title_rect.min, title, color);
    widgets::pointer_on_hover(&title_response);
    if title_response.clicked() {
        actions.push(UiAction::OpenDetail {
            id: episode.id.clone(),
            kind: episode.kind.to_owned(),
        });
    }
    let meta_y = title_rect.bottom() + 5.0 + 8.0;
    let mut x = left;
    let mut meta = |text: &str, font: egui::FontId, color: Color32| {
        if text.is_empty() || x >= left + text_width {
            return;
        }
        let galley = widgets::elided(ui, text, font, color, left + text_width - x);
        x = widgets::galley_v_centered(&painter, x, meta_y, galley).right() + 12.0;
    };
    meta(
        &episode.subtitle,
        theme::regular(12.0),
        theme::TEXT_SECONDARY,
    );
    if episode.rating > 0.0 {
        meta(
            &format!("★ {}", (episode.rating * 10.0).round() / 10.0),
            theme::bold(12.0),
            theme::RATING_GOLD,
        );
    }
    meta(&episode.ends_at, theme::regular(12.0), theme::TEXT_MUTED);
    if !episode.overview.is_empty() {
        let overview = widgets::wrapped(
            ui,
            &episode.overview,
            theme::regular(12.0),
            theme::TEXT_MUTED,
            text_width,
            Some(3),
        );
        painter.galley(pos2(left, meta_y + 8.0 + 5.0), overview, Color32::WHITE);
    }
}

/// The poster (or episode still), floating over the hero/content boundary.
fn draw_poster_card(ui: &mut Ui, state: &mut UiState, layout: &Layout, page_top: f32) {
    let detail = &state.detail;
    let rect = Rect::from_min_size(
        pos2(
            ui.max_rect().left() + CONTENT_MARGIN,
            page_top + HERO_HEIGHT - POSTER_OVERLAP,
        ),
        layout.poster,
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().clone();
    widgets::drop_shadow(
        &painter,
        rect,
        theme::RADIUS_MEDIUM,
        22.0,
        6.0,
        theme::with_alpha(theme::SCRIM, 0.63),
    );
    painter.rect_filled(rect, theme::RADIUS_MEDIUM, theme::SURFACE);
    match resident(&mut state.images, detail.artwork.as_ref()) {
        Some((texture, size)) => {
            widgets::image_cover(&painter, texture, size, rect, theme::RADIUS_MEDIUM)
        }
        None => {
            let galley = widgets::wrapped(
                ui,
                &detail.title,
                theme::regular(15.0),
                theme::BORDER_STRONG,
                rect.width() - 24.0,
                Some(4),
            );
            widgets::galley_centered(&painter, rect, galley);
        }
    }
    widgets::border(&painter, rect, theme::RADIUS_MEDIUM, 1.0, theme::BORDER);
    // Jellyfin's corner indicator: what is left to watch, or a check once a
    // series or season is done.
    if detail.unplayed_count > 0 {
        let text = detail.unplayed_count.to_string();
        let width = (widgets::text_width(ui, &text, theme::bold(13.0)) + 16.0).max(30.0);
        let badge = Rect::from_min_size(
            pos2(rect.right() - width - 10.0, rect.top() + 10.0),
            vec2(width, 30.0),
        );
        painter.rect_filled(badge, 15.0, theme::ACCENT);
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            text,
            theme::bold(13.0),
            theme::TEXT_BRIGHT,
        );
    } else if detail.watched && matches!(detail.kind, Some(DetailKind::Season | DetailKind::Series))
    {
        let badge = Rect::from_min_size(
            pos2(rect.right() - 40.0, rect.top() + 10.0),
            vec2(30.0, 30.0),
        );
        painter.rect_filled(badge, 15.0, theme::ACCENT);
        icons::paint_centered(
            &painter,
            Icon::Check,
            badge.center(),
            18.0,
            theme::TEXT_BRIGHT,
        );
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
        assert_eq!(
            detail_metadata_line(&series),
            "2019 – Present • TV-MA • 50m"
        );

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
