# Platform matrix

Where JellyBridge is known to install and play, how each platform gets it, and
how the results were produced. Last full run: 2026-10-15, version 0.1.0.

## Results

Every row installed the package the way a user would, launched the app,
opened Home, a series page and search, played Bookish S1:E1 (4K HEVC, direct
play) for 25 seconds, then removed the package and checked nothing was left
outside the user's config and cache.

| System | Desktop / session | glibc | Package | Result |
| --- | --- | --- | --- | --- |
| Ubuntu 22.04.5 LTS | GNOME, X11 | 2.35 | .deb | pass |
| Ubuntu 24.04.5 LTS | GNOME | 2.39 | .deb | pass (menu entry and dock icon checked) |
| Ubuntu 26.04.1 LTS | GNOME, Wayland | 2.43 | .deb | pass |
| Debian 12 | GNOME, Wayland | 2.36 | .deb | pass |
| Debian 13 | GNOME, Wayland | 2.41 | .deb | pass |
| Linux Mint 22.3 | Cinnamon, X11 | 2.39 | .deb | pass |
| Pop!_OS 24.04 LTS | COSMIC, Wayland | 2.39 | .deb | pass |
| elementary OS 8.1 | Pantheon, X11 | 2.39 | .deb | pass |
| Fedora 44 Workstation | GNOME, Wayland | 2.43 | .rpm | pass |
| Fedora 44 KDE | Plasma, Wayland | 2.43 | .rpm | pass |
| openSUSE Tumbleweed | Plasma, Wayland | 2.44 | .rpm | pass |
| Manjaro KDE 26.1 | Plasma, Wayland | 2.44 | .pkg.tar.zst, AppImage, tarball | pass |
| EndeavourOS (2026.08) | Plasma, X11 | 2.44 | .pkg.tar.zst, AppImage | pass |
| Windows 10 Pro 22H2 | — | — | setup.exe | pass |
| Windows 11 25H2 | — | — | setup.exe | pass |

Not covered: openSUSE Leap 16 (no live image; the Tumbleweed result uses the
same .rpm), ARM64, and hardware decoding (VMs have no video decoder: every run
decoded in software, which is what a machine without VA-API/NVDEC gets).

## What each platform gets, and why

- **Linux, everything except Arch**: one portable bundle built on Ubuntu 22.04
  (glibc 2.35 floor) by `scripts/build-linux.sh`. It carries libmpv 0.41 built
  LGPL with FFmpeg, dav1d, libass and libplacebo linked in, plus libva and
  libva-drm. Distribution libmpv is not used: Ubuntu 22.04 only has the old
  soname, and Fedora/openSUSE ship FFmpeg without H.264/HEVC. libva is bundled
  because Ubuntu 22.04 keeps it in universe (absent from its live session);
  its driver search path covers every distribution's driver folder, and a
  newer libva loads older drivers. GL/EGL, VA-API drivers, Wayland/X11, audio
  and fonts always come from the host. `scripts/package-linux.sh` wraps the
  bundle as .deb, .rpm (dependencies from rpmbuild's soname scan, so one file
  serves Fedora and openSUSE), AppImage (static runtime, no libfuse2) and a
  tarball with `install.sh`.
- **Arch, Manjaro, EndeavourOS**: `packaging/arch/PKGBUILD`, a native package
  that depends on Arch's own `mpv` (current soname, full FFmpeg) instead of
  bundling it. Ready for the AUR.
- **Windows 10/11**: `scripts/build-windows.sh` cross-builds the exe and a
  per-user NSIS installer (`%LOCALAPPDATA%\Programs\JellyBridge`, Start menu
  entry, Apps & Features uninstall) with `libmpv-2.dll` and the Khronos Vulkan
  loader (`vulkan-1.dll`) beside it. That libmpv imports the Vulkan loader,
  which only GPU drivers install: without it Windows 10 could not load libmpv
  at all.

## Problems found and fixed by this run

| Found on | Problem | Fix |
| --- | --- | --- |
| all but Arch | binary needed glibc 2.43 (built on Arch) | container build on Ubuntu 22.04 |
| GNOME (Ubuntu, Fedora, Debian, elementary) | no title bar, so no close/move/resize | winit's Adwaita client-side decorations enabled |
| bundled libmpv | `osc` and the `load-*` options exist only with Lua: player failed to start | those options are skipped when absent |
| Ubuntu 22.04 | .deb could not install: libva in universe | libva bundled, dependency dropped |
| small screens (1366x768, VMs) | 1280x800 window ran off-screen | window is maximized when it does not fit |
| Windows 10 | libmpv-2.dll would not load (no vulkan-1.dll) | Vulkan loader shipped |
| Windows without a GL driver | silent exit | dialog naming the cause and the fix |
| menu launches | startup errors went to an invisible stderr | native dialog (MessageBox / zenity / kdialog) |
| player errors | raw loader text, "missing" for a present DLL | messages name the missing piece and what to do |

Startup is unchanged by the decoration change (time to interactive on the
dev machine, 6 launches each: 93–108 ms before, 93–106 ms after).

## Rerunning the tests

All in `scripts/vm-test/`; VMs are headless (nothing appears on screen).

```bash
scripts/build-linux.sh && scripts/package-linux.sh
scripts/vm-test/download-isos.sh            # live ISOs from matrix.tsv (~60 GB)
export JB_TEST_SERIES_ID=<series id> JB_TEST_EPISODE_ID=<episode id> JB_TEST_SEARCH=<title>
scripts/vm-test/prepare.sh                  # stage packages + test session, start the payload server
scripts/vm-test/vm.py start ubuntu-2404 ~/.local/share/jellybridge-vmtest/iso/ubuntu-2404.iso
scripts/vm-test/vm.py shot ubuntu-2404 /tmp/s.png   # look, then open a terminal in the guest:
scripts/vm-test/vm.py key ubuntu-2404 ctrl-alt-t
scripts/vm-test/run-guest.sh ubuntu-2404 [package|appimage|tar]
scripts/vm-test/check-shots.py ~/.local/share/jellybridge-vmtest/www/results/*
scripts/vm-test/vm.py stop ubuntu-2404
```

Live-session notes: Fedora, Debian (password `live`), Manjaro (`manjaro`),
openSUSE, Pop!_OS and elementary need `sudo -v` typed in the guest terminal
first; click into the terminal and wait a moment before typing. The guest
script mutes playback and disables playback reporting, and only plays the
items cleared for testing.

Windows runs use dockurr/windows containers (`jb-windows` = Windows 11, ssh on
127.0.0.1:2222; `jb-windows10`, ssh on 2223). The VMs have no OpenGL 2 driver,
so functional runs copy Mesa's `opengl32.dll`/`libgallium_wgl.dll` beside the
installed exe (VM only, never shipped); GUI launches go through a scheduled
task with an interactive principal.
