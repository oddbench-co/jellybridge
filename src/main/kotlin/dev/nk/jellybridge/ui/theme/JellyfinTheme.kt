package dev.nk.jellybridge.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

// Jellyfin brand palette
val JellyfinBlue    = Color(0xFF00A4DC)
val JellyfinPurple  = Color(0xFFAA5CC3)
val BackgroundDark  = Color(0xFF0F1113)
val SurfaceDark     = Color(0xFF181C1F)
val SurfaceVariant  = Color(0xFF252B30)
val OnSurfaceDim    = Color(0xFFB7C0C6)

private val JellyfinColorScheme = darkColorScheme(
    primary          = JellyfinBlue,
    onPrimary        = Color.White,
    primaryContainer = Color(0xFF004D6B),
    secondary        = JellyfinPurple,
    onSecondary      = Color.White,
    background       = BackgroundDark,
    onBackground     = Color.White,
    surface          = SurfaceDark,
    onSurface        = Color.White,
    surfaceVariant   = SurfaceVariant,
    onSurfaceVariant = OnSurfaceDim,
    error            = Color(0xFFCF6679),
    outline          = Color(0xFF3B464D),
)

private val JellyfinShapes = Shapes(
    extraSmall = RoundedCornerShape(4.dp),
    small = RoundedCornerShape(6.dp),
    medium = RoundedCornerShape(8.dp),
    large = RoundedCornerShape(8.dp),
    extraLarge = RoundedCornerShape(8.dp)
)

@Composable
fun JellyfinTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = JellyfinColorScheme,
        shapes = JellyfinShapes,
        content = content
    )
}
