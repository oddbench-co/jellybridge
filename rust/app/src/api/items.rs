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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfoBlock {
    pub heading: String,
    pub rows: Vec<(String, String)>,
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
        let item: serde_json::Value = self
            .send_json(
                self.request(
                    Method::GET,
                    &format!(
                        "Users/{}/Items/{}",
                        encode_path_segment(&self.session.user_id),
                        encode_path_segment(item_id)
                    ),
                )?
                .query(&[("Fields", "MediaSources,Path")]),
                "loading media info",
            )
            .await?;
        let name = item["Name"].as_str().unwrap_or_default().to_owned();
        let mut blocks = Vec::new();
        for source in item["MediaSources"].as_array().into_iter().flatten() {
            blocks.push(InfoBlock {
                heading: source["Name"].as_str().unwrap_or("Media").to_owned(),
                rows: info_rows(
                    source,
                    &[
                        ("Container", "Container"),
                        ("Path", "Path"),
                        ("Size", "Size"),
                        ("Bitrate", "Bitrate"),
                    ],
                ),
            });
            for stream in source["MediaStreams"].as_array().into_iter().flatten() {
                let kind = stream["Type"].as_str().unwrap_or("Stream");
                blocks.push(InfoBlock {
                    heading: kind.to_owned(),
                    rows: info_rows(
                        stream,
                        &[
                            ("Title", "DisplayTitle"),
                            ("Language", "Language"),
                            ("Codec", "Codec"),
                            ("Codec tag", "CodecTag"),
                            ("Profile", "Profile"),
                            ("Level", "Level"),
                            ("Resolution", "Resolution"),
                            ("Aspect ratio", "AspectRatio"),
                            ("Anamorphic", "IsAnamorphic"),
                            ("Interlaced", "IsInterlaced"),
                            ("Framerate", "RealFrameRate"),
                            ("Bitrate", "BitRate"),
                            ("Bit depth", "BitDepth"),
                            ("Video range", "VideoRange"),
                            ("Video range type", "VideoRangeType"),
                            ("Color space", "ColorSpace"),
                            ("Color transfer", "ColorTransfer"),
                            ("Color primaries", "ColorPrimaries"),
                            ("Pixel format", "PixelFormat"),
                            ("Ref frames", "RefFrames"),
                            ("Layout", "ChannelLayout"),
                            ("Channels", "Channels"),
                            ("Sample rate", "SampleRate"),
                            ("Default", "IsDefault"),
                            ("Forced", "IsForced"),
                            ("External", "IsExternal"),
                        ],
                    ),
                });
            }
        }
        Ok((name, blocks))
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

/// Label/value rows for the keys `item` has, formatted for reading.
fn info_rows(item: &serde_json::Value, keys: &[(&str, &str)]) -> Vec<(String, String)> {
    keys.iter()
        .filter_map(|(label, key)| {
            let value = &item[*key];
            let text = match value {
                serde_json::Value::String(text) if !text.is_empty() => text.clone(),
                serde_json::Value::Bool(flag) => {
                    if !flag {
                        return None;
                    }
                    "Yes".to_owned()
                }
                serde_json::Value::Number(number) => match *key {
                    "Size" => format_bytes(number.as_f64()?),
                    "Bitrate" | "BitRate" => {
                        format!("{} kbps", (number.as_f64()? / 1000.0).round())
                    }
                    "SampleRate" => format!("{} Hz", number),
                    "RealFrameRate" => format!("{:.3}", number.as_f64()?),
                    _ => number.to_string(),
                },
                _ => return None,
            };
            Some(((*label).to_owned(), text))
        })
        .collect()
}

fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
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
    fn media_info_rows_read_like_jellyfin() {
        let stream = serde_json::json!({
            "Codec": "hevc", "BitRate": 8_000_000, "IsDefault": true, "IsForced": false,
            "SampleRate": 48000, "Size": 1_610_612_736u64
        });
        let rows = info_rows(
            &stream,
            &[
                ("Codec", "Codec"),
                ("Bitrate", "BitRate"),
                ("Default", "IsDefault"),
                ("Forced", "IsForced"),
                ("Sample rate", "SampleRate"),
                ("Size", "Size"),
                ("Missing", "Nope"),
            ],
        );
        assert_eq!(
            rows,
            vec![
                ("Codec".into(), "hevc".into()),
                ("Bitrate".into(), "8000 kbps".into()),
                ("Default".into(), "Yes".into()),
                ("Sample rate".into(), "48000 Hz".into()),
                ("Size".into(), "1.5 GB".into()),
            ]
        );
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
