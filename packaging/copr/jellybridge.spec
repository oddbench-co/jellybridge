# The Fedora COPR package (copr.fedorainfracloud.org/coprs/oddbench/jellybridge),
# built from source in COPR's own chroot, against Fedora's mpv. Unlike the
# release .rpm it does not bundle libmpv, so codecs come from the system:
# Fedora's mpv uses the ffmpeg-free build, and H.264/HEVC need RPM Fusion's
# ffmpeg (sudo dnf swap ffmpeg-free ffmpeg --allowerasing).
#
# COPR has no network inside the build, so the crates come vendored from the
# release (scripts/vendor-crates.sh publishes Source1 with every release).
# scripts/copr.sh builds the SRPM and submits it.
Name:           jellybridge
Version:        0.1.0
Release:        1%{?dist}
Summary:        Native desktop client for Jellyfin
License:        GPL-3.0-or-later
URL:            https://jellybridge.oddbench.co
Source0:        https://github.com/oddbench-co/jellybridge/archive/refs/tags/v%{version}.tar.gz#/jellybridge-%{version}.tar.gz
Source1:        https://github.com/oddbench-co/jellybridge/releases/download/v%{version}/jellybridge-%{version}-vendor.tar.xz
ExclusiveArch:  x86_64

BuildRequires:  cargo >= 1.98
BuildRequires:  rust >= 1.98
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  cmake
BuildRequires:  ImageMagick
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# libmpv and the windowing libraries are loaded with dlopen, so the automatic
# soname scan cannot see them.
Requires:       mpv-libs >= 0.35
Requires:       libGL.so.1()(64bit)
Requires:       libEGL.so.1()(64bit)
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libXrandr.so.2()(64bit)
Requires:       hicolor-icon-theme
Recommends:     libva
Recommends:     mesa-va-drivers

%description
JellyBridge browses and plays your Jellyfin library in a fast native window:
home, libraries, search, detail pages and a full player with subtitles and
hardware-accelerated decoding.

This package plays through the system's mpv. Fedora's own ffmpeg does not
decode H.264 or HEVC; for those, replace it with RPM Fusion's:
sudo dnf swap ffmpeg-free ffmpeg --allowerasing

%prep
%autosetup -n jellybridge-%{version}
tar xf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<'CARGO'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CARGO

%build
export CARGO_TARGET_DIR=target
cargo build --release --frozen --offline -p jellybridge-app

%install
install -Dm755 target/release/jellybridge %{buildroot}%{_bindir}/jellybridge
install -Dm644 packaging/linux/co.oddbench.JellyBridge.desktop %{buildroot}%{_datadir}/applications/co.oddbench.JellyBridge.desktop
install -Dm644 packaging/linux/co.oddbench.JellyBridge.metainfo.xml %{buildroot}%{_metainfodir}/co.oddbench.JellyBridge.metainfo.xml
install -Dm644 rust/app/assets/icon.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/co.oddbench.JellyBridge.svg
for size in 32 48 64 128 256 512; do
    install -dm755 %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps
    magick rust/app/assets/icon.png -resize ${size}x${size} \
        %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/co.oddbench.JellyBridge.png
done
install -Dm644 THIRD-PARTY-NOTICES.md %{buildroot}%{_docdir}/jellybridge/THIRD-PARTY-NOTICES.md

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/co.oddbench.JellyBridge.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/co.oddbench.JellyBridge.metainfo.xml

%files
%license LICENSE
%doc README.md
%{_docdir}/jellybridge/THIRD-PARTY-NOTICES.md
%{_bindir}/jellybridge
%{_datadir}/applications/co.oddbench.JellyBridge.desktop
%{_metainfodir}/co.oddbench.JellyBridge.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/co.oddbench.JellyBridge.*

%changelog
* Thu Oct 15 2026 Oddbench <support@oddbench.co> - 0.1.0-1
- First release
