package dev.nk.jellybridge.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.pointerHoverIcon
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.nk.jellybridge.api.JellyfinClient
import dev.nk.jellybridge.api.models.BaseItemPerson
import dev.nk.jellybridge.api.models.LibraryItem
import dev.nk.jellybridge.api.models.MediaSource
import dev.nk.jellybridge.mpv.MpvIpcClient
import dev.nk.jellybridge.mpv.MpvProcessManager
import dev.nk.jellybridge.playback.PlaybackController
import dev.nk.jellybridge.playback.PlaybackStreamOverride
import dev.nk.jellybridge.playback.PlaybackStreamPreferences
import dev.nk.jellybridge.playback.ResumeManager
import dev.nk.jellybridge.playback.SubtitleManager
import dev.nk.jellybridge.settings.SettingsStore
import dev.nk.jellybridge.ui.components.AsyncImage
import dev.nk.jellybridge.ui.theme.JellyfinBlue
import dev.nk.jellybridge.ui.theme.OnSurfaceDim
import dev.nk.jellybridge.ui.theme.SurfaceVariant
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

private data class LibraryLocation(
    val routeIndex: Int,
    val id: String,
    val name: String
)

private sealed class LibraryRoute {
    object Home : LibraryRoute()
    data class Folder(val id: String, val name: String) : LibraryRoute()
    data class Detail(val id: String, val name: String, val type: String) : LibraryRoute()
}

private data class DetailParentLink(
    val id: String,
    val name: String,
    val type: String
)

private data class DashboardRow(
    val title: String,
    val items: List<LibraryItem>
)

private enum class MediaFilter(
    val label: String,
    val includeItemTypes: String?
) {
    All("All media", null),
    Movies("Movies", "Movie"),
    Shows("Shows", "Series"),
    Episodes("Episodes", "Episode")
}

private enum class StatusFilter(
    val label: String,
    val isPlayed: Boolean? = null,
    val isFavorite: Boolean? = null
) {
    All("All", null, null),
    Unplayed("Unplayed", false, null),
    Played("Played", true, null),
    Favorites("Favorites", null, true)
}

private val MediaCardMinWidth = 220.dp
private val MediaCardDefaultWidth = 260.dp
private val MediaCardMaxWidth = 360.dp
private val MediaCardStepWidth = 20.dp
private val DetailDesktopBreakpoint = 760.dp
private const val SearchableMediaTypes = "Movie,Series,Episode"

@Composable
fun LibraryScreen(client: JellyfinClient, onLogout: () -> Unit) {
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    var items by remember { mutableStateOf<List<LibraryItem>>(emptyList()) }
    var dashboardRows by remember { mutableStateOf<List<DashboardRow>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var routeStack by remember { mutableStateOf<List<LibraryRoute>>(listOf(LibraryRoute.Home)) }
    var searchQuery by remember { mutableStateOf("") }
    var mediaFilter by remember { mutableStateOf(MediaFilter.All) }
    var statusFilter by remember { mutableStateOf(StatusFilter.All) }
    var sortOption by remember { mutableStateOf(loadStoredLibrarySortOption()) }
    var pageSize by remember { mutableIntStateOf(loadStoredLibraryPageSize()) }
    var pageIndex by remember { mutableIntStateOf(0) }
    var totalRecordCount by remember { mutableIntStateOf(0) }
    var reloadToken by remember { mutableIntStateOf(0) }
    var playbackStartingItemId by remember { mutableStateOf<String?>(null) }
    var watchedActionItemIds by remember { mutableStateOf<Set<String>>(emptySet()) }
    var playbackMessage by remember { mutableStateOf<String?>(null) }
    var mediaCardWidth by remember { mutableStateOf(loadStoredMediaCardWidth()) }
    var detailItem by remember { mutableStateOf<LibraryItem?>(null) }
    var detailLoading by remember { mutableStateOf(false) }
    var detailError by remember { mutableStateOf<String?>(null) }
    var detailPlaybackMediaSource by remember { mutableStateOf<MediaSource?>(null) }
    var detailPlaybackLoading by remember { mutableStateOf(false) }
    var selectedAudioStreamIndex by remember { mutableStateOf<Int?>(null) }
    var selectedSubtitleStreamIndex by remember { mutableStateOf<Int?>(null) }
    var seriesSeasons by remember { mutableStateOf<List<LibraryItem>>(emptyList()) }
    var seriesSeasonsLoading by remember { mutableStateOf(false) }
    var seriesSeasonsError by remember { mutableStateOf<String?>(null) }
    var seasonEpisodes by remember { mutableStateOf<List<LibraryItem>>(emptyList()) }
    var seasonEpisodesLoading by remember { mutableStateOf(false) }
    var seasonEpisodesError by remember { mutableStateOf<String?>(null) }

    val currentRoute = routeStack.last()
    val currentFolderRoute = currentRoute as? LibraryRoute.Folder
    val currentDetailRoute = currentRoute as? LibraryRoute.Detail
    val currentParentId = currentFolderRoute?.id
    val filtersActive = mediaFilter != MediaFilter.All || statusFilter != StatusFilter.All
    val browseQueryActive = searchQuery.trim().isNotEmpty() || filtersActive
    val gridControlsVisible = currentDetailRoute == null && (currentParentId != null || browseQueryActive)
    val pageInfo = LibraryPageInfo(
        pageIndex = pageIndex,
        pageSize = pageSize,
        totalRecordCount = totalRecordCount,
        loadedItemCount = items.size
    )
    val navigationStack = routeStack.mapIndexedNotNull { index, route ->
        (route as? LibraryRoute.Folder)?.let { LibraryLocation(index, it.id, it.name) }
    }

    val processManager = remember { MpvProcessManager() }
    val ipc = remember { MpvIpcClient() }
    val playbackController = remember {
        PlaybackController(
            client = client,
            processManager = processManager,
            ipc = ipc,
            resumeManager = ResumeManager(client),
            subtitleManager = SubtitleManager(client, ipc),
            scope = scope
        )
    }

    DisposableEffect(Unit) {
        onDispose { playbackController.stop() }
    }

    fun resetDetailState() {
        detailItem = null
        detailLoading = false
        detailError = null
        detailPlaybackMediaSource = null
        detailPlaybackLoading = false
        selectedAudioStreamIndex = null
        selectedSubtitleStreamIndex = null
        seriesSeasons = emptyList()
        seriesSeasonsLoading = false
        seriesSeasonsError = null
        seasonEpisodes = emptyList()
        seasonEpisodesLoading = false
        seasonEpisodesError = null
    }

    fun pushRoute(route: LibraryRoute) {
        resetDetailState()
        routeStack = routeStack + route
    }

    fun resetBrowserControls() {
        searchQuery = ""
        mediaFilter = MediaFilter.All
        statusFilter = StatusFilter.All
        pageIndex = 0
    }

    fun updateMediaCardWidth(width: Dp) {
        val normalizedWidth = width.normalizedCardWidth()
        mediaCardWidth = normalizedWidth
        SettingsStore.update { settings ->
            settings.copy(cardWidthDp = normalizedWidth.value.roundToInt())
        }
    }

    fun updateSortOption(option: LibrarySortOption) {
        sortOption = option
        pageIndex = 0
        SettingsStore.update { settings ->
            settings.copy(librarySortKey = option.settingsKey)
        }
    }

    fun updatePageSize(size: Int) {
        val normalizedSize = normalizedLibraryPageSize(size)
        pageSize = normalizedSize
        pageIndex = 0
        SettingsStore.update { settings ->
            settings.copy(libraryPageSize = normalizedSize)
        }
    }

    fun navigateHome() {
        resetDetailState()
        routeStack = listOf(LibraryRoute.Home)
        resetBrowserControls()
    }

    fun browseItem(item: LibraryItem) {
        pushRoute(LibraryRoute.Folder(item.Id, item.Name))
        resetBrowserControls()
    }

    fun playItem(item: LibraryItem, streamOverride: PlaybackStreamOverride? = null) {
        if (!item.isPlayable) return
        scope.launch {
            playbackStartingItemId = item.Id
            runCatching { playbackController.play(item.Id, streamOverride) }
                .onFailure {
                    playbackMessage =
                        it.message?.takeIf { message -> message.isNotBlank() }
                            ?: "Playback failed"
                }
            playbackStartingItemId = null
        }
    }

    fun setItemWatchedState(item: LibraryItem, played: Boolean) {
        if (item.Id in watchedActionItemIds) return
        scope.launch {
            watchedActionItemIds = watchedActionItemIds + item.Id
            runCatching { client.setPlayedState(item.Id, played) }
                .onSuccess { reloadToken++ }
                .onFailure {
                    playbackMessage =
                        it.message?.takeIf { message -> message.isNotBlank() }
                            ?: "Failed to update watched state"
                }
            watchedActionItemIds = watchedActionItemIds - item.Id
        }
    }

    fun openItem(item: LibraryItem) {
        when {
            item.hasDetailPage -> {
                pushRoute(LibraryRoute.Detail(item.Id, item.Name, item.Type))
            }
            item.isContainer -> browseItem(item)
        }
    }

    fun openDetail(link: DetailParentLink) {
        pushRoute(LibraryRoute.Detail(link.id, link.name, link.type))
    }

    fun popRoute() {
        if (routeStack.size > 1) {
            val returningFromDetail = currentRoute is LibraryRoute.Detail
            resetDetailState()
            routeStack = routeStack.dropLast(1)
            if (!returningFromDetail) resetBrowserControls()
        }
    }

    LaunchedEffect(
        currentParentId,
        currentDetailRoute?.id,
        searchQuery,
        mediaFilter,
        statusFilter,
        sortOption,
        pageIndex,
        pageSize,
        reloadToken
    ) {
        if (currentDetailRoute != null) {
            loading = false
            return@LaunchedEffect
        }
        loading = true
        error = null
        runCatching {
            val query = searchQuery.trim().takeIf { it.isNotEmpty() }
            val queryActive = query != null || filtersActive
            val includeItemTypes = mediaFilter.includeItemTypes ?: SearchableMediaTypes
            if (currentParentId == null) {
                if (!queryActive) {
                    val libraries = client.getLibraries()

                    suspend fun loadDashboardRow(
                        title: String,
                        loader: suspend () -> List<LibraryItem>
                    ): DashboardRow {
                        return try {
                            DashboardRow(title, loader())
                        } catch (e: CancellationException) {
                            throw e
                        } catch (_: Exception) {
                            DashboardRow(title, emptyList())
                        }
                    }

                    dashboardRows = listOf(
                        DashboardRow("Libraries", libraries),
                        loadDashboardRow("Continue Watching") { client.getContinueWatching() },
                        loadDashboardRow("Next Up") { client.getNextUp() },
                        loadDashboardRow("Latest Media") { client.getLatestMedia() },
                        loadDashboardRow("Favorites") { client.getFavoriteItems() },
                        loadDashboardRow("Recently Added") { client.getRecentlyAdded() }
                    ).filter { it.items.isNotEmpty() }
                    items = libraries
                    totalRecordCount = libraries.size
                } else {
                    dashboardRows = emptyList()
                    val page = client.getFilteredItemsPage(
                        searchTerm = query,
                        recursive = true,
                        includeItemTypes = includeItemTypes,
                        isPlayed = statusFilter.isPlayed,
                        isFavorite = statusFilter.isFavorite,
                        startIndex = libraryStartIndex(pageIndex, pageSize),
                        limit = pageSize,
                        sortBy = sortOption.sortBy,
                        sortOrder = sortOption.sortOrder
                    )
                    val clampedPageIndex = clampLibraryPageIndex(pageIndex, page.totalRecordCount, pageSize)
                    totalRecordCount = page.totalRecordCount
                    if (clampedPageIndex != pageIndex) {
                        pageIndex = clampedPageIndex
                    } else {
                        items = page.items
                    }
                }
            } else {
                dashboardRows = emptyList()
                val page = if (queryActive) {
                    client.getFilteredItemsPage(
                        parentId = currentParentId,
                        searchTerm = query,
                        recursive = true,
                        includeItemTypes = includeItemTypes,
                        isPlayed = statusFilter.isPlayed,
                        isFavorite = statusFilter.isFavorite,
                        startIndex = libraryStartIndex(pageIndex, pageSize),
                        limit = pageSize,
                        sortBy = sortOption.sortBy,
                        sortOrder = sortOption.sortOrder
                    )
                } else {
                    client.getItemsPage(
                        parentId = currentParentId,
                        startIndex = libraryStartIndex(pageIndex, pageSize),
                        limit = pageSize,
                        sortBy = sortOption.sortBy,
                        sortOrder = sortOption.sortOrder
                    )
                }
                val clampedPageIndex = clampLibraryPageIndex(pageIndex, page.totalRecordCount, pageSize)
                totalRecordCount = page.totalRecordCount
                if (clampedPageIndex != pageIndex) {
                    pageIndex = clampedPageIndex
                } else {
                    items = page.items
                }
            }
        }.onFailure {
            error = it.message?.takeIf { message -> message.isNotBlank() } ?: "Failed to load library"
        }
        loading = false
    }

    LaunchedEffect(currentDetailRoute?.id, reloadToken) {
        val itemId = currentDetailRoute?.id ?: return@LaunchedEffect
        detailLoading = true
        detailError = null
        runCatching {
            detailItem = client.getItemDetails(itemId)
        }.onFailure {
            detailError = it.message?.takeIf { message -> message.isNotBlank() } ?: "Failed to load item details"
        }
        detailLoading = false
    }

    LaunchedEffect(detailItem?.Id, detailItem?.Type, reloadToken) {
        val series = detailItem?.takeIf { it.Type == "Series" }
        if (series == null) {
            seriesSeasons = emptyList()
            seriesSeasonsLoading = false
            seriesSeasonsError = null
            return@LaunchedEffect
        }

        seriesSeasonsLoading = true
        seriesSeasonsError = null
        runCatching {
            seriesSeasons = client.getSeriesSeasons(series.Id)
        }.onFailure {
            seriesSeasonsError =
                it.message?.takeIf { message -> message.isNotBlank() } ?: "Failed to load seasons"
        }
        seriesSeasonsLoading = false
    }

    LaunchedEffect(detailItem?.Id, detailItem?.Type, reloadToken) {
        val playableItem = detailItem?.takeIf { it.isPlayable }
        if (playableItem == null) {
            detailPlaybackMediaSource = null
            detailPlaybackLoading = false
            selectedAudioStreamIndex = null
            selectedSubtitleStreamIndex = null
            return@LaunchedEffect
        }

        detailPlaybackLoading = true
        detailPlaybackMediaSource = null
        selectedAudioStreamIndex = null
        selectedSubtitleStreamIndex = null
        runCatching {
            client.getPlaybackInfo(playableItem.Id).MediaSources.firstOrNull()
        }.onSuccess { mediaSource ->
            detailPlaybackMediaSource = mediaSource
            if (mediaSource != null) {
                val preferenceKey = PlaybackStreamPreferences.preferenceKey(playableItem, playableItem.Id)
                val selection = PlaybackStreamPreferences.resolve(
                    mediaSource = mediaSource,
                    preference = PlaybackStreamPreferences.storedPreference(preferenceKey)
                )
                selectedAudioStreamIndex = selection.audioStreamIndex
                selectedSubtitleStreamIndex = selection.subtitleStreamIndex
            }
        }
        detailPlaybackLoading = false
    }

    LaunchedEffect(detailItem?.Id, detailItem?.Type, reloadToken) {
        val season = detailItem?.takeIf { it.Type == "Season" }
        if (season == null) {
            seasonEpisodes = emptyList()
            seasonEpisodesLoading = false
            seasonEpisodesError = null
            return@LaunchedEffect
        }

        seasonEpisodesLoading = true
        seasonEpisodesError = null
        runCatching {
            seasonEpisodes = client.getSeasonEpisodes(season.Id)
        }.onFailure {
            seasonEpisodesError =
                it.message?.takeIf { message -> message.isNotBlank() } ?: "Failed to load episodes"
        }
        seasonEpisodesLoading = false
    }

    LaunchedEffect(playbackMessage) {
        val message = playbackMessage ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        playbackMessage = null
    }

    Scaffold(
        topBar = {
            JellyfinTopBar(
                canGoBack = routeStack.size > 1,
                onBack = { popRoute() },
                onHome = { navigateHome() },
                onLogout = onLogout
            )
        },
        snackbarHost = { SnackbarHost(hostState = snackbarHostState) },
        containerColor = MaterialTheme.colorScheme.background
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
        ) {
            if (currentDetailRoute == null) {
                LibraryHeader(
                    title = when {
                        currentFolderRoute != null -> currentFolderRoute.name
                        !browseQueryActive -> "Home"
                        else -> "Search results"
                    },
                    itemCount = if (currentParentId == null && !browseQueryActive) {
                        dashboardRows.sumOf { it.items.size }
                    } else if (gridControlsVisible) {
                        totalRecordCount
                    } else {
                        items.size
                    },
                    loading = loading,
                    searchQuery = searchQuery,
                    onSearchChange = {
                        searchQuery = it
                        pageIndex = 0
                    },
                    mediaFilter = mediaFilter,
                    onMediaFilterChange = {
                        mediaFilter = it
                        pageIndex = 0
                    },
                    statusFilter = statusFilter,
                    onStatusFilterChange = {
                        statusFilter = it
                        pageIndex = 0
                    },
                    onClearFilters = { resetBrowserControls() },
                    showGridControls = gridControlsVisible,
                    sortOption = sortOption,
                    onSortOptionChange = { updateSortOption(it) },
                    pageSize = pageSize,
                    onPageSizeChange = { updatePageSize(it) },
                    cardWidth = mediaCardWidth,
                    onCardWidthChange = { updateMediaCardWidth(it) },
                    navigationStack = navigationStack,
                    onNavigateHome = { navigateHome() },
                    onNavigateTo = { routeIndex ->
                        resetDetailState()
                        routeStack = routeStack.take(routeIndex + 1)
                        resetBrowserControls()
                    }
                )
            }

            Column(modifier = Modifier.fillMaxSize()) {
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxWidth()
                ) {
                    when {
                        currentDetailRoute != null -> {
                            when {
                                detailLoading -> LoadingState(modifier = Modifier.align(Alignment.Center))
                                detailError != null -> ErrorState(
                                    message = detailError.orEmpty(),
                                    onRetry = { reloadToken++ },
                                    modifier = Modifier.align(Alignment.Center)
                                )
                                detailItem?.Id == currentDetailRoute.id -> ItemDetailView(
                                    detail = detailItem!!,
                                    client = client,
                                    isStarting = playbackStartingItemId == detailItem!!.Id,
                                    onPlay = {
                                        val streamOverride = detailPlaybackMediaSource
                                            ?.takeIf { detailItem!!.isPlayable }
                                            ?.let {
                                                PlaybackStreamOverride(
                                                    audioStreamIndex = selectedAudioStreamIndex,
                                                    subtitleStreamIndex = selectedSubtitleStreamIndex
                                                )
                                            }
                                        playItem(detailItem!!, streamOverride)
                                    },
                                    onBrowse = { browseItem(detailItem!!) },
                                    watchedActionItemIds = watchedActionItemIds,
                                    onSetWatched = { item, played -> setItemWatchedState(item, played) },
                                    onOpenDetail = ::openDetail,
                                    playbackMediaSource = detailPlaybackMediaSource,
                                    playbackInfoLoading = detailPlaybackLoading,
                                    selectedAudioStreamIndex = selectedAudioStreamIndex,
                                    selectedSubtitleStreamIndex = selectedSubtitleStreamIndex,
                                    onAudioStreamSelected = { selectedAudioStreamIndex = it },
                                    onSubtitleStreamSelected = { selectedSubtitleStreamIndex = it },
                                    seriesSeasons = seriesSeasons,
                                    seriesSeasonsLoading = seriesSeasonsLoading,
                                    seriesSeasonsError = seriesSeasonsError,
                                    seasonEpisodes = seasonEpisodes,
                                    seasonEpisodesLoading = seasonEpisodesLoading,
                                    seasonEpisodesError = seasonEpisodesError,
                                    playbackStartingItemId = playbackStartingItemId,
                                    onOpenSeason = ::openItem,
                                    onOpenEpisode = ::openItem,
                                    onPlayEpisode = { playItem(it) },
                                    onRetrySeriesSeasons = { reloadToken++ },
                                    onRetrySeasonEpisodes = { reloadToken++ }
                                )
                                else -> LoadingState(modifier = Modifier.align(Alignment.Center))
                            }
                        }
                        loading -> LoadingState(modifier = Modifier.align(Alignment.Center))
                        error != null -> ErrorState(
                            message = error.orEmpty(),
                            onRetry = { reloadToken++ },
                            modifier = Modifier.align(Alignment.Center)
                        )
                        currentParentId == null && !browseQueryActive -> {
                            if (dashboardRows.isEmpty()) {
                                EmptyState(
                                    searchQuery = searchQuery,
                                    filtersActive = filtersActive,
                                    modifier = Modifier.align(Alignment.Center)
                                )
                            } else {
                                HomeDashboard(
                                    rows = dashboardRows,
                                    client = client,
                                    cardWidth = mediaCardWidth,
                                    playbackStartingItemId = playbackStartingItemId,
                                    onItemClick = ::openItem
                                )
                            }
                        }
                        items.isEmpty() -> EmptyState(
                            searchQuery = searchQuery,
                            filtersActive = filtersActive,
                            modifier = Modifier.align(Alignment.Center)
                        )
                        else -> LibraryGrid(
                            items = items,
                            client = client,
                            cardWidth = mediaCardWidth,
                            playbackStartingItemId = playbackStartingItemId,
                            onItemClick = ::openItem
                        )
                    }
                }

                if (gridControlsVisible && !loading && error == null && totalRecordCount > 0) {
                    PaginationFooter(
                        pageInfo = pageInfo,
                        onFirstPage = { pageIndex = 0 },
                        onPreviousPage = { pageIndex = (pageIndex - 1).coerceAtLeast(0) },
                        onNextPage = {
                            pageIndex = (pageIndex + 1).coerceAtMost(pageInfo.totalPages - 1)
                        },
                        onLastPage = { pageIndex = pageInfo.totalPages - 1 }
                    )
                }
            }
        }
    }
}

@Composable
private fun LibraryHeader(
    title: String,
    itemCount: Int,
    loading: Boolean,
    searchQuery: String,
    onSearchChange: (String) -> Unit,
    mediaFilter: MediaFilter,
    onMediaFilterChange: (MediaFilter) -> Unit,
    statusFilter: StatusFilter,
    onStatusFilterChange: (StatusFilter) -> Unit,
    onClearFilters: () -> Unit,
    showGridControls: Boolean,
    sortOption: LibrarySortOption,
    onSortOptionChange: (LibrarySortOption) -> Unit,
    pageSize: Int,
    onPageSizeChange: (Int) -> Unit,
    cardWidth: Dp,
    onCardWidthChange: (Dp) -> Unit,
    navigationStack: List<LibraryLocation>,
    onNavigateHome: () -> Unit,
    onNavigateTo: (Int) -> Unit
) {
    Surface(
        color = MaterialTheme.colorScheme.background,
        tonalElevation = 0.dp
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp, vertical = 18.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
            BoxWithConstraints {
                val compact = maxWidth < 720.dp
                if (compact) {
                    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        HeaderTitle(
                            title = title,
                            itemCount = itemCount,
                            loading = loading
                        )
                        SearchField(
                            value = searchQuery,
                            onValueChange = onSearchChange,
                            modifier = Modifier.fillMaxWidth()
                        )
                        CardSizeControl(
                            cardWidth = cardWidth,
                            onCardWidthChange = onCardWidthChange,
                            modifier = Modifier.fillMaxWidth()
                        )
                    }
                } else {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(18.dp)
                    ) {
                        HeaderTitle(
                            title = title,
                            itemCount = itemCount,
                            loading = loading,
                            modifier = Modifier.weight(1f)
                        )

                        SearchField(
                            value = searchQuery,
                            onValueChange = onSearchChange,
                            modifier = Modifier.widthIn(min = 240.dp, max = 320.dp)
                        )

                        CardSizeControl(
                            cardWidth = cardWidth,
                            onCardWidthChange = onCardWidthChange,
                            modifier = Modifier.widthIn(min = 230.dp, max = 260.dp)
                        )
                    }
                }
            }

            BrowseFilterControls(
                mediaFilter = mediaFilter,
                onMediaFilterChange = onMediaFilterChange,
                statusFilter = statusFilter,
                onStatusFilterChange = onStatusFilterChange,
                onClearFilters = onClearFilters
            )

            if (showGridControls) {
                GridResultControls(
                    sortOption = sortOption,
                    onSortOptionChange = onSortOptionChange,
                    pageSize = pageSize,
                    onPageSizeChange = onPageSizeChange
                )
            }

            Breadcrumb(
                navigationStack = navigationStack,
                onNavigateHome = onNavigateHome,
                onNavigateTo = onNavigateTo
            )
        }
    }
}

@Composable
private fun HeaderTitle(
    title: String,
    itemCount: Int,
    loading: Boolean,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
        Text(
            text = if (loading) "Loading" else "$itemCount ${if (itemCount == 1) "item" else "items"}",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun BrowseFilterControls(
    mediaFilter: MediaFilter,
    onMediaFilterChange: (MediaFilter) -> Unit,
    statusFilter: StatusFilter,
    onStatusFilterChange: (StatusFilter) -> Unit,
    onClearFilters: () -> Unit,
    modifier: Modifier = Modifier
) {
    val hasActiveFilters = mediaFilter != MediaFilter.All || statusFilter != StatusFilter.All

    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        FilterChipRow(
            label = "Type",
            options = MediaFilter.entries.map { it.label to (it == mediaFilter) },
            onOptionClick = { index -> onMediaFilterChange(MediaFilter.entries[index]) }
        )
        FilterChipRow(
            label = "Status",
            options = StatusFilter.entries.map { it.label to (it == statusFilter) },
            onOptionClick = { index -> onStatusFilterChange(StatusFilter.entries[index]) },
            trailingContent = if (hasActiveFilters) {
                {
                    FilterChip(
                        label = "Reset",
                        selected = false,
                        onClick = onClearFilters
                    )
                }
            } else {
                null
            }
        )
    }
}

@Composable
private fun GridResultControls(
    sortOption: LibrarySortOption,
    onSortOptionChange: (LibrarySortOption) -> Unit,
    pageSize: Int,
    onPageSizeChange: (Int) -> Unit,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState()),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        Text(
            text = "Sort",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(46.dp)
        )
        SortDropdown(
            selected = sortOption,
            onSelected = onSortOptionChange
        )
        PageSizeDropdown(
            selected = pageSize,
            onSelected = onPageSizeChange
        )
    }
}

@Composable
private fun SortDropdown(
    selected: LibrarySortOption,
    onSelected: (LibrarySortOption) -> Unit
) {
    var expanded by remember { mutableStateOf(false) }

    Box {
        Button(
            onClick = { expanded = true },
            modifier = Modifier
                .widthIn(min = 190.dp, max = 240.dp)
                .pointerHoverIcon(PointerIcon.Hand)
        ) {
            Text(
                text = selected.label,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }

        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            LibrarySortOption.entries.forEach { option ->
                DropdownMenuItem(
                    text = {
                        Text(
                            text = option.label,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    },
                    onClick = {
                        expanded = false
                        onSelected(option)
                    }
                )
            }
        }
    }
}

@Composable
private fun PageSizeDropdown(
    selected: Int,
    onSelected: (Int) -> Unit
) {
    var expanded by remember { mutableStateOf(false) }

    Box {
        Button(
            onClick = { expanded = true },
            modifier = Modifier
                .widthIn(min = 132.dp, max = 170.dp)
                .pointerHoverIcon(PointerIcon.Hand)
        ) {
            Text(
                text = "$selected / page",
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }

        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            LibraryPageSizeOptions.forEach { option ->
                DropdownMenuItem(
                    text = { Text("$option / page") },
                    onClick = {
                        expanded = false
                        onSelected(option)
                    }
                )
            }
        }
    }
}

@Composable
private fun FilterChipRow(
    label: String,
    options: List<Pair<String, Boolean>>,
    onOptionClick: (Int) -> Unit,
    trailingContent: (@Composable () -> Unit)? = null
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState()),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(46.dp)
        )
        options.forEachIndexed { index, (optionLabel, selected) ->
            FilterChip(
                label = optionLabel,
                selected = selected,
                onClick = { onOptionClick(index) }
            )
        }
        trailingContent?.invoke()
    }
}

@Composable
private fun FilterChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit
) {
    Surface(
        modifier = Modifier
            .clip(RoundedCornerShape(16.dp))
            .clickable(onClick = onClick)
            .pointerHoverIcon(PointerIcon.Hand),
        shape = RoundedCornerShape(16.dp),
        color = if (selected) JellyfinBlue else MaterialTheme.colorScheme.surface,
        tonalElevation = if (selected) 0.dp else 2.dp
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodySmall,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
            color = Color.White,
            maxLines = 1,
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 7.dp)
        )
    }
}

@Composable
private fun CardSizeControl(
    cardWidth: Dp,
    onCardWidthChange: (Dp) -> Unit,
    modifier: Modifier = Modifier
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(8.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 2.dp
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Text(
                    text = "Card size",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f)
                )
                Text(
                    text = "${cardWidth.value.roundToInt()} dp",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                IconButton(
                    onClick = { onCardWidthChange(cardWidth - MediaCardStepWidth) },
                    modifier = Modifier
                        .size(32.dp)
                        .pointerHoverIcon(PointerIcon.Hand)
                ) {
                    Text(
                        text = "-",
                        color = Color.White,
                        fontSize = 20.sp,
                        fontWeight = FontWeight.SemiBold
                    )
                }
                Slider(
                    value = cardWidth.value,
                    onValueChange = { onCardWidthChange(it.dp) },
                    valueRange = MediaCardMinWidth.value..MediaCardMaxWidth.value,
                    steps = ((MediaCardMaxWidth.value - MediaCardMinWidth.value) / MediaCardStepWidth.value)
                        .roundToInt() - 1,
                    modifier = Modifier.weight(1f)
                )
                IconButton(
                    onClick = { onCardWidthChange(cardWidth + MediaCardStepWidth) },
                    modifier = Modifier
                        .size(32.dp)
                        .pointerHoverIcon(PointerIcon.Hand)
                ) {
                    Text(
                        text = "+",
                        color = Color.White,
                        fontSize = 20.sp,
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }
        }
    }
}

@Composable
private fun SearchField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        singleLine = true,
        modifier = modifier,
        label = { Text("Search") },
        leadingIcon = {
            Icon(
                Icons.Default.Search,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant
            )
        },
        trailingIcon = {
            if (value.isNotEmpty()) {
                IconButton(
                    onClick = { onValueChange("") },
                    modifier = Modifier.pointerHoverIcon(PointerIcon.Hand)
                ) {
                    Icon(
                        Icons.Default.Close,
                        contentDescription = "Clear search",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        },
        shape = RoundedCornerShape(8.dp),
        colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = JellyfinBlue,
            unfocusedBorderColor = MaterialTheme.colorScheme.outline,
            focusedLabelColor = JellyfinBlue,
            cursorColor = JellyfinBlue,
            focusedTextColor = Color.White,
            unfocusedTextColor = Color.White
        )
    )
}

@Composable
private fun Breadcrumb(
    navigationStack: List<LibraryLocation>,
    onNavigateHome: () -> Unit,
    onNavigateTo: (Int) -> Unit
) {
    Row(
        modifier = Modifier.horizontalScroll(rememberScrollState()),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        BreadcrumbText(
            text = "Home",
            selected = navigationStack.isEmpty(),
            onClick = onNavigateHome
        )
        navigationStack.forEachIndexed { index, location ->
            Text("/", color = OnSurfaceDim, style = MaterialTheme.typography.bodySmall)
            BreadcrumbText(
                text = location.name,
                selected = index == navigationStack.lastIndex,
                onClick = { onNavigateTo(location.routeIndex) }
            )
        }
    }
}

@Composable
private fun BreadcrumbText(text: String, selected: Boolean, onClick: () -> Unit) {
    Text(
        text = text,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        style = MaterialTheme.typography.bodySmall,
        color = if (selected) Color.White else JellyfinBlue,
        modifier = Modifier
            .clip(RoundedCornerShape(4.dp))
            .clickable(enabled = !selected, onClick = onClick)
            .pointerHoverIcon(PointerIcon.Hand)
            .padding(horizontal = 4.dp, vertical = 2.dp)
    )
}

@Composable
private fun ItemDetailView(
    detail: LibraryItem,
    client: JellyfinClient,
    isStarting: Boolean,
    onPlay: () -> Unit,
    onBrowse: () -> Unit,
    watchedActionItemIds: Set<String>,
    onSetWatched: (LibraryItem, Boolean) -> Unit,
    onOpenDetail: (DetailParentLink) -> Unit,
    playbackMediaSource: MediaSource?,
    playbackInfoLoading: Boolean,
    selectedAudioStreamIndex: Int?,
    selectedSubtitleStreamIndex: Int?,
    onAudioStreamSelected: (Int?) -> Unit,
    onSubtitleStreamSelected: (Int) -> Unit,
    seriesSeasons: List<LibraryItem>,
    seriesSeasonsLoading: Boolean,
    seriesSeasonsError: String?,
    seasonEpisodes: List<LibraryItem>,
    seasonEpisodesLoading: Boolean,
    seasonEpisodesError: String?,
    playbackStartingItemId: String?,
    onOpenSeason: (LibraryItem) -> Unit,
    onOpenEpisode: (LibraryItem) -> Unit,
    onPlayEpisode: (LibraryItem) -> Unit,
    onRetrySeriesSeasons: () -> Unit,
    onRetrySeasonEpisodes: () -> Unit
) {
    val loadedSeasonEpisodes = seasonEpisodes.takeIf {
        detail.Type == "Season" &&
            seasonEpisodes.isNotEmpty() &&
            !seasonEpisodesLoading &&
            seasonEpisodesError == null
    }
    val detailUnwatchedEpisodeCount = detail.unwatchedEpisodeCount(loadedSeasonEpisodes)

    LazyColumn(
        contentPadding = PaddingValues(bottom = 36.dp),
        verticalArrangement = Arrangement.spacedBy(26.dp),
        modifier = Modifier.fillMaxSize()
    ) {
        item {
            DetailHero(
                item = detail,
                client = client,
                isStarting = isStarting,
                onPlay = onPlay,
                onBrowse = onBrowse,
                isWatchedUpdating = detail.Id in watchedActionItemIds,
                onSetWatched = { played -> onSetWatched(detail, played) },
                onOpenDetail = onOpenDetail,
                playbackMediaSource = playbackMediaSource,
                playbackInfoLoading = playbackInfoLoading,
                selectedAudioStreamIndex = selectedAudioStreamIndex,
                selectedSubtitleStreamIndex = selectedSubtitleStreamIndex,
                onAudioStreamSelected = onAudioStreamSelected,
                onSubtitleStreamSelected = onSubtitleStreamSelected,
                unwatchedEpisodeCount = detailUnwatchedEpisodeCount
            )
        }

        if (detail.Type == "Series") {
            item {
                DetailContentFrame {
                    SeriesSeasonsSection(
                        seasons = seriesSeasons,
                        client = client,
                        loading = seriesSeasonsLoading,
                        error = seriesSeasonsError,
                        onOpenSeason = onOpenSeason,
                        onRetry = onRetrySeriesSeasons
                    )
                }
            }
        }

        if (detail.Type == "Season") {
            item {
                DetailContentFrame {
                    SeasonEpisodesSection(
                        episodes = seasonEpisodes,
                        client = client,
                        loading = seasonEpisodesLoading,
                        error = seasonEpisodesError,
                        playbackStartingItemId = playbackStartingItemId,
                        watchedActionItemIds = watchedActionItemIds,
                        onOpenEpisode = onOpenEpisode,
                        onPlayEpisode = onPlayEpisode,
                        onSetEpisodeWatched = onSetWatched,
                        onRetry = onRetrySeasonEpisodes
                    )
                }
            }
        }

        item {
            DetailContentFrame {
                DetailSecondarySections(detail = detail, client = client)
            }
        }
    }
}

@Composable
private fun DetailContentFrame(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit
) {
    BoxWithConstraints(modifier = modifier.fillMaxWidth()) {
        val compact = maxWidth < DetailDesktopBreakpoint
        val startPadding = if (compact) 24.dp else maxWidth * 0.3245f
        val endPadding = if (compact) 24.dp else (maxWidth * 0.02f).coerceAtLeast(24.dp)

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = startPadding, end = endPadding),
            verticalArrangement = Arrangement.spacedBy(24.dp),
            content = content
        )
    }
}

@Composable
private fun SeriesSeasonsSection(
    seasons: List<LibraryItem>,
    client: JellyfinClient,
    loading: Boolean,
    error: String?,
    onOpenSeason: (LibraryItem) -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                text = "Seasons",
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                color = Color.White,
                modifier = Modifier.weight(1f)
            )
            if (!loading && error == null) {
                Text(
                    text = "${seasons.size} ${if (seasons.size == 1) "season" else "seasons"}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

        when {
            loading -> Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFF202020).copy(alpha = 0.82f)
            ) {
                Row(
                    modifier = Modifier.padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(20.dp),
                        strokeWidth = 2.dp,
                        color = JellyfinBlue
                    )
                    Text(
                        text = "Loading seasons",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            error != null -> Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFF202020).copy(alpha = 0.82f)
            ) {
                Row(
                    modifier = Modifier.padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.weight(1f)
                    )
                    Button(
                        onClick = onRetry,
                        modifier = Modifier.pointerHoverIcon(PointerIcon.Hand)
                    ) {
                        Text("Retry")
                    }
                }
            }

            seasons.isEmpty() -> Text(
                text = "No seasons found for this show.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            else -> BoxWithConstraints(modifier = Modifier.fillMaxWidth()) {
                val cardWidth = if (maxWidth < 560.dp) {
                    132.dp
                } else {
                    (maxWidth * 0.18f).coerceIn(138.dp, 190.dp)
                }

                LazyRow(
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                    contentPadding = PaddingValues(end = 4.dp)
                ) {
                    lazyItems(seasons, key = { it.Id }) { season ->
                        SeasonCard(
                            season = season,
                            posterUrl = if (season.ImageTags.containsKey("Primary")) {
                                client.posterUrl(season.Id, 420)
                            } else {
                                null
                            },
                            onClick = { onOpenSeason(season) },
                            modifier = Modifier.width(cardWidth)
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SeasonCard(
    season: LibraryItem,
    posterUrl: String?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    val watched = season.UserData?.Played == true
    val progress = season.resumeProgress()
    val unwatchedEpisodeCount = season.unwatchedEpisodeCount()

    Column(
        modifier = modifier
            .clip(RoundedCornerShape(4.dp))
            .clickable(onClick = onClick)
            .pointerHoverIcon(PointerIcon.Hand),
        verticalArrangement = Arrangement.spacedBy(0.dp)
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(2f / 3f)
                .clip(RoundedCornerShape(topStart = 4.dp, topEnd = 4.dp))
                .background(SurfaceVariant)
        ) {
            if (posterUrl != null) {
                AsyncImage(
                    url = posterUrl,
                    contentDescription = season.Name,
                    modifier = Modifier.fillMaxSize(),
                    placeholder = { PosterPlaceholder(season.Name) }
                )
            } else {
                PosterPlaceholder(season.Name)
            }

            Box(
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth()
                    .height(58.dp)
                    .background(
                        Brush.verticalGradient(
                            listOf(
                                Color.Transparent,
                                Color.Black.copy(alpha = 0.78f)
                            )
                        )
                    )
            )

            if (watched) {
                Icon(
                    Icons.Default.CheckCircle,
                    contentDescription = "Watched",
                    tint = JellyfinBlue,
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(8.dp)
                        .size(24.dp)
                )
            }

            unwatchedEpisodeCount?.let { count ->
                UnwatchedEpisodeBadge(
                    count = count,
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(8.dp)
                )
            }

            if (progress != null && progress > 0f && !watched) {
                LinearProgressIndicator(
                    progress = { progress / 100f },
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .fillMaxWidth()
                        .height(4.dp),
                    color = JellyfinBlue,
                    trackColor = Color.Black.copy(alpha = 0.48f)
                )
            }
        }

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(Color(0xFF202020).copy(alpha = 0.82f))
                .padding(horizontal = 8.dp, vertical = 7.dp),
            verticalArrangement = Arrangement.spacedBy(3.dp)
        ) {
            Text(
                text = season.Name,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                color = Color.White,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )

            Text(
                text = season.seasonCardSubtitle(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Composable
private fun SeasonEpisodesSection(
    episodes: List<LibraryItem>,
    client: JellyfinClient,
    loading: Boolean,
    error: String?,
    playbackStartingItemId: String?,
    watchedActionItemIds: Set<String>,
    onOpenEpisode: (LibraryItem) -> Unit,
    onPlayEpisode: (LibraryItem) -> Unit,
    onSetEpisodeWatched: (LibraryItem, Boolean) -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                text = "Episodes",
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                color = Color.White,
                modifier = Modifier.weight(1f)
            )
            if (!loading && error == null) {
                Text(
                    text = "${episodes.size} ${if (episodes.size == 1) "episode" else "episodes"}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

        when {
            loading -> Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFF202020).copy(alpha = 0.82f)
            ) {
                Row(
                    modifier = Modifier.padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(20.dp),
                        strokeWidth = 2.dp,
                        color = JellyfinBlue
                    )
                    Text(
                        text = "Loading episodes",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            error != null -> Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFF202020).copy(alpha = 0.82f)
            ) {
                Row(
                    modifier = Modifier.padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.weight(1f)
                    )
                    Button(
                        onClick = onRetry,
                        modifier = Modifier.pointerHoverIcon(PointerIcon.Hand)
                    ) {
                        Text("Retry")
                    }
                }
            }

            episodes.isEmpty() -> Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(4.dp),
                color = Color(0xFF202020).copy(alpha = 0.82f)
            ) {
                Text(
                    text = "No episodes found for this season.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(16.dp)
                )
            }

            else -> Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                episodes.forEach { episode ->
                    EpisodeRow(
                        episode = episode,
                        client = client,
                        isStarting = playbackStartingItemId == episode.Id,
                        isWatchedUpdating = episode.Id in watchedActionItemIds,
                        onOpen = { onOpenEpisode(episode) },
                        onPlay = { onPlayEpisode(episode) },
                        onSetWatched = { played -> onSetEpisodeWatched(episode, played) }
                    )
                }
            }
        }
    }
}

@Composable
private fun EpisodeRow(
    episode: LibraryItem,
    client: JellyfinClient,
    isStarting: Boolean,
    isWatchedUpdating: Boolean,
    onOpen: () -> Unit,
    onPlay: () -> Unit,
    onSetWatched: (Boolean) -> Unit
) {
    val watched = episode.UserData?.Played == true
    val progress = episode.resumeProgress()
    val imageId = episode.thumbnailImageId()
    val imageType = episode.thumbnailImageType()

    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(4.dp))
            .clickable(onClick = onOpen)
            .pointerHoverIcon(PointerIcon.Hand),
        shape = RoundedCornerShape(4.dp),
        color = Color(0xFF202020).copy(alpha = 0.82f)
    ) {
        Column {
            Row(
                modifier = Modifier.padding(10.dp),
                verticalAlignment = Alignment.Top,
                horizontalArrangement = Arrangement.spacedBy(14.dp)
            ) {
                EpisodeThumbnail(
                    episode = episode,
                    url = imageId?.let { client.imageUrl(it, imageType, 500) },
                    watched = watched,
                    modifier = Modifier.width(170.dp)
                )

                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    Text(
                        text = episode.episodeTitle(),
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold,
                        color = Color.White,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis
                    )

                    Text(
                        text = episode.episodeRowSubtitle(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )

                    episode.Overview?.takeIf { it.isNotBlank() }?.let { overview ->
                        Text(
                            text = overview,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }

                Column(
                    horizontalAlignment = Alignment.End,
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    ItemPill(if (watched) "Watched" else "Unwatched", accent = watched)

                    DetailActionButton(
                        icon = Icons.Default.PlayArrow,
                        label = "Play",
                        enabled = !isStarting,
                        compact = true,
                        onClick = onPlay,
                        loadingContent = if (isStarting) {
                            {
                                CircularProgressIndicator(
                                    modifier = Modifier.size(20.dp),
                                    strokeWidth = 2.dp,
                                    color = Color.White
                                )
                            }
                        } else {
                            null
                        }
                    )

                    DetailActionButton(
                        icon = if (watched) Icons.Default.Close else Icons.Default.CheckCircle,
                        label = if (watched) "Mark unwatched" else "Mark watched",
                        enabled = !isWatchedUpdating,
                        compact = true,
                        onClick = { onSetWatched(!watched) },
                        loadingContent = if (isWatchedUpdating) {
                            {
                                CircularProgressIndicator(
                                    modifier = Modifier.size(20.dp),
                                    strokeWidth = 2.dp,
                                    color = Color.White
                                )
                            }
                        } else {
                            null
                        }
                    )
                }
            }

            if (progress != null && progress > 0f && !watched) {
                LinearProgressIndicator(
                    progress = { progress / 100f },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(3.dp),
                    color = JellyfinBlue,
                    trackColor = MaterialTheme.colorScheme.outline
                )
            }
        }
    }
}

@Composable
private fun EpisodeThumbnail(
    episode: LibraryItem,
    url: String?,
    watched: Boolean,
    modifier: Modifier = Modifier
) {
    Box(
        modifier = modifier
            .aspectRatio(16f / 9f)
            .clip(RoundedCornerShape(4.dp))
            .background(SurfaceVariant)
    ) {
        if (url != null) {
            AsyncImage(
                url = url,
                contentDescription = episode.Name,
                modifier = Modifier.fillMaxSize(),
                placeholder = { PosterPlaceholder(episode.Name) }
            )
        } else {
            PosterPlaceholder(episode.Name)
        }

        Box(
            modifier = Modifier
                .align(Alignment.BottomStart)
                .fillMaxWidth()
                .height(42.dp)
                .background(
                    Brush.verticalGradient(
                        listOf(
                            Color.Transparent,
                            Color.Black.copy(alpha = 0.78f)
                        )
                    )
                )
        )

        Text(
            text = episode.IndexNumber?.toString() ?: "-",
            color = Color.White,
            fontWeight = FontWeight.SemiBold,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier
                .align(Alignment.BottomStart)
                .padding(8.dp)
        )

        if (watched) {
            Icon(
                Icons.Default.CheckCircle,
                contentDescription = "Watched",
                tint = JellyfinBlue,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(7.dp)
                    .size(20.dp)
            )
        }
    }
}

@Composable
private fun DetailHero(
    item: LibraryItem,
    client: JellyfinClient,
    isStarting: Boolean,
    onPlay: () -> Unit,
    onBrowse: () -> Unit,
    isWatchedUpdating: Boolean,
    onSetWatched: (Boolean) -> Unit,
    onOpenDetail: (DetailParentLink) -> Unit,
    playbackMediaSource: MediaSource?,
    playbackInfoLoading: Boolean,
    selectedAudioStreamIndex: Int?,
    selectedSubtitleStreamIndex: Int?,
    onAudioStreamSelected: (Int?) -> Unit,
    onSubtitleStreamSelected: (Int) -> Unit,
    unwatchedEpisodeCount: Int?
) {
    BoxWithConstraints(modifier = Modifier.fillMaxWidth()) {
        if (maxWidth < DetailDesktopBreakpoint) {
            CompactDetailHero(
                item = item,
                client = client,
                isStarting = isStarting,
                onPlay = onPlay,
                onBrowse = onBrowse,
                isWatchedUpdating = isWatchedUpdating,
                onSetWatched = onSetWatched,
                onOpenDetail = onOpenDetail,
                playbackMediaSource = playbackMediaSource,
                playbackInfoLoading = playbackInfoLoading,
                selectedAudioStreamIndex = selectedAudioStreamIndex,
                selectedSubtitleStreamIndex = selectedSubtitleStreamIndex,
                onAudioStreamSelected = onAudioStreamSelected,
                onSubtitleStreamSelected = onSubtitleStreamSelected,
                unwatchedEpisodeCount = unwatchedEpisodeCount
            )
        } else {
            DesktopDetailHero(
                item = item,
                client = client,
                isStarting = isStarting,
                onPlay = onPlay,
                onBrowse = onBrowse,
                isWatchedUpdating = isWatchedUpdating,
                onSetWatched = onSetWatched,
                onOpenDetail = onOpenDetail,
                playbackMediaSource = playbackMediaSource,
                playbackInfoLoading = playbackInfoLoading,
                selectedAudioStreamIndex = selectedAudioStreamIndex,
                selectedSubtitleStreamIndex = selectedSubtitleStreamIndex,
                onAudioStreamSelected = onAudioStreamSelected,
                onSubtitleStreamSelected = onSubtitleStreamSelected,
                unwatchedEpisodeCount = unwatchedEpisodeCount,
                pageWidth = maxWidth
            )
        }
    }
}

@Composable
private fun DesktopDetailHero(
    item: LibraryItem,
    client: JellyfinClient,
    isStarting: Boolean,
    onPlay: () -> Unit,
    onBrowse: () -> Unit,
    isWatchedUpdating: Boolean,
    onSetWatched: (Boolean) -> Unit,
    onOpenDetail: (DetailParentLink) -> Unit,
    playbackMediaSource: MediaSource?,
    playbackInfoLoading: Boolean,
    selectedAudioStreamIndex: Int?,
    selectedSubtitleStreamIndex: Int?,
    onAudioStreamSelected: (Int?) -> Unit,
    onSubtitleStreamSelected: (Int) -> Unit,
    unwatchedEpisodeCount: Int?,
    pageWidth: Dp
) {
    val backdropHeight = (pageWidth * 0.32f).coerceIn(280.dp, 420.dp)
    val posterWidth = (pageWidth * 0.25f).coerceIn(190.dp, 340.dp)
    val posterOverlap = (posterWidth * 0.52f).coerceIn(118.dp, 190.dp)
    val bottomHeight = ((posterWidth * 1.5f) - posterOverlap + 56.dp).coerceAtLeast(300.dp)
    val leftGutter = (pageWidth * 0.033f).coerceAtLeast(24.dp)
    val contentStart = (pageWidth * 0.3245f).coerceAtLeast(leftGutter + posterWidth + 28.dp)
    val contentEnd = (pageWidth * 0.02f).coerceAtLeast(24.dp)
    val posterUrl = item.primaryImageId()?.let { client.posterUrl(it, 700) }
    val backdropId = item.backdropImageId()
    val progress = item.resumeProgress()

    Column(modifier = Modifier.fillMaxWidth()) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(backdropHeight)
                .background(SurfaceVariant)
        ) {
            DetailBackdrop(
                item = item,
                backdropUrl = backdropId?.let { client.backdropUrl(it) },
                logoUrl = if (item.ImageTags.containsKey("Logo")) client.logoUrl(item.Id) else null
            )

            if (progress != null && progress > 0f && item.UserData?.Played != true) {
                LinearProgressIndicator(
                    progress = { progress / 100f },
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .fillMaxWidth()
                        .height(4.dp),
                    color = JellyfinBlue,
                    trackColor = Color.Black.copy(alpha = 0.42f)
                )
            }
        }

        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(bottomHeight)
        ) {
            DetailPoster(
                item = item,
                posterUrl = posterUrl,
                unwatchedEpisodeCount = unwatchedEpisodeCount,
                modifier = Modifier
                    .width(posterWidth)
                    .offset(x = leftGutter, y = -posterOverlap)
            )

            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = contentStart, end = contentEnd)
                    .offset(y = -112.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                DetailHierarchy(
                    item = item,
                    onOpenDetail = onOpenDetail
                )

                Text(
                    text = item.Name,
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.SemiBold,
                    color = Color.White,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )

                item.Taglines.firstOrNull { it.isNotBlank() }?.let { tagline ->
                    Text(
                        text = tagline,
                        style = MaterialTheme.typography.bodyMedium,
                        color = Color.White.copy(alpha = 0.86f),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }

                DetailMiscInfo(item)
                DetailActionRow(
                    item = item,
                    isStarting = isStarting,
                    onPlay = onPlay,
                    onBrowse = onBrowse,
                    isWatchedUpdating = isWatchedUpdating,
                    onSetWatched = onSetWatched
                )
                DetailStreamSelectorRow(
                    item = item,
                    mediaSource = playbackMediaSource,
                    loading = playbackInfoLoading,
                    selectedAudioStreamIndex = selectedAudioStreamIndex,
                    selectedSubtitleStreamIndex = selectedSubtitleStreamIndex,
                    onAudioStreamSelected = onAudioStreamSelected,
                    onSubtitleStreamSelected = onSubtitleStreamSelected
                )

                item.Overview?.takeIf { it.isNotBlank() }?.let { overview ->
                    Text(
                        text = overview,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 6,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(top = 6.dp)
                    )
                }
            }
        }
    }
}

@Composable
private fun CompactDetailHero(
    item: LibraryItem,
    client: JellyfinClient,
    isStarting: Boolean,
    onPlay: () -> Unit,
    onBrowse: () -> Unit,
    isWatchedUpdating: Boolean,
    onSetWatched: (Boolean) -> Unit,
    onOpenDetail: (DetailParentLink) -> Unit,
    playbackMediaSource: MediaSource?,
    playbackInfoLoading: Boolean,
    selectedAudioStreamIndex: Int?,
    selectedSubtitleStreamIndex: Int?,
    onAudioStreamSelected: (Int?) -> Unit,
    onSubtitleStreamSelected: (Int) -> Unit,
    unwatchedEpisodeCount: Int?
) {
    val posterUrl = item.primaryImageId()?.let { client.posterUrl(it, 500) }
    val backdropId = item.backdropImageId()

    Column(modifier = Modifier.fillMaxWidth()) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(260.dp)
                .background(SurfaceVariant)
        ) {
            DetailBackdrop(
                item = item,
                backdropUrl = backdropId?.let { client.backdropUrl(it) },
                logoUrl = if (item.ImageTags.containsKey("Logo")) client.logoUrl(item.Id) else null
            )
        }

        Column(
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            DetailPoster(
                item = item,
                posterUrl = posterUrl,
                unwatchedEpisodeCount = unwatchedEpisodeCount,
                modifier = Modifier.width(150.dp)
            )

            DetailHierarchy(
                item = item,
                onOpenDetail = onOpenDetail
            )

            Text(
                text = item.Name,
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.SemiBold,
                color = Color.White,
                textAlign = TextAlign.Center
            )

            item.Taglines.firstOrNull { it.isNotBlank() }?.let { tagline ->
                Text(
                    text = tagline,
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.86f),
                    textAlign = TextAlign.Center,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )
            }

            DetailMiscInfo(item, centered = true)
            DetailActionRow(
                item = item,
                isStarting = isStarting,
                onPlay = onPlay,
                onBrowse = onBrowse,
                isWatchedUpdating = isWatchedUpdating,
                onSetWatched = onSetWatched,
                centered = true
            )
            DetailStreamSelectorRow(
                item = item,
                mediaSource = playbackMediaSource,
                loading = playbackInfoLoading,
                selectedAudioStreamIndex = selectedAudioStreamIndex,
                selectedSubtitleStreamIndex = selectedSubtitleStreamIndex,
                onAudioStreamSelected = onAudioStreamSelected,
                onSubtitleStreamSelected = onSubtitleStreamSelected,
                centered = true
            )

            item.Overview?.takeIf { it.isNotBlank() }?.let { overview ->
                Text(
                    text = overview,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 6,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
    }
}

@Composable
private fun BoxScope.DetailBackdrop(
    item: LibraryItem,
    backdropUrl: String?,
    logoUrl: String?
) {
    if (backdropUrl != null) {
        AsyncImage(
            url = backdropUrl,
            contentDescription = item.Name,
            modifier = Modifier.fillMaxSize(),
            placeholder = { DetailBackdropPlaceholder() }
        )
    } else {
        DetailBackdropPlaceholder()
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    listOf(
                        Color.Black.copy(alpha = 0.04f),
                        Color.Black.copy(alpha = 0.36f),
                        MaterialTheme.colorScheme.background
                    )
                )
            )
    )

    if (logoUrl != null) {
        AsyncImage(
            url = logoUrl,
            contentDescription = "${item.Name} logo",
            contentScale = ContentScale.Fit,
            modifier = Modifier
                .align(Alignment.TopEnd)
                .padding(top = 42.dp, end = 24.dp)
                .width(280.dp)
                .height(120.dp)
        )
    }
}

@Composable
private fun DetailHierarchy(
    item: LibraryItem,
    onOpenDetail: (DetailParentLink) -> Unit
) {
    val parents = item.parentDetailLinks()
    if (parents.isEmpty()) return

    Row(
        modifier = Modifier.horizontalScroll(rememberScrollState()),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        parents.forEach { parent ->
            BreadcrumbText(
                text = parent.name,
                selected = false,
                onClick = { onOpenDetail(parent) }
            )
            Text(">", color = OnSurfaceDim, style = MaterialTheme.typography.bodySmall)
        }
        Text(
            text = item.Name,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            style = MaterialTheme.typography.bodySmall,
            color = Color.White
        )
    }
}

@Composable
private fun DetailPoster(
    item: LibraryItem,
    posterUrl: String?,
    unwatchedEpisodeCount: Int?,
    modifier: Modifier = Modifier
) {
    Surface(
        modifier = modifier.aspectRatio(2f / 3f),
        shape = RoundedCornerShape(4.dp),
        color = SurfaceVariant,
        tonalElevation = 2.dp,
        shadowElevation = 8.dp
    ) {
        Box(modifier = Modifier.fillMaxSize()) {
            if (posterUrl != null) {
                AsyncImage(
                    url = posterUrl,
                    contentDescription = item.Name,
                    modifier = Modifier.fillMaxSize(),
                    placeholder = { PosterPlaceholder(item.Name) }
                )
            } else {
                PosterPlaceholder(item.Name)
            }

            unwatchedEpisodeCount?.let { count ->
                UnwatchedEpisodeBadge(
                    count = count,
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(9.dp)
                )
            }
        }
    }
}

@Composable
private fun DetailBackdropPlaceholder() {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    listOf(
                        SurfaceVariant,
                        Color(0xFF152426)
                    )
                )
            )
    )
}

@Composable
private fun DetailMiscInfo(item: LibraryItem, centered: Boolean = false) {
    val metadata = item.detailMetadataItems()
    if (metadata.isEmpty()) return

    Row(
        modifier = if (centered) Modifier.fillMaxWidth() else Modifier.horizontalScroll(rememberScrollState()),
        horizontalArrangement = if (centered) {
            Arrangement.spacedBy(7.dp, Alignment.CenterHorizontally)
        } else {
            Arrangement.spacedBy(7.dp)
        },
        verticalAlignment = Alignment.CenterVertically
    ) {
        metadata.forEachIndexed { index, label ->
            Text(
                text = label,
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White.copy(alpha = 0.84f),
                fontWeight = if (index == 0) FontWeight.SemiBold else FontWeight.Normal
            )
            if (index != metadata.lastIndex) {
                Text(
                    text = "•",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

private data class StreamOption(
    val index: Int,
    val label: String
)

@Composable
private fun DetailStreamSelectorRow(
    item: LibraryItem,
    mediaSource: MediaSource?,
    loading: Boolean,
    selectedAudioStreamIndex: Int?,
    selectedSubtitleStreamIndex: Int?,
    onAudioStreamSelected: (Int?) -> Unit,
    onSubtitleStreamSelected: (Int) -> Unit,
    centered: Boolean = false
) {
    if (!item.isPlayable) return

    val audioStreams = mediaSource?.MediaStreams
        ?.filter { it.Type == "Audio" }
        .orEmpty()
    val subtitleStreams = mediaSource?.MediaStreams
        ?.filter { it.Type == "Subtitle" }
        .orEmpty()

    if (loading) {
        Text(
            text = "Loading playback options",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = if (centered) Modifier.fillMaxWidth() else Modifier
        )
        return
    }

    if (audioStreams.size <= 1 && subtitleStreams.isEmpty()) return

    val audioOptions = audioStreams.map { stream ->
        StreamOption(
            index = stream.Index,
            label = PlaybackStreamPreferences.streamLabel(stream, "Audio")
        )
    }
    val subtitleOptions = listOf(StreamOption(index = -1, label = "Off")) +
        subtitleStreams.map { stream ->
            StreamOption(
                index = stream.Index,
                label = PlaybackStreamPreferences.streamLabel(stream, "Subtitle")
            )
        }

    Row(
        modifier = if (centered) {
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
        } else {
            Modifier.horizontalScroll(rememberScrollState())
        },
        horizontalArrangement = if (centered) {
            Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally)
        } else {
            Arrangement.spacedBy(8.dp)
        },
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (audioOptions.size > 1) {
            StreamDropdownSelector(
                label = "Audio",
                selectedLabel = audioOptions
                    .firstOrNull { it.index == selectedAudioStreamIndex }
                    ?.label
                    ?: "Default",
                options = audioOptions,
                onSelected = { onAudioStreamSelected(it.index) }
            )
        }

        if (subtitleStreams.isNotEmpty()) {
            StreamDropdownSelector(
                label = "Subtitles",
                selectedLabel = subtitleOptions
                    .firstOrNull { it.index == selectedSubtitleStreamIndex }
                    ?.label
                    ?: "Off",
                options = subtitleOptions,
                onSelected = { onSubtitleStreamSelected(it.index) }
            )
        }
    }
}

@Composable
private fun StreamDropdownSelector(
    label: String,
    selectedLabel: String,
    options: List<StreamOption>,
    onSelected: (StreamOption) -> Unit
) {
    var expanded by remember { mutableStateOf(false) }

    Box {
        Button(
            onClick = { expanded = true },
            modifier = Modifier
                .widthIn(min = 132.dp, max = 240.dp)
                .pointerHoverIcon(PointerIcon.Hand)
        ) {
            Text(
                text = "$label: $selectedLabel",
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }

        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            options.forEach { option ->
                DropdownMenuItem(
                    text = {
                        Text(
                            text = option.label,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    },
                    onClick = {
                        expanded = false
                        onSelected(option)
                    }
                )
            }
        }
    }
}

@Composable
private fun DetailActionRow(
    item: LibraryItem,
    isStarting: Boolean,
    onPlay: () -> Unit,
    onBrowse: () -> Unit,
    isWatchedUpdating: Boolean,
    onSetWatched: (Boolean) -> Unit,
    centered: Boolean = false
) {
    val watched = item.UserData?.Played == true

    Row(
        modifier = if (centered) Modifier.fillMaxWidth() else Modifier.horizontalScroll(rememberScrollState()),
        horizontalArrangement = if (centered) {
            Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally)
        } else {
            Arrangement.spacedBy(6.dp)
        },
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (item.isPlayable) {
            DetailActionButton(
                icon = Icons.Default.PlayArrow,
                label = "Play",
                enabled = !isStarting,
                onClick = onPlay,
                loadingContent = if (isStarting) {
                    {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            strokeWidth = 2.dp,
                            color = Color.White
                        )
                    }
                } else {
                    null
                }
            )
        }

        if (item.isBrowsableDetail) {
            DetailActionButton(
                icon = Icons.Default.Info,
                label = "Browse",
                onClick = onBrowse
            )
        }

        if (item.hasDetailPage) {
            DetailActionButton(
                icon = if (watched) Icons.Default.Close else Icons.Default.CheckCircle,
                label = if (watched) "Mark unwatched" else "Mark watched",
                enabled = !isWatchedUpdating,
                onClick = { onSetWatched(!watched) },
                loadingContent = if (isWatchedUpdating) {
                    {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            strokeWidth = 2.dp,
                            color = Color.White
                        )
                    }
                } else {
                    null
                }
            )
        }
    }
}

@Composable
private fun DetailActionButton(
    icon: ImageVector,
    label: String,
    enabled: Boolean = true,
    compact: Boolean = false,
    onClick: () -> Unit,
    loadingContent: @Composable (() -> Unit)? = null
) {
    val tint = if (enabled) Color.White.copy(alpha = 0.86f) else MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        modifier = Modifier
            .widthIn(min = if (compact) 58.dp else 72.dp)
            .clip(RoundedCornerShape(4.dp))
            .clickable(enabled = enabled, onClick = onClick)
            .pointerHoverIcon(PointerIcon.Hand),
        color = Color.Transparent,
        shape = RoundedCornerShape(4.dp)
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = if (compact) 7.dp else 9.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(if (compact) 4.dp else 7.dp)
        ) {
            if (loadingContent != null) {
                loadingContent()
            } else {
                Icon(
                    icon,
                    contentDescription = label,
                    tint = tint,
                    modifier = Modifier.size(if (compact) 23.dp else 30.dp)
                )
            }
            Text(
                text = label,
                style = MaterialTheme.typography.bodySmall,
                fontSize = if (compact) 11.sp else 12.sp,
                color = tint,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Composable
private fun DetailSecondarySections(detail: LibraryItem, client: JellyfinClient) {
    val sections = listOfNotNull(
        detail.Genres.takeIf { it.isNotEmpty() }?.joinToString(", ")?.let { "Genres" to it },
        detail.Studios.takeIf { it.isNotEmpty() }?.joinToString(", ") { it.Name }?.let { "Studios" to it },
        detail.mediaInfoSummary().takeIf { it.isNotBlank() }?.let { "Media Info" to it }
    )
    val cast = detail.peopleByType("Actor").take(16)
    val crew = detail.peopleSummary("Director", "Writer", "Producer")

    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(26.dp)
    ) {
        if (cast.isNotEmpty()) {
            PeopleSection(title = "Cast", people = cast, client = client)
        }

        if (crew.isNotBlank()) {
            DetailTextSection(title = "Crew", body = crew)
        }

        sections.forEach { (title, body) ->
            DetailTextSection(title = title, body = body)
        }
    }
}

@Composable
private fun PeopleSection(title: String, people: List<BaseItemPerson>, client: JellyfinClient) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.SemiBold,
            color = Color.White
        )

        LazyRow(
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            contentPadding = PaddingValues(end = 4.dp)
        ) {
            lazyItems(people, key = { it.Id ?: "${it.Name}-${it.Role}" }) { person ->
                PersonTile(person = person, client = client)
            }
        }
    }
}

@Composable
private fun PersonTile(person: BaseItemPerson, client: JellyfinClient) {
    val imageUrl = person.Id
        ?.takeIf { it.isNotBlank() && !person.PrimaryImageTag.isNullOrBlank() }
        ?.let { client.posterUrl(it, 260) }

    Column(
        modifier = Modifier.width(118.dp),
        verticalArrangement = Arrangement.spacedBy(7.dp)
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(2f / 3f),
            shape = RoundedCornerShape(4.dp),
            color = SurfaceVariant,
            shadowElevation = 2.dp
        ) {
            if (imageUrl != null) {
                AsyncImage(
                    url = imageUrl,
                    contentDescription = person.Name,
                    modifier = Modifier.fillMaxSize(),
                    placeholder = { PosterPlaceholder(person.Name) }
                )
            } else {
                PosterPlaceholder(person.Name)
            }
        }

        Text(
            text = person.Name,
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.SemiBold,
            color = Color.White,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis
        )

        person.Role?.takeIf { it.isNotBlank() }?.let { role ->
            Text(
                text = role,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Composable
private fun DetailTextSection(title: String, body: String, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.SemiBold,
            color = Color.White
        )
        Text(
            text = body,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun HomeDashboard(
    rows: List<DashboardRow>,
    client: JellyfinClient,
    cardWidth: Dp,
    playbackStartingItemId: String?,
    onItemClick: (LibraryItem) -> Unit
) {
    LazyColumn(
        contentPadding = PaddingValues(start = 24.dp, end = 24.dp, bottom = 28.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
        modifier = Modifier.fillMaxSize()
    ) {
        lazyItems(rows, key = { it.title }) { row ->
            DashboardSection(
                row = row,
                client = client,
                cardWidth = cardWidth,
                playbackStartingItemId = playbackStartingItemId,
                onItemClick = onItemClick
            )
        }
    }
}

@Composable
private fun DashboardSection(
    row: DashboardRow,
    client: JellyfinClient,
    cardWidth: Dp,
    playbackStartingItemId: String?,
    onItemClick: (LibraryItem) -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(
            text = row.title,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            color = Color.White
        )

        LazyRow(
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            contentPadding = PaddingValues(end = 4.dp)
        ) {
            lazyItems(row.items, key = { "${row.title}-${it.Id}" }) { item ->
                ItemCard(
                    item = item,
                    posterUrl = item.primaryImageId()?.let { client.posterUrl(it) },
                    isStarting = playbackStartingItemId == item.Id,
                    onClick = { onItemClick(item) },
                    showEpisodeSeriesName = true,
                    modifier = Modifier.width(cardWidth)
                )
            }
        }
    }
}

@Composable
private fun LibraryGrid(
    items: List<LibraryItem>,
    client: JellyfinClient,
    cardWidth: Dp,
    playbackStartingItemId: String?,
    onItemClick: (LibraryItem) -> Unit
) {
    LazyVerticalGrid(
        columns = GridCells.Adaptive(cardWidth),
        contentPadding = PaddingValues(start = 24.dp, end = 24.dp, bottom = 28.dp),
        horizontalArrangement = Arrangement.spacedBy(18.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp)
    ) {
        items(items, key = { it.Id }) { item ->
            ItemCard(
                item = item,
                posterUrl = item.primaryImageId()?.let { client.posterUrl(it) },
                isStarting = playbackStartingItemId == item.Id,
                onClick = { onItemClick(item) },
                modifier = Modifier.fillMaxWidth()
            )
        }
    }
}

@Composable
private fun PaginationFooter(
    pageInfo: LibraryPageInfo,
    onFirstPage: () -> Unit,
    onPreviousPage: () -> Unit,
    onNextPage: () -> Unit,
    onLastPage: () -> Unit
) {
    Surface(
        color = MaterialTheme.colorScheme.background,
        tonalElevation = 2.dp
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 24.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            Text(
                text = pageInfo.rangeLabel,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.widthIn(min = 170.dp)
            )
            Text(
                text = "Page ${pageInfo.clampedPageIndex + 1} of ${pageInfo.totalPages}",
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White,
                modifier = Modifier.widthIn(min = 92.dp)
            )
            PaginationIconButton(
                label = "|<",
                contentDescription = "First page",
                enabled = pageInfo.canGoPrevious,
                onClick = onFirstPage
            )
            PaginationIconButton(
                icon = Icons.AutoMirrored.Filled.ArrowBack,
                label = "<",
                contentDescription = "Previous page",
                enabled = pageInfo.canGoPrevious,
                onClick = onPreviousPage
            )
            PaginationIconButton(
                icon = Icons.AutoMirrored.Filled.ArrowForward,
                label = ">",
                contentDescription = "Next page",
                enabled = pageInfo.canGoNext,
                onClick = onNextPage
            )
            PaginationIconButton(
                label = ">|",
                contentDescription = "Last page",
                enabled = pageInfo.canGoNext,
                onClick = onLastPage
            )
        }
    }
}

@Composable
private fun PaginationIconButton(
    label: String,
    contentDescription: String,
    enabled: Boolean,
    onClick: () -> Unit,
    icon: ImageVector? = null
) {
    IconButton(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier
            .size(34.dp)
            .semantics { this.contentDescription = contentDescription }
            .pointerHoverIcon(PointerIcon.Hand)
    ) {
        if (icon == null) {
            Text(
                text = label,
                color = if (enabled) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center
            )
        } else {
            Icon(
                icon,
                contentDescription = null,
                tint = if (enabled) Color.White else MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun LoadingState(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        CircularProgressIndicator(color = JellyfinBlue, strokeWidth = 3.dp)
        Text("Loading library", color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun ErrorState(message: String, onRetry: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier.widthIn(max = 420.dp),
        shape = RoundedCornerShape(8.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 2.dp
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text("Could not load this view", style = MaterialTheme.typography.titleMedium)
            Text(
                message,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error
            )
            Button(onClick = onRetry, modifier = Modifier.pointerHoverIcon(PointerIcon.Hand)) {
                Text("Retry")
            }
        }
    }
}

@Composable
private fun EmptyState(
    searchQuery: String,
    filtersActive: Boolean,
    modifier: Modifier = Modifier
) {
    val hasSearch = searchQuery.isNotBlank()
    val hasActiveQuery = hasSearch || filtersActive

    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(
            text = if (hasActiveQuery) "No matches" else "Nothing here",
            style = MaterialTheme.typography.titleMedium
        )
        Text(
            text = when {
                hasSearch -> "No results for \"$searchQuery\""
                filtersActive -> "No items match the selected filters."
                else -> "This folder has no visible items."
            },
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun JellyfinTopBar(
    canGoBack: Boolean,
    onBack: () -> Unit,
    onHome: () -> Unit,
    onLogout: () -> Unit
) {
    TopAppBar(
        title = {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                modifier = Modifier
                    .clip(RoundedCornerShape(6.dp))
                    .clickable(onClick = onHome)
                    .pointerHoverIcon(PointerIcon.Hand)
                    .padding(end = 8.dp)
            ) {
                Surface(
                    shape = RoundedCornerShape(6.dp),
                    color = SurfaceVariant,
                    modifier = Modifier.size(30.dp)
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Text(
                            "J",
                            color = JellyfinBlue,
                            fontWeight = FontWeight.Bold,
                            fontSize = 16.sp
                        )
                    }
                }
                Text(
                    "JellyBridge",
                    fontWeight = FontWeight.Bold,
                    fontSize = 20.sp,
                    color = JellyfinBlue
                )
            }
        },
        navigationIcon = {
            if (canGoBack) {
                IconButton(
                    onClick = onBack,
                    modifier = Modifier.pointerHoverIcon(PointerIcon.Hand)
                ) {
                    Icon(
                        Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = "Back",
                        tint = Color.White
                    )
                }
            }
        },
        actions = {
            IconButton(
                onClick = onLogout,
                modifier = Modifier.pointerHoverIcon(PointerIcon.Hand)
            ) {
                Icon(
                    Icons.Default.AccountCircle,
                    contentDescription = "Logout",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = MaterialTheme.colorScheme.surface,
            titleContentColor = JellyfinBlue
        )
    )
}

@Composable
private fun ItemCard(
    item: LibraryItem,
    posterUrl: String?,
    isStarting: Boolean,
    onClick: () -> Unit,
    showEpisodeSeriesName: Boolean = false,
    modifier: Modifier = Modifier
) {
    val isWatched = item.UserData?.Played == true
    val progress = item.resumeProgress()
    val resolutionLabel = item.resolutionLabel()
    val unwatchedEpisodeCount = item.unwatchedEpisodeCount()

    Surface(
        modifier = modifier
            .clip(RoundedCornerShape(8.dp))
            .clickable(onClick = onClick)
            .pointerHoverIcon(PointerIcon.Hand),
        shape = RoundedCornerShape(8.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 2.dp,
        shadowElevation = 3.dp
    ) {
        Column {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(2f / 3f)
                    .background(SurfaceVariant)
            ) {
                if (posterUrl != null) {
                    AsyncImage(
                        url = posterUrl,
                        contentDescription = item.Name,
                        modifier = Modifier.fillMaxSize(),
                        placeholder = { PosterPlaceholder(item.Name) }
                    )
                } else {
                    PosterPlaceholder(item.Name)
                }

                Box(
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .fillMaxWidth()
                        .height(72.dp)
                        .background(
                            Brush.verticalGradient(
                                listOf(
                                    Color.Transparent,
                                    Color.Black.copy(alpha = 0.78f)
                                )
                            )
                        )
                )

                Row(
                    modifier = Modifier
                        .align(Alignment.TopStart)
                        .padding(
                            start = 8.dp,
                            top = 8.dp,
                            end = if (resolutionLabel == null) 8.dp else 64.dp
                        ),
                    horizontalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    ItemPill(item.displayType)
                    if (isWatched) ItemPill("Watched", accent = true)
                }

                unwatchedEpisodeCount?.let { count ->
                    UnwatchedEpisodeBadge(
                        count = count,
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .padding(8.dp)
                    )
                }

                resolutionLabel?.let {
                    Box(
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .padding(
                                top = if (unwatchedEpisodeCount == null) 8.dp else 42.dp,
                                end = 8.dp
                            )
                    ) {
                        ItemPill(it, accent = true)
                    }
                }

                if (isStarting) {
                    Surface(
                        modifier = Modifier.align(Alignment.Center),
                        shape = RoundedCornerShape(8.dp),
                        color = Color.Black.copy(alpha = 0.68f)
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier
                                .padding(14.dp)
                                .size(24.dp),
                            color = JellyfinBlue,
                            strokeWidth = 3.dp
                        )
                    }
                }
            }

            if (progress != null && progress > 0f && !isWatched) {
                LinearProgressIndicator(
                    progress = { progress / 100f },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(4.dp),
                    color = JellyfinBlue,
                    trackColor = MaterialTheme.colorScheme.outline
                )
            } else {
                Spacer(Modifier.height(4.dp))
            }

            Column(
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                Text(
                    item.Name,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    color = Color.White
                )

                Text(
                    item.cardSubtitleText(showEpisodeSeriesName),
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@Composable
private fun PosterPlaceholder(name: String) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    colors = listOf(
                        SurfaceVariant,
                        Color(0xFF162326)
                    )
                )
            )
    ) {
        Text(
            text = name.initials(),
            style = MaterialTheme.typography.headlineLarge,
            color = JellyfinBlue.copy(alpha = 0.72f),
            fontWeight = FontWeight.Bold,
            modifier = Modifier.align(Alignment.Center)
        )
    }
}

@Composable
private fun ItemPill(text: String, accent: Boolean = false) {
    Surface(
        shape = RoundedCornerShape(6.dp),
        color = if (accent) JellyfinBlue else Color.Black.copy(alpha = 0.62f)
    ) {
        Text(
            text = text,
            color = Color.White,
            fontSize = 11.sp,
            fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(horizontal = 7.dp, vertical = 3.dp),
            maxLines = 1
        )
    }
}

@Composable
private fun UnwatchedEpisodeBadge(count: Int, modifier: Modifier = Modifier) {
    val label = "$count ${if (count == 1) "unwatched episode" else "unwatched episodes"}"

    Surface(
        modifier = modifier
            .height(26.dp)
            .widthIn(min = 26.dp)
            .semantics { contentDescription = label },
        shape = RoundedCornerShape(999.dp),
        color = JellyfinBlue,
        shadowElevation = 4.dp
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(
                text = if (count > 99) "99+" else count.toString(),
                color = Color.White,
                fontSize = 12.sp,
                fontWeight = FontWeight.Bold,
                maxLines = 1,
                modifier = Modifier.padding(horizontal = 7.dp)
            )
        }
    }
}

private val LibraryItem.isPlayable: Boolean
    get() = Type == "Movie" || Type == "Episode"

private val LibraryItem.hasDetailPage: Boolean
    get() = Type == "Movie" || Type == "Episode" || Type == "Series" || Type == "Season"

private val LibraryItem.isBrowsableDetail: Boolean
    get() = Type == "Series" || Type == "Season"

private val LibraryItem.isContainer: Boolean
    get() = Type == "CollectionFolder" || Type == "Series" || Type == "Season" || !isPlayable

private val LibraryItem.displayType: String
    get() = when (Type) {
        "CollectionFolder" -> "Library"
        "Series" -> "Series"
        "Season" -> "Season"
        "Episode" -> "Episode"
        "Movie" -> "Movie"
        else -> Type
    }

private fun LibraryItem.resumeProgress(): Float? = when {
    UserData == null -> null
    UserData.PlayedPercentage != null && UserData.PlayedPercentage > 0.0 ->
        UserData.PlayedPercentage.toFloat().coerceIn(0f, 100f)
    RunTimeTicks != null && RunTimeTicks > 0 && UserData.PlaybackPositionTicks > 0 ->
        (UserData.PlaybackPositionTicks.toDouble() / RunTimeTicks * 100.0).toFloat().coerceIn(0f, 100f)
    else -> null
}

private fun LibraryItem.unwatchedEpisodeCount(episodes: List<LibraryItem>? = null): Int? {
    if (Type != "Series" && Type != "Season") return null
    if (UserData?.Played == true) return null

    val count = episodes
        ?.count { it.Type == "Episode" && it.UserData?.Played != true }
        ?: UserData?.UnplayedItemCount

    return count?.takeIf { it > 0 }
}

private fun LibraryItem.resolutionLabel(): String? {
    val videoStream = MediaStreams.firstOrNull { it.Type.equals("Video", ignoreCase = true) }
    val width = videoStream?.Width ?: Width
    val height = videoStream?.Height ?: Height
    return resolutionLabel(width, height)
}

private fun LibraryItem.primaryImageId(): String? = when {
    ImageTags.containsKey("Primary") -> Id
    !ParentPrimaryImageItemId.isNullOrBlank() -> ParentPrimaryImageItemId
    else -> null
}

private fun LibraryItem.backdropImageId(): String? = when {
    BackdropImageTags.isNotEmpty() -> Id
    !ParentBackdropItemId.isNullOrBlank() -> ParentBackdropItemId
    else -> null
}

private fun LibraryItem.thumbnailImageId(): String? = when {
    ImageTags.containsKey("Thumb") -> Id
    ImageTags.containsKey("Primary") -> Id
    else -> null
}

private fun LibraryItem.thumbnailImageType(): String =
    if (ImageTags.containsKey("Thumb")) "Thumb" else "Primary"

private fun LibraryItem.detailMetadataItems(): List<String> =
    listOfNotNull(
        SeriesName?.takeIf { Type == "Episode" },
        seasonEpisodeLabel(),
        ProductionYear?.toString(),
        OfficialRating,
        runtimeLabel(RunTimeTicks),
        communityRatingLabel(),
        CriticRating?.roundToInt()?.takeIf { it > 0 }?.let { "$it%" },
        resolutionLabel(),
        resumeProgress()
            ?.takeIf { it > 0f && UserData?.Played != true }
            ?.let { "${it.roundToInt()}% watched" },
        if (UserData?.Played == true) "Played" else null
    ).distinct()

private fun LibraryItem.detailSubtitle(): String =
    listOfNotNull(
        displayType,
        SeriesName?.takeIf { Type == "Episode" },
        seasonEpisodeLabel(),
        ProductionYear?.toString(),
        OfficialRating,
        runtimeLabel(RunTimeTicks)
    ).distinct().joinToString(" • ").ifBlank { displayType }

private fun LibraryItem.parentDetailLinks(): List<DetailParentLink> = when (Type) {
    "Episode" -> listOfNotNull(seriesParentLink(), seasonParentLink())
    "Season" -> listOfNotNull(seriesParentLink())
    else -> emptyList()
}

private fun LibraryItem.seriesParentLink(): DetailParentLink? {
    val id = when (Type) {
        "Episode" -> SeriesId
        "Season" -> SeriesId ?: ParentId
        else -> null
    }?.takeIf { it.isNotBlank() && it != Id } ?: return null

    return DetailParentLink(
        id = id,
        name = SeriesName?.takeIf { it.isNotBlank() } ?: "Series",
        type = "Series"
    )
}

private fun LibraryItem.seasonParentLink(): DetailParentLink? {
    if (Type != "Episode") return null
    val id = (SeasonId ?: ParentId)
        ?.takeIf { it.isNotBlank() && it != Id }
        ?: return null

    return DetailParentLink(
        id = id,
        name = SeasonName?.takeIf { it.isNotBlank() }
            ?: ParentIndexNumber?.let { "Season $it" }
            ?: "Season",
        type = "Season"
    )
}

private fun LibraryItem.seasonEpisodeLabel(): String? = when {
    ParentIndexNumber != null && IndexNumber != null -> "S$ParentIndexNumber E$IndexNumber"
    Type == "Season" && IndexNumber != null -> "Season $IndexNumber"
    Type == "Episode" && IndexNumber != null -> "Episode $IndexNumber"
    else -> null
}

private fun LibraryItem.seasonCardSubtitle(): String {
    val episodeCount = (ChildCount ?: RecursiveItemCount)
        ?.takeIf { it > 0 }
        ?.let { "$it ${if (it == 1) "episode" else "episodes"}" }
    val progress = resumeProgress()
        ?.takeIf { it > 0f && UserData?.Played != true }
        ?.let { "${it.roundToInt()}% watched" }

    return listOfNotNull(
        episodeCount,
        progress,
        if (UserData?.Played == true) "Watched" else null
    ).joinToString(" • ").ifBlank { displayType }
}

private fun LibraryItem.episodeRowSubtitle(): String {
    val progress = resumeProgress()
        ?.takeIf { it > 0f && UserData?.Played != true }
        ?.let { "${it.roundToInt()}% watched" }

    return listOfNotNull(
        seasonEpisodeLabel(),
        runtimeLabel(RunTimeTicks),
        progress
    ).distinct().joinToString(" • ").ifBlank { "Episode" }
}

private fun LibraryItem.episodeTitle(): String =
    IndexNumber?.let { "$it. $Name" } ?: Name

private fun LibraryItem.communityRatingLabel(): String? {
    val rating = CommunityRating ?: return null
    val rounded = (rating * 10.0).roundToInt() / 10.0
    return "$rounded/10"
}

private fun LibraryItem.peopleSummary(vararg personTypes: String): String {
    val allowedTypes = personTypes.map { it.lowercase() }.toSet()
    return People
        .filter { it.Type?.lowercase() in allowedTypes }
        .take(12)
        .joinToString(", ") { person ->
            if (person.Type.equals("Actor", ignoreCase = true) && !person.Role.isNullOrBlank()) {
                "${person.Name} as ${person.Role}"
            } else {
                person.Name
            }
        }
}

private fun LibraryItem.peopleByType(vararg personTypes: String): List<BaseItemPerson> {
    val allowedTypes = personTypes.map { it.lowercase() }.toSet()
    return People.filter { it.Type?.lowercase() in allowedTypes }
}

private fun LibraryItem.mediaInfoSummary(): String {
    val video = MediaStreams.firstOrNull { it.Type.equals("Video", ignoreCase = true) }
        ?.let { stream ->
            listOfNotNull(
                stream.Codec?.uppercase(),
                resolutionLabel(stream.Width, stream.Height),
                stream.DisplayTitle
            ).distinct().joinToString(" • ").ifBlank { null }
        }
        ?.let { "Video: $it" }

    val audio = MediaStreams
        .filter { it.Type.equals("Audio", ignoreCase = true) }
        .mapNotNull { stream ->
            listOfNotNull(
                stream.Language?.uppercase(),
                stream.Codec?.uppercase(),
                stream.DisplayTitle
            ).distinct().joinToString(" • ").ifBlank { null }
        }
        .distinct()
        .take(4)
        .takeIf { it.isNotEmpty() }
        ?.joinToString("; ")
        ?.let { "Audio: $it" }

    val subtitleCount = MediaStreams.count { it.Type.equals("Subtitle", ignoreCase = true) }
        .takeIf { it > 0 }
        ?.let { "Subtitles: $it ${if (it == 1) "track" else "tracks"}" }

    return listOfNotNull(video, audio, subtitleCount).joinToString("\n")
}

private fun runtimeLabel(ticks: Long?): String? {
    val totalMinutes = ticks?.takeIf { it > 0 }?.div(600_000_000L) ?: return null
    val hours = totalMinutes / 60
    val minutes = totalMinutes % 60
    return when {
        hours > 0 && minutes > 0 -> "${hours}h ${minutes}m"
        hours > 0 -> "${hours}h"
        else -> "${minutes}m"
    }
}

private fun resolutionLabel(width: Int?, height: Int?): String? {
    val h = height ?: return when {
        width == null -> null
        width >= 3840 -> "4K"
        width >= 2560 -> "2K"
        width >= 1920 -> "1080p"
        width >= 1280 -> "720p"
        else -> null
    }
    val w = width ?: 0
    return when {
        h >= 2160 || w >= 3840 -> "4K"
        h >= 1440 || w >= 2560 -> "2K"
        h >= 1080 -> "1080p"
        h >= 720 -> "720p"
        h >= 576 -> "576p"
        h >= 480 -> "480p"
        else -> null
    }
}

private fun LibraryItem.cardSubtitleText(showEpisodeSeriesName: Boolean): String {
    if (showEpisodeSeriesName && Type == "Episode" && !SeriesName.isNullOrBlank()) {
        return listOfNotNull(
            SeriesName,
            seasonEpisodeLabel(),
            runtimeLabel(RunTimeTicks)
        ).distinct().joinToString(" • ")
    }

    return subtitleText()
}

private fun LibraryItem.subtitleText(): String {
    val year = ProductionYear?.toString()
    val episode = when {
        ParentIndexNumber != null && IndexNumber != null -> "S$ParentIndexNumber E$IndexNumber"
        IndexNumber != null -> "Episode $IndexNumber"
        else -> null
    }
    return listOfNotNull(year, episode, if (isContainer) "Browse" else "Direct play").joinToString(" • ")
        .ifBlank { displayType }
}

private fun String.initials(): String {
    val words = trim().split(Regex("\\s+")).filter { it.isNotBlank() }
    return words.take(2).joinToString("") { it.first().uppercase() }.ifBlank { "JB" }
}

private fun loadStoredMediaCardWidth(): Dp {
    val savedWidth = SettingsStore.load()?.cardWidthDp ?: return MediaCardDefaultWidth
    val minWidth = MediaCardMinWidth.value.roundToInt()
    val maxWidth = MediaCardMaxWidth.value.roundToInt()
    return savedWidth
        .takeIf { it in minWidth..maxWidth }
        ?.dp
        ?: MediaCardDefaultWidth
}

private fun Dp.normalizedCardWidth(): Dp =
    value
        .roundToInt()
        .coerceIn(MediaCardMinWidth.value.roundToInt(), MediaCardMaxWidth.value.roundToInt())
        .dp

private fun loadStoredLibrarySortOption(): LibrarySortOption =
    LibrarySortOption.fromSettingsKey(SettingsStore.load()?.librarySortKey)

private fun loadStoredLibraryPageSize(): Int =
    normalizedLibraryPageSize(SettingsStore.load()?.libraryPageSize)
