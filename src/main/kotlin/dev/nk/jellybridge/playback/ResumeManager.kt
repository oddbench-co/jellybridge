package dev.nk.jellybridge.playback

import dev.nk.jellybridge.api.JellyfinClient

// Jellyfin uses 100-nanosecond ticks — 10,000,000 ticks = 1 second
private const val TICKS_PER_SECOND = 10_000_000L

// Mark as watched when playback reaches this fraction of total duration
private const val WATCHED_THRESHOLD = 0.90

class ResumeManager(private val client: JellyfinClient) {
    fun ticksToSeconds(ticks: Long): Double = ticks.toDouble() / TICKS_PER_SECOND
    fun secondsToTicks(seconds: Double): Long = (seconds * TICKS_PER_SECOND).toLong()

    /**
     * Returns the resume position in seconds for the given item,
     * or 0.0 if the item has never been played.
     */
    suspend fun getResumePosition(itemId: String): Double {
        val item = client.getItem(itemId)
        val ticks = item.UserData?.PlaybackPositionTicks ?: 0L
        return ticksToSeconds(ticks)
    }

    /**
     * Returns true if the current position is past the watched threshold.
     * Used to automatically mark an item as played when MPV exits.
     */
    fun isNearEnd(positionSeconds: Double, durationSeconds: Double): Boolean {
        if (durationSeconds <= 0) return false
        return positionSeconds / durationSeconds >= WATCHED_THRESHOLD
    }
}
