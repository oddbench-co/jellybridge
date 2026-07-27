package dev.nk.jellybridge.playback

import dev.nk.jellybridge.api.JellyfinClient
import dev.nk.jellybridge.api.models.MediaStream
import dev.nk.jellybridge.api.models.PlaybackProgressReport
import dev.nk.jellybridge.api.models.PlaybackStopReport
import dev.nk.jellybridge.mpv.MpvIpcClient
import dev.nk.jellybridge.mpv.MpvTrack
import dev.nk.jellybridge.mpv.MpvProcessManager
import kotlinx.coroutines.*

private const val PROGRESS_INTERVAL_MS = 10_000L // report to Jellyfin every 10 seconds

private data class SubtitleFile(
    val stream: MediaStream,
    val url: String
)

class PlaybackController(
    private val client: JellyfinClient,
    private val processManager: MpvProcessManager,
    private val ipc: MpvIpcClient,
    private val resumeManager: ResumeManager,
    private val subtitleManager: SubtitleManager,
    private val scope: CoroutineScope
) {
    private var progressJob: Job? = null
    private var currentItemId: String = ""
    private var currentPlaySessionId: String = ""
    private var currentPreferenceKey: String? = null
    private var currentMediaStreams: List<MediaStream> = emptyList()
    private var currentSubtitleFiles: List<SubtitleFile> = emptyList()
    private var currentAudioStreamIndex: Int? = null
    private var currentSubtitleStreamIndex: Int? = null
    // Tracks the last successfully read position so we don't report 0 when
    // MPV closes before the first periodic report fires (e.g. brief accidental open).
    private var lastKnownPositionSeconds: Double = 0.0

    /**
     * Full playback launch sequence:
     * 1. Fetch playback info (subtitle streams, media source ID)
     * 2. Resolve resume position
     * 3. Resolve subtitle URLs for text-based tracks
     * 4. Launch MPV subprocess
     * 5. Connect IPC socket
     * 6. Report playback start to Jellyfin
     * 7. Start progress polling loop
     * 8. Await MPV exit → report stop
     */
    suspend fun play(itemId: String, override: PlaybackStreamOverride? = null) {
        val item = runCatching { client.getItem(itemId) }.getOrNull()
        val playbackInfo = client.getPlaybackInfo(itemId)
        val mediaSource = playbackInfo.MediaSources.firstOrNull()
            ?: error("No media source found for item $itemId")

        currentItemId = itemId
        currentPlaySessionId = playbackInfo.PlaySessionId
        currentPreferenceKey = PlaybackStreamPreferences.preferenceKey(item, itemId)
        currentMediaStreams = mediaSource.MediaStreams
        currentSubtitleFiles = subtitleManager
            .parseSubtitleStreams(mediaSource.MediaStreams)
            // Side-load anything deliverable as an external file. Don't gate on
            // IsTextSubtitleStream alone — external .srt files sitting next to the
            // movie must reach MPV via --sub-file or they never appear as tracks.
            .filter { it.IsTextSubtitleStream || it.IsExternal || it.DeliveryMethod == "External" }
            .mapNotNull { stream ->
                subtitleManager.resolveSubtitleUrl(itemId, mediaSource.Id, stream)?.let { url ->
                    SubtitleFile(stream, url)
                }
            }

        val preferenceKey = currentPreferenceKey
        val storedPreference = preferenceKey?.let { PlaybackStreamPreferences.storedPreference(it) }
        val streamSelection = PlaybackStreamPreferences.resolve(mediaSource, storedPreference, override)
        currentAudioStreamIndex = streamSelection.audioStream?.Index
        currentSubtitleStreamIndex = streamSelection.subtitleStreamIndex
        if (override != null && preferenceKey != null) {
            PlaybackStreamPreferences.savePreference(
                key = preferenceKey,
                mediaStreams = currentMediaStreams,
                audioStreamIndex = currentAudioStreamIndex,
                subtitleStreamIndex = currentSubtitleStreamIndex
            )
        }

        val resumeSeconds = resumeManager.getResumePosition(itemId)
        lastKnownPositionSeconds = resumeSeconds

        val streamUrl = client.directPlayUrl(itemId, mediaSource.Id)

        processManager.launch(
            streamUrl = streamUrl,
            startPositionSeconds = resumeSeconds
        )

        ipc.connect()
        loadExternalSubtitles()
        applyPlaybackSelection(streamSelection)
        syncSelectedStreamsFromMpv(savePreference = false)

        client.reportPlaybackStart(
            progressReport(positionSeconds = resumeSeconds, isPaused = false)
        )
        runCatching { client.reportProgress(progressReport(positionSeconds = resumeSeconds, isPaused = false)) }

        startProgressReporting()

        // Wait for user to close MPV window
        scope.launch {
            processManager.awaitExit()
            onPlaybackEnded()
        }
    }

    private fun startProgressReporting() {
        progressJob = scope.launch {
            while (processManager.isRunning()) {
                delay(PROGRESS_INTERVAL_MS)
                reportCurrentProgress()
            }
        }
    }

    private suspend fun reportCurrentProgress() {
        val position = ipc.getCurrentPosition() ?: return
        lastKnownPositionSeconds = position
        val isPaused = ipc.isPaused()
        syncSelectedStreamsFromMpv(savePreference = true)

        client.reportProgress(progressReport(positionSeconds = position, isPaused = isPaused))
    }

    private suspend fun onPlaybackEnded() {
        progressJob?.cancel()
        // MPV may have already closed the socket before we get here.
        // Fall back to the last position we successfully read from IPC (seeded with
        // the resume position at start, so a brief accidental open never resets to 0).
        val finalPosition = runCatching { ipc.getCurrentPosition() }.getOrNull()
            ?: lastKnownPositionSeconds
        runCatching { syncSelectedStreamsFromMpv(savePreference = true) }
        runCatching {
            client.reportProgress(
                progressReport(
                    positionSeconds = finalPosition,
                    isPaused = runCatching { ipc.isPaused() }.getOrDefault(false)
                )
            )
        }
        ipc.disconnect()

        client.reportPlaybackStopped(
            PlaybackStopReport(
                ItemId = currentItemId,
                PlaySessionId = currentPlaySessionId,
                PositionTicks = resumeManager.secondsToTicks(finalPosition)
            )
        )
    }

    fun stop() {
        progressJob?.cancel()
        processManager.kill()
        ipc.disconnect()
    }

    private fun progressReport(positionSeconds: Double, isPaused: Boolean): PlaybackProgressReport =
        PlaybackProgressReport(
            ItemId = currentItemId,
            PlaySessionId = currentPlaySessionId,
            PositionTicks = resumeManager.secondsToTicks(positionSeconds),
            IsPaused = isPaused,
            AudioStreamIndex = currentAudioStreamIndex,
            SubtitleStreamIndex = currentSubtitleStreamIndex
        )

    // Preload external subtitle files via IPC so each gets a readable title in
    // MPV's subtitle menu. select=false: track selection happens afterwards.
    private suspend fun loadExternalSubtitles() {
        currentSubtitleFiles.forEach { sub ->
            val title = sub.stream.DisplayTitle?.takeIf { it.isNotBlank() }
                ?: PlaybackStreamPreferences.streamLabel(sub.stream, "Subtitle")
            ipc.addSubtitle(
                url = sub.url,
                title = title,
                lang = sub.stream.Language?.takeIf { it.isNotBlank() },
                select = false
            )
        }
    }

    private suspend fun applyPlaybackSelection(selection: PlaybackStreamSelection) {
        val tracks = waitForMpvTracks()
        selection.audioStream
            ?.let { findMpvTrackForStream(it, tracks) }
            ?.let { ipc.setAudioTrack(it.id) }

        when {
            selection.subtitlesDisabled -> ipc.disableSubtitles()
            selection.subtitleStream != null -> {
                val track = findMpvTrackForStream(selection.subtitleStream, tracks)
                if (track != null) {
                    ipc.setSubtitleTrack(track.id)
                } else if (!selection.subtitleStream.IsTextSubtitleStream) {
                    ipc.setSubtitleTrack(selection.subtitleStream.Index)
                }
            }
            else -> ipc.disableSubtitles()
        }
    }

    private suspend fun waitForMpvTracks(): List<MpvTrack> {
        repeat(10) {
            val tracks = ipc.getTrackList()
            if (tracks.isNotEmpty()) return tracks
            delay(200)
        }
        return emptyList()
    }

    private suspend fun syncSelectedStreamsFromMpv(savePreference: Boolean) {
        val tracks = ipc.getTrackList()
        if (tracks.isEmpty()) return

        val selectedAudioStream = tracks
            .firstOrNull { it.selected && it.type == "audio" }
            ?.let { findMediaStreamForMpvTrack(it, "Audio") }
        val selectedSubtitleTrack = tracks.firstOrNull { it.selected && it.type == "sub" }
        val selectedSubtitleStream = selectedSubtitleTrack?.let { findMediaStreamForMpvTrack(it, "Subtitle") }

        val nextAudioIndex = selectedAudioStream?.Index ?: currentAudioStreamIndex
        val nextSubtitleIndex = when {
            selectedSubtitleTrack == null -> -1
            selectedSubtitleStream != null -> selectedSubtitleStream.Index
            else -> currentSubtitleStreamIndex
        }

        val changed = nextAudioIndex != currentAudioStreamIndex ||
            nextSubtitleIndex != currentSubtitleStreamIndex

        currentAudioStreamIndex = nextAudioIndex
        currentSubtitleStreamIndex = nextSubtitleIndex

        if (changed && savePreference) {
            saveCurrentPlaybackPreference()
        }
    }

    private fun saveCurrentPlaybackPreference() {
        val key = currentPreferenceKey ?: return
        PlaybackStreamPreferences.savePreference(
            key = key,
            mediaStreams = currentMediaStreams,
            audioStreamIndex = currentAudioStreamIndex,
            subtitleStreamIndex = currentSubtitleStreamIndex
        )
    }

    private fun findMpvTrackForStream(stream: MediaStream, tracks: List<MpvTrack>): MpvTrack? {
        val type = if (stream.Type == "Audio") "audio" else "sub"
        val candidates = tracks.filter { it.type == type }

        if (stream.Type == "Subtitle") {
            val subtitleUrl = currentSubtitleFiles.firstOrNull { it.stream.Index == stream.Index }?.url
            if (subtitleUrl != null) {
                candidates.firstOrNull { it.externalFilename == subtitleUrl }?.let { return it }
            }
        }

        return candidates.firstOrNull { it.ffIndex == stream.Index }
            ?: candidates.firstOrNull { it.id == stream.Index }
            ?: candidates
                .map { it to it.preferenceMatchScore(stream.Language, stream.DisplayTitle) }
                .filter { it.second > 0 }
                .maxByOrNull { it.second }
                ?.first
    }

    private fun findMediaStreamForMpvTrack(track: MpvTrack, type: String): MediaStream? {
        if (type == "Subtitle" && track.externalFilename != null) {
            currentSubtitleFiles
                .firstOrNull { it.url == track.externalFilename }
                ?.stream
                ?.let { return it }
        }

        val candidates = currentMediaStreams.filter { it.Type == type }
        return candidates.firstOrNull { track.ffIndex == it.Index }
            ?: candidates.firstOrNull { track.id == it.Index }
            ?: candidates
                .map { it to track.preferenceMatchScore(it.Language, it.DisplayTitle) }
                .filter { it.second > 0 }
                .maxByOrNull { it.second }
                ?.first
    }

    private fun MpvTrack.preferenceMatchScore(language: String?, title: String?): Int {
        val normalizedLanguage = language.normalizePreference()
        val normalizedTitle = title.normalizePreference()
        var score = 0
        if (normalizedLanguage != null && this.language.normalizePreference() == normalizedLanguage) score += 4
        if (normalizedTitle != null && this.title.normalizePreference() == normalizedTitle) score += 2
        return score
    }
}
