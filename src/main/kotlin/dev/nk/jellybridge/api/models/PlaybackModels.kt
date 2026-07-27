package dev.nk.jellybridge.api.models

import kotlinx.serialization.Serializable

@Serializable
data class PlaybackInfoResponse(
    val MediaSources: List<MediaSource>,
    val PlaySessionId: String
)

// Sent as the POST body to /Items/{id}/PlaybackInfo. The DeviceProfile tells
// Jellyfin what the client can play directly and how it wants subtitles
// delivered — without it, external subtitle streams come back with no
// DeliveryUrl and can't be side-loaded into MPV.
@Serializable
data class PlaybackInfoRequest(
    val DeviceProfile: DeviceProfile
)

@Serializable
data class DeviceProfile(
    val Name: String,
    val MaxStreamingBitrate: Long = 120_000_000,
    val DirectPlayProfiles: List<DirectPlayProfile> = emptyList(),
    val SubtitleProfiles: List<SubtitleProfile> = emptyList()
)

@Serializable
data class DirectPlayProfile(
    val Type: String,                       // "Video" | "Audio"
    val Container: String? = null           // null / empty = any container
)

@Serializable
data class SubtitleProfile(
    val Format: String,                     // "srt", "ass", "vtt", ...
    val Method: String                      // "External" | "Embed" | "Encode"
)

@Serializable
data class MediaSource(
    val Id: String,
    val Path: String? = null,
    val Protocol: String = "File",
    val MediaStreams: List<MediaStream> = emptyList(),
    val RunTimeTicks: Long? = null,
    val Bitrate: Long? = null,
    val SupportsDirectPlay: Boolean = false,
    val SupportsDirectStream: Boolean = false,
    val DefaultAudioStreamIndex: Int? = null,
    val DefaultSubtitleStreamIndex: Int? = null
)

@Serializable
data class MediaStream(
    val Index: Int,
    val Type: String,                       // "Video" | "Audio" | "Subtitle"
    val Codec: String? = null,
    val Width: Int? = null,
    val Height: Int? = null,
    val Language: String? = null,
    val DisplayTitle: String? = null,
    val IsDefault: Boolean = false,
    val IsExternal: Boolean = false,
    val IsTextSubtitleStream: Boolean = false,
    val DeliveryMethod: String? = null,     // "External" | "Embed"
    val DeliveryUrl: String? = null
)

// Sent to POST /Sessions/Playing and /Sessions/Playing/Progress
@Serializable
data class PlaybackProgressReport(
    val ItemId: String,
    val PlaySessionId: String,
    val PositionTicks: Long,
    val IsPaused: Boolean,
    val IsMuted: Boolean = false,
    val VolumeLevel: Int = 100,
    val PlayMethod: String = "DirectPlay",
    val AudioStreamIndex: Int? = null,
    val SubtitleStreamIndex: Int? = null
)

@Serializable
data class PlaybackStopReport(
    val ItemId: String,
    val PlaySessionId: String,
    val PositionTicks: Long,
    val PlayMethod: String = "DirectPlay"
)
