# Packaging

Release packaging is fail-closed. The local Homebrew/system mpv graph may run
a development binary, but it cannot enter a public package.

## macOS package

`scripts/package-macos.sh` builds `JellyBridge.app` and a .dmg for the Mac's
own CPU (arm64 or x86_64):

```bash
brew install mpv dylibbundler
scripts/package-macos.sh
```

It embeds Homebrew's complete mpv dependency graph in
`Contents/Frameworks`, rewrites every reference to it, and fails if a library
is missing or still points at the build machine. Homebrew builds its
libraries for the macOS they are installed on, so that version is the app's
minimum (`LSMinimumSystemVersion`). With `MACOS_SIGNING_IDENTITY` it signs
with the hardened runtime (entitlements in `packaging/macos/`), and with the
`APPLE_API_*` variables it notarizes and staples the .dmg; without them the
app is signed ad hoc for local use. The graph includes GPL FFmpeg parts,
allowed by JellyBridge's GPL-3.0-or-later licence.

Releases are built by `.github/workflows/release.yml`; see
`docs/RELEASING.md`.

## Linux and Windows packages

`scripts/build-linux.sh` builds the portable Linux bundle (the layout below,
under `target/linux/bundle/usr`) and `scripts/package-linux.sh` wraps it as
.deb, .rpm, AppImage and tarball, plus a native Arch package that uses the
system mpv. `scripts/build-windows.sh` makes the Windows installer and
portable zip. Results and the VM test procedure: `platform-matrix.md`. The
Windows libmpv is a GPL third-party build, allowed by JellyBridge's
GPL-3.0-or-later licence; its sources are listed in `THIRD-PARTY-NOTICES.md`.

## Native bundle contract (public release, not yet exercised)

The audited-release workflow was proven during the feasibility gate with
slice-specific scripts that have since been deleted (see git history:
`scripts/package-rust-slice.sh`, `scripts/prepare-slice-package.sh`,
`scripts/audit-slice-package.sh`). The contract below still applies to any
future public-release packaging of `rust/app`.

Set `JELLYBRIDGE_RELEASE_NATIVE_BUNDLE` to a directory with this layout:

```text
bundle/
  SBOM.spdx.json
  build-manifest.json
  corresponding-source.txt
  lib/
    libmpv...
    other LGPL-compatible native dependencies...
    pkgconfig/mpv.pc
  share/pkgconfig/                 # optional
```

`build-manifest.json` must attest `"releaseEligible": true` and
`"mpvVersion": "0.41.0"`. It must also record every source revision/hash,
patch, configure/Meson command, compiler/linker version, license conclusion,
and generated build-log location. `corresponding-source.txt` identifies the
corresponding-source archive or compliant written offer.

On macOS, every bundled dylib must use `@rpath` install names and refer only to
other bundled `@rpath` libraries or Apple system paths. On Linux, each ELF
library must have a portable SONAME and no build-machine RPATH/RUNPATH. Host
OpenGL/EGL/Vulkan/VA-API/VDPAU loaders and drivers are prohibited. The
application binary links with rpaths pointing inside the package
(`@executable_path/../Resources/lib` on macOS,
`$ORIGIN/../lib/jellybridge/lib` on Linux — see `rust/app/build.rs`).

Signing identity, hardened runtime, notarization, stapling, quarantine launch,
and clean-machine evidence remain mandatory for any public release.
