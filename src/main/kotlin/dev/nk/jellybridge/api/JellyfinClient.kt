package dev.nk.jellybridge.api

import dev.nk.jellybridge.api.models.*
import io.ktor.client.*
import io.ktor.client.call.*
import io.ktor.client.engine.cio.*
import io.ktor.client.plugins.contentnegotiation.*
import io.ktor.client.request.*
import io.ktor.http.*
import io.ktor.serialization.kotlinx.json.*
import kotlinx.serialization.json.Json

// Client ID sent in every request so Jellyfin identifies our app
private const val CLIENT_NAME = "JellyBridge"
private const val CLIENT_VERSION = "0.1.0"
private const val DEVICE_NAME = "Desktop"
private const val DEVICE_ID = "jellybridge-desktop"
private const val ITEM_CARD_FIELDS =
    "ParentId,SeriesId,SeasonId,SeasonName,ParentPrimaryImageItemId,ParentBackdropItemId,ImageTags,UserData,Overview,RunTimeTicks,MediaStreams,ChildCount,RecursiveItemCount,PrimaryImageAspectRatio"
private const val ITEM_DETAIL_FIELDS =
    "ParentId,SeriesId,SeasonId,SeasonName,ParentPrimaryImageItemId,ParentBackdropItemId,ImageTags,BackdropImageTags,UserData,Overview,RunTimeTicks,MediaStreams,Genres,People,Studios,OfficialRating,CommunityRating,CriticRating,Taglines,PremiereDate,DateCreated,ChildCount,RecursiveItemCount,PrimaryImageAspectRatio"
private const val DASHBOARD_ITEM_TYPES = "Movie,Episode,Series"

class JellyfinClient(
    val serverUrl: String,
    var token: String = "",
    var userId: String = ""
) {
    private val http = HttpClient(CIO) {
        install(ContentNegotiation) {
            json(Json { ignoreUnknownKeys = true })
        }
    }

    // Authorization header value required by Jellyfin for every request
    private fun authHeader(): String {
        val base = """MediaBrowser Client="$CLIENT_NAME", Device="$DEVICE_NAME", DeviceId="$DEVICE_ID", Version="$CLIENT_VERSION""""
        return if (token.isNotBlank()) "$base, Token=\"$token\"" else base
    }

    private fun HttpRequestBuilder.applyItemCardParameters(
        limit: Int? = null,
        imageTypes: String = "Primary"
    ) {
        header("X-Emby-Authorization", authHeader())
        parameter("Fields", ITEM_CARD_FIELDS)
        parameter("EnableUserData", true)
        parameter("EnableImages", true)
        parameter("ImageTypeLimit", 1)
        parameter("EnableImageTypes", imageTypes)
        if (limit != null) parameter("Limit", limit)
    }

    // ── Auth ─────────────────────────────────────────────────────────────────

    suspend fun authenticate(username: String, password: String): AuthResponse {
        val response: AuthResponse = http.post("$serverUrl/Users/AuthenticateByName") {
            header("X-Emby-Authorization", authHeader())
            contentType(ContentType.Application.Json)
            setBody(AuthRequest(Username = username, Pw = password))
        }.body()

        token = response.AccessToken
        userId = response.User.Id
        return response
    }

    // ── Library ───────────────────────────────────────────────────────────────

    // Top-level libraries (Movies, TV Shows, Music, etc.)
    suspend fun getLibraries(): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters()
            parameter("Recursive", false)
        }.body<ItemsResponse>().Items
    }

    suspend fun getItems(parentId: String, searchTerm: String? = null): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters()
            parameter("ParentId", parentId)
            parameter("Recursive", false)
            parameter("SortBy", "SortName")
            parameter("SortOrder", "Ascending")
            if (searchTerm != null) parameter("SearchTerm", searchTerm)
        }.body<ItemsResponse>().Items
    }

    suspend fun getItemsPage(
        parentId: String,
        startIndex: Int,
        limit: Int,
        sortBy: String,
        sortOrder: String
    ): LibraryItemsPage {
        val response = http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters(limit)
            parameter("ParentId", parentId)
            parameter("Recursive", false)
            parameter("StartIndex", startIndex)
            parameter("EnableTotalRecordCount", true)
            parameter("SortBy", sortBy)
            parameter("SortOrder", sortOrder)
        }.body<ItemsResponse>()

        return response.toLibraryItemsPage(defaultStartIndex = startIndex)
    }

    suspend fun getFilteredItems(
        parentId: String? = null,
        searchTerm: String? = null,
        recursive: Boolean = true,
        includeItemTypes: String? = DASHBOARD_ITEM_TYPES,
        isPlayed: Boolean? = null,
        isFavorite: Boolean? = null,
        limit: Int? = null
    ): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters(limit)
            if (!parentId.isNullOrBlank()) parameter("ParentId", parentId)
            parameter("Recursive", recursive)
            parameter("SortBy", "SortName")
            parameter("SortOrder", "Ascending")
            if (!includeItemTypes.isNullOrBlank()) parameter("IncludeItemTypes", includeItemTypes)
            if (!searchTerm.isNullOrBlank()) parameter("SearchTerm", searchTerm)
            if (isPlayed != null) parameter("IsPlayed", isPlayed)
            if (isFavorite != null) parameter("IsFavorite", isFavorite)
        }.body<ItemsResponse>().Items
    }

    suspend fun getFilteredItemsPage(
        parentId: String? = null,
        searchTerm: String? = null,
        recursive: Boolean = true,
        includeItemTypes: String? = DASHBOARD_ITEM_TYPES,
        isPlayed: Boolean? = null,
        isFavorite: Boolean? = null,
        startIndex: Int,
        limit: Int,
        sortBy: String,
        sortOrder: String
    ): LibraryItemsPage {
        val response = http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters(limit)
            if (!parentId.isNullOrBlank()) parameter("ParentId", parentId)
            parameter("Recursive", recursive)
            parameter("StartIndex", startIndex)
            parameter("EnableTotalRecordCount", true)
            parameter("SortBy", sortBy)
            parameter("SortOrder", sortOrder)
            if (!includeItemTypes.isNullOrBlank()) parameter("IncludeItemTypes", includeItemTypes)
            if (!searchTerm.isNullOrBlank()) parameter("SearchTerm", searchTerm)
            if (isPlayed != null) parameter("IsPlayed", isPlayed)
            if (isFavorite != null) parameter("IsFavorite", isFavorite)
        }.body<ItemsResponse>()

        return response.toLibraryItemsPage(defaultStartIndex = startIndex)
    }

    suspend fun getSeasonEpisodes(seasonId: String): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters(imageTypes = "Primary,Thumb")
            parameter("ParentId", seasonId)
            parameter("Recursive", false)
            parameter("IncludeItemTypes", "Episode")
            parameter("SortBy", "ParentIndexNumber,IndexNumber,SortName")
            parameter("SortOrder", "Ascending")
        }.body<ItemsResponse>().Items
            .sortedWith(
                compareBy<LibraryItem> { it.ParentIndexNumber ?: Int.MAX_VALUE }
                    .thenBy { it.IndexNumber ?: Int.MAX_VALUE }
                    .thenBy { it.Name.lowercase() }
            )
    }

    suspend fun getSeriesSeasons(seriesId: String): List<LibraryItem> {
        return http.get("$serverUrl/Shows/$seriesId/Seasons") {
            header("X-Emby-Authorization", authHeader())
            parameter("UserId", userId)
            parameter("Fields", ITEM_CARD_FIELDS)
            parameter("EnableUserData", true)
            parameter("EnableImages", true)
            parameter("ImageTypeLimit", 1)
            parameter("EnableImageTypes", "Primary")
        }.body<ItemsResponse>().Items
            .sortedWith(
                compareBy<LibraryItem> { it.IndexNumber ?: Int.MAX_VALUE }
                    .thenBy { it.Name.lowercase() }
            )
    }

    suspend fun getContinueWatching(limit: Int = 20): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items/Resume") {
            applyItemCardParameters(limit)
            parameter("StartIndex", 0)
            parameter("MediaTypes", "Video")
            parameter("IncludeItemTypes", "Movie,Episode")
        }.body<ItemsResponse>().Items
    }

    suspend fun getNextUp(limit: Int = 20): List<LibraryItem> {
        return http.get("$serverUrl/Shows/NextUp") {
            applyItemCardParameters(limit)
            parameter("UserId", userId)
            parameter("StartIndex", 0)
            parameter("EnableTotalRecordCount", false)
        }.body<ItemsResponse>().Items
    }

    suspend fun getLatestMedia(limit: Int = 20): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items/Latest") {
            applyItemCardParameters(limit)
            parameter("IncludeItemTypes", DASHBOARD_ITEM_TYPES)
            parameter("GroupItems", false)
        }.body()
    }

    suspend fun getFavoriteItems(limit: Int = 20): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters(limit)
            parameter("Recursive", true)
            parameter("IncludeItemTypes", DASHBOARD_ITEM_TYPES)
            parameter("IsFavorite", true)
            parameter("SortBy", "SortName")
            parameter("SortOrder", "Ascending")
        }.body<ItemsResponse>().Items
    }

    suspend fun getRecentlyAdded(limit: Int = 20): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            applyItemCardParameters(limit)
            parameter("Recursive", true)
            parameter("IncludeItemTypes", DASHBOARD_ITEM_TYPES)
            parameter("SortBy", "DateCreated")
            parameter("SortOrder", "Descending")
        }.body<ItemsResponse>().Items
    }

    suspend fun getItem(itemId: String): LibraryItem {
        return http.get("$serverUrl/Users/$userId/Items/$itemId") {
            header("X-Emby-Authorization", authHeader())
            parameter("Fields", ITEM_CARD_FIELDS)
        }.body()
    }

    suspend fun getItemDetails(itemId: String): LibraryItem {
        return http.get("$serverUrl/Users/$userId/Items/$itemId") {
            header("X-Emby-Authorization", authHeader())
            parameter("Fields", ITEM_DETAIL_FIELDS)
            parameter("EnableUserData", true)
            parameter("EnableImages", true)
            parameter("ImageTypeLimit", 1)
            parameter("EnableImageTypes", "Primary,Backdrop,Logo,Thumb")
        }.body()
    }

    suspend fun setPlayedState(itemId: String, played: Boolean) {
        val response = if (played) {
            http.post("$serverUrl/UserPlayedItems/$itemId") {
                header("X-Emby-Authorization", authHeader())
                parameter("userId", userId)
            }
        } else {
            http.delete("$serverUrl/UserPlayedItems/$itemId") {
                header("X-Emby-Authorization", authHeader())
                parameter("userId", userId)
            }
        }
        if (response.status.value !in 200..299) {
            throw IllegalStateException("Failed to update watched state (${response.status.value})")
        }
    }

    // ── Playback info (needed for subtitle stream metadata) ───────────────────

    suspend fun getPlaybackInfo(itemId: String): PlaybackInfoResponse {
        return http.post("$serverUrl/Items/$itemId/PlaybackInfo") {
            header("X-Emby-Authorization", authHeader())
            parameter("UserId", userId)
            contentType(ContentType.Application.Json)
            setBody(PlaybackInfoRequest(DeviceProfile = defaultDeviceProfile()))
        }.body()
    }

    // Minimal profile: allow direct play of any video/audio, and declare that we
    // can consume common text subtitle formats as external side-loaded files.
    // This makes Jellyfin populate DeliveryMethod="External" + DeliveryUrl for
    // external subtitle tracks so MPV can load them via --sub-file.
    private fun defaultDeviceProfile(): DeviceProfile = DeviceProfile(
        Name = CLIENT_NAME,
        DirectPlayProfiles = listOf(
            DirectPlayProfile(Type = "Video"),
            DirectPlayProfile(Type = "Audio")
        ),
        SubtitleProfiles = listOf("srt", "subrip", "ass", "ssa", "vtt", "webvtt")
            .map { SubtitleProfile(Format = it, Method = "External") }
    )

    // ── Direct play URL ───────────────────────────────────────────────────────

    // Constructs the URL for MPV — Jellyfin serves the raw file as-is
    fun directPlayUrl(itemId: String, mediaSourceId: String): String =
        "$serverUrl/Videos/$itemId/stream?static=true&mediaSourceId=$mediaSourceId&api_key=$token"

    // Subtitle file URL for a given stream index. Jellyfin's subtitle route
    // requires the mediaSourceId segment, otherwise it 404s.
    fun subtitleUrl(itemId: String, mediaSourceId: String, streamIndex: Int, format: String = "srt"): String =
        "$serverUrl/Videos/$itemId/$mediaSourceId/Subtitles/$streamIndex/0/Stream.$format?api_key=$token"

    // ── Session reporting ─────────────────────────────────────────────────────

    suspend fun reportPlaybackStart(report: PlaybackProgressReport) {
        http.post("$serverUrl/Sessions/Playing") {
            header("X-Emby-Authorization", authHeader())
            contentType(ContentType.Application.Json)
            setBody(report)
        }
    }

    suspend fun reportProgress(report: PlaybackProgressReport) {
        http.post("$serverUrl/Sessions/Playing/Progress") {
            header("X-Emby-Authorization", authHeader())
            contentType(ContentType.Application.Json)
            setBody(report)
        }
    }

    suspend fun reportPlaybackStopped(report: PlaybackStopReport) {
        http.post("$serverUrl/Sessions/Playing/Stopped") {
            header("X-Emby-Authorization", authHeader())
            contentType(ContentType.Application.Json)
            setBody(report)
        }
    }

    // ── Image URL helper ──────────────────────────────────────────────────────

    fun imageUrl(itemId: String, imageType: String, maxWidth: Int = 400): String =
        "$serverUrl/Items/$itemId/Images/$imageType?maxWidth=$maxWidth&api_key=$token"

    fun posterUrl(itemId: String, maxWidth: Int = 400): String =
        imageUrl(itemId, "Primary", maxWidth)

    fun backdropUrl(itemId: String, maxWidth: Int = 1400): String =
        imageUrl(itemId, "Backdrop", maxWidth)

    fun thumbUrl(itemId: String, maxWidth: Int = 700): String =
        imageUrl(itemId, "Thumb", maxWidth)

    fun logoUrl(itemId: String, maxWidth: Int = 600): String =
        imageUrl(itemId, "Logo", maxWidth)

    fun close() = http.close()
}

private fun ItemsResponse.toLibraryItemsPage(defaultStartIndex: Int): LibraryItemsPage =
    LibraryItemsPage(
        items = Items,
        totalRecordCount = TotalRecordCount,
        startIndex = StartIndex?.takeIf { it >= 0 } ?: defaultStartIndex
    )
