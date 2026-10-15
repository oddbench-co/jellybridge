# Packaging

Release packaging is fail-closed. The local Homebrew/system mpv graph may run
a development binary, but it cannot enter a public package.

## Personal full-codec macOS package

The repository provides an explicitly personal-use workflow for macOS arm64:

```bash
brew install mpv dylibbundler
scripts/package-rust-app-personal.sh
```

It embeds the complete installed Homebrew mpv dependency graph, rewrites every
dependency to the app's `Contents/Frameworks` directory, applies an ad-hoc
signature, audits for missing or build-machine-linked libraries, and creates a
self-contained DMG. This graph includes GPL-enabled FFmpeg components and is
not an audited public-release artifact.

## Linux and Windows packages

`scripts/build-linux.sh` builds the portable Linux bundle (the layout below,
under `target/linux/bundle/usr`) and `scripts/package-linux.sh` wraps it as
.deb, .rpm, AppImage and tarball, plus a native Arch package that uses the
system mpv. `scripts/build-windows.sh` makes the Windows installer and
portable zip. Results and the VM test procedure: `platform-matrix.md`. The
Windows libmpv is still a GPL third-party build (personal use only).

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
