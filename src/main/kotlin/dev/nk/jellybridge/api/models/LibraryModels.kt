package dev.nk.jellybridge.api.models

import kotlinx.serialization.Serializable

@Serializable
data class ItemsResponse(
    val Items: List<LibraryItem>,
    val TotalRecordCount: Int
)

@Serializable
data class LibraryItem(
    val Id: String,
    val Name: String,
    val Type: String,               // "CollectionFolder", "Movie", "Series", "Episode"
    val ImageTags: Map<String, String> = emptyMap(),
    val UserData: UserData? = null,
    val ProductionYear: Int? = null,
    val Overview: String? = null,
    val RunTimeTicks: Long? = null,
    val Width: Int? = null,
    val Height: Int? = null,
    val MediaStreams: List<MediaStream> = emptyList(),
    val SeriesName: String? = null,
    val IndexNumber: Int? = null,   // episode number
    val ParentIndexNumber: Int? = null // season number
)

@Serializable
data class UserData(
    val PlaybackPositionTicks: Long = 0,
    val PlayedPercentage: Double? = null, // server-computed, 0-100
    val Played: Boolean = false,
    val PlayCount: Int = 0,
    val LastPlayedDate: String? = null
)
