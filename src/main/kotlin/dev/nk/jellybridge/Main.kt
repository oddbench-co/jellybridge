package dev.nk.jellybridge

import androidx.compose.ui.graphics.painter.BitmapPainter
import androidx.compose.ui.res.loadImageBitmap
import androidx.compose.ui.res.useResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState

fun main() = application {
    val appIcon = BitmapPainter(useResource("icon.png") { loadImageBitmap(it) })
    Window(
        onCloseRequest = ::exitApplication,
        title = "JellyBridge",
        icon = appIcon,
        state = rememberWindowState(width = 1280.dp, height = 800.dp)
    ) {
        App()
    }
}
