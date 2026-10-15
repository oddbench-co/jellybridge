# Third-party notices

JellyBridge is licensed under the GNU General Public License, version 3 or
(at your option) any later version; see `LICENSE`. It bundles or embeds the
components below, each under its own licence. JellyBridge is an independent
client and is not affiliated with or endorsed by the Jellyfin project.

## Embedded in the application

| Component | Licence | Source |
| --- | --- | --- |
| Noto Sans fonts | SIL Open Font License 1.1 (`rust/app/assets/fonts/LICENSE-NotoSans.txt`) | https://github.com/notofonts/latin-greek-cyrillic |
| Material icons (glyph outlines) | Apache License 2.0 | https://github.com/google/material-design-icons |
| Jellyfin logo ("banner-light") | CC BY-SA 4.0 (`rust/app/assets/LICENSE-jellyfin-logo.txt`) | https://github.com/jellyfin/jellyfin-ux |
| Rust crates (see `Cargo.lock`) | Mostly MIT / Apache-2.0, as declared by each crate | https://crates.io |

## Linux packages (.deb, .rpm, AppImage, tarball)

Built from source by `scripts/build-linux.sh` (exact versions and URLs in
`scripts/linux/build-in-container.sh`) and installed under
`/usr/lib/jellybridge/lib`. mpv is configured with `-Dgpl=false` and FFmpeg
without GPL or non-free parts, so these libraries are LGPL.

| Component | Version | Licence | Source |
| --- | --- | --- | --- |
| mpv (libmpv) | 0.41.0 | LGPL-2.1-or-later | https://github.com/mpv-player/mpv |
| FFmpeg | 8.1.3 | LGPL-2.1-or-later | https://ffmpeg.org |
| dav1d | 1.5.4 | BSD-2-Clause | https://code.videolan.org/videolan/dav1d |
| libass | 0.17.5 | ISC | https://github.com/libass/libass |
| libplacebo | 7.360.1 | LGPL-2.1-or-later | https://code.videolan.org/videolan/libplacebo |
| libdisplay-info | 0.2.0 | MIT | https://gitlab.freedesktop.org/emersion/libdisplay-info |
| libva, libva-drm | 2.24.1 | MIT | https://github.com/intel/libva |
| nv-codec-headers | n13.0.19.0 | MIT | https://github.com/FFmpeg/nv-codec-headers |

The Arch package uses the system's mpv instead and bundles none of these.

## Windows installer and portable zip

| Component | Licence | Source |
| --- | --- | --- |
| libmpv-2.dll (shinchiro build `20261014`, mpv git `ec38361c56`) | GPL-3.0-or-later (mpv and FFmpeg built with GPL parts) | build recipe: https://github.com/shinchiro/mpv-winbuild-cmake; mpv: https://github.com/mpv-player/mpv; FFmpeg: https://ffmpeg.org |
| vulkan-1.dll (Khronos Vulkan loader, LunarG runtime 1.4.363.0) | Apache License 2.0 (`VulkanRT-License.txt`, shipped beside it) | https://github.com/KhronosGroup/Vulkan-Loader |

The complete corresponding source for these binaries is available from the
links above; on request (support@oddbench.co) we provide it for the exact
versions shipped.
