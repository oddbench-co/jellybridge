package dev.nk.jellybridge.mpv

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import java.io.File
import java.net.StandardProtocolFamily
import java.net.UnixDomainSocketAddress
import java.nio.ByteBuffer
import java.nio.channels.SocketChannel

/**
 * JSON-RPC client over MPV's Unix domain socket.
 *
 * MPV IPC protocol: newline-delimited JSON objects.
 * Request:  {"command": ["get_property", "time-pos"], "request_id": 1}
 * Response: {"data": 42.3, "error": "success", "request_id": 1}
 *
 * Docs: https://mpv.io/manual/stable/#json-ipc
 */
class MpvIpcClient(private val socketPath: String = MPV_IPC_SOCKET) {
    private var channel: SocketChannel? = null
    private var requestId = 0

    suspend fun connect(retries: Int = 10, delayMs: Long = 300) {
        // Close any stale channel from a previous session before reconnecting
        channel?.runCatching { close() }
        channel = null

        repeat(retries) { attempt ->
            if (File(socketPath).exists()) {
                runCatching {
                    val addr = UnixDomainSocketAddress.of(socketPath)
                    channel = SocketChannel.open(StandardProtocolFamily.UNIX)
                    channel!!.connect(addr)
                    channel!!.configureBlocking(false)
                    return
                }
            }
            if (attempt < retries - 1) delay(delayMs)
        }
        error("Could not connect to MPV IPC socket at $socketPath after $retries attempts")
    }

    fun disconnect() {
        channel?.close()
        channel = null
        File(socketPath).delete()
    }

    // ── Low-level send/receive ────────────────────────────────────────────────

    private suspend fun sendCommand(vararg args: Any): JsonElement? = withContext(Dispatchers.IO) {
        val ch = channel ?: return@withContext null
        val id = ++requestId

        val payload = buildJsonObject {
            putJsonArray("command") { args.forEach { add(JsonPrimitive(it.toString())) } }
            put("request_id", id)
        }.toString() + "\n"

        // MPV may have already closed the socket (e.g. user quit mpv).
        // Wrap the entire send/receive in runCatching so we return null instead of crashing.
        runCatching { ch.write(ByteBuffer.wrap(payload.toByteArray())) }.getOrElse { return@withContext null }

        // Read response — MPV writes one JSON line per response
        val buf = ByteBuffer.allocate(4096)
        val sb = StringBuilder()
        var timeout = 50 // ~2.5s at 50ms each
        while (timeout-- > 0) {
            buf.clear()
            val n = runCatching { ch.read(buf) }.getOrElse { return@withContext null }
            if (n > 0) {
                sb.append(String(buf.array(), 0, n))
                if (sb.contains('\n')) break
            } else {
                delay(50)
            }
        }

        runCatching {
            val line = sb.lines().firstOrNull { it.contains("request_id") } ?: return@withContext null
            val obj = Json.parseToJsonElement(line).jsonObject
            if (obj["error"]?.jsonPrimitive?.content == "success") obj["data"] else null
        }.getOrNull()
    }

    // ── High-level helpers ────────────────────────────────────────────────────

    suspend fun getCurrentPosition(): Double? =
        sendCommand("get_property", "time-pos")?.jsonPrimitive?.doubleOrNull

    suspend fun isPaused(): Boolean =
        sendCommand("get_property", "pause")?.jsonPrimitive?.booleanOrNull ?: false

    suspend fun getDuration(): Double? =
        sendCommand("get_property", "duration")?.jsonPrimitive?.doubleOrNull

    // Load and immediately select an external subtitle file
    suspend fun addSubtitle(url: String) {
        sendCommand("sub-add", url, "select")
    }

    // Switch to a subtitle track by stream index (for embedded subs)
    suspend fun setSubtitleTrack(index: Int) {
        sendCommand("set_property", "sid", index)
    }

    suspend fun pause() = sendCommand("set_property", "pause", "yes")
    suspend fun resume() = sendCommand("set_property", "pause", "no")
    suspend fun seek(seconds: Double) = sendCommand("seek", seconds, "absolute")
}
