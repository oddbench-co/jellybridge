package dev.nk.jellybridge.mpv

import kotlinx.coroutines.delay
import java.io.File

const val MPV_IPC_SOCKET = "/tmp/mpv-jellyfin.sock"

private const val MPV_PATH_ENV = "JELLYBRIDGE_MPV_PATH"

private val DEFAULT_MPV_PATHS = listOf(
    "/opt/homebrew/bin/mpv",
    "/usr/local/bin/mpv",
    "/opt/local/bin/mpv"
)

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
     *
     * External subtitles are loaded after launch via IPC sub-add (not --sub-file)
     * so we can give each track a readable title instead of its URL.
     */
    fun launch(
        streamUrl: String,
        startPositionSeconds: Double = 0.0
    ) {
        require(!isRunning()) { "MPV is already running" }

        val mpvExecutable = resolveMpvExecutable()
        val args = buildList {
            add(mpvExecutable)
            add("--input-ipc-server=$MPV_IPC_SOCKET")
            add("--no-terminal")
            add("--force-window=yes")
            add("--keep-open=yes") // don't close window when playback ends
            if (startPositionSeconds > 0) add("--start=$startPositionSeconds")
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

    private fun resolveMpvExecutable(): String {
        val configuredPath = System.getenv(MPV_PATH_ENV)
            ?.trim()
            ?.takeIf { it.isNotEmpty() }

        if (configuredPath != null) {
            val configuredFile = File(configuredPath)
            if (configuredFile.canExecute()) return configuredFile.absolutePath
            error("$MPV_PATH_ENV points to a non-executable file: $configuredPath")
        }

        DEFAULT_MPV_PATHS
            .map(::File)
            .firstOrNull { it.canExecute() }
            ?.let { return it.absolutePath }

        System.getenv("PATH")
            .orEmpty()
            .split(File.pathSeparatorChar)
            .asSequence()
            .map(String::trim)
            .filter(String::isNotEmpty)
            .map { File(it, "mpv") }
            .firstOrNull { it.canExecute() }
            ?.let { return it.absolutePath }

        return "mpv"
    }
}
