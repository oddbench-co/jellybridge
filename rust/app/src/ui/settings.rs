//! The settings pages, drawn as jellyfin-web draws its user preferences: a
//! 54em column of headings, checkboxes, selects and inputs with a Save
//! button, and the menu that lists the pages.

use std::collections::HashMap;

use egui::{Color32, Id, Rect, ScrollArea, Sense, TextEdit, Ui, pos2, vec2};

use super::app::UiAction;
use super::icons::{self, Icon};
use super::images::{Art, ArtSource, ImageCache};
use super::jf::{self, EM};
use super::{dialogs, scrollbar, theme, widgets};
use crate::features::settings::{self, Field, FieldValue, Key, SettingsData, SettingsPage};

/// A settings page on screen, with the user's unsaved edits.
pub struct SettingsVm {
    pub page: SettingsPage,
    pub data: SettingsData,
    /// Only the fields the user changed.
    pub values: HashMap<Key, FieldValue>,
    /// The select whose option sheet is open.
    open_select: Option<Key>,
    /// The Profile and Quick Connect boxes; never stored.
    current_password: String,
    new_password: String,
    confirm_password: String,
    code: String,
    /// The "Delete Image" confirmation is up.
    confirm_delete: bool,
}

impl SettingsVm {
    pub fn new(page: SettingsPage, data: SettingsData) -> Self {
        Self {
            page,
            data,
            values: HashMap::new(),
            open_select: None,
            current_password: String::new(),
            new_password: String::new(),
            confirm_password: String::new(),
            code: String::new(),
            confirm_delete: false,
        }
    }

    fn value(&self, key: &Key) -> FieldValue {
        self.values
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.data.value(key))
    }

    /// The library order being edited: `OrderedViews`, then the libraries it
    /// does not name.
    fn order(&self) -> Vec<String> {
        let stored = match self.value(&Key::Config("OrderedViews")) {
            FieldValue::List(ids) => ids,
            _ => Vec::new(),
        };
        self.data
            .ordered_libraries(&stored)
            .into_iter()
            .map(|library| library.id)
            .collect()
    }
}

// jellyfin-web's form metrics.
const COLUMN: f32 = 54.0 * EM;
const H2: f32 = 1.5 * EM;
const H3: f32 = 1.17 * EM;
const INPUT_FONT: f32 = 1.1 * EM;
const SELECT_HEIGHT: f32 = 40.34;
const INPUT_HEIGHT: f32 = 39.09;
const CHECK_ROW: f32 = 34.95;
const CHECK_BOX: f32 = 27.22;
const SAVE_HEIGHT: f32 = 46.86;
const ORDER_ROW: f32 = 52.8;
const MENU_ROW: f32 = 54.55;

fn lu(value: f32) -> f32 {
    jf::layout_unit(value)
}

pub fn draw(
    ui: &mut Ui,
    images: &mut ImageCache,
    vm: &mut SettingsVm,
    actions: &mut Vec<UiAction>,
) {
    let body = ui.max_rect();
    let window_top = ui.ctx().content_rect().top();
    let column_width = COLUMN.min(body.width() - 2.0 * jf::page_padding(body.width()));
    let column_left = body.center().x - column_width / 2.0;
    let output = ScrollArea::vertical()
        .id_salt(("settings", format!("{:?}", vm.page)))
        .scroll_bar_visibility(scrollbar::HIDDEN)
        .auto_shrink(false)
        .show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let top = ui.max_rect().top();
            // Page coordinates count from the window top (the document runs
            // under the header): page y 0 sits this far above the content.
            let origin = window_top - viewport.top();
            let mut form = Form {
                left: column_left,
                width: column_width,
                y: origin + lu(7.0 * EM),
                pending: 0.0,
            };
            match vm.page {
                SettingsPage::Menu => draw_menu(ui, &mut form, vm, actions),
                SettingsPage::Profile => draw_profile(ui, &mut form, images, vm, actions),
                SettingsPage::QuickConnect => {
                    // Quick Connect sits at the page's left padding.
                    form.left = body.left() + jf::page_padding(body.width());
                    draw_quick_connect(ui, &mut form, vm, actions);
                }
                _ => draw_form(ui, &mut form, vm, actions),
            }
            let height = form.y + lu(5.0 * EM) - top;
            ui.allocate_exact_size(vec2(body.width(), height.max(0.0)), Sense::hover());
        });
    scrollbar::draw(ui, &output);
    draw_select_sheet(ui, vm);
}

/// A menu row: its icon, label and what it opens.
type MenuRow<'a> = (Icon, &'a str, UiAction);

/// The layout cursor, with CSS's collapsing vertical margins.
struct Form {
    left: f32,
    width: f32,
    y: f32,
    /// The bottom margin of the last block, collapsed with the next top one.
    pending: f32,
}

impl Form {
    /// Starts a block with top margin `margin`; returns its top.
    fn block(&mut self, margin: f32) -> f32 {
        self.y += self.pending.max(margin);
        self.pending = 0.0;
        self.y
    }

    fn end(&mut self, height: f32, margin: f32) {
        self.y += height;
        self.pending = margin;
    }
}

#[allow(clippy::too_many_arguments)]
fn text(
    ui: &Ui,
    left: f32,
    top: f32,
    size: f32,
    line: f32,
    text: &str,
    color: Color32,
    bold: bool,
) {
    let font = if bold {
        theme::bold(size)
    } else {
        theme::regular(size)
    };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    widgets::galley_on_baseline(ui.painter(), left, jf::baseline(top, size, line), galley);
}

/// Wrapped text at `size` on 1.35 lines; returns its height.
fn wrapped(ui: &Ui, form: &Form, top: f32, inset: f32, body: &str, color: Color32) -> f32 {
    jf::paragraph(
        ui,
        body,
        theme::regular(EM),
        1.35 * EM,
        top,
        form.left + inset,
        form.width - inset,
        false,
        color,
    )
}

fn heading(ui: &Ui, form: &mut Form, title: &str, size: f32, inset: f32) {
    let margin = lu(if size == H2 { 0.83 * H2 } else { H3 });
    let line = lu(1.35 * size);
    let top = form.block(margin);
    text(
        ui,
        form.left + inset,
        top,
        size,
        line,
        title,
        jf::TEXT_80,
        false,
    );
    form.end(line, margin);
}

fn draw_menu(ui: &mut Ui, form: &mut Form, vm: &SettingsVm, actions: &mut Vec<UiAction>) {
    // The menu sits 1em lower and 5px further in than the forms.
    form.left += 5.0;
    form.y += lu(EM);
    let sections: [(&str, &[MenuRow]); 2] = [
        (
            vm.data.user_name.as_str(),
            &[
                (
                    Icon::Person,
                    "Profile",
                    UiAction::OpenSettings(SettingsPage::Profile),
                ),
                (
                    Icon::PhonelinkLock,
                    "Quick Connect",
                    UiAction::OpenSettings(SettingsPage::QuickConnect),
                ),
                (
                    Icon::Tv,
                    "Display",
                    UiAction::OpenSettings(SettingsPage::Display),
                ),
                (
                    Icon::Home,
                    "Home",
                    UiAction::OpenSettings(SettingsPage::Home),
                ),
                (
                    Icon::PlayCircleFilled,
                    "Playback",
                    UiAction::OpenSettings(SettingsPage::Playback),
                ),
                (
                    Icon::ClosedCaption,
                    "Subtitles",
                    UiAction::OpenSettings(SettingsPage::Subtitles),
                ),
            ],
        ),
        ("User", &[(Icon::ExitToApp, "Sign Out", UiAction::Logout)]),
    ];
    for (title, rows) in sections {
        heading(ui, form, title, H2, lu(0.25 * H2));
        form.y += form.pending;
        form.pending = 0.0;
        for (index, (icon, label, action)) in rows.iter().enumerate() {
            let row = Rect::from_min_size(pos2(form.left, form.y), vec2(form.width, MENU_ROW));
            let response = ui.interact(
                row,
                Id::new(("settings-menu", title, index)),
                Sense::click(),
            );
            widgets::describe_button(&response, true, label);
            if response.hovered() {
                ui.painter().rect_filled(row, 0.0, Color32::from_gray(36));
            }
            // `.listItem-border`: a 1px #202020 rule under each row.
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(row.left(), row.bottom() - 1.0), row.max),
                0.0,
                Color32::from_gray(32),
            );
            icons::paint_centered(
                ui.painter(),
                *icon,
                pos2(row.left() + 7.44 + 21.28 / 2.0, row.center().y),
                21.28,
                jf::TEXT_80,
            );
            let line = lu(1.2 * EM);
            text(
                ui,
                row.left() + 45.18,
                row.top() + 16.36 + 1.49,
                EM,
                line,
                label,
                jf::TEXT_80,
                false,
            );
            widgets::pointer_on_hover(&response);
            if response.clicked() {
                actions.push(action.clone());
            }
            form.y += MENU_ROW;
        }
        // `.verticalSection`: 2.7em under each group.
        form.pending = lu(2.7 * EM);
    }
}

fn draw_form(ui: &mut Ui, form: &mut Form, vm: &mut SettingsVm, actions: &mut Vec<UiAction>) {
    let order = vm.order();
    let fields = settings::fields(vm.page, &vm.data, &order);
    for (index, field) in fields.iter().enumerate() {
        match field {
            Field::Heading(title) => heading(ui, form, title, H2, 0.0),
            Field::SubHeading(title) => heading(ui, form, title, H3, 0.0),
            Field::Paragraph(body) => {
                let top = form.block(lu(EM));
                let height = wrapped(ui, form, top, 0.0, body, jf::TEXT_80);
                form.end(height, lu(EM));
            }
            Field::SectionEnd => form.pending = form.pending.max(lu(2.7 * EM)),
            Field::Check { key, label, help } => {
                let top = form.block(0.0);
                let checked = matches!(vm.value(key), FieldValue::Bool(true));
                let row = Rect::from_min_size(pos2(form.left, top), vec2(form.width, CHECK_ROW));
                let response = ui.interact(row, Id::new(("settings-check", index)), Sense::click());
                widgets::describe_button(&response, true, label);
                let check =
                    Rect::from_min_size(pos2(form.left, top + 3.0), vec2(CHECK_BOX, CHECK_BOX));
                paint_checkbox(ui, check, checked);
                text(
                    ui,
                    form.left + lu(2.4 * EM),
                    top + 7.43,
                    EM,
                    lu(1.35 * EM),
                    label,
                    jf::TEXT_80,
                    false,
                );
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    vm.values.insert(key.clone(), FieldValue::Bool(!checked));
                }
                let mut height = CHECK_ROW;
                if let Some(help) = help {
                    height += wrapped(
                        ui,
                        form,
                        top + CHECK_ROW,
                        2.232,
                        help,
                        theme::JF_TEXT_SECONDARY,
                    );
                }
                form.end(height, lu(1.8 * EM));
            }
            Field::Select {
                key,
                label,
                options,
                help,
                option_help,
            } => {
                let top = form.block(0.0);
                let line = lu(1.35 * EM);
                text(ui, form.left, top, EM, line, label, jf::TEXT_80, false);
                let field = Rect::from_min_size(
                    pos2(form.left, top + line + lu(0.25 * EM)),
                    vec2(form.width, SELECT_HEIGHT),
                );
                let current = match vm.value(key) {
                    FieldValue::Text(value) => value,
                    FieldValue::Bool(value) => value.to_string(),
                    FieldValue::List(_) => String::new(),
                };
                let shown = options
                    .iter()
                    .find(|(value, _)| *value == current)
                    .map_or("", |(_, name)| name.as_str());
                let response =
                    ui.interact(field, Id::new(("settings-select", index)), Sense::click());
                widgets::describe_button(&response, true, label);
                paint_select(ui, field, shown, response.has_focus());
                widgets::pointer_on_hover(&response);
                if response.clicked() {
                    vm.open_select = Some(key.clone());
                }
                let mut height = field.bottom() - top;
                let chosen_help = option_help
                    .iter()
                    .find(|(value, _)| *value == current)
                    .map(|(_, help)| help);
                if let Some(help) = help.as_ref().or(chosen_help) {
                    height += lu(0.25 * EM)
                        + wrapped(
                            ui,
                            form,
                            field.bottom() + lu(0.25 * EM),
                            2.232,
                            help,
                            theme::JF_TEXT_SECONDARY,
                        );
                }
                form.end(height, lu(1.8 * EM));
            }
            Field::Number { key, label, help } => {
                let top = form.block(0.0);
                let line = lu(1.35 * EM);
                text(
                    ui,
                    form.left,
                    top,
                    EM,
                    line,
                    label,
                    theme::with_alpha(Color32::WHITE, 0.7),
                    false,
                );
                let field = Rect::from_min_size(
                    pos2(form.left, top + line + lu(0.25 * EM)),
                    vec2(form.width, INPUT_HEIGHT),
                );
                let mut current = match vm.value(key) {
                    FieldValue::Text(value) => value,
                    _ => String::new(),
                };
                let before = current.clone();
                let radius = 0.2 * INPUT_FONT;
                ui.painter()
                    .rect_filled(field, radius, Color32::from_gray(41));
                let response = ui.put(
                    field.shrink(2.0),
                    TextEdit::singleline(&mut current)
                        .id(Id::new(("settings-number", index)))
                        .font(theme::regular(INPUT_FONT))
                        .text_color(jf::TEXT_80)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 6)))
                        .vertical_align(egui::Align::Center)
                        .desired_width(f32::INFINITY),
                );
                if response.has_focus() {
                    widgets::border(ui.painter(), field, radius, 2.0, theme::JF_ACCENT);
                }
                if current != before && current.chars().all(|c| c.is_ascii_digit()) {
                    vm.values.insert(key.clone(), FieldValue::Text(current));
                }
                let mut height = field.bottom() - top;
                if let Some(help) = help {
                    height += lu(0.25 * EM)
                        + wrapped(
                            ui,
                            form,
                            field.bottom() + lu(0.25 * EM),
                            2.232,
                            help,
                            theme::JF_TEXT_SECONDARY,
                        );
                }
                form.end(height, lu(1.8 * EM));
            }
            Field::LibraryOrder => draw_order(ui, form, vm, &order),
            Field::SubtitlePreview => {
                let top = form.block(0.0);
                let height = draw_subtitle_preview(ui, form, vm, top);
                form.end(height, lu(EM));
            }
            Field::Slider {
                key,
                label,
                range,
                help,
            } => {
                let top = form.block(0.0);
                let line = lu(1.35 * EM);
                text(ui, form.left, top, EM, line, label, jf::TEXT_80, false);
                let value = match vm.value(key) {
                    FieldValue::Text(value) => value.parse::<i32>().unwrap_or(0),
                    _ => 0,
                };
                let track_top = top + line + lu(0.25 * EM);
                let area = Rect::from_min_size(pos2(form.left, track_top), vec2(form.width, 28.0));
                let response = ui.interact(
                    area,
                    Id::new(("settings-slider", index)),
                    Sense::click_and_drag(),
                );
                widgets::describe_button(&response, true, label);
                let span = (range.1 - range.0).max(1) as f32;
                let fraction = (value - range.0) as f32 / span;
                let rail = Rect::from_center_size(area.center(), vec2(area.width(), 3.0));
                ui.painter().rect_filled(rail, 1.5, Color32::from_gray(80));
                let filled = Rect::from_min_max(
                    rail.min,
                    pos2(rail.left() + rail.width() * fraction, rail.bottom()),
                );
                ui.painter().rect_filled(filled, 1.5, theme::JF_ACCENT);
                ui.painter().circle_filled(
                    pos2(filled.right(), rail.center().y),
                    7.5,
                    theme::JF_ACCENT,
                );
                widgets::pointer_on_hover(&response);
                if let Some(pointer) = response.interact_pointer_pos()
                    && (response.dragged() || response.clicked())
                {
                    let picked = ((pointer.x - rail.left()) / rail.width()).clamp(0.0, 1.0);
                    let new = range.0 + (picked * span).round() as i32;
                    if new != value {
                        vm.values
                            .insert(key.clone(), FieldValue::Text(new.to_string()));
                    }
                }
                let mut height = area.bottom() - top;
                if let Some(help) = help {
                    height += lu(0.25 * EM)
                        + wrapped(
                            ui,
                            form,
                            area.bottom() + lu(0.25 * EM),
                            2.232,
                            help,
                            theme::JF_TEXT_SECONDARY,
                        );
                }
                form.end(height, lu(1.8 * EM));
            }
        }
    }
    // The Save button.
    let top = form.block(lu(0.25 * EM));
    let button = Rect::from_min_size(pos2(form.left, top), vec2(form.width, SAVE_HEIGHT));
    let response = ui.interact(button, Id::new("settings-save"), Sense::click());
    widgets::describe_button(&response, true, "Save");
    let fill = if response.hovered() {
        theme::with_alpha(theme::JF_ACCENT, 0.85)
    } else {
        theme::JF_ACCENT
    };
    ui.painter().rect_filled(button, 0.2 * EM, fill);
    let galley = ui
        .painter()
        .layout_no_wrap("Save".to_owned(), theme::bold(EM), Color32::WHITE);
    let x = button.center().x - galley.size().x / 2.0;
    widgets::galley_on_baseline(
        ui.painter(),
        x,
        jf::baseline(button.top() + 13.392, EM, lu(1.35 * EM)),
        galley,
    );
    widgets::pointer_on_hover(&response);
    if response.clicked() {
        let save = settings::apply(&vm.data, &vm.values);
        actions.push(UiAction::SaveSettings(Box::new(save)));
    }
    form.end(SAVE_HEIGHT, lu(0.25 * EM));
}

/// A labelled text box (`emby-input`): label at 70%, then the box.
/// Returns the box's response.
fn input(
    ui: &mut Ui,
    form: &mut Form,
    label: &str,
    value: &mut String,
    password: bool,
    id: &str,
) -> egui::Response {
    let top = form.block(0.0);
    let line = lu(1.35 * EM);
    text(
        ui,
        form.left,
        top,
        EM,
        line,
        label,
        theme::with_alpha(Color32::WHITE, 0.7),
        false,
    );
    let field = Rect::from_min_size(
        pos2(form.left, top + line + lu(0.25 * EM)),
        vec2(form.width, INPUT_HEIGHT),
    );
    let radius = 0.2 * INPUT_FONT;
    ui.painter()
        .rect_filled(field, radius, Color32::from_gray(41));
    let response = ui.put(
        field.shrink(2.0),
        TextEdit::singleline(value)
            .id(Id::new(("settings-input", id)))
            .password(password)
            .font(theme::regular(INPUT_FONT))
            .text_color(jf::TEXT_80)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 6)))
            .vertical_align(egui::Align::Center)
            .desired_width(f32::INFINITY),
    );
    if response.has_focus() {
        widgets::border(ui.painter(), field, radius, 2.0, theme::JF_ACCENT);
    }
    form.end(field.bottom() - top, lu(1.8 * EM));
    response
}

/// A raised accent button (`emby-button raised button-submit`); full
/// width when `block`, else hugging its label. Returns whether clicked.
fn raised_button(ui: &Ui, left: f32, top: f32, width: Option<f32>, label: &str, id: &str) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::bold(EM), Color32::WHITE);
    let width = width.unwrap_or(galley.size().x + 2.0 * EM);
    let button = Rect::from_min_size(pos2(left, top), vec2(width, SAVE_HEIGHT));
    let response = ui.interact(button, Id::new(("settings-button", id)), Sense::click());
    widgets::describe_button(&response, true, label);
    let fill = if response.hovered() {
        theme::with_alpha(theme::JF_ACCENT, 0.85)
    } else {
        theme::JF_ACCENT
    };
    ui.painter().rect_filled(button, 0.2 * EM, fill);
    let x = button.center().x - galley.size().x / 2.0;
    widgets::galley_on_baseline(
        ui.painter(),
        x,
        jf::baseline(button.top() + 13.392, EM, lu(1.35 * EM)),
        galley,
    );
    widgets::pointer_on_hover(&response);
    response.clicked()
}

/// Profile: the user's picture (or a silhouette) with its button, their
/// name, and the password form.
fn draw_profile(
    ui: &mut Ui,
    form: &mut Form,
    images: &mut ImageCache,
    vm: &mut SettingsVm,
    actions: &mut Vec<UiAction>,
) {
    form.left += 5.0;
    let top = form.y;
    let avatar = Rect::from_min_size(pos2(form.left, top), vec2(200.0, 200.0));
    let art = vm.data.image_tag.as_ref().map(|tag| Art {
        key: format!("user-image:{tag}"),
        source: ArtSource::User { tag: tag.clone() },
    });
    match art
        .as_ref()
        .and_then(|art| images.texture(art).zip(images.size(&art.key)))
    {
        Some((texture, size)) => widgets::image_cover(ui.painter(), texture, size, avatar, 100.0),
        None => icons::paint_centered(
            ui.painter(),
            Icon::Person,
            avatar.center(),
            240.0,
            Color32::WHITE,
        ),
    }
    // The name and the image button, centred together right of the picture.
    let center = avatar.right() + 89.31;
    let name_size = 32.0;
    let name_line = lu(1.35 * name_size);
    let name = ui.painter().layout_no_wrap(
        vm.data.user_name.clone(),
        theme::regular(name_size),
        jf::TEXT_80,
    );
    let name_top = top + 40.48;
    let x = center - name.size().x / 2.0;
    widgets::galley_on_baseline(
        ui.painter(),
        x,
        jf::baseline(name_top, name_size, name_line),
        name,
    );
    let label = if vm.data.image_tag.is_some() {
        "Delete Image"
    } else {
        "Add Image"
    };
    let label_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::bold(EM), Color32::WHITE)
        .size()
        .x;
    let button_width = label_width + 2.0 * EM;
    if raised_button(
        ui,
        center - button_width / 2.0,
        top + 108.2,
        None,
        label,
        "profile-image",
    ) {
        if vm.data.image_tag.is_some() {
            vm.confirm_delete = true;
        } else {
            actions.push(UiAction::AddUserImage);
        }
    }
    form.y = top + 226.78;
    form.pending = 0.0;
    input(
        ui,
        form,
        "Current password",
        &mut vm.current_password,
        true,
        "current",
    );
    input(ui, form, "New password", &mut vm.new_password, true, "new");
    input(
        ui,
        form,
        "New password confirm",
        &mut vm.confirm_password,
        true,
        "confirm",
    );
    let save_top = form.y + 50.58;
    if raised_button(
        ui,
        form.left,
        save_top,
        Some(form.width),
        "Save Password",
        "save-password",
    ) {
        if vm.new_password != vm.confirm_password {
            actions.push(UiAction::Toast(
                "Password and password confirmation must match.".to_owned(),
            ));
        } else {
            actions.push(UiAction::ChangePassword {
                current: std::mem::take(&mut vm.current_password),
                new: std::mem::take(&mut vm.new_password),
            });
            vm.confirm_password.clear();
        }
    }
    form.y = save_top + SAVE_HEIGHT;
    form.pending = 0.0;
    if vm.confirm_delete {
        let modal = egui::Modal::new(Id::new("delete-user-image")).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.label(
                egui::RichText::new("Delete Image")
                    .font(theme::regular(H2))
                    .color(jf::TEXT_80),
            );
            ui.add_space(EM);
            ui.label(
                egui::RichText::new("Are you sure you wish to delete this image?")
                    .font(theme::regular(EM))
                    .color(jf::TEXT_80),
            );
            ui.add_space(EM);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    vm.confirm_delete = false;
                }
                if ui.button("Delete").clicked() {
                    vm.confirm_delete = false;
                    actions.push(UiAction::DeleteUserImage);
                }
            });
        });
        if modal.should_close() {
            vm.confirm_delete = false;
        }
    }
}

/// Quick Connect: authorize another device's sign-in by its code.
fn draw_quick_connect(
    ui: &mut Ui,
    form: &mut Form,
    vm: &mut SettingsVm,
    actions: &mut Vec<UiAction>,
) {
    heading(ui, form, "Quick Connect", H2, 0.0);
    let top = form.block(0.0);
    let height = wrapped(
        ui,
        form,
        top,
        0.0,
        "To sign in with Quick Connect, select the 'Quick Connect' button on the device you are logging in from and enter the displayed code below.",
        jf::TEXT_80,
    );
    // The paragraph, then a line break.
    form.end(height + lu(1.35 * EM), 0.0);
    let response = input(
        ui,
        form,
        "Quick Connect code",
        &mut vm.code,
        false,
        "quick-connect",
    );
    let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let top = form.y + form.pending;
    if (raised_button(
        ui,
        form.left,
        top,
        Some(form.width),
        "Authorize",
        "authorize",
    ) || enter)
        && !vm.code.trim().is_empty()
    {
        actions.push(UiAction::AuthorizeQuickConnect(std::mem::take(
            &mut vm.code,
        )));
    }
    form.y = top + SAVE_HEIGHT;
    form.pending = 0.0;
}

/// The subtitle preview: jellyfin's purple-to-blue banner with a sample
/// line styled by the page's current choices. Returns its height.
fn draw_subtitle_preview(ui: &Ui, form: &Form, vm: &SettingsVm, top: f32) -> f32 {
    let height = 6.0 * EM;
    let rect = Rect::from_min_size(pos2(form.left, top), vec2(form.width, height));
    let mut mesh = egui::Mesh::default();
    let start = Color32::from_rgb(0xaa, 0x5c, 0xc3);
    let end = Color32::from_rgb(0x00, 0xa4, 0xdc);
    mesh.colored_vertex(rect.left_top(), start);
    mesh.colored_vertex(rect.right_top(), end);
    mesh.colored_vertex(rect.right_bottom(), end);
    mesh.colored_vertex(rect.left_bottom(), start);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
    let style =
        crate::features::settings::SubtitleStyle::from_values(|key| vm.value(&Key::Device(key)));
    let size = 1.36 * EM * style.scale;
    let color = style.color;
    let font = if style.bold {
        theme::bold(size)
    } else {
        theme::regular(size)
    };
    let sample = "These settings affect subtitles on this device";
    let galley = ui
        .painter()
        .layout_no_wrap(sample.to_owned(), font.clone(), color);
    let at = pos2(
        rect.center().x - galley.size().x / 2.0,
        rect.center().y - galley.size().y / 2.0,
    );
    if let Some(shadow) = style.shadow {
        let shade = ui.painter().layout_no_wrap(sample.to_owned(), font, shadow);
        ui.painter().galley(at + vec2(1.5, 1.5), shade, shadow);
    }
    ui.painter().galley(at, galley, color);
    height
}

/// `.checkboxOutline`: an outlined box, filled accent with a check when
/// ticked.
pub(super) fn paint_checkbox(ui: &Ui, rect: Rect, checked: bool) {
    let painter = ui.painter();
    let radius = 2.08;
    if checked {
        painter.rect_filled(rect, radius, theme::JF_ACCENT);
        icons::paint_centered(
            painter,
            Icon::Check,
            rect.center(),
            rect.width() * 0.85,
            Color32::WHITE,
        );
    } else {
        widgets::border(painter, rect, radius, 2.0, jf::TEXT_80);
    }
}

/// An `emby-select`: the choice on #292929, an arrow at the right.
pub(super) fn paint_select(ui: &Ui, field: Rect, shown: &str, focused: bool) {
    let painter = ui.painter();
    let radius = 0.2 * INPUT_FONT;
    painter.rect_filled(field, radius, Color32::from_gray(41));
    if focused {
        widgets::border(painter, field, radius, 1.0, theme::JF_ACCENT);
    }
    let pad = 0.5 * INPUT_FONT;
    let arrow = 1.7 * EM;
    let galley = widgets::elided(
        ui,
        shown,
        theme::regular(INPUT_FONT),
        jf::TEXT_80,
        field.width() - pad - 1.9 * INPUT_FONT,
    );
    let ascent = (1.069 * INPUT_FONT).round();
    widgets::galley_on_baseline(
        painter,
        field.left() + 1.0 + pad,
        field.top() + 1.0 + pad + ascent,
        galley,
    );
    icons::paint_centered(
        painter,
        Icon::ExpandMore,
        pos2(
            field.right() - 0.3 * EM - arrow / 2.0,
            field.center().y + 2.0,
        ),
        arrow,
        jf::TEXT_80,
    );
}

/// The open select's options, as jellyfin-web's action sheet.
fn draw_select_sheet(ui: &Ui, vm: &mut SettingsVm) {
    let Some(key) = vm.open_select.clone() else {
        return;
    };
    let order = vm.order();
    let fields = settings::fields(vm.page, &vm.data, &order);
    let Some(options) = fields.iter().find_map(|field| match field {
        Field::Select {
            key: field_key,
            options,
            ..
        } if *field_key == key => Some(options),
        _ => None,
    }) else {
        vm.open_select = None;
        return;
    };
    let current = match vm.value(&key) {
        FieldValue::Text(value) => value,
        _ => String::new(),
    };
    let selected = options
        .iter()
        .position(|(value, _)| *value == current)
        .unwrap_or(0);
    let items = options
        .iter()
        .map(|(value, name)| dialogs::SheetItem {
            label: name,
            icon: None,
            checked: *value == current,
        })
        .collect::<Vec<_>>();
    let anchor = Rect::from_center_size(ui.ctx().content_rect().center(), vec2(1.0, 1.0));
    let (picked, close) = dialogs::action_sheet(
        ui.ctx(),
        Id::new("settings-select-sheet"),
        anchor,
        &items,
        Some(selected),
    );
    if let Some(index) = picked {
        vm.values
            .insert(key, FieldValue::Text(options[index].0.clone()));
        vm.open_select = None;
    } else if close {
        vm.open_select = None;
    }
}

/// The library order list: each library with a round folder icon, moved
/// with up and down buttons.
fn draw_order(ui: &mut Ui, form: &mut Form, vm: &mut SettingsVm, order: &[String]) {
    let margin = lu(0.5 * EM);
    let top = form.block(margin);
    let list = Rect::from_min_size(
        pos2(form.left, top),
        vec2(form.width, order.len() as f32 * ORDER_ROW),
    );
    ui.painter().rect_filled(list, 0.0, Color32::from_gray(32));
    let mut moved = None;
    for (index, id) in order.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + index as f32 * ORDER_ROW),
            vec2(list.width(), ORDER_ROW),
        );
        let name = vm
            .data
            .libraries
            .iter()
            .find(|library| library.id == *id)
            .map_or("", |library| library.name.as_str());
        let circle = 42.52;
        let center = pos2(row.left() + 7.44 + 8.51 + circle / 2.0, row.center().y);
        ui.painter()
            .circle_filled(center, circle / 2.0, theme::JF_ACCENT);
        icons::paint_centered(
            ui.painter(),
            Icon::FolderOpen,
            center,
            21.28,
            Color32::WHITE,
        );
        let text_left = row.left() + 7.44 + 8.51 + circle + 4.26 + 11.16;
        text(
            ui,
            text_left,
            row.top() + 3.72 + 12.65,
            EM,
            lu(1.35 * EM),
            name,
            jf::TEXT_80,
            false,
        );
        let button = vec2(41.36, 41.38);
        let down = Rect::from_min_size(
            pos2(
                row.right() - 3.72 - 4.32 - button.x,
                row.center().y - button.y / 2.0,
            ),
            button,
        );
        let up = down.translate(vec2(-(button.x + 2.0 * 4.32), 0.0));
        // jellyfin-web keeps both arrows live; the first row's up does nothing.
        if jf::round_icon_button(
            ui,
            up,
            Icon::ExpandLess,
            jf::TEXT_80,
            24.84,
            "Up",
            true,
            Id::new(("order-up", index)),
        )
        .clicked()
            && index > 0
        {
            moved = Some((index, index - 1));
        }
        if jf::round_icon_button(
            ui,
            down,
            Icon::ExpandMore,
            jf::TEXT_80,
            24.84,
            "Down",
            true,
            Id::new(("order-down", index)),
        )
        .clicked()
            && index + 1 < order.len()
        {
            moved = Some((index, index + 1));
        }
    }
    if let Some((from, to)) = moved {
        let mut order = order.to_vec();
        order.swap(from, to);
        vm.values
            .insert(Key::Config("OrderedViews"), FieldValue::List(order));
    }
    form.end(list.height(), margin);
}
