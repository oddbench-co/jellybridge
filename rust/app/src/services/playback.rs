//! Jellyfin playback preparation: resolves a raw direct-stream URL through
//! PlaybackInfo, materializes authenticated external text subtitles, and
//! starts the token-hiding loopback proxy mpv plays from.
//!
//! Browsing lives in [`crate::api`]; this module reuses its server-URL
//! normalization and client identity.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api;
use crate::services::proxy::{PlaybackOrigin, ProxySession, start_proxy, strict_client};
use crate::services::session::StoredSession;

const SUBTITLE_DIRECTORY_PREFIX: &str = "jellybridge-subs-v1-";
const SUBTITLE_MARKER: &str = ".jellybridge-subtitle-cache-v1";
const MAX_SUBTITLE_BYTES: usize = 16 * 1024 * 1024;
const TEXT_SUBTITLE_CODECS: &[&str] = &["srt", "subrip", "ass", "ssa", "vtt", "webvtt"];

#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    #[error("the Jellyfin server URL is invalid")]
    InvalidServerUrl,
    #[error("the session token is empty or unreasonably large")]
    InvalidToken,
    #[error("the Jellyfin playback-info request failed")]
    PlaybackInfoRequest(#[source] reqwest::Error),
    #[error("Jellyfin returned playback-info status {0}")]
    PlaybackInfoStatus(reqwest::StatusCode),
    #[error("the Jellyfin playback-info response was invalid")]
    PlaybackInfoJson(#[source] reqwest::Error),
    #[error("Jellyfin returned no media source with an id for raw playback")]
    NoMediaSource,
    #[error("the selected media source has no id")]
    MissingMediaSourceId,
    #[error("Jellyfin returned a cross-origin subtitle URL")]
    CrossOriginSubtitle,
    #[error("an authenticated subtitle request failed")]
    SubtitleRequest(#[source] reqwest::Error),
    #[error("Jellyfin returned subtitle status {0}")]
    SubtitleStatus(reqwest::StatusCode),
    #[error("an external subtitle exceeded the 16 MiB gate limit")]
    SubtitleTooLarge,
    #[error("temporary subtitle storage failed")]
    SubtitleIo(#[source] std::io::Error),
    #[error("the loopback playback proxy failed")]
    Proxy(#[from] crate::services::proxy::ProxyError),
}

#[derive(Clone)]
pub struct PlaybackConfig {
    pub server: Url,
    pub item_id: String,
    pub user_id: String,
    token: String,
}

impl PlaybackConfig {
    pub fn from_session(
        session: &StoredSession,
        item_id: String,
    ) -> Result<Self, PlaybackError> {
        let server = api::normalize_server_url(&session.server_url)
            .map_err(|_| PlaybackError::InvalidServerUrl)?;
        let token = session.token.trim().to_owned();
        if token.is_empty() || token.len() > 64 * 1024 {
            return Err(PlaybackError::InvalidToken);
        }
        Ok(Self {
            server,
            item_id,
            user_id: session.user_id.clone(),
            token,
        })
    }

    fn authorization(&self) -> Result<HeaderValue, PlaybackError> {
        api::authorization_value(&self.token).map_err(|_| PlaybackError::InvalidToken)
    }
}

pub struct PreparedPlayback {
    pub local_url: String,
    pub subtitle_streams: Vec<PreparedSubtitle>,
    proxy: ProxySession,
    _temporary_subtitles: TemporarySubtitles,
}

#[derive(Clone, Debug)]
pub struct PreparedSubtitle {
    pub path: PathBuf,
    pub stream_index: i32,
    pub language: Option<String>,
    pub title: Option<String>,
}

impl PreparedPlayback {
    pub async fn shutdown(self) {
        self.proxy.shutdown().await;
    }
}

pub async fn prepare_playback(
    config: PlaybackConfig,
) -> Result<PreparedPlayback, PlaybackError> {
    cleanup_stale_subtitles();
    let client = strict_client().map_err(PlaybackError::PlaybackInfoRequest)?;
    let authorization = config.authorization()?;
    let playback_info = request_playback_info(&client, &config, &authorization).await?;
    let source = select_raw_source(&playback_info)?;
    let media_source_id = source
        .id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or(PlaybackError::MissingMediaSourceId)?;

    let mut temporary_subtitles = TemporarySubtitles::create()?;
    let mut subtitle_streams = Vec::new();
    for stream in source
        .media_streams
        .iter()
        .filter(|stream| stream.is_external_text())
    {
        let remote_url = subtitle_url(&config, media_source_id, stream)?;
        let extension = safe_extension(stream.codec.as_deref());
        let path = temporary_subtitles
            .download(&client, remote_url, authorization.clone(), extension)
            .await?;
        tracing::info!(
            stream_index = stream.index,
            "materialized authenticated subtitle"
        );
        subtitle_streams.push(PreparedSubtitle {
            path: path.clone(),
            stream_index: stream.index,
            language: stream.language.clone(),
            title: stream.display_title.clone(),
        });
        temporary_subtitles.files.push(path);
    }

    let stream_url = raw_stream_url(&config, media_source_id)?;
    let mut headers = HeaderMap::new();
    headers.insert("x-emby-authorization", authorization);
    let proxy = start_proxy(
        client,
        PlaybackOrigin {
            url: stream_url,
            headers,
        },
    )
    .await?;
    let local_url = proxy.local_url().as_str().to_owned();
    Ok(PreparedPlayback {
        local_url,
        subtitle_streams,
        proxy,
        _temporary_subtitles: temporary_subtitles,
    })
}

async fn request_playback_info(
    client: &reqwest::Client,
    config: &PlaybackConfig,
    authorization: &HeaderValue,
) -> Result<PlaybackInfoResponse, PlaybackError> {
    let mut url = config
        .server
        .join(&format!(
            "Items/{}/PlaybackInfo",
            encode_path_segment(&config.item_id)
        ))
        .map_err(|_| PlaybackError::InvalidServerUrl)?;
    url.query_pairs_mut().append_pair("UserId", &config.user_id);
    let response = client
        .post(url)
        .header("x-emby-authorization", authorization.clone())
        .json(&PlaybackInfoRequest {
            device_profile: mpv_device_profile(),
        })
        .send()
        .await
        .map_err(PlaybackError::PlaybackInfoRequest)?;
    if !response.status().is_success() {
        return Err(PlaybackError::PlaybackInfoStatus(response.status()));
    }
    response
        .json()
        .await
        .map_err(PlaybackError::PlaybackInfoJson)
}

fn select_raw_source(response: &PlaybackInfoResponse) -> Result<&MediaSource, PlaybackError> {
    let has_id =
        |source: &&MediaSource| source.id.as_deref().is_some_and(|id| !id.trim().is_empty());
    response
        .media_sources
        .iter()
        .filter(has_id)
        .find(|source| source.supports_direct_play)
        .or_else(|| response.media_sources.iter().find(has_id))
        .ok_or(PlaybackError::NoMediaSource)
}

fn raw_stream_url(
    config: &PlaybackConfig,
    media_source_id: &str,
) -> Result<Url, PlaybackError> {
    let mut url = config
        .server
        .join(&format!(
            "Videos/{}/stream",
            encode_path_segment(&config.item_id)
        ))
        .map_err(|_| PlaybackError::InvalidServerUrl)?;
    url.query_pairs_mut()
        .append_pair("static", "true")
        .append_pair("mediaSourceId", media_source_id);
    Ok(url)
}

fn subtitle_url(
    config: &PlaybackConfig,
    media_source_id: &str,
    stream: &MediaStream,
) -> Result<Url, PlaybackError> {
    let mut url = if let Some(delivery_url) = stream.delivery_url.as_deref() {
        config
            .server
            .join(delivery_url)
            .map_err(|_| PlaybackError::InvalidServerUrl)?
    } else {
        config
            .server
            .join(&format!(
                "Videos/{}/{}/Subtitles/{}/0/Stream.{}",
                encode_path_segment(&config.item_id),
                encode_path_segment(media_source_id),
                stream.index,
                safe_extension(stream.codec.as_deref())
            ))
            .map_err(|_| PlaybackError::InvalidServerUrl)?
    };
    if canonical_origin(&config.server) != canonical_origin(&url) {
        return Err(PlaybackError::CrossOriginSubtitle);
    }
    strip_query_credentials(&mut url);
    Ok(url)
}

fn strip_query_credentials(url: &mut Url) {
    let retained: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                "api_key" | "apikey" | "token" | "access_token"
            )
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !retained.is_empty() {
        url.query_pairs_mut().extend_pairs(retained);
    }
}

fn canonical_origin(url: &Url) -> (String, String, Option<u16>) {
    (
        url.scheme().to_ascii_lowercase(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default(),
    )
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn safe_extension(codec: Option<&str>) -> &'static str {
    match codec.unwrap_or_default().to_ascii_lowercase().as_str() {
        "ass" => "ass",
        "ssa" => "ssa",
        "vtt" | "webvtt" => "vtt",
        _ => "srt",
    }
}

#[derive(Serialize)]
struct PlaybackInfoRequest {
    #[serde(rename = "DeviceProfile")]
    device_profile: DeviceProfile,
}

#[derive(Serialize)]
struct DeviceProfile {
    #[serde(rename = "Name")]
    name: &'static str,
    #[serde(rename = "MaxStreamingBitrate")]
    max_streaming_bitrate: u64,
    #[serde(rename = "DirectPlayProfiles")]
    direct_play_profiles: Vec<DirectPlayProfile>,
    #[serde(rename = "SubtitleProfiles")]
    subtitle_profiles: Vec<SubtitleProfile>,
}

#[derive(Serialize)]
struct DirectPlayProfile {
    #[serde(rename = "Type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct SubtitleProfile {
    #[serde(rename = "Format")]
    format: &'static str,
    #[serde(rename = "Method")]
    method: &'static str,
}

/// libmpv/FFmpeg determines actual support. Leaving codec and container
/// constraints absent prevents Jellyfin from rejecting formats mpv can open.
fn mpv_device_profile() -> DeviceProfile {
    DeviceProfile {
        name: "JellyBridge mpv passthrough v1",
        max_streaming_bitrate: 120_000_000,
        direct_play_profiles: vec![
            DirectPlayProfile { kind: "Video" },
            DirectPlayProfile { kind: "Audio" },
        ],
        subtitle_profiles: TEXT_SUBTITLE_CODECS
            .iter()
            .copied()
            .map(|format| SubtitleProfile {
                format,
                method: "External",
            })
            .collect(),
    }
}

#[derive(Deserialize)]
struct PlaybackInfoResponse {
    #[serde(rename = "MediaSources", default)]
    media_sources: Vec<MediaSource>,
}

#[derive(Debug, Deserialize)]
struct MediaSource {
    #[serde(rename = "Id")]
    id: Option<String>,
    #[serde(rename = "SupportsDirectPlay", default)]
    supports_direct_play: bool,
    #[serde(rename = "MediaStreams", default)]
    media_streams: Vec<MediaStream>,
}

#[derive(Debug, Deserialize)]
struct MediaStream {
    #[serde(rename = "Index")]
    index: i32,
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "Codec")]
    codec: Option<String>,
    #[serde(rename = "Language")]
    language: Option<String>,
    #[serde(rename = "DisplayTitle")]
    display_title: Option<String>,
    #[serde(rename = "IsExternal", default)]
    is_external: bool,
    #[serde(rename = "IsTextSubtitleStream", default)]
    is_text_subtitle: bool,
    #[serde(rename = "DeliveryMethod")]
    delivery_method: Option<String>,
    #[serde(rename = "DeliveryUrl")]
    delivery_url: Option<String>,
}

impl MediaStream {
    fn is_external_text(&self) -> bool {
        let has_text_codec = normalized(self.codec.as_deref())
            .is_some_and(|codec| TEXT_SUBTITLE_CODECS.contains(&codec.as_str()));
        self.kind.eq_ignore_ascii_case("subtitle")
            && (self.is_text_subtitle || has_text_codec)
            && (self.is_external
                || self
                    .delivery_method
                    .as_deref()
                    .is_some_and(|method| method.eq_ignore_ascii_case("external")))
    }
}

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

struct TemporarySubtitles {
    directory: PathBuf,
    files: Vec<PathBuf>,
}

impl TemporarySubtitles {
    fn create() -> Result<Self, PlaybackError> {
        let directory = std::env::temp_dir().join(format!(
            "{SUBTITLE_DIRECTORY_PREFIX}{}",
            random_hex_256().map_err(PlaybackError::SubtitleIo)?
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(PlaybackError::SubtitleIo)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&directory).map_err(PlaybackError::SubtitleIo)?;

        let marker_path = directory.join(SUBTITLE_MARKER);
        let mut marker = secure_create(&marker_path)?;
        marker
            .write_all(format!("{}\n", std::process::id()).as_bytes())
            .map_err(PlaybackError::SubtitleIo)?;
        marker.sync_all().map_err(PlaybackError::SubtitleIo)?;
        Ok(Self {
            directory,
            files: Vec::new(),
        })
    }

    async fn download(
        &self,
        client: &reqwest::Client,
        url: Url,
        authorization: HeaderValue,
        extension: &str,
    ) -> Result<PathBuf, PlaybackError> {
        let response = client
            .get(url)
            .header("x-emby-authorization", authorization)
            .send()
            .await
            .map_err(PlaybackError::SubtitleRequest)?;
        if !response.status().is_success() {
            return Err(PlaybackError::SubtitleStatus(response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length as usize > MAX_SUBTITLE_BYTES)
        {
            return Err(PlaybackError::SubtitleTooLarge);
        }

        let path = self
            .directory
            .join(format!("subtitle-{}.{}", self.files.len(), extension));
        let mut file = secure_create(&path)?;
        let mut total = 0usize;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(PlaybackError::SubtitleRequest)?;
            total = total.saturating_add(chunk.len());
            if total > MAX_SUBTITLE_BYTES {
                let _ = fs::remove_file(&path);
                return Err(PlaybackError::SubtitleTooLarge);
            }
            file.write_all(&chunk)
                .map_err(PlaybackError::SubtitleIo)?;
        }
        file.sync_all().map_err(PlaybackError::SubtitleIo)?;
        Ok(path)
    }
}

impl Drop for TemporarySubtitles {
    fn drop(&mut self) {
        for path in &self.files {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(self.directory.join(SUBTITLE_MARKER));
        let _ = fs::remove_dir(&self.directory);
    }
}

fn secure_create(path: &Path) -> Result<File, PlaybackError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(PlaybackError::SubtitleIo)
}

fn random_hex_256() -> Result<String, std::io::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(result, "{byte:02x}");
    }
    Ok(result)
}

fn cleanup_stale_subtitles() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(SUBTITLE_DIRECTORY_PREFIX) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let marker = entry.path().join(SUBTITLE_MARKER);
        if fs::symlink_metadata(&marker).is_ok_and(|metadata| metadata.file_type().is_file())
            && !marker_process_is_alive(&marker)
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(unix)]
fn marker_process_is_alive(marker: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(marker) else {
        return false;
    };
    let Ok(process_id) = contents.trim().parse::<i32>() else {
        return false;
    };
    if process_id <= 0 {
        return false;
    }
    // SAFETY: signal zero performs existence/permission checking only.
    unsafe {
        libc::kill(process_id, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn marker_process_is_alive(_marker: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::net::{Ipv4Addr, SocketAddr};

    use http_body_util::Full;
    use hyper::body::{Bytes, Incoming};
    use hyper::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    async fn mock_jellyfin() -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let service = service_fn(|request: Request<Incoming>| async move {
                        let authorized = request
                            .headers()
                            .get("x-emby-authorization")
                            .and_then(|value| value.to_str().ok())
                            .is_some_and(|value| value.contains("Token=\"test-token\""));
                        if !authorized
                            || request.uri().query().is_some_and(|query| {
                                query.contains("api_key") || query.contains("access_token")
                            })
                        {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::UNAUTHORIZED)
                                    .body(Full::new(Bytes::new()))
                                    .unwrap(),
                            );
                        }

                        match request.uri().path() {
                            "/Items/item/PlaybackInfo" => {
                                let body = serde_json::json!({
                                    "MediaSources": [{
                                        "Id": "source",
                                        "SupportsDirectPlay": false,
                                        "SupportsDirectStream": true,
                                        "TranscodingUrl": "/Videos/item/master.m3u8",
                                        "Container": "mpegts",
                                        "MediaStreams": [
                                            {"Index": 0, "Type": "Video", "Codec": "vc1"},
                                            {"Index": 1, "Type": "Audio", "Codec": "dts"},
                                            {
                                                "Index": 2,
                                                "Type": "Subtitle",
                                                "Codec": "srt",
                                                "Language": "eng",
                                                "DisplayTitle": "English Signs",
                                                "IsExternal": true,
                                                "IsTextSubtitleStream": true,
                                                "DeliveryMethod": "External",
                                                "DeliveryUrl": "/Videos/item/source/Subtitles/2/0/Stream.srt"
                                            }
                                        ]
                                    }]
                                })
                                .to_string();
                                Ok(Response::builder()
                                    .status(StatusCode::OK)
                                    .header(CONTENT_LENGTH, body.len())
                                    .body(Full::new(Bytes::from(body)))
                                    .unwrap())
                            }
                            "/Videos/item/source/Subtitles/2/0/Stream.srt" => {
                                let body = Bytes::from_static(
                                    b"1\n00:00:00,000 --> 00:00:01,000\nhello\n",
                                );
                                Ok(Response::builder()
                                    .status(StatusCode::OK)
                                    .header(CONTENT_LENGTH, body.len())
                                    .body(Full::new(body))
                                    .unwrap())
                            }
                            "/Videos/item/stream" => {
                                let requested_range = request.headers().get(RANGE)
                                    == Some(&hyper::header::HeaderValue::from_static("bytes=2-5"));
                                let (status, body) = if requested_range {
                                    (StatusCode::PARTIAL_CONTENT, Bytes::from_static(b"2345"))
                                } else {
                                    (StatusCode::OK, Bytes::from_static(b"0123456789"))
                                };
                                let mut response = Response::builder()
                                    .status(status)
                                    .header(CONTENT_LENGTH, body.len());
                                if requested_range {
                                    response = response.header(CONTENT_RANGE, "bytes 2-5/10");
                                }
                                Ok(response.body(Full::new(body)).unwrap())
                            }
                            _ => Ok(Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Full::new(Bytes::new()))
                                .unwrap()),
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (address, task)
    }

    #[tokio::test]
    async fn prepares_authenticated_raw_playback_without_exposing_credentials() {
        let (address, upstream) = mock_jellyfin().await;
        let session = StoredSession {
            server_url: format!("http://{address}/"),
            token: "test-token".to_owned(),
            user_id: "user".to_owned(),
        };
        let config = PlaybackConfig::from_session(&session, "item".to_owned()).unwrap();
        let prepared = prepare_playback(config).await.unwrap();
        assert!(!prepared.local_url.contains("test-token"));
        assert!(!prepared.local_url.contains("api_key"));
        assert_eq!(prepared.subtitle_streams.len(), 1);
        assert_eq!(prepared.subtitle_streams[0].stream_index, 2);
        assert_eq!(
            prepared.subtitle_streams[0].language.as_deref(),
            Some("eng")
        );
        assert_eq!(
            prepared.subtitle_streams[0].title.as_deref(),
            Some("English Signs")
        );
        let subtitle_path = prepared.subtitle_streams[0].path.clone();
        assert!(
            fs::read_to_string(&subtitle_path)
                .unwrap()
                .contains("hello")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&subtitle_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let response = reqwest::Client::new()
            .get(&prepared.local_url)
            .header(RANGE, "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.bytes().await.unwrap(), Bytes::from_static(b"2345"));

        prepared.shutdown().await;
        assert!(!subtitle_path.exists());
        upstream.abort();
    }

    #[test]
    fn selects_raw_source_when_jellyfin_does_not_flag_direct_play() {
        let response: PlaybackInfoResponse = serde_json::from_value(serde_json::json!({
            "MediaSources": [
                {"Id": "transcode", "SupportsDirectPlay": false, "SupportsDirectStream": true},
                {
                    "Id": "direct",
                    "SupportsDirectPlay": true,
                    "Container": "mp4",
                    "MediaStreams": [
                        {"Index": 0, "Type": "Video", "Codec": "h264"},
                        {"Index": 1, "Type": "Audio", "Codec": "aac"}
                    ]
                }
            ],
            "FutureField": {"newMajor": 12}
        }))
        .unwrap();
        assert_eq!(
            select_raw_source(&response).unwrap().id.as_deref(),
            Some("direct")
        );

        let raw_fallback: PlaybackInfoResponse = serde_json::from_value(serde_json::json!({
            "MediaSources": [{"Id": "remux", "SupportsDirectPlay": false, "SupportsDirectStream": true}]
        })).unwrap();
        assert_eq!(
            select_raw_source(&raw_fallback).unwrap().id.as_deref(),
            Some("remux")
        );
    }

    #[test]
    fn accepts_unusual_formats_without_second_guessing_mpv() {
        let response: PlaybackInfoResponse = serde_json::from_value(serde_json::json!({
            "MediaSources": [{
                "Id": "raw-source",
                "SupportsDirectPlay": true,
                "Container": "mpegts",
                "MediaStreams": [
                    {"Index": 0, "Type": "Video", "Codec": "vc1"},
                    {"Index": 1, "Type": "Audio", "Codec": "dts"},
                    {"Index": 2, "Type": "Subtitle", "Codec": "pgs"}
                ]
            }]
        }))
        .unwrap();

        assert_eq!(
            select_raw_source(&response).unwrap().id.as_deref(),
            Some("raw-source")
        );
    }

    #[test]
    fn prefers_a_server_direct_play_source_from_multiple_options() {
        let response: PlaybackInfoResponse = serde_json::from_value(serde_json::json!({
            "MediaSources": [
                {
                    "Id": "raw-fallback",
                    "SupportsDirectPlay": false,
                    "Container": "avi",
                    "MediaStreams": [{"Index": 0, "Type": "Video", "Codec": "h264"}]
                },
                {
                    "Id": "supported",
                    "SupportsDirectPlay": true,
                    "Container": "mkv",
                    "MediaStreams": [
                        {"Index": 0, "Type": "Video", "Codec": "hevc"},
                        {"Index": 1, "Type": "Audio", "Codec": "eac3"},
                        {"Index": 2, "Type": "Subtitle", "Codec": "ass"}
                    ]
                }
            ]
        }))
        .unwrap();

        assert_eq!(
            select_raw_source(&response).unwrap().id.as_deref(),
            Some("supported")
        );
    }

    #[test]
    fn removes_all_known_query_credential_names() {
        let mut url = Url::parse(
            "https://jellyfin.example/sub.srt?api_key=secret&format=srt&access_token=also-secret",
        )
        .unwrap();
        strip_query_credentials(&mut url);
        assert_eq!(url.as_str(), "https://jellyfin.example/sub.srt?format=srt");
    }

    #[test]
    fn mpv_profile_does_not_restrict_containers_or_codecs() {
        let value = serde_json::to_value(mpv_device_profile()).unwrap();
        let video = &value["DirectPlayProfiles"][0];
        assert_eq!(video["Type"], "Video");
        assert!(video.get("Container").is_none());
        assert!(video.get("VideoCodec").is_none());
        assert!(video.get("AudioCodec").is_none());
    }
}
