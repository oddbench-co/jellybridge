//! jellyfin-web's item menu (the "⋮" on cards, detail pages and list rows,
//! and a card's right-click), the dialogs it opens, and the toast that
//! confirms what it did. Drawn above whichever screen is showing.

use egui::{Align2, Color32, Id, Rect, Sense, TextEdit, Ui, pos2, vec2};

use super::app::UiAction;
use super::dialogs::{self, SheetItem};
use super::icons::Icon;
use super::theme;
use super::widgets;
use crate::api::{Container, InfoBlock};

/// What the menu was opened on.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuRequest {
    pub item_id: String,
    pub name: String,
    /// Jellyfin's type name ("Movie", "Series", …).
    pub kind: String,
    pub series_id: Option<String>,
    pub season_id: Option<String>,
    pub anchor: Rect,
    pub origin: MenuOrigin,
}

/// Where the menu was opened, which decides some of its entries.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuOrigin {
    /// A card in a grid or shelf; `queue` holds the playable ids from this
    /// card on, for "Play all from here".
    Card { queue: Vec<String> },
    /// The detail page's own "⋮".
    Detail,
    /// A playlist's entry row.
    PlaylistEntry {
        playlist_id: String,
        entry_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    Play,
    PlayAllFromHere,
    Shuffle,
    AddToCollection,
    AddToPlaylist,
    RemoveFromPlaylist,
    Download,
    CopyStreamUrl,
    MediaInfo,
    GoToSeason,
    GoToSeries,
}

impl MenuCommand {
    pub fn label(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::PlayAllFromHere => "Play all from here",
            Self::Shuffle => "Shuffle",
            Self::AddToCollection => "Add to collection",
            Self::AddToPlaylist => "Add to playlist",
            Self::RemoveFromPlaylist => "Remove from playlist",
            Self::Download => "Download",
            Self::CopyStreamUrl => "Copy Stream URL",
            Self::MediaInfo => "Media Info",
            Self::GoToSeason => "Go to Season",
            Self::GoToSeries => "Go to Series",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Play | Self::PlayAllFromHere => Icon::Play,
            Self::Shuffle => Icon::Shuffle,
            Self::AddToCollection | Self::AddToPlaylist => Icon::PlaylistAdd,
            Self::RemoveFromPlaylist => Icon::Remove,
            Self::Download => Icon::Download,
            Self::CopyStreamUrl => Icon::ContentCopy,
            Self::MediaInfo => Icon::Info,
            Self::GoToSeason | Self::GoToSeries => Icon::Tv,
        }
    }
}

/// jellyfin-web's menu entries for a viewer (no admin entries), in its
/// order.
pub fn menu_commands(request: &MenuRequest) -> Vec<MenuCommand> {
    use MenuCommand::*;
    let kind = request.kind.as_str();
    let video = matches!(kind, "Movie" | "Episode" | "Video" | "MusicVideo");
    let container = matches!(kind, "Series" | "Season" | "BoxSet" | "Playlist" | "Folder");
    let mut commands = Vec::new();
    if matches!(
        request.origin,
        MenuOrigin::Card { .. } | MenuOrigin::PlaylistEntry { .. }
    ) && (video || container)
    {
        commands.push(Play);
    }
    if let MenuOrigin::Card { queue } = &request.origin
        && video
        && queue.len() > 1
    {
        commands.push(PlayAllFromHere);
    }
    if container && !matches!(request.origin, MenuOrigin::Detail) {
        commands.push(Shuffle);
    }
    if matches!(
        kind,
        "Movie" | "Series" | "Video" | "BoxSet" | "Episode" | "Season"
    ) && kind != "BoxSet"
    {
        commands.push(AddToCollection);
    }
    if video || matches!(kind, "Series" | "Season" | "BoxSet") {
        commands.push(AddToPlaylist);
    }
    if matches!(request.origin, MenuOrigin::PlaylistEntry { .. }) {
        commands.push(RemoveFromPlaylist);
    }
    if video {
        commands.extend([Download, CopyStreamUrl, MediaInfo]);
    }
    if kind == "Episode" && !matches!(request.origin, MenuOrigin::Detail) {
        if request.season_id.is_some() {
            commands.push(GoToSeason);
        }
        if request.series_id.is_some() {
            commands.push(GoToSeries);
        }
    }
    commands
}

/// An open menu.
#[derive(Clone, Debug)]
pub struct OpenMenu {
    pub request: MenuRequest,
    pub commands: Vec<MenuCommand>,
}

/// The dialogs the menu opens.
#[derive(Clone, Debug)]
pub enum ItemDialog {
    /// Add to a playlist (`collection` false) or collection. An empty
    /// `item_ids` is jellyfin-web's "New Playlist"/"New Collection".
    AddTo {
        collection: bool,
        item_ids: Vec<String>,
        /// `None` while the list loads.
        containers: Option<Vec<Container>>,
        /// `None` is "New…".
        selected: Option<usize>,
        name: String,
        public: bool,
        busy: bool,
    },
    MediaInfo {
        title: String,
        /// `None` while it loads.
        blocks: Option<Vec<InfoBlock>>,
    },
}

/// A short confirmation at the bottom of the window.
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub until: f64,
}

pub const TOAST_SECONDS: f64 = 3.5;

/// Draws the open menu, if any.
pub fn draw_menu(ui: &mut Ui, menu: &mut Option<OpenMenu>, actions: &mut Vec<UiAction>) {
    let Some(open) = menu.as_ref() else { return };
    let items = open
        .commands
        .iter()
        .map(|command| SheetItem {
            label: command.label(),
            icon: Some(command.icon()),
            checked: false,
        })
        .collect::<Vec<_>>();
    let (picked, close) = dialogs::action_sheet(
        ui.ctx(),
        Id::new("item-menu"),
        open.request.anchor,
        &items,
        None,
    );
    if let Some(index) = picked {
        actions.push(UiAction::MenuCommand(
            open.commands[index],
            Box::new(open.request.clone()),
        ));
    }
    if close {
        *menu = None;
    }
}

/// Draws the open dialog, if any. Edits stay in the dialog; submitting
/// becomes an action.
pub fn draw_dialog(ui: &mut Ui, dialog: &mut Option<ItemDialog>, actions: &mut Vec<UiAction>) {
    let Some(open) = dialog.as_mut() else { return };
    let ctx = ui.ctx().clone();
    let close = match open {
        ItemDialog::AddTo {
            collection,
            item_ids,
            containers,
            selected,
            name,
            public,
            busy,
        } => {
            let title = match (*collection, item_ids.is_empty()) {
                (false, false) => "Add to Playlist",
                (true, false) => "Add to Collection",
                (false, true) => "New Playlist",
                (true, true) => "New Collection",
            };
            let (submit, close) =
                dialogs::modal(&ctx, Id::new("add-to-dialog"), 560.0, None, |ui| {
                    let close = dialog_header(ui, title);
                    let mut submit = false;
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(32, 20))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
                            let kind = if *collection {
                                "Collection"
                            } else {
                                "Playlist"
                            };
                            if !item_ids.is_empty() {
                                field_label(ui, kind);
                                match containers {
                                    None => {
                                        ui.add(egui::Spinner::new().color(theme::JF_ACCENT));
                                    }
                                    Some(list) => {
                                        let current = selected
                                            .and_then(|index| list.get(index))
                                            .map_or("New…", |container| container.name.as_str());
                                        let width = ui.available_width();
                                        egui::ComboBox::from_id_salt("add-to-select")
                                            .width(width)
                                            .selected_text(
                                                egui::RichText::new(current)
                                                    .font(theme::regular(theme::JF_FONT))
                                                    .color(theme::JF_TEXT),
                                            )
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(selected, None, "New…");
                                                for (index, container) in list.iter().enumerate() {
                                                    ui.selectable_value(
                                                        selected,
                                                        Some(index),
                                                        &container.name,
                                                    );
                                                }
                                            });
                                    }
                                }
                                ui.add_space(12.0);
                            }
                            if selected.is_none() {
                                field_label(ui, "Name");
                                let response = ui.add(
                                    TextEdit::singleline(name)
                                        .font(theme::regular(16.37))
                                        .desired_width(f32::INFINITY)
                                        .margin(egui::Margin::symmetric(8, 9)),
                                );
                                if response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                {
                                    submit = true;
                                }
                                if !*collection {
                                    ui.add_space(12.0);
                                    if dialogs::checkbox(
                                        ui,
                                        "Allow public access",
                                        *public,
                                        Id::new("playlist-public"),
                                    ) {
                                        *public = !*public;
                                    }
                                    ui.label(
                                    egui::RichText::new(
                                        "Allow this playlist to be viewed by any logged in user.",
                                    )
                                    .font(theme::regular(theme::JF_FONT_SECONDARY))
                                    .color(theme::JF_TEXT_SECONDARY),
                                );
                                }
                            }
                            ui.add_space(20.0);
                            let ready = !*busy
                                && (item_ids.is_empty() || containers.is_some())
                                && (selected.is_some() || !name.trim().is_empty());
                            let label = if item_ids.is_empty() { "Create" } else { "Add" };
                            if accent_button(ui, label, ready, Id::new("add-to-submit")) {
                                submit = true;
                            }
                        });
                    (submit, close)
                });
            let (submit, header_close) = submit;
            let ready = selected.is_some() || !name.trim().is_empty();
            if submit && ready && !*busy {
                *busy = true;
                let target = selected
                    .and_then(|index| containers.as_ref()?.get(index))
                    .map(|container| container.id.clone());
                actions.push(UiAction::SubmitAddTo {
                    collection: *collection,
                    target,
                    name: name.trim().to_owned(),
                    public: *public,
                    item_ids: item_ids.clone(),
                });
            }
            close || header_close
        }
        ItemDialog::MediaInfo { title, blocks } => {
            let (header_close, close) = dialogs::modal(
                &ctx,
                Id::new("media-info"),
                640.0,
                Some(ctx.content_rect().height() * 0.8),
                |ui| {
                    let close = dialog_header(ui, title);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(32, 16))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                            match blocks {
                                None => {
                                    ui.add(egui::Spinner::new().color(theme::JF_ACCENT));
                                }
                                Some(blocks) => {
                                    for block in blocks.iter() {
                                        ui.add_space(10.0);
                                        ui.label(
                                            egui::RichText::new(&block.heading)
                                                .font(theme::bold(theme::JF_FONT_SUBTITLE))
                                                .color(theme::JF_TEXT),
                                        );
                                        for (label, value) in &block.rows {
                                            info_row(ui, label, value);
                                        }
                                    }
                                }
                            }
                        });
                    close
                },
            );
            close || header_close
        }
    };
    if close {
        *dialog = None;
    }
}

/// jellyfin-web's dialog header: a back arrow and the title. Returns
/// whether the arrow was clicked.
fn dialog_header(ui: &mut Ui, title: &str) -> bool {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 57.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::JF_HEADER);
    let back = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 8.0), vec2(41.0, 41.0));
    let clicked = super::jf::round_icon_button(
        ui,
        back,
        Icon::ArrowBack,
        Color32::WHITE,
        24.8,
        "Close",
        true,
        Id::new(("dialog-back", title)),
    )
    .clicked();
    ui.painter().text(
        pos2(rect.left() + 59.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::regular(theme::JF_FONT_SUBTITLE),
        theme::JF_TEXT,
    );
    clicked
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::regular(theme::JF_FONT))
            .color(theme::JF_TEXT_LABEL),
    );
}

fn info_row(ui: &mut Ui, label: &str, value: &str) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    job.append(
        &format!("{label}: "),
        0.0,
        egui::TextFormat::simple(theme::bold(theme::JF_FONT), theme::JF_TEXT),
    );
    job.append(
        value,
        0.0,
        egui::TextFormat::simple(theme::regular(theme::JF_FONT), theme::JF_TEXT),
    );
    ui.label(job);
}

/// jellyfin-web's accent submit button: full width, 47 px, semibold.
fn accent_button(ui: &mut Ui, label: &str, enabled: bool, id: Id) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 47.0), Sense::hover());
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    widgets::describe_button(&response, enabled, label);
    let fill = if !enabled {
        theme::JF_ACCENT.gamma_multiply(0.4)
    } else if response.hovered() {
        theme::ACCENT_BRIGHT
    } else {
        theme::JF_ACCENT
    };
    ui.painter().rect_filled(rect, theme::JF_RADIUS, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::bold(theme::JF_FONT),
        Color32::WHITE,
    );
    if enabled {
        widgets::pointer_on_hover(&response);
    }
    response.clicked()
}

/// Draws the toast until it expires, and wakes once to take it down.
pub fn draw_toast(ui: &mut Ui, toast: &mut Option<Toast>) {
    let Some(shown) = toast.as_ref() else { return };
    let now = ui.input(|input| input.time);
    if now >= shown.until {
        *toast = None;
        return;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(shown.until - now));
    let screen = ui.ctx().content_rect();
    let galley = widgets::elided(
        ui,
        &shown.text,
        theme::regular(theme::JF_FONT),
        Color32::WHITE,
        screen.width() - 80.0,
    );
    let size = galley.size() + vec2(48.0, 28.0);
    let rect = Rect::from_center_size(
        pos2(screen.center().x, screen.bottom() - 40.0 - size.y / 2.0),
        size,
    );
    egui::Area::new(Id::new("toast"))
        .order(egui::Order::Tooltip)
        .fixed_pos(rect.min)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            let painter = ui.painter();
            painter.rect_filled(rect, theme::JF_RADIUS, theme::JF_RAISED);
            widgets::galley_centered(painter, rect, galley);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: &str, origin: MenuOrigin) -> MenuRequest {
        MenuRequest {
            item_id: "id".into(),
            name: "Name".into(),
            kind: kind.into(),
            series_id: Some("series".into()),
            season_id: Some("season".into()),
            anchor: Rect::NOTHING,
            origin,
        }
    }

    #[test]
    fn a_movie_card_offers_jellyfin_viewer_entries() {
        let commands = menu_commands(&request(
            "Movie",
            MenuOrigin::Card {
                queue: vec!["id".into(), "next".into()],
            },
        ));
        assert_eq!(
            commands,
            vec![
                MenuCommand::Play,
                MenuCommand::PlayAllFromHere,
                MenuCommand::AddToCollection,
                MenuCommand::AddToPlaylist,
                MenuCommand::Download,
                MenuCommand::CopyStreamUrl,
                MenuCommand::MediaInfo,
            ]
        );
    }

    #[test]
    fn episodes_go_to_their_season_and_series_from_cards_only() {
        let card = menu_commands(&request("Episode", MenuOrigin::Card { queue: Vec::new() }));
        assert!(card.contains(&MenuCommand::GoToSeries));
        assert!(card.contains(&MenuCommand::GoToSeason));
        let detail = menu_commands(&request("Episode", MenuOrigin::Detail));
        assert!(!detail.contains(&MenuCommand::GoToSeries));
        assert!(!detail.contains(&MenuCommand::Play));
    }

    #[test]
    fn a_series_card_shuffles_and_a_playlist_entry_can_be_removed() {
        let series = menu_commands(&request("Series", MenuOrigin::Card { queue: Vec::new() }));
        assert!(series.contains(&MenuCommand::Shuffle));
        assert!(!series.contains(&MenuCommand::Download));
        let entry = menu_commands(&request(
            "Episode",
            MenuOrigin::PlaylistEntry {
                playlist_id: "p".into(),
                entry_id: "e".into(),
            },
        ));
        assert!(entry.contains(&MenuCommand::RemoveFromPlaylist));
    }
}
