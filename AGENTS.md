# Repository Guidelines

## Project Structure & Module Organization

JellyBridge is a Kotlin JVM Compose Desktop application. Source code lives under `src/main/kotlin/dev/nk/jellybridge`, organized by responsibility: `api` for Jellyfin HTTP access and DTOs, `auth` for saved sessions, `mpv` for process and IPC integration, `playback` for playback coordination, and `ui` for Compose screens, components, and theme. Runtime assets live in `src/main/resources`; the desktop package currently uses `src/main/resources/icon.png`. There is no `src/test` tree yet.

## Build, Test, and Development Commands

Use the checked-in Gradle wrapper so everyone runs the same build tooling.

- `./gradlew run` launches the Compose Desktop app locally.
- `./gradlew build` compiles the app and runs verification tasks.
- `./gradlew test` runs the test suite when tests are present.
- `./gradlew packageDistributionForCurrentOS` creates the native package for the current OS.
- `./gradlew clean` removes generated build output.

`install.sh` is a Linux-oriented helper that builds an AppImage and installs JellyBridge under `/opt/jellybridge`; review it before running because it uses `sudo`.

## Coding Style & Naming Conventions

The project uses Kotlin official style (`kotlin.code.style=official`) and JVM toolchain 21. Use four-space indentation, Kotlin package names in lowercase, `PascalCase` for classes and composables, `camelCase` for functions and properties, and `UPPER_SNAKE_CASE` only for true constants. Keep Compose UI code in `ui/screens`, `ui/components`, or `ui/theme`; keep Jellyfin and MPV integration details out of composables when practical.

## Testing Guidelines

No test framework is configured beyond Gradle's default test task. Add tests under `src/test/kotlin` when introducing behavior that can be isolated from the desktop UI, especially API parsing, token storage, resume logic, subtitle selection, and MPV command construction. Name tests after the behavior under test, for example `ResumeManagerTest` or `MpvIpcClientTest`, and run `./gradlew test` before submitting.

## Commit & Pull Request Guidelines

The current history uses short, descriptive messages such as `Initial JellyBridge app`. Continue with concise, sentence-case commit subjects that describe the change, for example `Add resume progress sync`. Pull requests should include a short summary, testing performed, linked issues when applicable, and screenshots or screen recordings for UI changes. Note any Jellyfin server or MPV setup assumptions needed to verify the change.

## Security & Configuration Tips

Do not commit real Jellyfin tokens, server URLs, user IDs, or local MPV paths. Keep machine-specific configuration in local files or environment-specific setup, and document new required configuration in the PR.
