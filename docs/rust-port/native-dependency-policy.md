# Native Dependency and Packaging Policy

The installed Homebrew mpv 0.41.0 bottle is allowed only for development. Its
dependency graph is not the release graph and is not evidence of LGPL compliance.

Release libmpv must be built dynamically from mpv 0.41.0 with `-Dgpl=false`.
FFmpeg must disable GPL and nonfree code. The build must preserve libass and
libplacebo while omitting host graphics drivers from the package.

## Required release evidence

- Source archive/revision and SHA-256 for every native library.
- Applied patches and complete Meson/configure command lines.
- Compiler and linker versions and generated build logs.
- Machine-readable SBOM and automated license allowlist.
- Corresponding-source archive or compliant source offer.
- `otool`, `ldd`, `lddtree`, `readelf -d`, and `readelf --version-info` reports.
- Automated rejection of GPL-only dependencies and build-machine paths.

macOS dependencies use application-relative install names, are rewritten before
signing, and use `MACOSX_DEPLOYMENT_TARGET=13.0`. Linux dependencies are built
inside Ubuntu 22.04 and use `$ORIGIN`-relative lookup paths. Mesa, libGL/EGL,
Vulkan ICDs, VA-API/VDPAU drivers, glibc, and the dynamic loader are never bundled.

The VA-API *loader* (libva, libva-drm) is the one exception to the rule
above: it is bundled because Ubuntu 22.04 keeps it outside its default
repositories. It searches every distribution's driver folder and loads the
host's drivers (`scripts/linux/build-in-container.sh`, `platform-matrix.md`).

Fontconfig and installed fonts are used on Linux; CoreText is used on macOS.
ASS/SSA embedded font attachments must remain available in both builds.
