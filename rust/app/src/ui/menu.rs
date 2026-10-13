//! jellyfin-web's item menu (the "⋮" on cards, detail pages and list rows,
//! and a card's right-click), the dialogs it opens, and the toast that
//! confirms what it did. Drawn above whichever screen is showing.

use egui::{Color32, Id, Rect, Sense, TextEdit, Ui, pos2, vec2};

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
            let id = Id::new("add-to-dialog");
            let select_id = id.with("select");
            let has_select = !item_ids.is_empty();
            if has_select {
                dialogs::autofocus(&ctx, id, select_id, true);
            }
            let options: Vec<String> = std::iter::once("New…".to_owned())
                .chain(containers.iter().flatten().map(|container| container.name.clone()))
                .collect();
            let (submit, header_close) = fixed_dialog(&ctx, id, title, |ui, body| {
                // jellyfin-web's centred 57em column inside 2em of padding.
                let left = body.center().x - 57.0 * theme::JF_FONT / 2.0 + 2.0 * theme::JF_FONT;
                let width = 57.0 * theme::JF_FONT - 4.0 * theme::JF_FONT;
                let line = 1.35 * theme::JF_FONT;
                let mut y = body.top() + 2.0 * theme::JF_FONT + 0.5 * theme::JF_FONT;
                let mut submit = false;
                if has_select {
                    let kind = if *collection { "Collection" } else { "Playlist" };
                    let field = Rect::from_min_size(pos2(left, y + line + 3.72), vec2(width, 40.34));
                    let response = ui.interact(field, select_id, Sense::click());
                    widgets::describe_button(&response, true, kind);
                    let open_key = id.with("sheet-open");
                    let sheet_open = ctx.data(|data| data.get_temp::<bool>(open_key)).unwrap_or(false);
                    let focused = response.has_focus() || sheet_open;
                    form_label(ui, left, y, kind, focused);
                    let shown = selected.map_or(0, |index| index + 1);
                    super::settings::paint_select(ui, field, &options[shown.min(options.len() - 1)], focused);
                    widgets::pointer_on_hover(&response);
                    let pressed = response.clicked()
                        || (response.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)));
                    if pressed {
                        ctx.data_mut(|data| data.insert_temp(open_key, true));
                    }
                    if ctx.data(|data| data.get_temp::<bool>(open_key)).unwrap_or(false) {
                        let items = options
                            .iter()
                            .enumerate()
                            .map(|(index, label)| SheetItem { label, icon: None, checked: index == shown })
                            .collect::<Vec<_>>();
                        let (picked, closed) = dialogs::action_sheet(&ctx, id.with("sheet"), field, &items, Some(shown));
                        if let Some(index) = picked {
                            *selected = index.checked_sub(1);
                        }
                        if closed {
                            ctx.data_mut(|data| data.remove::<bool>(open_key));
                        }
                    }
                    y = field.bottom() + 1.8 * theme::JF_FONT;
                }
                if selected.is_none() {
                    let input_id = id.with("name");
                    let focused = ctx.memory(|memory| memory.has_focus(input_id));
                    form_label(ui, left, y, "Name", focused);
                    let field = Rect::from_min_size(pos2(left, y + line + 3.72), vec2(width, 39.09));
                    let radius = 0.2 * 1.1 * theme::JF_FONT;
                    ui.painter().rect_filled(field, radius, Color32::from_gray(41));
                    let response = ui.put(
                        field.shrink(2.0),
                        TextEdit::singleline(name)
                            .id(input_id)
                            .font(theme::regular(1.1 * theme::JF_FONT))
                            .text_color(theme::JF_TEXT)
                            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 6)))
                            .vertical_align(egui::Align::Center)
                            .desired_width(f32::INFINITY),
                    );
                    if response.has_focus() {
                        widgets::border(ui.painter(), field, radius, 2.0, theme::JF_ACCENT);
                    }
                    if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    y = field.bottom() + 1.8 * theme::JF_FONT;
                    if !*collection {
                        let row = Rect::from_min_size(pos2(left, y), vec2(width, 34.95));
                        let response = ui.interact(row, id.with("public"), Sense::click());
                        widgets::describe_button(&response, true, "Allow public access");
                        super::settings::paint_checkbox(
                            ui,
                            Rect::from_min_size(pos2(left, y + 3.0), vec2(27.22, 27.22)),
                            *public,
                        );
                        text_line(ui, left + 2.4 * theme::JF_FONT, y + 7.43, "Allow public access", theme::JF_TEXT);
                        text_line(
                            ui,
                            left + 2.232,
                            y + 34.95,
                            "Allow this playlist to be viewed by any logged in user.",
                            theme::JF_TEXT_SECONDARY,
                        );
                        widgets::pointer_on_hover(&response);
                        if response.clicked() {
                            *public = !*public;
                        }
                    }
                }
                // The footer: the submit button, 12em, centred.
                let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - 91.48), body.right_bottom());
                ui.painter().rect_filled(footer, 0.0, theme::JF_HEADER);
                let ready = !*busy
                    && (item_ids.is_empty() || containers.is_some())
                    && (selected.is_some() || !name.trim().is_empty());
                let label = if item_ids.is_empty() { "Create" } else { "Add" };
                let button = Rect::from_min_size(
                    pos2(footer.center().x - 6.0 * theme::JF_FONT, footer.top() + 22.31),
                    vec2(12.0 * theme::JF_FONT, 46.86),
                );
                if submit_button(ui, button, label, ready, id.with("submit")) {
                    submit = true;
                }
                submit
            });
            if header_close {
                dialogs::autofocus(&ctx, id, Id::NULL, false);
            }
            let close = false;
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
        ItemDialog::MediaInfo { blocks, .. } => {
            let id = Id::new("media-info");
            let (copied, close) = fixed_dialog(&ctx, id, "Media Info", |ui, body| match blocks {
                None => {
                    ui.put(
                        Rect::from_center_size(body.center(), vec2(32.0, 32.0)),
                        egui::Spinner::new().size(32.0).color(theme::JF_ACCENT),
                    );
                    None
                }
                Some(blocks) => {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
                    egui::ScrollArea::vertical()
                        .id_salt(id.with("scroll"))
                        .auto_shrink([false, false])
                        .show(&mut child, |ui| draw_media_info(ui, body, blocks, id))
                        .inner
                }
            });
            if let Some(text) = copied {
                ctx.copy_text(text);
                actions.push(UiAction::Toast("Copied".to_owned()));
            }
            close
        }
    };
    if close {
        *dialog = None;
    }
}

// jellyfin-web's media info metrics, at 1em = 14.88 px.
/// An `h2` with its copy button: the button sets the line's height.
const INFO_HEADING: f32 = 47.13;
const INFO_HEADING_MARGIN: f32 = 13.883;
/// From an `h2`'s top to its text baseline, beside the button.
const INFO_HEADING_BASELINE: f32 = 28.67;
const INFO_ROW: f32 = 1.35 * theme::JF_FONT;
const INFO_STREAM_GAP: f32 = 3.0 * theme::JF_FONT;

/// Lays out and draws the media info blocks in the dialog's scroll area:
/// each source spans the 85% column, its streams follow side by side and
/// wrap, as inline blocks do. Returns a block's text when its copy button
/// was pressed.
fn draw_media_info(ui: &mut Ui, body: Rect, blocks: &[InfoBlock], id: Id) -> Option<String> {
    let font = theme::regular(theme::JF_FONT);
    let bold = theme::bold(theme::JF_FONT);
    let heading_font = theme::regular(1.5 * theme::JF_FONT);
    let width_of = |text: &str, font: &egui::FontId| {
        ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x
    };
    let row_width = |(label, value): &(String, String)| width_of(label, &bold) + theme::JF_FONT + width_of(value, &font);
    let inner = body.width() - 2.0 * theme::JF_FONT;
    let column = inner * 0.85;
    let left = body.left() + theme::JF_FONT + (inner - column) / 2.0;
    let sources = blocks.iter().filter(|block| !block.stream).count();
    // First pass: where each block goes, relative to the content's top.
    let mut placed = Vec::with_capacity(blocks.len());
    let mut y = theme::JF_FONT + if sources > 1 { INFO_ROW } else { 0.0 };
    let (mut x, mut line_top, mut line_height) = (left, y, 0.0f32);
    let mut separator = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let heading = INFO_HEADING_MARGIN + INFO_HEADING + INFO_HEADING_MARGIN;
        let height = heading + block.rows.len() as f32 * INFO_ROW;
        if block.stream {
            let width = block
                .rows
                .iter()
                .map(row_width)
                .fold(width_of(&block.heading, &heading_font) + 2.0 * 6.47 + INFO_HEADING, f32::max)
                .min(column);
            if x > left && x + width > left + column {
                x = left;
                line_top += line_height;
                line_height = 0.0;
            }
            placed.push((index, pos2(x, line_top), width));
            x += width + INFO_STREAM_GAP;
            line_height = line_height.max(height);
        } else {
            if index > 0 {
                // Sources are parted by a #444 rule with 1em around it.
                y = line_top + line_height + theme::JF_FONT;
                separator.push(y);
                y += 1.0 + theme::JF_FONT;
            }
            placed.push((index, pos2(left, y), column));
            y += height;
            (x, line_top, line_height) = (left, y, 0.0);
        }
    }
    let total = line_top + line_height + theme::JF_FONT;
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), total), Sense::hover());
    let top = area.top();
    let painter = ui.painter().clone();
    for rule in separator {
        painter.rect_filled(
            Rect::from_min_size(pos2(left, top + rule), vec2(column, 1.0)),
            0.0,
            Color32::from_gray(0x44),
        );
    }
    let mut copied = None;
    for (index, at, width) in placed {
        let block = &blocks[index];
        let heading_top = top + at.y + INFO_HEADING_MARGIN;
        if !ui.is_rect_visible(Rect::from_min_size(pos2(at.x, top + at.y), vec2(width, 75.0 + block.rows.len() as f32 * INFO_ROW))) {
            continue;
        }
        let galley = painter.layout_no_wrap(block.heading.clone(), heading_font.clone(), theme::JF_TEXT);
        let text_width = galley.size().x;
        widgets::galley_on_baseline(&painter, at.x, heading_top + INFO_HEADING_BASELINE, galley);
        let button = Rect::from_min_size(
            pos2(at.x + text_width + 6.47, heading_top),
            vec2(INFO_HEADING, INFO_HEADING),
        );
        if super::jf::round_icon_button(
            ui,
            button,
            Icon::ContentCopy,
            theme::JF_TEXT,
            1.5 * theme::JF_FONT,
            "Copy",
            true,
            id.with(("copy", index)),
        )
        .clicked()
        {
            let mut text = block.heading.clone();
            for (label, value) in &block.rows {
                text.push_str(&format!("\n{label}: {value}"));
            }
            copied = Some(text);
        }
        let mut row_top = heading_top + INFO_HEADING + INFO_HEADING_MARGIN;
        let clip = painter.with_clip_rect(Rect::from_min_size(pos2(at.x, top), vec2(width, total)));
        for (label, value) in &block.rows {
            let baseline = super::jf::baseline(row_top, theme::JF_FONT, INFO_ROW);
            let label_galley = clip.layout_no_wrap(label.clone(), bold.clone(), theme::JF_TEXT);
            let label_width = label_galley.size().x;
            widgets::galley_on_baseline(&clip, at.x, baseline, label_galley);
            let value_galley = clip.layout_no_wrap(value.clone(), font.clone(), theme::JF_TEXT);
            widgets::galley_on_baseline(&clip, at.x + label_width + theme::JF_FONT, baseline, value_galley);
            row_top += INFO_ROW;
        }
    }
    copied
}

/// jellyfin-web's fixed dialog (`dialog-fixed`): 60% of the page wide, 80%
/// of the window tall, 10% from its top, square, with the #202020 header
/// (back arrow and title). `content` draws below the header. Returns its
/// result and whether the dialog closed (back, Escape or the dim).
fn fixed_dialog<R>(
    ctx: &egui::Context,
    id: Id,
    title: &str,
    content: impl FnOnce(&mut Ui, Rect) -> R,
) -> (R, bool) {
    let screen = ctx.content_rect();
    let page_width = screen.width() - super::scrollbar::reserved(ctx);
    let size = vec2(page_width * 0.6, screen.height() * 0.8);
    let ((inner, back), close) = dialogs::panel(ctx, id, size, Some(screen.height() * 0.1), 0.0, |ui, rect| {
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 71.13));
        ui.painter().rect_filled(header, 0.0, theme::JF_HEADER);
        let back = super::jf::round_icon_button(
            ui,
            Rect::from_min_size(pos2(rect.left() + 11.75, rect.top() + 14.88), vec2(41.36, 41.36)),
            Icon::ArrowBack,
            theme::JF_TEXT,
            24.84,
            "Back",
            true,
            id.with("back"),
        )
        .clicked();
        let size = 1.17 * theme::JF_FONT;
        let galley = ui.painter().layout_no_wrap(title.to_owned(), theme::regular(size), theme::JF_TEXT);
        widgets::galley_on_baseline(
            ui.painter(),
            rect.left() + 61.77,
            dialogs::layer_baseline(ui, rect.top() + 23.81, size, 23.48),
            galley,
        );
        let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
        (content(ui, body), back)
    });
    (inner, close || back)
}

/// A form field's label: 70% white, accent while its field is focused.
fn form_label(ui: &Ui, left: f32, top: f32, text: &str, focused: bool) {
    let color = if focused { theme::JF_ACCENT } else { theme::JF_TEXT_LABEL };
    text_line(ui, left, top, text, color);
}

/// One line of 1em text on a 1.35em line box starting at `top`.
fn text_line(ui: &Ui, left: f32, top: f32, text: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::regular(theme::JF_FONT), color);
    widgets::galley_on_baseline(
        ui.painter(),
        left,
        dialogs::layer_baseline(ui, top, theme::JF_FONT, 1.35 * theme::JF_FONT),
        galley,
    );
}

/// jellyfin-web's raised submit button in `rect`. Returns whether clicked.
fn submit_button(ui: &Ui, rect: Rect, label: &str, enabled: bool, id: Id) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, id, sense);
    widgets::describe_button(&response, enabled, label);
    let fill = if !enabled {
        theme::with_alpha(theme::JF_ACCENT, 0.5)
    } else if response.has_focus() {
        Color32::from_rgb(0x0c, 0xb0, 0xe8)
    } else {
        theme::JF_ACCENT
    };
    ui.painter().rect_filled(rect, 0.2 * theme::JF_FONT, fill);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::bold(theme::JF_FONT), Color32::WHITE);
    widgets::galley_on_baseline(
        ui.painter(),
        rect.center().x - galley.size().x / 2.0,
        dialogs::layer_baseline(ui, rect.top() + 13.392, theme::JF_FONT, 1.35 * theme::JF_FONT),
        galley,
    );
    if enabled {
        widgets::pointer_on_hover(&response);
    }
    enabled && response.clicked()
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
