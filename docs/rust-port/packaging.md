# Vertical-Slice Packaging

Release packaging is fail-closed. The local Homebrew/system mpv graph may run
the feasibility binary, but it cannot enter a package.

## Personal full-codec macOS package

The repository also provides a separate, explicitly personal-use workflow for
macOS arm64:

```bash
brew install mpv dylibbundler
scripts/package-rust-app-personal.sh
```

It embeds the complete installed Homebrew mpv dependency graph, rewrites every
dependency to the app's `Contents/Frameworks` directory, applies an ad-hoc
signature, audits for missing or build-machine-linked libraries, and creates a
self-contained DMG. This graph includes GPL-enabled FFmpeg components and is
not the audited public-release path described below.

## Native bundle contract

Set `JELLYBRIDGE_RELEASE_NATIVE_BUNDLE` to an absolute or relative directory
with this layout:

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
OpenGL/EGL/Vulkan/VA-API/VDPAU loaders and drivers are prohibited.

## Commands

Install exactly cargo-packager 0.11.8, then run one format at a time:

```bash
JELLYBRIDGE_RELEASE_NATIVE_BUNDLE=/path/to/bundle \
  scripts/package-rust-slice.sh dmg

JELLYBRIDGE_RELEASE_NATIVE_BUNDLE=/path/to/bundle \
  scripts/package-rust-slice.sh deb

JELLYBRIDGE_RELEASE_NATIVE_BUNDLE=/path/to/bundle \
  scripts/package-rust-slice.sh appimage
```

If cargo-packager is not on `PATH`, set `CARGO_PACKAGER_BIN` to its executable.
The wrapper fixes the Rust target, macOS deployment target, pkg-config path,
member manifest, profile, and package format. It refuses a release binary that
still names a Homebrew or build-machine dependency.

Audit the `.app`, extracted DEB tree, and extracted AppImage/AppDir separately,
using a fresh report directory for every artifact:

```bash
scripts/audit-slice-package.sh /path/to/JellyBridge.app target/audit/macos-app
scripts/audit-slice-package.sh /path/to/deb-root target/audit/linux-deb
scripts/audit-slice-package.sh /path/to/AppDir target/audit/linux-appimage
```

The audit writes persistent hashes, `otool`/`readelf`, RPATH, ABI, `ldd`,
optional `lddtree`, and codesign reports. Hash the outer DMG/DEB/AppImage as an
additional release artifact. Signing identity, hardened runtime, notarization,
stapling, quarantine launch, and clean-machine evidence remain mandatory even
when cargo-packager creates the package successfully.
