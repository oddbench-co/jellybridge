#!/usr/bin/env bash
# Runs inside docker.io/library/ubuntu:22.04 (scripts/build-linux.sh).
# /src = the repository, /native = cache of sources and the built prefix.
set -euo pipefail

MPV=0.41.0
FFMPEG=8.1.3
DAV1D=1.5.4
LIBASS=0.17.5
LIBPLACEBO=7.360.1
NVHEADERS=n13.0.19.0
DISPLAYINFO=0.2.0
LIBVA=2.24.1
STAMP="mpv$MPV-ff$FFMPEG-dav1d$DAV1D-ass$LIBASS-pl$LIBPLACEBO-nv$NVHEADERS-di$DISPLAYINFO-va$LIBVA-r1"
PREFIX="/native/$STAMP/prefix"
SOURCES="/native/sources"
JOBS="$(nproc)"
export DEBIAN_FRONTEND=noninteractive

apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends \
    build-essential pkg-config curl ca-certificates xz-utils git nasm patchelf \
    python3-pip python3-jinja2 file imagemagick hwdata \
    libfreetype-dev libfribidi-dev libharfbuzz-dev libfontconfig-dev \
    libva-dev libdrm-dev libegl-dev libgl-dev liblcms2-dev zlib1g-dev \
    libpulse-dev libasound2-dev >/dev/null
# Ubuntu 22.04's meson is too old for mpv 0.41.
pip3 install -q --disable-pip-version-check 'meson==1.6.1' 'ninja==1.11.1.4' 2>/dev/null

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/lib/x86_64-linux-gnu/pkgconfig:$PREFIX/share/pkgconfig"
export CFLAGS="-O2 -fPIC" CXXFLAGS="-O2 -fPIC"

fetch() { # url dest-name
    mkdir -p "$SOURCES"
    [[ -s "$SOURCES/$2" ]] && return
    curl -fsSL --retry 3 -o "$SOURCES/$2.part" "$1" || { echo "download failed: $1" >&2; exit 1; }
    mv -f "$SOURCES/$2.part" "$SOURCES/$2"
}

# Static libraries carry their own dependencies in Libs.private; mpv links
# them into one shared libmpv, so promote those to plain Libs/Requires.
promote_private() {
    for pc in "$PREFIX"/lib/pkgconfig/*.pc "$PREFIX"/lib/x86_64-linux-gnu/pkgconfig/*.pc; do
        [[ -f "$pc" ]] || continue
        python3 - "$pc" <<'PY'
import sys, re
path = sys.argv[1]
lines = open(path).read().splitlines()
fields = {}
for i, l in enumerate(lines):
    m = re.match(r'^(Libs|Libs\.private|Requires|Requires\.private):(.*)$', l)
    if m: fields[m.group(1)] = (i, m.group(2).strip())
for base in ('Libs', 'Requires'):
    if base + '.private' in fields:
        pi, pv = fields[base + '.private']
        if base in fields:
            bi, bv = fields[base]
            joiner = ', ' if base == 'Requires' and bv and pv else ' '
            lines[bi] = f'{base}: {bv}{joiner}{pv}'.rstrip()
        else:
            lines.append(f'{base}: {pv}')
        lines[pi] = f'{base}.private:'
open(path, 'w').write('\n'.join(lines) + '\n')
PY
    done
}

# Each native step runs once per $STAMP: a finished step leaves a marker, a
# failed one prints the end of its log.
step() { # name, then the command
    local name="$1"; shift
    [[ -e "/native/$STAMP/done/$name" ]] && return
    echo "  $name"
    mkdir -p "/native/$STAMP/done" "/native/$STAMP/logs"
    if ! (cd /native/work && "$@") > "/native/$STAMP/logs/$name.log" 2>&1; then
        echo "step $name failed:" >&2; tail -40 "/native/$STAMP/logs/$name.log" >&2; exit 1
    fi
    touch "/native/$STAMP/done/$name"
}

nvheaders() {
    fetch "https://github.com/FFmpeg/nv-codec-headers/archive/refs/tags/$NVHEADERS.tar.gz" "nv-$NVHEADERS.tar.gz"
    rm -rf "nv-codec-headers-$NVHEADERS" && tar xf "$SOURCES/nv-$NVHEADERS.tar.gz"
    make -C "nv-codec-headers-$NVHEADERS" PREFIX="$PREFIX" install
}

dav1d() {
    fetch "https://code.videolan.org/videolan/dav1d/-/archive/$DAV1D/dav1d-$DAV1D.tar.gz" "dav1d-$DAV1D.tar.gz"
    rm -rf "dav1d-$DAV1D" && tar xf "$SOURCES/dav1d-$DAV1D.tar.gz"
    meson setup "dav1d-$DAV1D/build" "dav1d-$DAV1D" --prefix="$PREFIX" --libdir=lib --buildtype=release \
        --default-library=static -Denable_tools=false -Denable_tests=false
    ninja -C "dav1d-$DAV1D/build" install
}

ffmpeg() {
    fetch "https://ffmpeg.org/releases/ffmpeg-$FFMPEG.tar.xz" "ffmpeg-$FFMPEG.tar.xz"
    rm -rf "ffmpeg-$FFMPEG" && tar xf "$SOURCES/ffmpeg-$FFMPEG.tar.xz"
    cd "ffmpeg-$FFMPEG"
    # LGPL only: no --enable-gpl / --enable-nonfree. Decoders, demuxers,
    # parsers and filters stay complete; encoders and muxers are not used by a
    # player. Hardware decoding through VA-API and NVDEC, both loaded from the
    # host's drivers at run time.
    ./configure --prefix="$PREFIX" --enable-static --disable-shared --enable-pic \
        --disable-programs --disable-doc --disable-avdevice --disable-autodetect \
        --disable-encoders --disable-muxers --enable-encoder=mjpeg,png --enable-muxer=image2 \
        --enable-libdav1d --enable-zlib --enable-iconv --enable-vaapi --enable-libdrm \
        --enable-ffnvcodec --enable-nvdec --enable-cuvid \
        --extra-cflags="-I$PREFIX/include" --extra-ldflags="-L$PREFIX/lib"
    make -j"$JOBS" && make install
}

libass() {
    fetch "https://github.com/libass/libass/releases/download/$LIBASS/libass-$LIBASS.tar.xz" "libass-$LIBASS.tar.xz"
    rm -rf "libass-$LIBASS" && tar xf "$SOURCES/libass-$LIBASS.tar.xz"
    cd "libass-$LIBASS"
    ./configure --prefix="$PREFIX" --enable-static --disable-shared --with-pic
    make -j"$JOBS" && make install
}

# VA-API's loader is bundled (shared, next to libmpv) instead of taken from
# the host: some distributions keep it out of the default repositories (Ubuntu
# 22.04 ships it in universe, which its live session lacks). A newer libva
# loads older drivers, and the drivers themselves always come from the host,
# so the search path lists every distribution's driver folder.
libva() {
    fetch "https://github.com/intel/libva/archive/refs/tags/$LIBVA.tar.gz" "libva-$LIBVA.tar.gz"
    rm -rf "libva-$LIBVA" && tar xf "$SOURCES/libva-$LIBVA.tar.gz"
    meson setup "libva-$LIBVA/build" "libva-$LIBVA" --prefix="$PREFIX" --libdir=lib --buildtype=release \
        -Ddriverdir=/usr/lib/x86_64-linux-gnu/dri:/usr/lib64/dri:/usr/lib/dri:/usr/lib64/dri-freeworld:/usr/lib64/dri-nonfree:/usr/local/lib/dri \
        -Dwith_x11=no -Dwith_glx=no -Dwith_wayland=no -Denable_docs=false
    ninja -C "libva-$LIBVA/build" install
}

# mpv's DRM support (needed for VA-API through the render node) requires it.
displayinfo() {
    fetch "https://gitlab.freedesktop.org/emersion/libdisplay-info/-/archive/$DISPLAYINFO/libdisplay-info-$DISPLAYINFO.tar.gz" \
        "libdisplay-info-$DISPLAYINFO.tar.gz"
    rm -rf "libdisplay-info-$DISPLAYINFO" && tar xf "$SOURCES/libdisplay-info-$DISPLAYINFO.tar.gz"
    meson setup "libdisplay-info-$DISPLAYINFO/build" "libdisplay-info-$DISPLAYINFO" --prefix="$PREFIX" --libdir=lib \
        --buildtype=release --default-library=static
    ninja -C "libdisplay-info-$DISPLAYINFO/build" install
}

libplacebo() {
    [[ -d "$SOURCES/libplacebo-$LIBPLACEBO" ]] || git clone -q -c advice.detachedHead=false --depth 1 \
        --branch "v$LIBPLACEBO" --recurse-submodules --shallow-submodules \
        https://code.videolan.org/videolan/libplacebo.git "$SOURCES/libplacebo-$LIBPLACEBO"
    rm -rf libplacebo-build
    meson setup libplacebo-build "$SOURCES/libplacebo-$LIBPLACEBO" --prefix="$PREFIX" --libdir=lib --buildtype=release \
        --default-library=static -Dvulkan=disabled -Dopengl=enabled -Dshaderc=disabled -Dglslang=disabled \
        -Dlcms=enabled -Ddemos=false -Dtests=false
    ninja -C libplacebo-build install
}

mpv() {
    promote_private
    fetch "https://github.com/mpv-player/mpv/archive/refs/tags/v$MPV.tar.gz" "mpv-$MPV.tar.gz"
    rm -rf "mpv-$MPV" mpv-build && tar xf "$SOURCES/mpv-$MPV.tar.gz"
    # Every optional feature off, then only what an embedded player needs:
    # the OpenGL render API with EGL (VA-API dmabuf import), VA-API through
    # the DRM render node (works under X11 and Wayland alike), CUDA/NVDEC,
    # PulseAudio (PipeWire serves it) and ALSA, and ICC profiles.
    meson setup mpv-build "mpv-$MPV" --prefix="$PREFIX" --libdir=lib --buildtype=release \
        --auto-features=disabled -Dlibmpv=true -Dcplayer=false -Dgpl=false -Dtests=false \
        -Dgl=enabled -Dplain-gl=enabled -Degl=enabled -Dvaapi=enabled -Dvaapi-drm=enabled -Ddrm=enabled \
        -Dcuda-hwaccel=enabled -Dcuda-interop=enabled -Dpulse=enabled -Dalsa=enabled \
        -Dlcms2=enabled -Diconv=enabled -Dzlib=enabled \
        -Dc_link_args="-Wl,--exclude-libs,ALL -Wl,-Bsymbolic"
    ninja -C mpv-build install
}

build_native() {
    mkdir -p /native/work "$PREFIX"
    step nvheaders nvheaders
    step libva libva
    step dav1d dav1d
    step ffmpeg ffmpeg
    step libass libass
    step libplacebo libplacebo
    step displayinfo displayinfo
    step mpv mpv
}

echo "native libraries ($STAMP): finished steps are reused"
build_native

# The app, with the toolchain pinned in rust-toolchain.toml.
export PATH="/root/.cargo/bin:$PATH" RUSTUP_HOME=/root/.rustup CARGO_HOME=/root/.cargo
command -v rustup >/dev/null || curl -fsSL https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain none >/dev/null
cd /src
CARGO_TARGET_DIR=/src/target/linux/cargo cargo build --release --locked -p jellybridge-app 2>&1 \
    | grep -E '^(error|warning)|-->|Finished' || true
BIN=/src/target/linux/cargo/release/jellybridge
[[ -x "$BIN" ]]

BUNDLE=/src/target/linux/bundle
rm -rf "$BUNDLE" && mkdir -p "$BUNDLE/usr/bin" "$BUNDLE/usr/lib/jellybridge/lib"
install -m755 "$BIN" "$BUNDLE/usr/bin/jellybridge"
install -m644 "$(readlink -f "$PREFIX/lib/libmpv.so.2")" "$BUNDLE/usr/lib/jellybridge/lib/libmpv.so.2"
for lib in libva.so.2 libva-drm.so.2; do
    install -m644 "$(readlink -f "$PREFIX/lib/$lib")" "$BUNDLE/usr/lib/jellybridge/lib/$lib"
done
for lib in "$BUNDLE"/usr/lib/jellybridge/lib/*.so.*; do
    strip --strip-unneeded "$lib"
    # Each bundled library finds its bundled siblings first.
    patchelf --set-rpath '$ORIGIN' "$lib"
    chmod 755 "$lib"
done
LINUX=/src/packaging/linux
install -Dm644 "$LINUX/co.oddbench.JellyBridge.desktop" "$BUNDLE/usr/share/applications/co.oddbench.JellyBridge.desktop"
install -Dm644 "$LINUX/co.oddbench.JellyBridge.metainfo.xml" "$BUNDLE/usr/share/metainfo/co.oddbench.JellyBridge.metainfo.xml"
for size in 32 48 64 128 256 512; do
    mkdir -p "$BUNDLE/usr/share/icons/hicolor/${size}x${size}/apps"
    convert /src/rust/app/assets/icon.png -resize "${size}x${size}" \
        "$BUNDLE/usr/share/icons/hicolor/${size}x${size}/apps/co.oddbench.JellyBridge.png"
done
install -Dm644 /src/LICENSE "$BUNDLE/usr/share/doc/jellybridge/LICENSE"
install -Dm644 /src/THIRD-PARTY-NOTICES.md "$BUNDLE/usr/share/doc/jellybridge/THIRD-PARTY-NOTICES.md"
printf 'mpv %s (LGPL, -Dgpl=false)\nFFmpeg %s (LGPL)\ndav1d %s\nlibass %s\nlibplacebo %s\nnv-codec-headers %s\n' \
    "$MPV" "$FFMPEG" "$DAV1D" "$LIBASS" "$LIBPLACEBO" "$NVHEADERS" > "$BUNDLE/usr/share/doc/jellybridge/native-libraries.txt"

# Evidence the policy asks for: what the bundle needs from the host, and the
# newest glibc symbol it uses.
REPORT=/src/target/linux/bundle-report
rm -rf "$REPORT" && mkdir -p "$REPORT"
for f in "$BUNDLE/usr/bin/jellybridge" "$BUNDLE"/usr/lib/jellybridge/lib/*.so.*; do
    name="$(basename "$f")"
    readelf -d "$f" > "$REPORT/$name.dynamic.txt"
    readelf --version-info "$f" > "$REPORT/$name.versions.txt"
    readelf -d "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' > "$REPORT/$name.needed.txt"
done
objdump -T "$BUNDLE/usr/bin/jellybridge" "$BUNDLE"/usr/lib/jellybridge/lib/*.so.* \
    | grep -oE 'GLIBC_[0-9.]+' | sort -Vu | tail -1 > "$REPORT/glibc-floor.txt"
echo "glibc floor: $(cat "$REPORT/glibc-floor.txt")"
echo "libmpv needs: $(tr '\n' ' ' < "$REPORT/libmpv.so.2.needed.txt")"
