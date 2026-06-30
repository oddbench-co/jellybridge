package dev.nk.jellybridge.settings

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.io.File

@Serializable
data class StoredSettings(
    val cardWidthDp: Int? = null,
    val librarySortKey: String? = null,
    val libraryPageSize: Int? = null,
    val playbackPreferences: Map<String, StoredPlaybackPreference> = emptyMap()
)

@Serializable
data class StoredPlaybackPreference(
    val audioLanguage: String? = null,
    val audioTitle: String? = null,
    val subtitleLanguage: String? = null,
    val subtitleTitle: String? = null,
    val subtitlesDisabled: Boolean = false
)

/**
 * Persists local app preferences that should survive logout.
 *
 * Storage: ~/.config/jellybridge/settings.json
 */
object SettingsStore {
    private val json = Json {
        ignoreUnknownKeys = true
        prettyPrint = true
    }

    private val file: File = File(
        System.getProperty("user.home"),
        ".config/jellybridge/settings.json"
    )

    fun save(settings: StoredSettings) {
        file.parentFile.mkdirs()
        file.writeText(json.encodeToString(StoredSettings.serializer(), settings))
    }

    fun load(): StoredSettings? = runCatching {
        if (!file.exists()) return null
        json.decodeFromString(StoredSettings.serializer(), file.readText())
    }.getOrNull()

    fun update(transform: (StoredSettings) -> StoredSettings) {
        save(transform(load() ?: StoredSettings()))
    }
}
