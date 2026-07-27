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

data class MpvTrack(
    val id: Int,
    val type: String,
    val language: String? = null,
    val title: String? = null,
    val external: Boolean = false,
    val externalFilename: String? = null,
    val ffIndex: Int? = null,
    val selected: Boolean = false
)

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
            putJsonArray("command") {
                args.forEach { arg ->
                    add(
                        when (arg) {
                            is JsonElement -> arg
                            is Number -> JsonPrimitive(arg)
                            is Boolean -> JsonPrimitive(arg)
                            else -> JsonPrimitive(arg.toString())
                        }
                    )
                }
            }
            put("request_id", id)
        }.toString() + "\n"

        // MPV may have already closed the socket (e.g. user quit mpv).
        // Wrap the entire send/receive in runCatching so we return null instead of crashing.
        runCatching { ch.write(ByteBuffer.wrap(payload.toByteArray())) }.getOrElse { return@withContext null }

        // Read response — MPV writes one JSON line per response
        val buf = ByteBuffer.allocate(4096)
        val sb = StringBuilder()
        var responseLine: String? = null
        var timeout = 50 // ~2.5s at 50ms each
        while (timeout-- > 0 && responseLine == null) {
            buf.clear()
            val n = runCatching { ch.read(buf) }.getOrElse { return@withContext null }
            if (n > 0) {
                sb.append(String(buf.array(), 0, n))
                responseLine = sb
                    .lines()
                    .firstOrNull {
                        it.contains("\"request_id\":$id") ||
                            it.contains("\"request_id\": $id")
                    }
            } else {
                delay(50)
            }
        }

        runCatching {
            val line = responseLine ?: return@withContext null
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

    suspend fun getTrackList(): List<MpvTrack> {
        return sendCommand("get_property", "track-list")
            ?.jsonArray
            ?.mapNotNull { element ->
                val track = element.jsonObject
                val id = track["id"]?.jsonPrimitive?.intOrNull ?: return@mapNotNull null
                val type = track["type"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
                MpvTrack(
                    id = id,
                    type = type,
                    language = track["lang"]?.jsonPrimitive?.contentOrNull,
                    title = track["title"]?.jsonPrimitive?.contentOrNull,
                    external = track["external"]?.jsonPrimitive?.booleanOrNull ?: false,
                    externalFilename = track["external-filename"]?.jsonPrimitive?.contentOrNull,
                    ffIndex = track["ff-index"]?.jsonPrimitive?.intOrNull,
                    selected = track["selected"]?.jsonPrimitive?.booleanOrNull ?: false
                )
            }
            ?: emptyList()
    }

    suspend fun setAudioTrack(trackId: Int) {
        sendCommand("set_property", "aid", trackId)
    }

    /**
     * Loads an external subtitle file.
     *
     * MPV's sub-add takes positional args: <url> <flags> <title> <lang>.
     * Passing a title makes MPV's subtitle menu show a readable name instead of
     * deriving one from the URL. Use select=false to preload without stealing
     * selection (track picking is handled separately).
     */
    suspend fun addSubtitle(url: String, title: String? = null, lang: String? = null, select: Boolean = true) {
        val flags = if (select) "select" else "auto"
        // sub-add is positional, so a lang requires a (possibly empty) title before it.
        val effectiveTitle = title ?: if (lang != null) "" else null
        when {
            effectiveTitle != null && lang != null -> sendCommand("sub-add", url, flags, effectiveTitle, lang)
            effectiveTitle != null -> sendCommand("sub-add", url, flags, effectiveTitle)
            else -> sendCommand("sub-add", url, flags)
        }
    }

    // Switch to a subtitle track by MPV track id.
    suspend fun setSubtitleTrack(index: Int) {
        sendCommand("set_property", "sid", index)
    }

    suspend fun disableSubtitles() {
        sendCommand("set_property", "sid", "no")
    }

    suspend fun pause() = sendCommand("set_property", "pause", "yes")
    suspend fun resume() = sendCommand("set_property", "pause", "no")
    suspend fun seek(seconds: Double) = sendCommand("seek", seconds, "absolute")
}
