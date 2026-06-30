package dev.nk.jellybridge.api.models

import kotlinx.serialization.Serializable

@Serializable
data class ItemsResponse(
    val Items: List<LibraryItem>,
    val TotalRecordCount: Int,
    val StartIndex: Int? = null
)

data class LibraryItemsPage(
    val items: List<LibraryItem>,
    val totalRecordCount: Int,
    val startIndex: Int
)

@Serializable
data class LibraryItem(
    val Id: String,
    val Name: String,
    val Type: String,               // "CollectionFolder", "Movie", "Series", "Episode"
    val ParentId: String? = null,
    val SeriesId: String? = null,
    val SeasonId: String? = null,
    val SeasonName: String? = null,
    val ParentPrimaryImageItemId: String? = null,
    val ParentBackdropItemId: String? = null,
    val ImageTags: Map<String, String> = emptyMap(),
    val BackdropImageTags: List<String> = emptyList(),
    val UserData: UserData? = null,
    val ProductionYear: Int? = null,
    val PremiereDate: String? = null,
    val DateCreated: String? = null,
    val OfficialRating: String? = null,
    val CommunityRating: Double? = null,
    val CriticRating: Double? = null,
    val Taglines: List<String> = emptyList(),
    val Genres: List<String> = emptyList(),
    val Studios: List<NameGuidPair> = emptyList(),
    val People: List<BaseItemPerson> = emptyList(),
    val Overview: String? = null,
    val RunTimeTicks: Long? = null,
    val Width: Int? = null,
    val Height: Int? = null,
    val MediaStreams: List<MediaStream> = emptyList(),
    val SeriesName: String? = null,
    val IndexNumber: Int? = null,   // episode number
    val ParentIndexNumber: Int? = null, // season number
    val ChildCount: Int? = null,
    val RecursiveItemCount: Int? = null,
    val PrimaryImageAspectRatio: Double? = null
)

@Serializable
data class NameGuidPair(
    val Name: String,
    val Id: String? = null
)

@Serializable
data class BaseItemPerson(
    val Name: String,
    val Id: String? = null,
    val Role: String? = null,
    val Type: String? = null,
    val PrimaryImageTag: String? = null
)

@Serializable
data class UserData(
    val PlaybackPositionTicks: Long = 0,
    val PlayedPercentage: Double? = null, // server-computed, 0-100
    val Played: Boolean = false,
    val PlayCount: Int = 0,
    val LastPlayedDate: String? = null,
    val UnplayedItemCount: Int? = null
)
