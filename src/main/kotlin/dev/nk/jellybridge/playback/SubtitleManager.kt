package dev.nk.jellybridge.playback

import dev.nk.jellybridge.api.JellyfinClient
import dev.nk.jellybridge.api.models.MediaStream
import dev.nk.jellybridge.mpv.MpvIpcClient

class SubtitleManager(
    private val client: JellyfinClient,
    private val ipc: MpvIpcClient
) {
    /**
     * Filters subtitle streams from all media streams and separates them into:
     *   - text subs (SRT, ASS) → served as external files by Jellyfin
     *   - image subs (PGS, DVD) → embedded in the stream, selected by index in MPV
     */
    fun parseSubtitleStreams(streams: List<MediaStream>): List<MediaStream> =
        streams.filter { it.Type == "Subtitle" }

    /**
     * Returns the URL MPV needs to load a text-based subtitle track.
     * Pass this to MpvProcessManager.launch(subtitleUrls) for preloaded subs,
     * or to MpvIpcClient.addSubtitle() for mid-playback switching.
     */
    fun resolveSubtitleUrl(itemId: String, mediaSourceId: String, stream: MediaStream): String? {
        // Only external / text subtitles can be side-loaded as files. Image subs
        // (PGS, DVD) stay embedded and are selected by index in MPV instead.
        if (!stream.IsTextSubtitleStream && !stream.IsExternal && stream.DeliveryMethod != "External") {
            return null
        }

        // Prefer Jellyfin's pre-computed delivery URL, fall back to constructing it.
        // DeliveryUrl may already carry a query string, so pick the right separator.
        return stream.DeliveryUrl
            ?.let { path ->
                val separator = if (path.contains('?')) '&' else '?'
                "${client.serverUrl}$path${separator}api_key=${client.token}"
            }
            ?: client.subtitleUrl(itemId, mediaSourceId, stream.Index, subtitleFormat(stream.Codec))
    }

    /**
     * Switches subtitle track mid-playback.
     * For text subs: loads the external file via IPC.
     * For image subs: tells MPV to select the embedded track by index.
     */
    suspend fun switchSubtitle(itemId: String, mediaSourceId: String, stream: MediaStream) {
        if (stream.IsTextSubtitleStream) {
            val url = resolveSubtitleUrl(itemId, mediaSourceId, stream) ?: return
            ipc.addSubtitle(url)
        } else {
            ipc.setSubtitleTrack(stream.Index)
        }
    }

    private fun subtitleFormat(codec: String?): String = when (codec?.lowercase()) {
        "ass", "ssa" -> "ass"
        "webvtt", "vtt" -> "vtt"
        else -> "srt" // default — Jellyfin can convert most text subs to SRT
    }
}
