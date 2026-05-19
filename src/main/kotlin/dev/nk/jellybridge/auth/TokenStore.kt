package dev.nk.jellybridge.auth

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.io.File

@Serializable
data class StoredSession(
    val serverUrl: String,
    val token: String,
    val userId: String
)

/**
 * Persists the Jellyfin session (server URL + access token + user ID) to disk so
 * the user doesn't have to log in every time.
 *
 * Jellyfin access tokens don't expire on their own — they stay valid until the user
 * logs out or an admin revokes them, so we keep the token indefinitely.
 *
     * Storage: ~/.config/jellybridge/session.json
 */
object TokenStore {
    private val file: File = File(
        System.getProperty("user.home"),
        ".config/jellybridge/session.json"
    )

    fun save(session: StoredSession) {
        file.parentFile.mkdirs()
        file.writeText(Json.encodeToString(StoredSession.serializer(), session))
    }

    fun load(): StoredSession? = runCatching {
        if (!file.exists()) return null
        Json.decodeFromString(StoredSession.serializer(), file.readText())
    }.getOrNull()

    fun clear() {
        file.delete()
    }
}
