//! What jellyfin-web's item menus do on the server: play queues, playlists
//! and collections, media info, the stream URL and downloads.

use reqwest::Method;
use serde::{Deserialize, Serialize};

use super::{ApiError, ItemsResponse, JellyfinApi, check_status, encode_path_segment};
use crate::features::detail::DetailItemData;
use crate::features::pages::{GridSettings, PageRoute};

/// jellyfin-web plays at most this many items from a "Play All" or
/// "Shuffle".
const QUEUE_LIMIT: &str = "300";

/// Where a play queue comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum QueueSource {
    /// A library grid, in its current sort and filters.
    Grid(Box<PageRoute>, Box<GridSettings>),
    /// Every episode of a series or season.
    Episodes(String),
    /// A collection's or folder's contents.
    Children(String),
    /// A playlist's entries, in order.
    Playlist(String),
}

/// A playlist or collection the user can add to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
}

/// One block of the media info dialog: a heading and its label/value rows.
/// A media source's block spans the dialog; its streams' blocks follow it
/// side by side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfoBlock {
    pub heading: String,
    pub rows: Vec<(String, String)>,
    pub stream: bool,
}

#[derive(Deserialize)]
struct IdOnly {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "MediaType", default)]
    media_type: Option<String>,
    #[serde(rename = "IsFolder", default)]
    is_folder: bool,
}

#[derive(Deserialize)]
struct IdsResponse {
    #[serde(rename = "Items", default)]
    items: Vec<IdOnly>,
}

#[derive(Serialize)]
struct NewPlaylist<'a> {
    #[serde(rename = "Name")]
    name: &'a str,
    #[serde(rename = "Ids")]
    ids: &'a [String],
    #[serde(rename = "UserId")]
    user_id: &'a str,
    #[serde(rename = "IsPublic")]
    is_public: bool,
}

#[derive(Deserialize)]
struct Created {
    #[serde(rename = "Id")]
    id: String,
}

impl JellyfinApi {
    /// The playable item ids a queue plays, in play order.
    pub async fn queue_ids(
        &self,
        source: &QueueSource,
        shuffle: bool,
    ) -> Result<Vec<String>, ApiError> {
        let user_items = format!("Users/{}/Items", encode_path_segment(&self.session.user_id));
        let random = if shuffle { Some("Random") } else { None };
        let response: IdsResponse = match source {
            QueueSource::Grid(route, settings) => {
                let sort = if shuffle {
                    "Random"
                } else {
                    settings.sort.as_str()
                };
                let order = if settings.descending {
                    "Descending"
                } else {
                    "Ascending"
                };
                let mut query: Vec<(&str, String)> = vec![
                    ("SortBy", sort.to_owned()),
                    ("SortOrder", order.to_owned()),
                    ("Limit", QUEUE_LIMIT.to_owned()),
                    ("Recursive", "true".to_owned()),
                    ("MediaTypes", "Video".to_owned()),
                    ("EnableImages", "false".to_owned()),
                ];
                match route.as_ref() {
                    PageRoute::Library { library, tab } => {
                        query.push(("ParentId", library.id.clone()));
                        if matches!(
                            tab,
                            crate::features::pages::LibraryTab::Movies
                                | crate::features::pages::LibraryTab::Favorites
                        ) {
                            query.push(("IncludeItemTypes", "Movie".to_owned()));
                        }
                        if *tab == crate::features::pages::LibraryTab::Favorites {
                            query.push(("Filters", "IsFavorite".to_owned()));
                        }
                    }
                    PageRoute::List(list) => {
                        if let Some(parent) = &list.parent_id {
                            query.push(("ParentId", parent.clone()));
                        }
                    }
                    _ => {}
                }
                for (key, value) in settings.filters.query() {
                    query.push((key, value));
                }
                let query = query
                    .iter()
                    .map(|(key, value)| (*key, value.as_str()))
                    .collect::<Vec<_>>();
                self.send_json(
                    self.request(Method::GET, &user_items)?.query(&query),
                    "building a play queue",
                )
                .await?
            }
            QueueSource::Episodes(parent) | QueueSource::Children(parent) => {
                let episodes = matches!(source, QueueSource::Episodes(_));
                let mut query = vec![
                    ("ParentId", parent.as_str()),
                    ("Recursive", "true"),
                    ("MediaTypes", "Video"),
                    ("Limit", QUEUE_LIMIT),
                    ("EnableImages", "false"),
                    (
                        "SortBy",
                        random.unwrap_or(if episodes {
                            "ParentIndexNumber,IndexNumber,SortName"
                        } else {
                            "PremiereDate,ProductionYear,SortName"
                        }),
                    ),
                ];
                if episodes {
                    query.push(("IncludeItemTypes", "Episode"));
                }
                self.send_json(
                    self.request(Method::GET, &user_items)?.query(&query),
                    "building a play queue",
                )
                .await?
            }
            QueueSource::Playlist(id) => {
                self.send_json(
                    self.request(
                        Method::GET,
                        &format!("Playlists/{}/Items", encode_path_segment(id)),
                    )?
                    .query(&[
                        ("UserId", self.session.user_id.as_str()),
                        ("EnableImages", "false"),
                    ]),
                    "building a play queue",
                )
                .await?
            }
        };
        let mut ids = response
            .items
            .into_iter()
            .filter(|item| {
                !item.is_folder
                    && item
                        .media_type
                        .as_deref()
                        .is_none_or(|kind| kind == "Video")
            })
            .map(|item| item.id)
            .collect::<Vec<_>>();
        if shuffle && matches!(source, QueueSource::Playlist(_)) {
            shuffle_in_place(&mut ids);
        }
        Ok(ids)
    }

    /// Items by id, with what the player needs to show them.
    pub async fn detail_items(&self, ids: &[String]) -> Result<Vec<DetailItemData>, ApiError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let joined = ids.join(",");
        let response: ItemsResponse = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                )?
                .query(&[
                    ("Ids", joined.as_str()),
                    ("Fields", super::DETAIL_FIELDS),
                    ("EnableUserData", "true"),
                    ("EnableImageTypes", "Primary,Backdrop,Logo,Thumb"),
                ]),
                "loading items",
            )
            .await?;
        // The server returns them in its own order; keep the caller's.
        let mut items = response
            .items
            .into_iter()
            .map(DetailItemData::from)
            .collect::<Vec<_>>();
        items.sort_by_key(|item| ids.iter().position(|id| *id == item.id));
        Ok(items)
    }

    /// The user's playlists or collections, by name.
    pub async fn containers(&self, kind: &str) -> Result<Vec<Container>, ApiError> {
        let response: IdsResponse = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!("Users/{}/Items", encode_path_segment(&self.session.user_id)),
                )?
                .query(&[
                    ("IncludeItemTypes", kind),
                    ("Recursive", "true"),
                    ("SortBy", "SortName"),
                    ("EnableImages", "false"),
                    ("Fields", "MediaType"),
                ]),
                "loading playlists",
            )
            .await?;
        Ok(response
            .items
            .into_iter()
            .map(|item| Container {
                id: item.id,
                name: item.name,
            })
            .collect())
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        ids: &[String],
        public: bool,
    ) -> Result<String, ApiError> {
        let created: Created = self
            .send_json(
                self.request(Method::POST, "Playlists")?.json(&NewPlaylist {
                    name,
                    ids,
                    user_id: &self.session.user_id,
                    is_public: public,
                }),
                "creating a playlist",
            )
            .await?;
        Ok(created.id)
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, ids: &[String]) -> Result<(), ApiError> {
        let joined = ids.join(",");
        let response = self
            .request(
                Method::POST,
                &format!("Playlists/{}/Items", encode_path_segment(playlist_id)),
            )?
            .query(&[
                ("Ids", joined.as_str()),
                ("UserId", self.session.user_id.as_str()),
            ])
            .send()
            .await?;
        check_status(response, "adding to a playlist")?;
        Ok(())
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        entry_ids: &[String],
    ) -> Result<(), ApiError> {
        let joined = entry_ids.join(",");
        let response = self
            .request(
                Method::DELETE,
                &format!("Playlists/{}/Items", encode_path_segment(playlist_id)),
            )?
            .query(&[("EntryIds", joined.as_str())])
            .send()
            .await?;
        check_status(response, "removing from a playlist")?;
        Ok(())
    }

    pub async fn create_collection(&self, name: &str, ids: &[String]) -> Result<String, ApiError> {
        let joined = ids.join(",");
        let created: Created = self
            .send_json(
                self.request(Method::POST, "Collections")?
                    .query(&[("Name", name), ("Ids", joined.as_str())]),
                "creating a collection",
            )
            .await?;
        Ok(created.id)
    }

    pub async fn add_to_collection(
        &self,
        collection_id: &str,
        ids: &[String],
    ) -> Result<(), ApiError> {
        let joined = ids.join(",");
        let response = self
            .request(
                Method::POST,
                &format!("Collections/{}/Items", encode_path_segment(collection_id)),
            )?
            .query(&[("Ids", joined.as_str())])
            .send()
            .await?;
        check_status(response, "adding to a collection")?;
        Ok(())
    }

    /// jellyfin-web's Media Info: the file, then each stream.
    pub async fn media_info(&self, item_id: &str) -> Result<(String, Vec<InfoBlock>), ApiError> {
        let user = encode_path_segment(&self.session.user_id);
        let item = self.send_json::<serde_json::Value>(
            self.request(
                Method::GET,
                &format!("Users/{user}/Items/{}", encode_path_segment(item_id)),
            )?,
            "loading media info",
        );
        // jellyfin-web shows the file path to administrators only.
        let me = self.send_json::<serde_json::Value>(
            self.request(Method::GET, &format!("Users/{user}"))?,
            "loading media info",
        );
        let (item, me) = tokio::try_join!(item, me)?;
        let admin = me["Policy"]["IsAdministrator"].as_bool().unwrap_or(false);
        let name = item["Name"].as_str().unwrap_or_default().to_owned();
        Ok((name, media_info_blocks(&item, admin)))
    }

    /// The direct stream URL jellyfin-web's "Copy Stream URL" copies. It
    /// carries the access token, as jellyfin-web's does.
    pub fn stream_url(&self, item_id: &str) -> String {
        format!(
            "{}Videos/{}/stream?static=true&api_key={}",
            self.server,
            encode_path_segment(item_id),
            self.session.token
        )
    }

    /// Starts a download of the item's original file.
    pub async fn download(&self, item_id: &str) -> Result<reqwest::Response, ApiError> {
        let response = self
            .request(
                Method::GET,
                &format!("Items/{}/Download", encode_path_segment(item_id)),
            )?
            .send()
            .await?;
        check_status(response, "downloading")
    }
}

/// jellyfin-web's media info (itemMediaInfo): per media source, a block
/// with its name, container, formats, path (administrators only) and size,
/// then a block per stream (data streams left out), tracks in jellyfin's
/// order, rows as it writes them.
fn media_info_blocks(item: &serde_json::Value, admin: bool) -> Vec<InfoBlock> {
    use serde_json::Value;
    let text = |value: &Value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(js_number(number)),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    };
    // JavaScript truthiness for the fields jellyfin tests with `&&`.
    let truthy = |value: &Value| match value {
        Value::String(text) => !text.is_empty(),
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::Bool(flag) => *flag,
        _ => false,
    };
    let yes_no = |flag: bool| if flag { "Yes" } else { "No" }.to_owned();
    let mut blocks = Vec::new();
    for source in item["MediaSources"].as_array().into_iter().flatten() {
        let mut rows = Vec::new();
        if truthy(&source["Container"]) {
            rows.push(("Container".to_owned(), text(&source["Container"]).unwrap_or_default()));
        }
        if let Some(formats) = source["Formats"].as_array().filter(|formats| !formats.is_empty()) {
            let joined = formats.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",");
            rows.push(("Format".to_owned(), joined));
        }
        if admin && truthy(&source["Path"]) {
            rows.push(("Path".to_owned(), text(&source["Path"]).unwrap_or_default()));
        }
        if let Some(size) = source["Size"].as_f64().filter(|size| *size != 0.0) {
            rows.push(("Size".to_owned(), readable_size(size)));
        }
        blocks.push(InfoBlock {
            heading: source["Name"].as_str().unwrap_or_default().to_owned(),
            rows,
            stream: false,
        });
        let mut streams: Vec<&Value> = source["MediaStreams"].as_array().into_iter().flatten().collect();
        // jellyfin's sortTracks: embedded first, then forced, then default,
        // then by index.
        let flag = |stream: &Value, key: &str| i64::from(stream[key].as_bool().unwrap_or(false));
        streams.sort_by_key(|stream| {
            (
                flag(stream, "IsExternal"),
                -flag(stream, "IsForced"),
                -flag(stream, "IsDefault"),
                stream["Index"].as_i64().unwrap_or(0),
            )
        });
        for stream in streams {
            let kind = stream["Type"].as_str().unwrap_or_default();
            let heading = match kind {
                "Data" => continue,
                "EmbeddedImage" => "Image",
                other => other,
            };
            let video = kind == "Video";
            let mut rows: Vec<(String, String)> = Vec::new();
            let mut add = |label: &str, value: String| rows.push((label.to_owned(), value));
            let field = |key: &str| &stream[key];
            if truthy(field("DisplayTitle")) {
                add("Title", text(field("DisplayTitle")).unwrap_or_default());
            }
            if truthy(field("Language")) && !video {
                add("Language", text(field("Language")).unwrap_or_default());
            }
            if truthy(field("Codec")) {
                add("Codec", text(field("Codec")).unwrap_or_default().to_uppercase());
            }
            if truthy(field("CodecTag")) {
                add("Codec tag", text(field("CodecTag")).unwrap_or_default());
            }
            if let Some(avc) = field("IsAVC").as_bool() {
                add("AVC", yes_no(avc));
            }
            if truthy(field("Profile")) {
                add("Profile", text(field("Profile")).unwrap_or_default());
            }
            if field("Level").as_f64().is_some_and(|level| level > 0.0) {
                add("Level", text(field("Level")).unwrap_or_default());
            }
            if truthy(field("Width")) || truthy(field("Height")) {
                let side = |key: &str| text(field(key)).unwrap_or_else(|| "undefined".to_owned());
                add("Resolution", format!("{}x{}", side("Width"), side("Height")));
            }
            if truthy(field("AspectRatio")) && field("Codec").as_str() != Some("mjpeg") {
                add("Aspect ratio", text(field("AspectRatio")).unwrap_or_default());
            }
            if video {
                if let Some(anamorphic) = field("IsAnamorphic").as_bool() {
                    add("Anamorphic", yes_no(anamorphic));
                }
                add("Interlaced", yes_no(field("IsInterlaced").as_bool().unwrap_or(false)));
            }
            let frame_rate = if truthy(field("AverageFrameRate")) { field("AverageFrameRate") } else { field("RealFrameRate") };
            if truthy(frame_rate) && video {
                add("Framerate", text(frame_rate).unwrap_or_default());
            }
            if truthy(field("ChannelLayout")) {
                add("Layout", text(field("ChannelLayout")).unwrap_or_default());
            }
            if truthy(field("Channels")) {
                add("Channels", format!("{} ch", text(field("Channels")).unwrap_or_default()));
            }
            if let Some(bitrate) = field("BitRate").as_f64().filter(|bitrate| *bitrate != 0.0) {
                add("Bitrate", format!("{} kbps", (bitrate / 1000.0).trunc()));
            }
            if truthy(field("SampleRate")) {
                add("Sample rate", format!("{} Hz", text(field("SampleRate")).unwrap_or_default()));
            }
            if truthy(field("BitDepth")) {
                add("Bit depth", format!("{} bit", text(field("BitDepth")).unwrap_or_default()));
            }
            if video && truthy(field("VideoRange")) {
                add("Video range", text(field("VideoRange")).unwrap_or_default());
            }
            if video && truthy(field("VideoRangeType")) {
                add("Video range type", text(field("VideoRangeType")).unwrap_or_default());
            }
            if truthy(field("VideoDoViTitle")) {
                add("DV title", text(field("VideoDoViTitle")).unwrap_or_default());
                for (label, key) in [
                    ("DV version major", "DvVersionMajor"),
                    ("DV version minor", "DvVersionMinor"),
                    ("DV profile", "DvProfile"),
                    ("DV level", "DvLevel"),
                    ("DV rpu preset flag", "RpuPresentFlag"),
                    ("DV el preset flag", "ElPresentFlag"),
                    ("DV bl preset flag", "BlPresentFlag"),
                    ("DV bl signal compatibility id", "DvBlSignalCompatibilityId"),
                ] {
                    if let Some(value) = text(field(key)) {
                        add(label, value);
                    }
                }
            }
            for (label, key) in [
                ("Color space", "ColorSpace"),
                ("Color transfer", "ColorTransfer"),
                ("Color primaries", "ColorPrimaries"),
                ("Pixel format", "PixelFormat"),
                ("Ref frames", "RefFrames"),
            ] {
                if truthy(field(key)) {
                    add(label, text(field(key)).unwrap_or_default());
                }
            }
            if video && truthy(field("Rotation")) {
                add("Rotation", text(field("Rotation")).unwrap_or_default());
            }
            if truthy(field("NalLengthSize")) {
                add("NAL", text(field("NalLengthSize")).unwrap_or_default());
            }
            if matches!(kind, "Subtitle" | "Audio") {
                add("Default", yes_no(field("IsDefault").as_bool().unwrap_or(false)));
                add("Forced", yes_no(field("IsForced").as_bool().unwrap_or(false)));
                add("External", yes_no(field("IsExternal").as_bool().unwrap_or(false)));
            }
            if video && truthy(&source["Timestamp"]) {
                add("Timestamp", text(&source["Timestamp"]).unwrap_or_default());
            }
            blocks.push(InfoBlock {
                heading: heading.to_owned(),
                rows,
                stream: true,
            });
        }
    }
    blocks
}

/// A JSON number as JavaScript prints it.
fn js_number(number: &serde_json::Number) -> String {
    match number.as_f64() {
        Some(value) if value.fract() == 0.0 && value.abs() < 1e21 => format!("{value:.0}"),
        Some(value) => value.to_string(),
        None => number.to_string(),
    }
}

/// jellyfin-web's getReadableSize: 1024 steps, one decimal, "GiB".
fn readable_size(bytes: f64) -> String {
    let power = (bytes.ln() / 1024f64.ln()).floor().max(0.0) as i32;
    let value = bytes / 1024f64.powi(power);
    let unit = match power {
        0 => "Bytes".to_owned(),
        power => format!("{}iB", "KMGTPEZY".chars().nth(power as usize - 1).unwrap_or('Y')),
    };
    format!("{value:.1} {unit}")
}

/// A Fisher–Yates shuffle on a clock-seeded xorshift: playlists have no
/// server-side random order.
fn shuffle_in_place(ids: &mut [String]) {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9e37_79b9, |since| since.as_nanos() as u64)
        | 1;
    for index in (1..ids.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let other = (seed % (index as u64 + 1)) as usize;
        ids.swap(index, other);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_info_reads_like_jellyfin() {
        let item = serde_json::json!({ "MediaSources": [{
            "Name": "Film", "Container": "mkv", "Path": "/movies/film.mkv", "Size": 35_540_000_000u64,
            "MediaStreams": [
                { "Type": "Subtitle", "Index": 3, "Codec": "subrip", "Language": "eng", "IsExternal": true },
                { "Type": "Audio", "Index": 1, "Codec": "eac3", "Channels": 6, "BitRate": 640_500,
                  "SampleRate": 48000, "IsDefault": true },
                { "Type": "Video", "Index": 0, "Codec": "hevc", "IsAVC": false, "Level": 153, "IsDefault": true,
                  "Width": 3840, "Height": 1584, "IsInterlaced": false, "RealFrameRate": 23.976 },
                { "Type": "Data", "Index": 4 }
            ]
        }]});
        let blocks = media_info_blocks(&item, false);
        assert_eq!(blocks.len(), 4);
        assert_eq!(
            blocks[0].rows,
            vec![("Container".into(), "mkv".into()), ("Size".into(), "33.1 GiB".into())]
        );
        assert_eq!(
            blocks.iter().map(|block| block.heading.as_str()).collect::<Vec<_>>(),
            ["Film", "Video", "Audio", "Subtitle"]
        );
        assert_eq!(
            blocks[1].rows,
            vec![
                ("Codec".into(), "HEVC".into()),
                ("AVC".into(), "No".into()),
                ("Level".into(), "153".into()),
                ("Resolution".into(), "3840x1584".into()),
                ("Interlaced".into(), "No".into()),
                ("Framerate".into(), "23.976".into()),
            ]
        );
        assert!(blocks[2].rows.contains(&("Bitrate".into(), "640 kbps".into())));
        assert!(blocks[2].rows.contains(&("Channels".into(), "6 ch".into())));
        assert!(blocks[3].rows.contains(&("External".into(), "Yes".into())));
        assert!(media_info_blocks(&item, true)[0].rows.contains(&("Path".into(), "/movies/film.mkv".into())));
    }

    #[test]
    fn shuffling_keeps_every_item() {
        let mut ids = (0..50).map(|n| n.to_string()).collect::<Vec<_>>();
        shuffle_in_place(&mut ids);
        let mut sorted = ids.clone();
        sorted.sort_by_key(|id| id.parse::<u32>().unwrap());
        assert_eq!(sorted, (0..50).map(|n| n.to_string()).collect::<Vec<_>>());
    }
}
