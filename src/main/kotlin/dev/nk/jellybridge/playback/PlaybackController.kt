package dev.nk.jellybridge.playback

import dev.nk.jellybridge.api.JellyfinClient
import dev.nk.jellybridge.api.models.PlaybackProgressReport
import dev.nk.jellybridge.api.models.PlaybackStopReport
import dev.nk.jellybridge.mpv.MpvIpcClient
import dev.nk.jellybridge.mpv.MpvProcessManager
import kotlinx.coroutines.*

private const val PROGRESS_INTERVAL_MS = 10_000L // report to Jellyfin every 10 seconds

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
    suspend fun play(itemId: String) {
        val playbackInfo = client.getPlaybackInfo(itemId)
        val mediaSource = playbackInfo.MediaSources.firstOrNull()
            ?: error("No media source found for item $itemId")

        currentItemId = itemId
        currentPlaySessionId = playbackInfo.PlaySessionId

        val resumeSeconds = resumeManager.getResumePosition(itemId)
        lastKnownPositionSeconds = resumeSeconds

        val subtitleUrls = subtitleManager
            .parseSubtitleStreams(mediaSource.MediaStreams)
            .filter { it.IsTextSubtitleStream && it.IsDefault }
            .mapNotNull { subtitleManager.resolveSubtitleUrl(itemId, it) }

        val streamUrl = client.directPlayUrl(itemId, mediaSource.Id)

        processManager.launch(
            streamUrl = streamUrl,
            startPositionSeconds = resumeSeconds,
            subtitleUrls = subtitleUrls
        )

        ipc.connect()

        client.reportPlaybackStart(
            PlaybackProgressReport(
                ItemId = itemId,
                PlaySessionId = currentPlaySessionId,
                PositionTicks = resumeManager.secondsToTicks(resumeSeconds),
                IsPaused = false
            )
        )

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

        client.reportProgress(
            PlaybackProgressReport(
                ItemId = currentItemId,
                PlaySessionId = currentPlaySessionId,
                PositionTicks = resumeManager.secondsToTicks(position),
                IsPaused = isPaused
            )
        )
    }

    private suspend fun onPlaybackEnded() {
        progressJob?.cancel()
        // MPV may have already closed the socket before we get here.
        // Fall back to the last position we successfully read from IPC (seeded with
        // the resume position at start, so a brief accidental open never resets to 0).
        val finalPosition = runCatching { ipc.getCurrentPosition() }.getOrNull()
            ?: lastKnownPositionSeconds
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
}
