package dev.nk.jellybridge.mpv

import kotlinx.coroutines.delay

const val MPV_IPC_SOCKET = "/tmp/mpv-jellyfin.sock"

class MpvProcessManager {
    private var process: Process? = null

    fun isRunning(): Boolean = process?.isAlive == true

    /**
     * Launches MPV with the given stream URL.
     *
     * Key flags:
     *   --input-ipc-server  — opens a Unix socket we connect to for JSON-RPC control
     *   --no-terminal       — don't steal our terminal
     *   --start             — seek to resume position on open
     *   --sub-file          — load external subtitle files (one per flag)
     */
    fun launch(
        streamUrl: String,
        startPositionSeconds: Double = 0.0,
        subtitleUrls: List<String> = emptyList()
    ) {
        require(!isRunning()) { "MPV is already running" }

        val args = buildList {
            add("mpv")
            add("--input-ipc-server=$MPV_IPC_SOCKET")
            add("--no-terminal")
            add("--force-window=yes")
            add("--keep-open=yes") // don't close window when playback ends
            if (startPositionSeconds > 0) add("--start=$startPositionSeconds")
            subtitleUrls.forEach { add("--sub-file=$it") }
            add(streamUrl)
        }

        process = ProcessBuilder(args)
            .inheritIO()
            .start()
    }

    fun kill() {
        process?.destroy()
        process = null
    }

    /**
     * Waits for MPV to exit, then returns.
     * Call this in a coroutine to detect when the user closes the MPV window.
     */
    suspend fun awaitExit(): Int {
        val p = process ?: return -1
        while (p.isAlive) delay(500)
        val code = p.exitValue()
        process = null
        return code
    }
}
