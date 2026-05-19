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
            header("X-Emby-Authorization", authHeader())
            parameter("Recursive", false)
            parameter("Fields", "ImageTags,UserData,RunTimeTicks,MediaStreams")
        }.body<ItemsResponse>().Items
    }

    suspend fun getItems(parentId: String, searchTerm: String? = null): List<LibraryItem> {
        return http.get("$serverUrl/Users/$userId/Items") {
            header("X-Emby-Authorization", authHeader())
            parameter("ParentId", parentId)
            parameter("Recursive", false)
            parameter("Fields", "ImageTags,UserData,Overview,RunTimeTicks,MediaStreams")
            parameter("SortBy", "SortName")
            parameter("SortOrder", "Ascending")
            if (searchTerm != null) parameter("SearchTerm", searchTerm)
        }.body<ItemsResponse>().Items
    }

    suspend fun getItem(itemId: String): LibraryItem {
        return http.get("$serverUrl/Users/$userId/Items/$itemId") {
            header("X-Emby-Authorization", authHeader())
            parameter("Fields", "ImageTags,UserData,RunTimeTicks,MediaStreams")
        }.body()
    }

    // ── Playback info (needed for subtitle stream metadata) ───────────────────

    suspend fun getPlaybackInfo(itemId: String): PlaybackInfoResponse {
        return http.get("$serverUrl/Items/$itemId/PlaybackInfo") {
            header("X-Emby-Authorization", authHeader())
            parameter("UserId", userId)
        }.body()
    }

    // ── Direct play URL ───────────────────────────────────────────────────────

    // Constructs the URL for MPV — Jellyfin serves the raw file as-is
    fun directPlayUrl(itemId: String, mediaSourceId: String): String =
        "$serverUrl/Videos/$itemId/stream?static=true&mediaSourceId=$mediaSourceId&api_key=$token"

    // Subtitle file URL for a given stream index
    fun subtitleUrl(itemId: String, streamIndex: Int, format: String = "srt"): String =
        "$serverUrl/Videos/$itemId/Subtitles/$streamIndex/0/Stream.$format?api_key=$token"

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

    fun posterUrl(itemId: String, maxWidth: Int = 400): String =
        "$serverUrl/Items/$itemId/Images/Primary?maxWidth=$maxWidth&api_key=$token"

    fun close() = http.close()
}
