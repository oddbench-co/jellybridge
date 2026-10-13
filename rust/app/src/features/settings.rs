//! The user's settings pages, as jellyfin-web lays them out: each page is a
//! list of fields, and each field is kept where jellyfin-web keeps it --
//! the user's server configuration, jellyfin-web's synced display
//! preferences, or this device.

use std::collections::HashMap;

use crate::features::pages::LibraryRef;

/// One settings page (jellyfin-web's "mypreferences…" routes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    Menu,
    Profile,
    QuickConnect,
    Display,
    Home,
    Playback,
    Subtitles,
}

impl SettingsPage {
    pub fn title(self) -> &'static str {
        match self {
            Self::Menu => "Settings",
            Self::Profile => "Profile",
            Self::QuickConnect => "Quick Connect",
            Self::Display => "Display",
            Self::Home => "Home",
            Self::Playback => "Playback",
            Self::Subtitles => "Subtitles",
        }
    }
}

/// Where a field's value is kept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// The user's server configuration (`Users/{id}`, `Configuration`).
    Config(&'static str),
    /// jellyfin-web's synced user settings (`CustomPrefs`, "emby" client).
    Pref(String),
    /// This device only.
    Device(&'static str),
    /// Whether a library is in one of the configuration's id lists;
    /// `exclusion` lists hold the libraries the box is *unticked* for.
    InList {
        list: &'static str,
        id: String,
        exclusion: bool,
    },
}

/// A field's value while the page is edited.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Bool(bool),
    Text(String),
    /// An ordered list of ids (the library order).
    List(Vec<String>),
}

/// One row of a settings form.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Heading(String),
    SubHeading(String),
    /// Plain text (a `p`).
    Paragraph(String),
    /// The end of a `.verticalSection-extrabottompadding`: 2.7em under it.
    SectionEnd,
    Check {
        key: Key,
        label: String,
        help: Option<String>,
    },
    Select {
        key: Key,
        label: String,
        options: Vec<(String, String)>,
        help: Option<String>,
        /// Help that follows the chosen option (by option value).
        option_help: Vec<(String, String)>,
    },
    Number {
        key: Key,
        label: String,
        help: Option<String>,
    },
    /// The libraries in their Home order, each movable up and down.
    LibraryOrder,
    /// The subtitle appearance banner, styled by the current choices.
    SubtitlePreview,
    /// A whole-number range (`emby-slider`).
    Slider {
        key: Key,
        label: String,
        range: (i32, i32),
        help: Option<String>,
    },
}

/// Everything the settings pages show and edit.
#[derive(Clone, Debug, Default)]
pub struct SettingsData {
    pub user_name: String,
    /// The user's picture, if they set one.
    pub image_tag: Option<String>,
    /// `Configuration` of `Users/{id}`, kept whole so saving keeps the
    /// fields this client does not show.
    pub config: serde_json::Map<String, serde_json::Value>,
    /// The whole `DisplayPreferences/usersettings` document.
    pub display: serde_json::Value,
    /// Every library of the user, hidden ones included.
    pub libraries: Vec<LibraryRef>,
    /// Folders that can be grouped into views ("Library Folders").
    pub folders: Vec<(String, String)>,
    /// Languages for the preference selects: (name, ISO 639-2 code).
    pub languages: Vec<(String, String)>,
    /// This device's settings.
    pub device: HashMap<String, String>,
}

impl SettingsData {
    fn pref(&self, key: &str) -> Option<String> {
        self.display["CustomPrefs"][key].as_str().map(str::to_owned)
    }

    fn config_list(&self, list: &str) -> Vec<String> {
        self.config
            .get(list)
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect()
    }

    /// The libraries in the user's Home order (`OrderedViews`, then the
    /// rest as the server lists them).
    pub fn ordered_libraries(&self, order: &[String]) -> Vec<LibraryRef> {
        let mut libraries = self.libraries.clone();
        libraries.sort_by_key(|library| {
            order
                .iter()
                .position(|id| *id == library.id)
                .unwrap_or(usize::MAX)
        });
        libraries
    }

    /// A field's stored value, or jellyfin-web's default.
    pub fn value(&self, key: &Key) -> FieldValue {
        match key {
            Key::Config(name) => match self.config.get(*name) {
                Some(serde_json::Value::Bool(value)) => FieldValue::Bool(*value),
                Some(serde_json::Value::String(value)) => FieldValue::Text(value.clone()),
                Some(serde_json::Value::Number(value)) => FieldValue::Text(value.to_string()),
                Some(serde_json::Value::Array(_)) => FieldValue::List(self.config_list(name)),
                _ => config_default(name),
            },
            // jellyfin-web reads an on-by-default setting as on unless it is
            // exactly "false", and an off-by-default one as off unless it is
            // exactly "true".
            Key::Pref(name) => match self.pref(name) {
                Some(value) if is_bool_pref(name) => {
                    FieldValue::Bool(if pref_default(name) == FieldValue::Bool(true) {
                        value != "false"
                    } else {
                        value == "true"
                    })
                }
                Some(value) => FieldValue::Text(value),
                None => pref_default(name),
            },
            Key::Device(name) => match self.device.get(*name) {
                Some(value) if is_bool_device(name) => FieldValue::Bool(value == "true"),
                Some(value) => FieldValue::Text(value.clone()),
                None => device_default(name),
            },
            Key::InList {
                list,
                id,
                exclusion,
            } => FieldValue::Bool(self.config_list(list).contains(id) != *exclusion),
        }
    }
}

fn config_default(name: &str) -> FieldValue {
    match name {
        "HidePlayedInLatest"
        | "PlayDefaultAudioTrack"
        | "EnableNextEpisodeAutoPlay"
        | "RememberAudioSelections"
        | "RememberSubtitleSelections" => FieldValue::Bool(true),
        "DisplayMissingEpisodes" => FieldValue::Bool(false),
        "OrderedViews" => FieldValue::List(Vec::new()),
        "SubtitleMode" => FieldValue::Text("Default".to_owned()),
        _ => FieldValue::Text(String::new()),
    }
}

fn is_bool_pref(name: &str) -> bool {
    matches!(
        name,
        "enableNextVideoInfoOverlay"
            | "enableCinemaMode"
            | "enableRewatchingInNextUp"
            | "useEpisodeImagesInNextUpAndResume"
            | "enableBackdrops"
            | "enableThemeSongs"
            | "enableThemeVideos"
            | "detailsBanner"
            | "enableFastFadein"
            | "enableBlurhash"
    )
}

fn pref_default(name: &str) -> FieldValue {
    if let Some(slot) = name.strip_prefix("homesection") {
        // jellyfin-web's defaults; slots 9 and 10 start blank.
        let value = match slot {
            "0" => "smalllibrarytiles",
            "1" => "resume",
            "2" => "resumeaudio",
            "3" => "resumebook",
            "4" => "livetv",
            "5" => "nextup",
            "6" => "latestmedia",
            "7" => "none",
            _ => "",
        };
        return FieldValue::Text(value.to_owned());
    }
    match name {
        "enableNextVideoInfoOverlay"
        | "enableCinemaMode"
        | "enableFastFadein"
        | "enableBlurhash"
        | "detailsBanner" => FieldValue::Bool(true),
        _ if is_bool_pref(name) => FieldValue::Bool(false),
        "skipForwardLength" => FieldValue::Text("30000".to_owned()),
        "skipBackLength" => FieldValue::Text("10000".to_owned()),
        "maxDaysForNextUp" => FieldValue::Text("365".to_owned()),
        "libraryPageSize" => FieldValue::Text("100".to_owned()),
        _ => FieldValue::Text(String::new()),
    }
}

fn is_bool_device(name: &str) -> bool {
    matches!(
        name,
        "disableVbrAudio"
            | "limitSupportedVideoResolution"
            | "preferFmp4HlsContainer"
            | "enableDts"
            | "enableTrueHd"
            | "enableHi10p"
            | "limitSegmentLength"
            | "alwaysRemuxFlac"
            | "alwaysRemuxMp3"
            | "subtitleRenderPgs"
            | "alwaysBurnInSubtitleWhenTranscoding"
    )
}

fn device_default(name: &str) -> FieldValue {
    match name {
        "alwaysRemuxFlac" | "alwaysRemuxMp3" => FieldValue::Bool(true),
        _ if is_bool_device(name) => FieldValue::Bool(false),
        "allowedAudioChannels" => FieldValue::Text("-1".to_owned()),
        "segmentTypeAction__Intro" | "segmentTypeAction__Outro" => {
            FieldValue::Text("AskToSkip".to_owned())
        }
        name if name.starts_with("segmentTypeAction__") => FieldValue::Text("None".to_owned()),
        "subtitleTextWeight" => FieldValue::Text("normal".to_owned()),
        "subtitleTextColor" => FieldValue::Text("#ffffff".to_owned()),
        "subtitleVerticalPosition" => FieldValue::Text("-3".to_owned()),
        "maxVideoWidth" => FieldValue::Text("0".to_owned()),
        _ => FieldValue::Text(String::new()),
    }
}

fn select(key: Key, label: &str, options: &[(&str, &str)], help: Option<&str>) -> Field {
    Field::Select {
        key,
        label: label.to_owned(),
        options: options
            .iter()
            .map(|(value, name)| ((*value).to_owned(), (*name).to_owned()))
            .collect(),
        help: help.map(str::to_owned),
        option_help: Vec::new(),
    }
}

fn check(key: Key, label: &str, help: Option<&str>) -> Field {
    Field::Check {
        key,
        label: label.to_owned(),
        help: help.map(str::to_owned),
    }
}

/// jellyfin-web's Home section choices.
const HOME_SECTIONS: [(&str, &str); 10] = [
    ("smalllibrarytiles", "My Media"),
    ("librarybuttons", "My Media (small)"),
    ("activerecordings", "Active Recordings"),
    ("resume", "Continue Watching"),
    ("resumeaudio", "Continue Listening"),
    ("resumebook", "Continue Reading"),
    ("latestmedia", "Recently Added Media"),
    ("nextup", "Next Up"),
    ("livetv", "Live TV"),
    ("none", "None"),
];

/// The "Default screen" choices of a library, by collection type.
fn landing_screens(collection_type: &str) -> &'static [(&'static str, &'static str)] {
    match collection_type {
        "movies" => &[
            ("movies", "Movies"),
            ("suggestions", "Suggestions"),
            ("favorites", "Favorites"),
            ("collections", "Collections"),
            ("genres", "Genres"),
        ],
        "tvshows" => &[
            ("shows", "Shows"),
            ("suggestions", "Suggestions"),
            ("upcoming", "Upcoming"),
            ("genres", "Genres"),
            ("networks", "Networks"),
            ("episodes", "Episodes"),
        ],
        "music" => &[
            ("albums", "Albums"),
            ("suggestions", "Suggestions"),
            ("albumartists", "Album Artists"),
            ("artists", "Artists"),
            ("playlists", "Playlists"),
            ("songs", "Songs"),
            ("genres", "Genres"),
        ],
        _ => &[],
    }
}

const LATEST_HELP: &str =
    "Display in home screen sections such as 'Recently Added Media' and 'Continue Watching'";

/// The fields of a form page (the menu, Profile and Quick Connect are drawn
/// apart).
pub fn fields(page: SettingsPage, data: &SettingsData, order: &[String]) -> Vec<Field> {
    match page {
        SettingsPage::Home => home_fields(data, order),
        SettingsPage::Playback => playback_fields(data),
        SettingsPage::Subtitles => subtitle_fields(data),
        SettingsPage::Display => display_fields(),
        SettingsPage::Menu | SettingsPage::Profile | SettingsPage::QuickConnect => Vec::new(),
    }
}

fn home_fields(data: &SettingsData, order: &[String]) -> Vec<Field> {
    let mut fields = vec![
        Field::Heading("Home".to_owned()),
        check(
            Key::Config("HidePlayedInLatest"),
            "Hide watched content from 'Recently Added Media'",
            None,
        ),
        Field::SectionEnd,
    ];
    for slot in 0..10 {
        fields.push(select(
            Key::Pref(format!("homesection{slot}")),
            &format!("Home screen section {}", slot + 1),
            &HOME_SECTIONS,
            None,
        ));
    }
    fields.push(Field::SectionEnd);
    fields.push(Field::Heading("Library Order".to_owned()));
    fields.push(Field::LibraryOrder);
    fields.push(Field::SectionEnd);
    for library in data.ordered_libraries(order) {
        fields.push(Field::Heading(library.name.clone()));
        if library.collection_type == "playlists" {
            fields.push(check(
                Key::InList {
                    list: "MyMediaExcludes",
                    id: library.id.clone(),
                    exclusion: true,
                },
                "Display on home screen",
                None,
            ));
        } else {
            fields.push(check(
                Key::InList {
                    list: "LatestItemsExcludes",
                    id: library.id.clone(),
                    exclusion: true,
                },
                LATEST_HELP,
                None,
            ));
        }
        let screens = landing_screens(&library.collection_type);
        if !screens.is_empty() {
            fields.push(select(
                Key::Pref(format!("landing-{}", library.id)),
                "Default screen",
                screens,
                None,
            ));
        }
        fields.push(Field::SectionEnd);
    }
    if !data.folders.is_empty() {
        fields.push(Field::Heading("Library Folders".to_owned()));
        fields.push(Field::Paragraph(
            "Automatically group content from the following folders into views such as 'Movies', 'Music' and 'TV'."
                .to_owned(),
        ));
        for (id, name) in &data.folders {
            fields.push(check(
                Key::InList {
                    list: "GroupedFolders",
                    id: id.clone(),
                    exclusion: false,
                },
                name,
                None,
            ));
        }
        fields.push(Field::Paragraph(
            "Folders that are unchecked will be displayed by themselves in their own view."
                .to_owned(),
        ));
    }
    fields
}

/// A language select's choices: "Any Language", then every culture.
fn language_options(data: &SettingsData) -> Vec<(String, String)> {
    std::iter::once((String::new(), "Any Language".to_owned()))
        .chain(
            data.languages
                .iter()
                .map(|(name, code)| (code.clone(), name.clone())),
        )
        .collect()
}

fn number(key: Key, label: &str, help: Option<&str>) -> Field {
    Field::Number {
        key,
        label: label.to_owned(),
        help: help.map(str::to_owned),
    }
}

/// Skip lengths, in milliseconds as jellyfin-web stores them.
const SKIP_LENGTHS: [(&str, &str); 6] = [
    ("5000", "5 seconds"),
    ("10000", "10 seconds"),
    ("15000", "15 seconds"),
    ("20000", "20 seconds"),
    ("25000", "25 seconds"),
    ("30000", "30 seconds"),
];

/// The media segment kinds the player can act on, in jellyfin-web's order.
pub const SEGMENT_KINDS: [(&str, &str); 5] = [
    ("Intro", "Intro Segments"),
    ("Preview", "Preview Segments"),
    ("Recap", "Recap Segments"),
    ("Commercial", "Commercial Segments"),
    ("Outro", "Outro Segments"),
];

/// The device setting holding what the player does with a segment kind.
pub fn segment_action_key(kind: &str) -> &'static str {
    match kind {
        "Intro" => "segmentTypeAction__Intro",
        "Preview" => "segmentTypeAction__Preview",
        "Recap" => "segmentTypeAction__Recap",
        "Commercial" => "segmentTypeAction__Commercial",
        _ => "segmentTypeAction__Outro",
    }
}

fn playback_fields(data: &SettingsData) -> Vec<Field> {
    let mut fields = vec![
        Field::Heading("Audio Settings".to_owned()),
        select(
            Key::Device("allowedAudioChannels"),
            "Maximum Allowed Audio Channels",
            &[
                ("-1", "Auto"),
                ("1", "Mono"),
                ("2", "Stereo"),
                ("6", "5.1 Channels"),
                ("8", "7.1 Channels"),
            ],
            None,
        ),
        Field::Select {
            key: Key::Config("AudioLanguagePreference"),
            label: "Preferred audio language".to_owned(),
            options: language_options(data),
            help: None,
            option_help: Vec::new(),
        },
        check(
            Key::Config("PlayDefaultAudioTrack"),
            "Play default audio track regardless of language",
            None,
        ),
        Field::SectionEnd,
        Field::Heading("Advanced".to_owned()),
        check(
            Key::Config("EnableNextEpisodeAutoPlay"),
            "Play next episode automatically",
            None,
        ),
        check(
            Key::Config("RememberAudioSelections"),
            "Set audio track based on previous item",
            Some("Try to set the audio track to the closest match to the last video."),
        ),
        check(
            Key::Config("RememberSubtitleSelections"),
            "Set subtitle track based on previous item",
            Some("Try to set the subtitle track to the closest match to the last video."),
        ),
        check(
            Key::Pref("enableNextVideoInfoOverlay".to_owned()),
            "Show next video info during playback",
            Some(
                "At the end of a video, display info about the next video coming up in the current playlist.",
            ),
        ),
        select(
            Key::Pref("skipForwardLength".to_owned()),
            "Skip forward length",
            &SKIP_LENGTHS,
            None,
        ),
        select(
            Key::Pref("skipBackLength".to_owned()),
            "Skip back length",
            &SKIP_LENGTHS,
            None,
        ),
        Field::SubHeading("Media Segment Actions".to_owned()),
    ];
    for (kind, label) in SEGMENT_KINDS {
        fields.push(select(
            Key::Device(segment_action_key(kind)),
            label,
            &[
                ("None", "None"),
                ("Skip", "Skip"),
                ("AskToSkip", "Ask To Skip"),
            ],
            None,
        ));
    }
    fields
}

fn subtitle_fields(data: &SettingsData) -> Vec<Field> {
    vec![
        Field::Heading("Subtitles".to_owned()),
        Field::Select {
            key: Key::Config("SubtitleLanguagePreference"),
            label: "Preferred subtitle language".to_owned(),
            options: language_options(data),
            help: None,
            option_help: Vec::new(),
        },
        Field::Select {
            key: Key::Config("SubtitleMode"),
            label: "Subtitle mode".to_owned(),
            options: [
                ("Default", "Default"),
                ("Smart", "Smart"),
                ("OnlyForced", "Only Forced"),
                ("Always", "Always Play"),
                ("None", "None"),
            ]
            .iter()
            .map(|(value, name)| ((*value).to_owned(), (*name).to_owned()))
            .collect(),
            help: None,
            option_help: [
                ("Default", "Subtitles are loaded based on the default and forced flags in the embedded metadata. Language preferences are considered when multiple options are available."),
                ("Smart", "Subtitles matching the language preference will be loaded when the audio is in a foreign language."),
                ("OnlyForced", "Only subtitles marked as forced will be loaded."),
                ("Always", "Subtitles matching the language preference will be loaded regardless of the audio language."),
                ("None", "Subtitles will not be loaded by default. They can still be turned on manually during playback."),
            ]
            .iter()
            .map(|(value, help)| ((*value).to_owned(), (*help).to_owned()))
            .collect(),
        },
        Field::SectionEnd,
        Field::Heading("Subtitle Appearance".to_owned()),
        Field::SubtitlePreview,
        Field::Paragraph(
            "Following settings do not apply to the graphical subtitles mentioned above or ASS/SSA subtitles that embed their own styles."
                .to_owned(),
        ),
        select(
            Key::Device("subtitleTextSize"),
            "Text size",
            &[
                ("smaller", "Smaller"),
                ("small", "Small"),
                ("", "Normal"),
                ("large", "Large"),
                ("larger", "Larger"),
                ("extralarge", "Extra Large"),
            ],
            None,
        ),
        select(
            Key::Device("subtitleTextWeight"),
            "Text weight",
            &[("normal", "Normal"), ("bold", "Bold")],
            None,
        ),
        select(
            Key::Device("subtitleTextColor"),
            "Text color",
            &[
                ("#ffffff", "White"),
                ("#d3d3d3", "Light Gray"),
                ("#808080", "Gray"),
                ("#ffff00", "Yellow"),
                ("#008000", "Green"),
                ("#00ffff", "Cyan"),
                ("#0000ff", "Blue"),
                ("#ff00ff", "Magenta"),
                ("#ff0000", "Red"),
                ("#000000", "Black"),
            ],
            None,
        ),
        select(
            Key::Device("subtitleDropShadow"),
            "Drop shadow",
            &[
                ("none", "None"),
                ("raised", "Raised"),
                ("depressed", "Depressed"),
                ("uniform", "Uniform"),
                ("", "Drop Shadow"),
            ],
            None,
        ),
        Field::Slider {
            key: Key::Device("subtitleVerticalPosition"),
            label: "Vertical position".to_owned(),
            range: (-16, 16),
            help: Some("Line number where text appears. Positive numbers indicate top down. Negative numbers indicate bottom up.".to_owned()),
        },
    ]
}

fn display_fields() -> Vec<Field> {
    vec![
        Field::Heading("Libraries".to_owned()),
        number(
            Key::Pref("libraryPageSize".to_owned()),
            "Library page size",
            Some(
                "Set the amount of items to show on a library page. Setting a value of 0 will disable pagination. Please note that setting this to 0 or any value greater than 100 may lead to bugs and reduced performance.",
            ),
        ),
        check(
            Key::Config("DisplayMissingEpisodes"),
            "Display missing episodes within seasons",
            Some("This must also be enabled for TV libraries in the server configuration."),
        ),
        Field::SectionEnd,
        Field::Heading("Next Up".to_owned()),
        number(
            Key::Pref("maxDaysForNextUp".to_owned()),
            "Max days in 'Next Up'",
            Some(
                "Set the maximum amount of days a show should stay in the 'Next Up' list without watching it.",
            ),
        ),
        check(
            Key::Pref("enableRewatchingInNextUp".to_owned()),
            "Enable Rewatching in Next Up",
            Some("Enable showing already watched episodes in 'Next Up' sections."),
        ),
        check(
            Key::Pref("useEpisodeImagesInNextUpAndResume".to_owned()),
            "Use episode images in 'Next Up' and 'Continue Watching' sections",
            Some(
                "'Next Up' and 'Continue Watching' sections will use episode images as thumbnails instead of the primary thumbnail of the show.",
            ),
        ),
    ]
}

/// The subtitle appearance chosen on the Subtitles page, for the preview
/// and for the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubtitleStyle {
    /// Relative to the normal size.
    pub scale: f32,
    pub bold: bool,
    pub color: egui::Color32,
    /// The drop shadow's colour, if any.
    pub shadow: Option<egui::Color32>,
    /// jellyfin-web's line number: negative counts up from the bottom.
    pub line: i32,
}

impl SubtitleStyle {
    pub fn from_values(value: impl Fn(&'static str) -> FieldValue) -> Self {
        let text = |key| match value(key) {
            FieldValue::Text(text) => text,
            _ => String::new(),
        };
        let scale = match text("subtitleTextSize").as_str() {
            "smaller" => 0.8,
            "small" => 0.9,
            "large" => 1.2,
            "larger" => 1.4,
            "extralarge" => 1.6,
            _ => 1.0,
        };
        let color = text("subtitleTextColor");
        let rgb = u32::from_str_radix(color.trim_start_matches('#'), 16).unwrap_or(0xffffff);
        let color = egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        Self {
            scale,
            bold: text("subtitleTextWeight") == "bold",
            color,
            shadow: (text("subtitleDropShadow") != "none").then_some(egui::Color32::BLACK),
            line: text("subtitleVerticalPosition").parse().unwrap_or(-3),
        }
    }
}

/// mpv properties for this device's settings: the audio channel limit and
/// the subtitle look (at mpv's default 55px font at 720p).
pub fn player_options(device: &HashMap<String, String>) -> Vec<(&'static str, String)> {
    let value = |key: &'static str| match device.get(key) {
        Some(value) => FieldValue::Text(value.clone()),
        None => device_default(key),
    };
    let channels = match value("allowedAudioChannels") {
        FieldValue::Text(channels) => match channels.as_str() {
            "1" => "mono",
            "2" => "stereo",
            "6" => "5.1",
            "8" => "7.1",
            _ => "auto-safe",
        },
        _ => "auto-safe",
    };
    let style = SubtitleStyle::from_values(value);
    let [r, g, b, _] = style.color.to_array();
    let shadow = match device.get("subtitleDropShadow").map(String::as_str) {
        Some("none") => ("0", "0"),
        Some("uniform") => ("3", "0"),
        Some("raised" | "depressed") => ("0", "2"),
        _ => ("0", "2"),
    };
    // jellyfin's lines count from -1 (the bottom line) up; -3 is its
    // default and mpv's default margin.
    let (align, margin) = if style.line < 0 {
        ("bottom", 22 + (-style.line - 3).max(-1) * 55)
    } else {
        ("top", 22 + (style.line - 1).max(0) * 55)
    };
    vec![
        ("audio-channels", channels.to_owned()),
        ("sub-font-size", format!("{}", (55.0 * style.scale).round())),
        ("sub-bold", if style.bold { "yes" } else { "no" }.to_owned()),
        ("sub-color", format!("#{r:02X}{g:02X}{b:02X}")),
        ("sub-border-size", shadow.0.to_owned()),
        ("sub-shadow-offset", shadow.1.to_owned()),
        ("sub-align-y", align.to_owned()),
        ("sub-margin-y", margin.max(0).to_string()),
    ]
}

/// What a save writes: the configuration and display preferences with the
/// page's values applied, and the device settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsSave {
    pub config: serde_json::Map<String, serde_json::Value>,
    pub display: serde_json::Value,
    pub device: HashMap<String, String>,
}

/// Applies the edited `values` to copies of the stored settings.
pub fn apply(data: &SettingsData, values: &HashMap<Key, FieldValue>) -> SettingsSave {
    let mut config = data.config.clone();
    let mut display = data.display.clone();
    let mut device = data.device.clone();
    if !display["CustomPrefs"].is_object() {
        display["CustomPrefs"] = serde_json::json!({});
    }
    for (key, value) in values {
        match (key, value) {
            (Key::Config(name), FieldValue::Bool(value)) => {
                config.insert((*name).to_owned(), serde_json::Value::Bool(*value));
            }
            (Key::Config(name), FieldValue::Text(value)) => {
                config.insert((*name).to_owned(), serde_json::Value::String(value.clone()));
            }
            (Key::Config(name), FieldValue::List(ids)) => {
                config.insert((*name).to_owned(), serde_json::json!(ids));
            }
            (Key::Pref(name), FieldValue::Bool(value)) => {
                display["CustomPrefs"][name.as_str()] =
                    serde_json::Value::String(value.to_string());
            }
            (Key::Pref(name), FieldValue::Text(value)) => {
                display["CustomPrefs"][name.as_str()] = serde_json::Value::String(value.clone());
            }
            (Key::Device(name), FieldValue::Bool(value)) => {
                device.insert((*name).to_owned(), value.to_string());
            }
            (Key::Device(name), FieldValue::Text(value)) => {
                device.insert((*name).to_owned(), value.clone());
            }
            (
                Key::InList {
                    list,
                    id,
                    exclusion,
                },
                FieldValue::Bool(ticked),
            ) => {
                let mut ids = data_list(&config, list);
                let wanted = *ticked != *exclusion;
                ids.retain(|existing| existing != id);
                if wanted {
                    ids.push(id.clone());
                }
                config.insert((*list).to_owned(), serde_json::json!(ids));
            }
            _ => {}
        }
    }
    SettingsSave {
        config,
        display,
        device,
    }
}

fn data_list(config: &serde_json::Map<String, serde_json::Value>, list: &str) -> Vec<String> {
    config
        .get(list)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}

/// Saves a settings page: the server's settings, then this device's; the
/// Home sections and library order the app follows are read again.
pub fn save(context: &crate::app::AppContext, save: SettingsSave) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let result = api.save_settings(&save).await;
        let store = ctx.settings_store.clone();
        let device = save.device.clone();
        let stored = tokio::task::spawn_blocking(move || store.save_device_settings(device)).await;
        let message = match (result, stored) {
            (Ok(()), Ok(Ok(()))) => "Settings saved.".to_owned(),
            (Err(error), _) => format!("The settings could not be saved: {error}"),
            (_, Ok(Err(error))) => format!("The settings could not be saved: {error}"),
            (_, Err(error)) => format!("The settings could not be saved: {error}"),
        };
        let device = save.device;
        let mut player = Vec::new();
        let prefs = api
            .user_prefs()
            .await
            .inspect_err(|error| tracing::warn!(%error, "settings could not be read back"))
            .ok();
        if let Ok(mut state) = ctx.state.lock() {
            if let Some(prefs) = prefs {
                state.prefs = prefs;
            }
            state.device = device;
            let behaviour =
                crate::features::now_playing::Behaviour::from_settings(&state.prefs, &state.device);
            if let Some(now) = state.now_playing.as_mut() {
                now.behaviour = behaviour;
            }
            player = player_options(&state.device);
            crate::features::pages::set_page_size(state.prefs.library_page_size());
            let (back, forward) = state.prefs.skip_lengths();
            crate::ui::player::present_skip_lengths(&ctx, back, forward);
        }
        let _ = ctx
            .player_sender
            .send(crate::services::player::PlayerCommand::Configure(player));
        ctx.ui
            .update(move |state| crate::ui::show_toast(state, &message));
    });
}

fn toast(context: &crate::app::AppContext, message: String) {
    context
        .ui
        .update(move |state| crate::ui::show_toast(state, &message));
}

/// Profile: changes the password (the form already checked the two new
/// entries match).
pub fn change_password(context: &crate::app::AppContext, current: String, new: String) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let message = match api.change_password(&current, &new).await {
            Ok(()) => "Password saved.".to_owned(),
            Err(error) => format!("The password could not be saved: {error}"),
        };
        toast(&ctx, message);
    });
}

/// Quick Connect: lets in the device showing `code`.
pub fn authorize_quick_connect(context: &crate::app::AppContext, code: String) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let message = match api.authorize_quick_connect(code.trim()).await {
            Ok(true) => "You have successfully authenticated your device!".to_owned(),
            Ok(false) | Err(_) => "Unknown Quick Connect code".to_owned(),
        };
        toast(&ctx, message);
    });
}

/// Profile: asks the desktop for a picture and makes it the user's.
pub fn add_user_image(context: &crate::app::AppContext) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        let picked = tokio::task::spawn_blocking(|| {
            let path = crate::services::portal::pick_image("Add Image")
                .map_err(|error| error.to_string())?;
            let Some(path) = path else { return Ok(None) };
            let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
            let mime = match path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("png") => "image/png",
                Some("webp") => "image/webp",
                _ => "image/jpeg",
            };
            Ok::<_, String>(Some((bytes, mime)))
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
        match picked {
            Ok(Some((bytes, mime))) => match api.upload_user_image(&bytes, mime).await {
                Ok(()) => crate::features::catalog::reload(&ctx),
                Err(error) => toast(&ctx, format!("The image could not be saved: {error}")),
            },
            Ok(None) => {}
            Err(error) => toast(&ctx, error),
        }
    });
}

/// Profile: removes the user's picture.
pub fn delete_user_image(context: &crate::app::AppContext) {
    let Some(api) = context.api() else { return };
    let ctx = context.clone();
    context.runtime.spawn(async move {
        match api.delete_user_image().await {
            Ok(()) => crate::features::catalog::reload(&ctx),
            Err(error) => toast(&ctx, format!("The image could not be deleted: {error}")),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> SettingsData {
        SettingsData {
            config: serde_json::json!({
                "HidePlayedInLatest": false,
                "LatestItemsExcludes": ["movies"],
                "OrderedViews": ["shows", "movies"],
                "Unrelated": 7,
            })
            .as_object()
            .cloned()
            .unwrap(),
            display: serde_json::json!({"Id": "x", "CustomPrefs": {"homesection1": "nextup"}}),
            libraries: vec![
                LibraryRef {
                    id: "movies".into(),
                    name: "Movies".into(),
                    collection_type: "movies".into(),
                },
                LibraryRef {
                    id: "shows".into(),
                    name: "Shows".into(),
                    collection_type: "tvshows".into(),
                },
            ],
            ..SettingsData::default()
        }
    }

    #[test]
    fn values_come_from_where_jellyfin_web_keeps_them() {
        let data = data();
        assert_eq!(
            data.value(&Key::Config("HidePlayedInLatest")),
            FieldValue::Bool(false)
        );
        assert_eq!(
            data.value(&Key::Pref("homesection1".into())),
            FieldValue::Text("nextup".into())
        );
        // Unset: jellyfin-web's default.
        assert_eq!(
            data.value(&Key::Pref("homesection0".into())),
            FieldValue::Text("smalllibrarytiles".into())
        );
        let latest = |id: &str| Key::InList {
            list: "LatestItemsExcludes",
            id: id.into(),
            exclusion: true,
        };
        assert_eq!(data.value(&latest("movies")), FieldValue::Bool(false));
        assert_eq!(data.value(&latest("shows")), FieldValue::Bool(true));
        let order = vec!["shows".to_owned(), "movies".to_owned()];
        assert_eq!(data.ordered_libraries(&order)[0].id, "shows");
    }

    #[test]
    fn saving_applies_the_edits_and_keeps_everything_else() {
        let data = data();
        let mut values = HashMap::new();
        values.insert(Key::Config("HidePlayedInLatest"), FieldValue::Bool(true));
        values.insert(
            Key::InList {
                list: "LatestItemsExcludes",
                id: "movies".into(),
                exclusion: true,
            },
            FieldValue::Bool(true),
        );
        values.insert(
            Key::Pref("homesection2".into()),
            FieldValue::Text("none".into()),
        );
        let saved = apply(&data, &values);
        assert_eq!(saved.config["HidePlayedInLatest"], true);
        assert_eq!(saved.config["LatestItemsExcludes"], serde_json::json!([]));
        assert_eq!(saved.config["Unrelated"], 7);
        assert_eq!(saved.display["CustomPrefs"]["homesection2"], "none");
        assert_eq!(saved.display["CustomPrefs"]["homesection1"], "nextup");
        assert_eq!(saved.display["Id"], "x");
    }
}
