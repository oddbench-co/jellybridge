package dev.nk.jellybridge.api.models

import kotlinx.serialization.Serializable

@Serializable
data class PlaybackInfoResponse(
    val MediaSources: List<MediaSource>,
    val PlaySessionId: String
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
    val SupportsDirectStream: Boolean = false
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
