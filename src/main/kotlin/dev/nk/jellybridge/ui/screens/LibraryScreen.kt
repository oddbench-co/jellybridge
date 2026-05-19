package dev.nk.jellybridge.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.pointerHoverIcon
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.nk.jellybridge.api.JellyfinClient
import dev.nk.jellybridge.api.models.LibraryItem
import dev.nk.jellybridge.mpv.MpvIpcClient
import dev.nk.jellybridge.mpv.MpvProcessManager
import dev.nk.jellybridge.playback.PlaybackController
import dev.nk.jellybridge.playback.ResumeManager
import dev.nk.jellybridge.playback.SubtitleManager
import dev.nk.jellybridge.ui.components.AsyncImage
import dev.nk.jellybridge.ui.theme.JellyfinBlue
import dev.nk.jellybridge.ui.theme.OnSurfaceDim
import dev.nk.jellybridge.ui.theme.SurfaceVariant
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

private data class LibraryLocation(
    val id: String,
    val name: String
)

private val MediaCardMinWidth = 220.dp
private val MediaCardDefaultWidth = 260.dp
private val MediaCardMaxWidth = 360.dp
private val MediaCardStepWidth = 20.dp

@Composable
fun LibraryScreen(client: JellyfinClient, onLogout: () -> Unit) {
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    var items by remember { mutableStateOf<List<LibraryItem>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var navigationStack by remember { mutableStateOf<List<LibraryLocation>>(emptyList()) }
    var searchQuery by remember { mutableStateOf("") }
    var reloadToken by remember { mutableIntStateOf(0) }
    var playbackStartingItemId by remember { mutableStateOf<String?>(null) }
    var playbackMessage by remember { mutableStateOf<String?>(null) }
    var mediaCardWidth by remember { mutableStateOf(MediaCardDefaultWidth) }

    val currentLocation = navigationStack.lastOrNull()
    val currentParentId = currentLocation?.id

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

    LaunchedEffect(currentParentId, searchQuery, reloadToken) {
        loading = true
        error = null
        runCatching {
            val query = searchQuery.trim().takeIf { it.isNotEmpty() }
            items = if (currentParentId == null) {
                val libraries = client.getLibraries()
                if (query == null) {
                    libraries
                } else {
                    libraries.filter { it.Name.contains(query, ignoreCase = true) }
                }
            } else {
                client.getItems(currentParentId, query)
            }
        }.onFailure {
            error = it.message?.takeIf { message -> message.isNotBlank() } ?: "Failed to load library"
        }
        loading = false
    }

    LaunchedEffect(playbackMessage) {
        val message = playbackMessage ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        playbackMessage = null
    }

    Scaffold(
        topBar = {
            JellyfinTopBar(
                canGoBack = navigationStack.isNotEmpty(),
                onBack = { navigationStack = navigationStack.dropLast(1) },
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
            LibraryHeader(
                title = currentLocation?.name ?: "Libraries",
                itemCount = items.size,
                loading = loading,
                searchQuery = searchQuery,
                onSearchChange = { searchQuery = it },
                cardWidth = mediaCardWidth,
                onCardWidthChange = { mediaCardWidth = it.coerceToCardWidth() },
                navigationStack = navigationStack,
                onNavigateHome = {
                    navigationStack = emptyList()
                    searchQuery = ""
                },
                onNavigateTo = { index ->
                    navigationStack = navigationStack.take(index + 1)
                    searchQuery = ""
                }
            )

            Box(modifier = Modifier.fillMaxSize()) {
                when {
                    loading -> LoadingState(modifier = Modifier.align(Alignment.Center))
                    error != null -> ErrorState(
                        message = error.orEmpty(),
                        onRetry = { reloadToken++ },
                        modifier = Modifier.align(Alignment.Center)
                    )
                    items.isEmpty() -> EmptyState(
                        searchQuery = searchQuery,
                        modifier = Modifier.align(Alignment.Center)
                    )
                    else -> LibraryGrid(
                        items = items,
                        client = client,
                        cardWidth = mediaCardWidth,
                        playbackStartingItemId = playbackStartingItemId,
                        onItemClick = { item ->
                            when {
                                item.isContainer -> {
                                    navigationStack = navigationStack + LibraryLocation(item.Id, item.Name)
                                    searchQuery = ""
                                }
                                item.isPlayable -> {
                                    scope.launch {
                                        playbackStartingItemId = item.Id
                                        runCatching { playbackController.play(item.Id) }
                                            .onFailure {
                                                playbackMessage =
                                                    it.message?.takeIf { message -> message.isNotBlank() }
                                                        ?: "Playback failed"
                                            }
                                        playbackStartingItemId = null
                                    }
                                }
                            }
                        }
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
                onClick = { onNavigateTo(index) }
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
                posterUrl = if (item.ImageTags.containsKey("Primary")) client.posterUrl(item.Id) else null,
                isStarting = playbackStartingItemId == item.Id,
                onClick = { onItemClick(item) }
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
private fun EmptyState(searchQuery: String, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(
            text = if (searchQuery.isBlank()) "Nothing here" else "No matches",
            style = MaterialTheme.typography.titleMedium
        )
        Text(
            text = if (searchQuery.isBlank()) "This folder has no visible items." else "No results for \"$searchQuery\"",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun JellyfinTopBar(canGoBack: Boolean, onBack: () -> Unit, onLogout: () -> Unit) {
    TopAppBar(
        title = {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp)
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
    onClick: () -> Unit
) {
    val isWatched = item.UserData?.Played == true
    val progress = item.resumeProgress()
    val resolutionLabel = item.resolutionLabel()

    Surface(
        modifier = Modifier
            .fillMaxWidth()
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

                resolutionLabel?.let {
                    Box(
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .padding(8.dp)
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
                    item.subtitleText(),
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

private val LibraryItem.isPlayable: Boolean
    get() = Type == "Movie" || Type == "Episode"

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

private fun LibraryItem.resolutionLabel(): String? {
    val videoStream = MediaStreams.firstOrNull { it.Type.equals("Video", ignoreCase = true) }
    val width = videoStream?.Width ?: Width
    val height = videoStream?.Height ?: Height
    return resolutionLabel(width, height)
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

private fun Dp.coerceToCardWidth(): Dp =
    value.coerceIn(MediaCardMinWidth.value, MediaCardMaxWidth.value).dp
