package dev.nk.jellybridge.ui.screens

import kotlin.math.min

internal const val DefaultLibraryPageSize = 60
internal val LibraryPageSizeOptions = listOf(30, 60, 120)

internal enum class LibrarySortOption(
    val settingsKey: String,
    val label: String,
    val sortBy: String,
    val sortOrder: String
) {
    NameAscending("name_asc", "Name A-Z", "SortName", "Ascending"),
    NameDescending("name_desc", "Name Z-A", "SortName", "Descending"),
    DateAddedNewest("date_added_desc", "Date added newest", "DateCreated", "Descending"),
    DateAddedOldest("date_added_asc", "Date added oldest", "DateCreated", "Ascending"),
    ReleaseDateNewest("release_date_desc", "Release date newest", "PremiereDate", "Descending"),
    ReleaseDateOldest("release_date_asc", "Release date oldest", "PremiereDate", "Ascending"),
    RuntimeShortest("runtime_asc", "Runtime shortest", "Runtime", "Ascending"),
    RuntimeLongest("runtime_desc", "Runtime longest", "Runtime", "Descending"),
    RatingHighest("rating_desc", "Rating highest", "CommunityRating", "Descending"),
    UnwatchedFirst("unwatched_first", "Unwatched first", "IsUnplayed,SortName", "Descending,Ascending"),
    WatchedFirst("watched_first", "Watched first", "IsPlayed,SortName", "Descending,Ascending");

    companion object {
        fun fromSettingsKey(settingsKey: String?): LibrarySortOption =
            entries.firstOrNull { it.settingsKey == settingsKey } ?: NameAscending
    }
}

internal data class LibraryPageInfo(
    val pageIndex: Int,
    val pageSize: Int,
    val totalRecordCount: Int,
    val loadedItemCount: Int
) {
    val totalPages: Int = libraryTotalPages(totalRecordCount, pageSize)
    val clampedPageIndex: Int = clampLibraryPageIndex(pageIndex, totalRecordCount, pageSize)
    val canGoPrevious: Boolean = clampedPageIndex > 0
    val canGoNext: Boolean = clampedPageIndex < totalPages - 1
    val rangeLabel: String = libraryRangeLabel(
        pageIndex = clampedPageIndex,
        pageSize = pageSize,
        totalRecordCount = totalRecordCount,
        loadedItemCount = loadedItemCount
    )
}

internal fun normalizedLibraryPageSize(pageSize: Int?): Int =
    pageSize?.takeIf { it in LibraryPageSizeOptions } ?: DefaultLibraryPageSize

internal fun libraryStartIndex(pageIndex: Int, pageSize: Int): Int =
    pageIndex.coerceAtLeast(0) * pageSize.coerceAtLeast(1)

internal fun libraryTotalPages(totalRecordCount: Int, pageSize: Int): Int {
    val normalizedTotal = totalRecordCount.coerceAtLeast(0)
    val normalizedPageSize = pageSize.coerceAtLeast(1)
    return if (normalizedTotal == 0) 1 else ((normalizedTotal - 1) / normalizedPageSize) + 1
}

internal fun clampLibraryPageIndex(pageIndex: Int, totalRecordCount: Int, pageSize: Int): Int =
    pageIndex.coerceIn(0, libraryTotalPages(totalRecordCount, pageSize) - 1)

internal fun libraryRangeLabel(
    pageIndex: Int,
    pageSize: Int,
    totalRecordCount: Int,
    loadedItemCount: Int
): String {
    val normalizedTotal = totalRecordCount.coerceAtLeast(0)
    val normalizedLoaded = loadedItemCount.coerceAtLeast(0)
    if (normalizedTotal == 0 || normalizedLoaded == 0) return "Showing 0 of $normalizedTotal"

    val start = libraryStartIndex(pageIndex, pageSize) + 1
    val end = min(start + normalizedLoaded - 1, normalizedTotal)
    return "Showing $start-$end of $normalizedTotal"
}
