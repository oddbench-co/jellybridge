package dev.nk.jellybridge.playback

import dev.nk.jellybridge.api.models.LibraryItem
import dev.nk.jellybridge.api.models.MediaSource
import dev.nk.jellybridge.api.models.MediaStream
import dev.nk.jellybridge.settings.SettingsStore
import dev.nk.jellybridge.settings.StoredPlaybackPreference

data class PlaybackStreamOverride(
    val audioStreamIndex: Int? = null,
    val subtitleStreamIndex: Int? = null
)

data class PlaybackStreamSelection(
    val audioStream: MediaStream?,
    val subtitleStream: MediaStream?,
    val subtitlesDisabled: Boolean
) {
    val audioStreamIndex: Int? = audioStream?.Index

    val subtitleStreamIndex: Int
        get() = when {
            subtitlesDisabled -> -1
            subtitleStream != null -> subtitleStream.Index
            else -> -1
        }
}

object PlaybackStreamPreferences {
    fun preferenceKey(item: LibraryItem?, itemId: String): String {
        return when {
            item == null -> "item:$itemId"
            item.Type == "Episode" && !item.SeriesId.isNullOrBlank() -> "series:${item.SeriesId}"
            item.Type == "Movie" -> "movie:${item.Id}"
            else -> "item:${item.Id}"
        }
    }

    fun storedPreference(key: String): StoredPlaybackPreference? =
        SettingsStore.load()?.playbackPreferences?.get(key)

    fun resolve(
        mediaSource: MediaSource,
        preference: StoredPlaybackPreference?,
        override: PlaybackStreamOverride? = null
    ): PlaybackStreamSelection {
        val fallbackAudio = preference
            ?.let { findPreferredStream(mediaSource.MediaStreams, "Audio", it.audioLanguage, it.audioTitle) }
            ?: mediaSource.defaultAudioStream()
        val audioStream = override
            ?.audioStreamIndex
            ?.let { index -> mediaSource.MediaStreams.firstOrNull { it.Type == "Audio" && it.Index == index } }
            ?: fallbackAudio

        val preferredSubtitleStream = preference
            ?.takeUnless { it.subtitlesDisabled }
            ?.let {
                findPreferredStream(
                    mediaSource.MediaStreams,
                    "Subtitle",
                    it.subtitleLanguage,
                    it.subtitleTitle
                )
            }
        val fallbackSubtitleDisabled = preference?.subtitlesDisabled == true ||
            (preferredSubtitleStream == null && mediaSource.DefaultSubtitleStreamIndex == -1)
        val fallbackSubtitle = if (fallbackSubtitleDisabled) {
            null
        } else {
            preferredSubtitleStream ?: mediaSource.defaultSubtitleStream()
        }

        val overrideSubtitleIndex = override?.subtitleStreamIndex
        val subtitleStream = when {
            overrideSubtitleIndex == -1 -> null
            overrideSubtitleIndex != null -> {
                mediaSource.MediaStreams.firstOrNull {
                    it.Type == "Subtitle" && it.Index == overrideSubtitleIndex
                } ?: fallbackSubtitle
            }
            else -> fallbackSubtitle
        }

        return PlaybackStreamSelection(
            audioStream = audioStream,
            subtitleStream = subtitleStream,
            subtitlesDisabled = overrideSubtitleIndex == -1 || subtitleStream == null
        )
    }

    fun savePreference(
        key: String,
        mediaStreams: List<MediaStream>,
        audioStreamIndex: Int?,
        subtitleStreamIndex: Int?
    ) {
        val audioStream = audioStreamIndex?.let { index ->
            mediaStreams.firstOrNull { it.Type == "Audio" && it.Index == index }
        }
        val subtitleStream = subtitleStreamIndex?.let { index ->
            mediaStreams.firstOrNull { it.Type == "Subtitle" && it.Index == index }
        }
        val hasSubtitleOptions = mediaStreams.any { it.Type == "Subtitle" }

        SettingsStore.update { settings ->
            val previous = settings.playbackPreferences[key] ?: StoredPlaybackPreference()
            var next = previous

            if (audioStream != null) {
                next = next.copy(
                    audioLanguage = audioStream.Language.cleanPreferenceValue(),
                    audioTitle = audioStream.DisplayTitle.cleanPreferenceValue()
                )
            }

            next = when {
                subtitleStream != null -> next.copy(
                    subtitleLanguage = subtitleStream.Language.cleanPreferenceValue(),
                    subtitleTitle = subtitleStream.DisplayTitle.cleanPreferenceValue(),
                    subtitlesDisabled = false
                )
                subtitleStreamIndex == -1 && hasSubtitleOptions -> next.copy(
                    subtitlesDisabled = true
                )
                else -> next
            }

            if (next == previous) {
                settings
            } else {
                settings.copy(playbackPreferences = settings.playbackPreferences + (key to next))
            }
        }
    }

    fun streamLabel(stream: MediaStream, fallbackPrefix: String): String {
        return listOfNotNull(
            stream.Language?.uppercase()?.takeIf { it.isNotBlank() },
            stream.DisplayTitle?.takeIf { it.isNotBlank() },
            stream.Codec?.uppercase()?.takeIf { it.isNotBlank() }
        ).distinct().joinToString(" - ").ifBlank { "$fallbackPrefix ${stream.Index}" }
    }

    private fun findPreferredStream(
        streams: List<MediaStream>,
        type: String,
        language: String?,
        title: String?
    ): MediaStream? {
        val normalizedLanguage = language.normalizePreference()
        val normalizedTitle = title.normalizePreference()
        if (normalizedLanguage == null && normalizedTitle == null) return null

        return streams
            .filter { it.Type == type }
            .map { stream ->
                val score = stream.preferenceMatchScore(language, title)
                stream to score
            }
            .filter { it.second > 0 }
            .maxByOrNull { it.second }
            ?.first
    }

    private fun MediaSource.defaultAudioStream(): MediaStream? =
        DefaultAudioStreamIndex
            ?.let { index -> MediaStreams.firstOrNull { it.Type == "Audio" && it.Index == index } }
            ?: MediaStreams.firstOrNull { it.Type == "Audio" && it.IsDefault }
            ?: MediaStreams.firstOrNull { it.Type == "Audio" }

    private fun MediaSource.defaultSubtitleStream(): MediaStream? =
        DefaultSubtitleStreamIndex
            ?.takeIf { it >= 0 }
            ?.let { index -> MediaStreams.firstOrNull { it.Type == "Subtitle" && it.Index == index } }

    private fun MediaStream.preferenceMatchScore(language: String?, title: String?): Int {
        val normalizedLanguage = language.normalizePreference()
        val normalizedTitle = title.normalizePreference()
        var score = 0
        if (normalizedLanguage != null && Language.normalizePreference() == normalizedLanguage) score += 4
        if (normalizedTitle != null && DisplayTitle.normalizePreference() == normalizedTitle) score += 2
        return score
    }
}

internal fun String?.normalizePreference(): String? =
    cleanPreferenceValue()?.lowercase()

internal fun String?.cleanPreferenceValue(): String? =
    this?.trim()?.takeIf { it.isNotEmpty() }
