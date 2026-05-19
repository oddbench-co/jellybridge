package dev.nk.jellybridge.api.models

import kotlinx.serialization.Serializable

@Serializable
data class AuthRequest(
    val Username: String,
    val Pw: String
)

@Serializable
data class AuthResponse(
    val AccessToken: String,
    val ServerId: String,
    val User: JellyfinUser
)

@Serializable
data class JellyfinUser(
    val Id: String,
    val Name: String
)
