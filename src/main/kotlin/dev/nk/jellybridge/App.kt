package dev.nk.jellybridge

import androidx.compose.runtime.*
import dev.nk.jellybridge.api.JellyfinClient
import dev.nk.jellybridge.auth.StoredSession
import dev.nk.jellybridge.auth.TokenStore
import dev.nk.jellybridge.ui.screens.AuthScreen
import dev.nk.jellybridge.ui.screens.LibraryScreen
import dev.nk.jellybridge.ui.theme.JellyfinTheme

// Simple top-level navigation state
sealed class Screen {
    object Auth : Screen()
    data class Library(val client: JellyfinClient) : Screen()
}

@Composable
fun App() {
    JellyfinTheme {
    // Restore a saved session so the user doesn't have to log in every launch.
    val initialScreen = remember {
        val session = TokenStore.load()
        if (session != null) {
            Screen.Library(JellyfinClient(session.serverUrl, session.token, session.userId))
        } else {
            Screen.Auth
        }
    }

    var screen by remember { mutableStateOf<Screen>(initialScreen) }

    when (val s = screen) {
        is Screen.Auth -> AuthScreen(
            onAuthenticated = { client ->
                TokenStore.save(StoredSession(client.serverUrl, client.token, client.userId))
                screen = Screen.Library(client)
            }
        )
        is Screen.Library -> LibraryScreen(
            client = s.client,
            onLogout = {
                TokenStore.clear()
                screen = Screen.Auth
            }
        )
    }
    } // JellyfinTheme
}
