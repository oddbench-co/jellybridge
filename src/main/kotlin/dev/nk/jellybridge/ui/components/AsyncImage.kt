package dev.nk.jellybridge.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.jetbrains.skia.Image as SkiaImage
import java.net.URI
import java.util.concurrent.ConcurrentHashMap

// Simple in-memory poster cache — keyed by URL, holds decoded bitmaps for the app lifetime.
// A ConcurrentHashMap is good enough here: posters are small and the library list is bounded.
private val imageCache = ConcurrentHashMap<String, ImageBitmap>()

/**
 * Loads an image from [url] asynchronously, shows [placeholder] while loading.
 * Results are cached in memory so navigating back doesn't re-fetch.
 */
@Composable
fun AsyncImage(
    url: String,
    contentDescription: String? = null,
    modifier: Modifier = Modifier,
    contentScale: ContentScale = ContentScale.Crop,
    placeholder: @Composable () -> Unit = {}
) {
    var bitmap by remember(url) { mutableStateOf(imageCache[url]) }

    LaunchedEffect(url) {
        if (bitmap != null) return@LaunchedEffect
        val loaded = withContext(Dispatchers.IO) {
            runCatching {
                val bytes = URI(url).toURL().readBytes()
                SkiaImage.makeFromEncoded(bytes).toComposeImageBitmap()
            }.getOrNull()
        }
        if (loaded != null) {
            imageCache[url] = loaded
            bitmap = loaded
        }
    }

    if (bitmap != null) {
        Image(
            bitmap = bitmap!!,
            contentDescription = contentDescription,
            modifier = modifier,
            contentScale = contentScale
        )
    } else {
        Box(modifier = modifier) {
            placeholder()
        }
    }
}
